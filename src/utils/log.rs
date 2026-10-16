use std::io::{Error, IsTerminal, Write};

use colog::format::CologStyle;
use colog::formatter;
use colored::Colorize;
use env_logger::fmt::Formatter;
use env_logger::{Builder, Target, WriteStyle};
use log::{Level, LevelFilter, Record};

struct CustomLevelTokens;

impl CologStyle for CustomLevelTokens {
    fn level_token(&self, level: &Level) -> &str {
        match *level {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }

    fn level_color(&self, level: &Level, msg: &str) -> String {
        match level {
            Level::Error => msg.red().bold(),
            Level::Warn => msg.yellow().bold(),
            Level::Info => msg.green().bold(),
            Level::Debug => msg.cyan(),
            Level::Trace => msg.magenta(),
        }
        .to_string()
    }

    fn prefix_token(&self, level: &Level) -> String {
        format!(
            "{} {}",
            chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S.%6f")
                .to_string()
                .white(),
            self.level_color(level, self.level_token(level)),
        )
    }

    fn format(&self, buf: &mut Formatter, record: &Record<'_>) -> Result<(), Error> {
        let sep = self.line_separator();
        let prefix = self.prefix_token(&record.level());

        let string = match &record.level() {
            Level::Error => record
                .args()
                .to_string()
                .replace('\n', &sep)
                .bold()
                .to_string(),
            Level::Warn | Level::Info => record.args().to_string().replace('\n', &sep),
            Level::Debug | Level::Trace => record
                .args()
                .to_string()
                .replace('\n', &sep)
                .white()
                .to_string(),
        };

        writeln!(buf, "{} {}", prefix, string)
    }
}

pub struct Logger;

impl Logger {
    /// Print and flush the request nonce before any deployment request is sent. Only
    /// this safety-critical message bypasses the ordinary verbosity filter.
    pub fn retry_id(request_id: u64) -> std::io::Result<()> {
        // Use this logger's presentation directly for the safety exception:
        // log's facade discards output errors, whereas a failed write/flush
        // must prevent submission (just as the former println + flush did).
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "request-id: {request_id}")?;
        stdout.flush()
    }

    // Terminal stdout is never a diagnostic target, including dependency logs.
    pub fn init_attach() {
        Self::attach_builder().init();
    }

    fn attach_builder() -> Builder {
        let mut builder = Builder::new();
        builder
            .filter_level(LevelFilter::Off)
            .target(Target::Stderr);
        builder
    }

    /// One auto-color policy for CLI diagnostics, tables and interactive views.
    pub fn use_color(terminal: bool) -> bool {
        terminal
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("CLICOLOR").as_deref() != Ok("0")
            && std::env::var("TERM").as_deref() != Ok("dumb")
    }

    pub fn client_error(error: &eyre::Report, verbose: bool) -> String {
        if verbose {
            return format!("error: {error:?}");
        }
        let message = error
            .chain()
            .take(3)
            .map(|cause| cause.to_string())
            .collect::<Vec<_>>()
            .join(": ");
        let label = if Self::use_color(std::io::stderr().is_terminal()) {
            "\x1b[1;31merror:\x1b[0m"
        } else {
            "error:"
        };
        format!(
            "{label} {}",
            crate::utils::plain_text::bounded(&message, 2048)
        )
    }

    pub fn init_client(level: LevelFilter) {
        let color = Self::use_color(std::io::stderr().is_terminal());
        Builder::new()
            .filter(Some("coeus"), level)
            .target(Target::Stderr)
            .write_style(WriteStyle::Auto)
            .format(move |buffer, record| {
                let label = match record.level() {
                    Level::Warn => "warning: ",
                    Level::Error => "error: ",
                    Level::Debug => "debug: ",
                    Level::Trace => "trace: ",
                    Level::Info => "",
                };
                let label = if color && record.level() == Level::Error {
                    "\x1b[1;31merror:\x1b[0m "
                } else if color && record.level() == Level::Warn {
                    "\x1b[33mwarning:\x1b[0m "
                } else {
                    label
                };
                writeln!(
                    buffer,
                    "{label}{}",
                    crate::utils::plain_text::bounded(&record.args().to_string(), 4096)
                )
            })
            .init();
    }

    pub fn init(level: LevelFilter) {
        Builder::new()
            .filter("coeus".into(), level)
            .target(Target::Stdout)
            .format(formatter(CustomLevelTokens))
            .write_style(WriteStyle::Auto)
            .init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::{Log, Metadata};
    #[test]
    fn attach_disables_every_diagnostic_target_not_only_coeus() {
        let logger = Logger::attach_builder().build();
        for target in ["coeus", "coeus::client", "dependency"] {
            for level in [
                Level::Error,
                Level::Warn,
                Level::Info,
                Level::Debug,
                Level::Trace,
            ] {
                assert!(!logger.enabled(&Metadata::builder().target(target).level(level).build()));
            }
        }
    }
}
