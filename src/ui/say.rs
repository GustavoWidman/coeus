//! one-line messages in the style of the fleet's deploy scripts:
//! `[›] info`, `[✓] success`, `[⚠] warning`, `[✗] error`, `[•] aside`

use std::fmt::Display;

use colored::{ColoredString, Colorize};

fn mark(mark: ColoredString, text: impl Display) -> String {
    format!("[{mark}] {text}")
}

pub fn info(text: impl Display) -> String {
    mark("›".blue(), text)
}

pub fn success(text: impl Display) -> String {
    mark("✓".green(), text)
}

pub fn warn(text: impl Display) -> String {
    mark("⚠".yellow(), text)
}

pub fn error(text: impl Display) -> String {
    mark("✗".red(), text)
}

pub fn aside(text: impl Display) -> String {
    mark("•".bright_black(), text)
}
