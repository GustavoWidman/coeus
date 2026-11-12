//! the machine coeusd runs on: what it is, the systems it runs, and how
//! coeusd starts the runners that change them

pub mod darwin;
mod host;
mod launcher;
mod machine;
mod system;

#[cfg(test)]
mod test;

pub use launcher::{Launcher, Plist, Task, systemd_run_args};
pub use machine::Machine;
pub use system::{SystemFacts, SystemRoot};
