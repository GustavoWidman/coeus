//! `coeus top`: the whole fleet on one screen. the model and its key
//! handling are pure; the CLI feeds them the fleet's views, the selected
//! job's log and the keyboard, and performs what they ask for.

pub mod ansi;
mod draw;
mod entry;
mod keys;
mod layout;
mod request;
mod state;
mod tail;

pub use draw::draw;
pub use entry::{Badge, Entry};
pub use keys::{Effect, Mode};
pub use layout::{Arrangement, Panes};
pub use request::{Intent, Request};
pub use state::{Notice, Top};
pub use tail::{Retarget, Tail};
