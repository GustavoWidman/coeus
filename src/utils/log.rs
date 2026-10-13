use std::io::{Error, Write};

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
    /// Log and flush the retry key before any deployment request is sent. Only
    /// this safety-critical message bypasses the ordinary verbosity filter.
    pub fn retry_id(request_id: u64) -> std::io::Result<()> {
        // Use this logger's presentation directly for the safety exception:
        // log's facade discards output errors, whereas a failed write/flush
        // must prevent submission (just as the former println + flush did).
        let mut stdout = std::io::stdout().lock();
        writeln!(
            stdout,
            "{} request-id: {request_id} (retry with identical options)",
            CustomLevelTokens.prefix_token(&Level::Info)
        )?;
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

    pub fn init(level: LevelFilter) {
        Builder::new()
            .filter("coeus".into(), level)
            .target(Target::Stdout)
            .format(formatter(CustomLevelTokens))
            .write_style(WriteStyle::Always)
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
