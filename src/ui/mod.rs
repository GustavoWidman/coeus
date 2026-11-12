//! everything the CLI draws, as pure functions from the model to lines

pub mod describe;
pub mod format;
pub mod say;
pub mod text;

mod theme;

#[cfg(test)]
pub(crate) mod test;

pub use theme::Theme;
