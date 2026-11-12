//! everything the CLI draws. views are pure, a model in and lines out; only
//! [Screen] touches the terminal.

pub mod board;
pub mod describe;
pub mod format;
pub mod say;
pub mod text;

mod plain;
mod screen;
mod summary;
mod table;
mod theme;

#[cfg(test)]
pub(crate) mod test;

pub use board::{Board, Frame, Heading};
pub use plain::Plain;
pub use screen::Screen;
pub use summary::Summary;
pub use theme::Theme;
