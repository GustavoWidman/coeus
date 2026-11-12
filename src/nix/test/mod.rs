mod ansi;
mod diff;
mod filter;
mod fold;
mod line;

pub const BUILD: &str = include_str!("../fixtures/build.jsonl");
pub const DOWNLOAD: &str = include_str!("../fixtures/download.jsonl");
pub const FAILING: &str = include_str!("../fixtures/failing.jsonl");
