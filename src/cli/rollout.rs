use std::num::NonZeroUsize;

use eyre::{Result, bail};

use crate::{
    common::model::Hostname,
    ui::{Board, board::Verdict},
};

/// the order in which an order reaches the fleet: canaries first, then
/// batches, each wave once the previous one settled, and none once more
/// machines failed than the budget allows
#[derive(Clone, Debug)]
pub struct Rollout {
    waves: Vec<Vec<Hostname>>,
    /// how many waves were ordered so far
    ordered: usize,
    max_failures: usize,
    skip_unreachable: bool,
}

/// what the rollout does next
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// order these machines, the wave numbered from one
    Order {
        wave: usize,
        hosts: Vec<Hostname>,
    },
    /// the current wave is still running
    Wait,
    /// too many failures: these machines are never ordered
    Stop(Vec<Hostname>),
    Done,
}

impl Rollout {
    pub fn plan(
        hosts: &[Hostname],
        canaries: &[Hostname],
        batch: Option<NonZeroUsize>,
        max_failures: usize,
        skip_unreachable: bool,
    ) -> Result<Self> {
        if let Some(stray) = canaries.iter().find(|canary| !hosts.contains(canary)) {
            bail!("the canary {stray} is not among the machines to deploy");
        }

        let rest: Vec<Hostname> = hosts
            .iter()
            .filter(|host| !canaries.contains(host))
            .cloned()
            .collect();
        let size = batch.map_or(rest.len().max(1), NonZeroUsize::get);

        let waves = std::iter::once(canaries.to_vec())
            .filter(|canaries| !canaries.is_empty())
            .chain(rest.chunks(size).map(<[Hostname]>::to_vec))
            .collect();

        Ok(Self {
            waves,
            ordered: 0,
            max_failures,
            skip_unreachable,
        })
    }

    pub fn waves(&self) -> &[Vec<Hostname>] {
        &self.waves
    }

    pub fn advance(&mut self, board: &Board) -> Step {
        if self.ordered > 0 && !self.current_settled(board) {
            return Step::Wait;
        }
        if self.ordered == self.waves.len() {
            return Step::Done;
        }

        if self.failures(board) > self.max_failures {
            let held = self.waves[self.ordered..].concat();
            self.ordered = self.waves.len();
            return Step::Stop(held);
        }

        self.ordered += 1;
        Step::Order {
            wave: self.ordered,
            hosts: self.waves[self.ordered - 1].clone(),
        }
    }

    fn current_settled(&self, board: &Board) -> bool {
        self.waves[self.ordered - 1]
            .iter()
            .all(|host| board.row(host).is_some_and(|row| row.verdict().is_some()))
    }

    fn failures(&self, board: &Board) -> usize {
        board
            .verdicts()
            .filter(|verdict| match verdict {
                Verdict::Failed | Verdict::Rejected => true,
                Verdict::Unreachable => !self.skip_unreachable,
                Verdict::Done | Verdict::Held => false,
            })
            .count()
    }
}
