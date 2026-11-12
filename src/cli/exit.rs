use crate::ui::board::Verdict;

/// what `coeus` exits with, the contract CI pipelines rely on
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// every job succeeded, was up to date, or was rightly superseded
    Success,
    /// a job failed, was cancelled, interrupted or rolled back. it outranks
    /// [Exit::Unreached]: a broken deployment matters more than a machine
    /// that was not reached.
    Failed,
    /// a machine could not be reached or refused the order
    Unreached,
    /// the user detached with ctrl-c; the jobs keep running
    Detached,
}

impl Exit {
    pub fn of(verdicts: impl IntoIterator<Item = Verdict>, skip_unreachable: bool) -> Self {
        let mut exit = Self::Success;
        for verdict in verdicts {
            match verdict {
                Verdict::Failed | Verdict::Held => return Self::Failed,
                Verdict::Rejected => exit = Self::Unreached,
                Verdict::Unreachable if !skip_unreachable => exit = Self::Unreached,
                Verdict::Unreachable | Verdict::Done => {}
            }
        }
        exit
    }

    pub const fn code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::Failed => 1,
            Self::Unreached => 2,
            Self::Detached => 130,
        }
    }
}
