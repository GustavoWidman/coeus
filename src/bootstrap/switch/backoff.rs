//! how long to wait before asking a machine again after it failed to answer

use std::time::Duration;

/// a wait that doubles after each failure, up to a ceiling
pub struct Backoff {
    first: Duration,
    ceiling: Duration,
    next: Duration,
}

impl Backoff {
    pub const fn new(first: Duration, ceiling: Duration) -> Self {
        Self {
            first,
            ceiling,
            next: first,
        }
    }

    /// the wait before the next try; the one after is twice as long
    pub fn next(&mut self) -> Duration {
        let wait = self.next;
        self.next = (wait * 2).min(self.ceiling);
        wait
    }

    /// back to the first wait, once a try went through
    pub const fn reset(&mut self) {
        self.next = self.first;
    }
}
