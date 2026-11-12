//! a bootstrap's switch to one snapshot, which may take more than one try.
//! shared by the CLI, reading coeusd's history, and `coeusd adopt`, reading
//! the job store, so both sides decide alike from the same jobs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{JobState, JobSummary, NarHash, OrderId, Outcome};

/// tries of one snapshot a bootstrap orders at most
pub const ATTEMPTS: u8 = 8;

/// the order id of try `attempt` (from 0) of switching to `nar_hash`. the
/// same snapshot always names the same orders, so a run finds the job an
/// earlier run ordered, through ssh or through coeusd, and a machine answers
/// a repeated order with the job it already made for it.
pub fn bootstrap_order(nar_hash: &NarHash, attempt: u8) -> OrderId {
    let digest = Sha256::new()
        .chain_update(b"coeus bootstrap\0")
        .chain_update(nar_hash.as_str())
        .chain_update([0, attempt])
        .finalize();
    let (head, _) = digest
        .split_first_chunk::<8>()
        .expect("a sha256 has 32 bytes");
    OrderId(u64::from_be_bytes(*head))
}

/// where a snapshot's switch stands on a machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attempt {
    /// nothing was ordered for this snapshot: order the first try
    Fresh(OrderId),
    /// its newest try is queued or running
    Running(JobSummary),
    /// its newest try put the machine on the snapshot
    Succeeded(JobSummary),
    /// its newest try ended any other way. `next` orders another try, which
    /// only `--retry` does; `None` once [ATTEMPTS] tries were made
    Failed {
        last: JobSummary,
        next: Option<OrderId>,
    },
}

impl Attempt {
    /// from a machine's jobs, in any order; jobs of other orders are ignored
    pub fn of<'a>(jobs: impl IntoIterator<Item = &'a JobSummary>, nar_hash: &NarHash) -> Self {
        let orders: Vec<OrderId> = (0..ATTEMPTS)
            .map(|attempt| bootstrap_order(nar_hash, attempt))
            .collect();

        let newest = jobs
            .into_iter()
            .filter_map(|job| {
                let attempt = orders.iter().position(|order| *order == job.order)?;
                Some((attempt, job))
            })
            .max_by_key(|(attempt, job)| (*attempt, job.id));

        let Some((attempt, job)) = newest else {
            return Self::Fresh(orders[0]);
        };

        match &job.state {
            JobState::Queued | JobState::Running { .. } => Self::Running(job.clone()),
            // superseded is a success for a deploy, but this machine then
            // runs something other than the snapshot
            JobState::Finished(Outcome::Succeeded | Outcome::UpToDate) => {
                Self::Succeeded(job.clone())
            }
            JobState::Finished(_) => Self::Failed {
                last: job.clone(),
                next: orders.get(attempt + 1).copied(),
            },
        }
    }
}
