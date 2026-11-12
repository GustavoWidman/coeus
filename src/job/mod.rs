//! jobs on disk: one directory per deployment, shared by coeusd and the
//! job's runner

mod dir;
mod log;
mod spec;
mod stop;
mod store;

#[cfg(test)]
mod test;

pub use dir::JobDir;
pub use log::{JobLog, LogItem, LogReader, LogWriter};
pub use spec::*;
pub use stop::StopRequest;
pub use store::JobStore;
