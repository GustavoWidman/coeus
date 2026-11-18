//! the fleet as the dashboard's browser reads it. these shapes are its own:
//! the daemon's wire is positional postcard and job logs are JSON a newer
//! daemon must still read, so neither encoding may bend to suit a browser.
//! [derive] builds them from the domain, which stays the only source of truth.
//!
//! `coeus web` keeps one live copy of the fleet ([fleet]), streams it to
//! browsers ([stream]) with job logs ([logs]) and closure diffs ([diffs]),
//! and serves the built web app beside them ([site]).

pub mod derive;
pub mod diffs;
pub mod fleet;
pub mod logs;
pub mod site;
pub mod source;
pub mod stream;
pub mod transcript;
pub mod view;

#[cfg(test)]
mod test;
