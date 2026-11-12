use std::{collections::VecDeque, time::Duration};

use crate::common::model::{JobState, JobSummary, Outcome, Phase, Source, Timestamp};

/// how many failed tries of one commit catching up makes before it waits for
/// the ref to move
const LIMIT: u32 = 3;
/// the wait after the first failed try, doubling after each one
const BACKOFF: Duration = Duration::from_secs(60);

/// whether catching up may order `source` at `now`, judged by this machine's
/// own tries of it in `recent`, newest first.
///
/// a try that failed before building most likely met a network that was not
/// up yet after a boot or a wake, and costs little to repeat, so the commit
/// is tried again after a while, a few times. one that failed later, that
/// someone cancelled, or whose activation was cut short waits for the ref to
/// move.
pub(super) fn may_try(recent: &VecDeque<JobSummary>, source: &Source, now: Timestamp) -> bool {
    let Source::Git { rev, .. } = source else {
        return true;
    };

    let mut failures = 0;
    let mut last = None;
    for job in recent
        .iter()
        .filter(|job| matches!(&job.source, Source::Git { rev: tried, .. } if tried == rev))
    {
        let JobState::Finished(outcome) = &job.state else {
            continue;
        };
        if outcome.is_success() {
            break;
        }
        if !passing(outcome) {
            return false;
        }

        failures += 1;
        last.get_or_insert(job.finished_at.unwrap_or(job.ordered_at));
    }

    match last {
        None => true,
        Some(_) if failures >= LIMIT => false,
        Some(last) => now >= last.after(BACKOFF.saturating_mul(1 << (failures - 1))),
    }
}

/// a failure before building, where the network is the usual cause
fn passing(outcome: &Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Failed(failure)
            if matches!(failure.phase, Phase::Preparing | Phase::Fetching | Phase::Evaluating)
    )
}
