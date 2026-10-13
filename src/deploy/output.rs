use crate::{common::proto::LogSource, jobs::JobContext, utils::plain_text::PlainText};
use eyre::Result;
use tokio::io::{AsyncRead, AsyncReadExt};

const CHUNK_BYTES: usize = 1024;

/// Readable snapshots, not a PTY: CR ends a progress update, CRLF is one break.
/// Chunks end on UTF-8 boundaries. The decoder/escape state spans every read.
#[derive(Default)]
struct Lines {
    line: String,
    after_cr: bool,
    compact: bool,
}
impl Lines {
    fn push(&mut self, c: char, lines: &mut Vec<String>) {
        match c {
            '\r' => {
                if !self.compact {
                    self.flush(lines);
                }
                self.after_cr = true;
            }
            '\n' => {
                if !self.after_cr || self.compact {
                    lines.push(std::mem::take(&mut self.line));
                }
                self.after_cr = false;
            }
            _ => {
                if self.after_cr && self.compact {
                    self.line.clear();
                }
                self.after_cr = false;
                if self.line.len() + c.len_utf8() > CHUNK_BYTES {
                    self.flush(lines);
                }
                self.line.push(c);
                if self.line.len() == CHUNK_BYTES {
                    self.flush(lines);
                }
            }
        }
    }
    fn flush(&mut self, lines: &mut Vec<String>) {
        if !self.line.is_empty() {
            lines.push(std::mem::take(&mut self.line));
        }
    }
}

/// Output and parser allocations are bounded by one read plus one chunk, even
/// for oversized lines or unbounded unterminated OSC/DCS payloads.
pub(super) async fn drain(
    mut reader: impl AsyncRead + Unpin,
    source: LogSource,
    context: JobContext,
) -> Result<()> {
    let mut input = [0; CHUNK_BYTES];
    let mut parser = PlainText::default();
    let mut lines = Lines {
        compact: matches!(source, LogSource::Terminal),
        ..Lines::default()
    };
    loop {
        let count = reader.read(&mut input).await?;
        let mut ready = Vec::new();
        if count == 0 {
            parser.finish(|c| lines.push(c, &mut ready));
            lines.flush(&mut ready);
        } else {
            if matches!(source, LogSource::Terminal) {
                context.terminal_bytes(&input[..count])?;
            }
            parser.push(&input[..count], |c| lines.push(c, &mut ready));
        }
        for line in ready {
            context.log(source, &line).await;
        }
        if count == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn normalize(input: &[u8], size: usize) -> Vec<String> {
        let mut parser = PlainText::default();
        let mut lines = Lines::default();
        let mut ready = Vec::new();
        for chunk in input.chunks(size) {
            parser.push(chunk, |c| lines.push(c, &mut ready));
            assert!(lines.line.len() < CHUNK_BYTES);
        }
        parser.finish(|c| lines.push(c, &mut ready));
        lines.flush(&mut ready);
        ready
    }
    #[test]
    fn ansi_cr_progress_and_crlf_do_not_join_or_leak_codes() {
        let input = b"\x1b[32mBuilding\x1b[0m\r\x1b[2KDone\r\nnext\titem\nlast";
        for size in 1..=input.len() {
            assert_eq!(
                normalize(input, size),
                ["Building", "Done", "next\titem", "last"]
            );
        }
    }
    #[test]
    fn long_utf8_lines_keep_complete_characters_at_the_limit() {
        let input = format!("{}{}\n", "a".repeat(1023), "🦀é".repeat(600));
        let expected = normalize(input.as_bytes(), 1);
        for size in [2, 3, 4, 7, 1024] {
            assert_eq!(normalize(input.as_bytes(), size), expected);
        }
        assert_eq!(expected.concat(), input.trim_end_matches('\n'));
        assert!(
            expected
                .iter()
                .all(|line| line.len() <= CHUNK_BYTES && !line.contains('�'))
        );
    }
    #[test]
    fn incomplete_sequences_invalid_utf8_and_eof_are_deliberate() {
        assert_eq!(
            normalize(b"first\r\r\nsecond\n\n\xe2", 1),
            ["first", "second", "", "�"]
        );
        assert_eq!(normalize(b"hello\x1b]hidden\ncontent", 1), ["hello"]);
        assert_eq!(normalize(b"before\x1b[3\nafter", 1), ["before", "after"]);
    }
}
