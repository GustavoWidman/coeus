use crate::common::model::{Failure, JobState, JobSummary, JobView, MachineView, Outcome};

/// bounds what goes on the wire so every packet fits one encrypted frame:
/// failure messages and tails are clipped, the full text stays in the job log
pub(super) trait Fit {
    fn fit(self) -> Self;
}

impl Fit for MachineView {
    fn fit(mut self) -> Self {
        self.running = self.running.map(Fit::fit);
        self.next = self.next.map(Fit::fit);
        self.recent = self.recent.into_iter().map(Fit::fit).collect();
        self
    }
}

impl Fit for JobView {
    fn fit(mut self) -> Self {
        self.summary = self.summary.fit();
        self
    }
}

impl Fit for JobSummary {
    fn fit(mut self) -> Self {
        if let JobState::Finished(Outcome::Failed(failure) | Outcome::RolledBack(failure)) =
            &mut self.state
        {
            failure.clip();
        }
        self
    }
}

impl Failure {
    const MESSAGE: usize = 512;
    const LINES: usize = 10;
    const LINE: usize = 240;

    fn clip(&mut self) {
        clip(&mut self.message, Self::MESSAGE);
        let skip = self.tail.len().saturating_sub(Self::LINES);
        self.tail.drain(..skip);
        for line in &mut self.tail {
            clip(line, Self::LINE);
        }
    }
}

fn clip(text: &mut String, limit: usize) {
    if text.len() > limit {
        let end = text.floor_char_boundary(limit);
        text.truncate(end);
        text.push('…');
    }
}
