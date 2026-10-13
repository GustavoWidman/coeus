//! Bounded terminal-control removal shared by pipe capture and display boundaries.
//! Not a terminal emulator: cursor motion/styles are discarded; the caller owns
//! CR/LF presentation. Escape payloads never accumulate in memory.
#[derive(Default)]
enum Escape {
    #[default]
    Ground,
    Start,
    Intermediate,
    Csi,
    String,
    StringTerminator,
}

#[derive(Default)]
pub(crate) struct PlainText {
    escape: Escape,
    string_bell: bool,
    utf8: [u8; 4],
    length: usize,
    expected: usize,
}

impl PlainText {
    pub(crate) fn push(&mut self, input: &[u8], mut emit: impl FnMut(char)) {
        for &byte in input {
            self.byte(byte, &mut emit);
        }
    }
    pub(crate) fn finish(&mut self, mut emit: impl FnMut(char)) {
        if self.length != 0 {
            self.character('\u{fffd}', &mut emit);
            self.length = 0;
        }
        // Discard an incomplete escape/string, rather than replaying it as text.
    }
    fn byte(&mut self, byte: u8, emit: &mut impl FnMut(char)) {
        if self.length != 0 {
            if byte & 0xc0 != 0x80 {
                self.length = 0;
                self.character('\u{fffd}', emit);
                self.byte(byte, emit);
                return;
            }
            self.utf8[self.length] = byte;
            self.length += 1;
            if self.length == self.expected {
                let character = std::str::from_utf8(&self.utf8[..self.length])
                    .ok()
                    .and_then(|text| text.chars().next())
                    .unwrap_or('\u{fffd}');
                self.length = 0;
                self.character(character, emit);
            }
            return;
        }
        self.expected = match byte {
            0x00..=0x7f => {
                self.character(char::from(byte), emit);
                return;
            }
            // Some terminal producers use standalone 8-bit C1 controls;
            // recognize them only outside a pending UTF-8 code point.
            0x80..=0x9f => {
                self.character(char::from(byte), emit);
                return;
            }
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => {
                self.character('\u{fffd}', emit);
                return;
            }
        };
        self.utf8[0] = byte;
        self.length = 1;
    }
    fn character(&mut self, c: char, emit: &mut impl FnMut(char)) {
        use Escape::*;
        self.escape = match self.escape {
            String => match c {
                '\u{9c}' => Ground,
                '\u{7}' if self.string_bell => Ground,
                '\u{1b}' => StringTerminator,
                _ => String,
            },
            StringTerminator => match c {
                '\\' | '\u{9c}' => Ground,
                '\u{7}' if self.string_bell => Ground,
                '\u{1b}' => StringTerminator,
                _ => String,
            },
            // Line breaks abort malformed non-string sequences. OSC strings accept BEL; other strings remain
            // hidden until ST, even if they contain line breaks.
            Start | Intermediate | Csi if matches!(c, '\r' | '\n') => {
                emit(c);
                Ground
            }
            Start | Intermediate | Csi if c == '\u{1b}' => Start,
            Start => match c {
                '[' => Csi,
                ']' | 'P' | 'X' | '^' | '_' => {
                    self.string_bell = c == ']';
                    String
                }
                '\u{20}'..='\u{2f}' => Intermediate,
                _ => Ground,
            },
            Intermediate => match c {
                '\u{20}'..='\u{2f}' => Intermediate,
                _ => Ground,
            },
            Csi => match c {
                '\u{40}'..='\u{7e}' => Ground,
                _ => Csi,
            },
            Ground => match c {
                '\u{1b}' => Start,
                '\u{9b}' => Csi,
                '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => {
                    self.string_bell = c == '\u{9d}';
                    String
                }
                '\r' | '\n' | '\t' => {
                    emit(c);
                    Ground
                }
                _ if c.is_control() || is_direction_control(c) => Ground,
                _ => {
                    emit(c);
                    Ground
                }
            },
        };
    }
}

// Avoid visually rearranging the cursor/source/status labels with child output.
fn is_direction_control(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// One bounded safe line for lifecycle/status and already framed logs.
/// CR/LF become spaces; tabs survive. Do not guess at old ESC-less fragments:
/// literal `[32m` text may be legitimate and its original meaning is lost.
pub(crate) fn bounded(text: &str, limit: usize) -> String {
    let mut result = String::with_capacity(text.len().min(limit));
    let mut full = false;
    let mut append = |c: char| {
        let c = if matches!(c, '\r' | '\n') { ' ' } else { c };
        if !full && result.len() + c.len_utf8() <= limit {
            result.push(c);
        } else {
            full = true;
        }
    };
    let mut parser = PlainText::default();
    parser.push(text.as_bytes(), &mut append);
    parser.finish(append);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strips_whole_sequences_not_just_escape_bytes() {
        assert_eq!(
            bounded("\x1b[32mBuilding\x1b[0m\r\x1b[2KDone\n", 1024),
            "Building Done "
        );
        assert_eq!(
            bounded(
                "before\x1b]8;;https://fixture.invalid\x1b\\link\x1b]8;;\x07after",
                1024
            ),
            "beforelinkafter"
        );
        assert_eq!(bounded("a\x1bPprivate payload\x1b\\b\x1b(0c", 1024), "abc");
    }
    #[test]
    fn sanitizes_lifecycle_and_status_without_control_injection() {
        assert_eq!(
            bounded(
                "error\n\x1b[2J\x1b]0;title\x07next\r\tend\x08\u{202e}",
                1024
            ),
            "error next \tend"
        );
        assert_eq!(bounded(&"🦀".repeat(600), 1023).len(), 1020);
        assert_eq!(bounded("🦀x", 3), "");
        assert_eq!(
            bounded("literal [32m and [2K", 1024),
            "literal [32m and [2K"
        );
    }
    #[test]
    fn split_sequences_and_unicode_are_chunk_independent() {
        let input = "é🦀\x1b[38;2;1;2;3mred\x1b[0m\x1b]0;hidden🦀\x1b\\\r\n終\t!\u{9b}31mX\u{9d}hidden\u{9c}";
        for size in 1..=input.len() {
            let mut parser = PlainText::default();
            let mut output = String::new();
            for chunk in input.as_bytes().chunks(size) {
                parser.push(chunk, |c| output.push(c));
            }
            parser.finish(|c| output.push(c));
            assert_eq!(output, "é🦀red\r\n終\t!X", "size {size}");
        }
    }
    #[test]
    fn raw_c1_controls_and_dcs_bell_are_not_printable_payload() {
        let mut parser = PlainText::default();
        let mut output = String::new();
        parser.push(
            b"a\x9b31mb\x9dhidden\x07c\x90hidden\x07still hidden\x9cd",
            |c| output.push(c),
        );
        parser.finish(|c| output.push(c));
        assert_eq!(output, "abcd");
    }
    #[test]
    fn malformed_and_unterminated_input_has_bounded_state() {
        let mut parser = PlainText::default();
        let mut output = String::new();
        parser.push(b"a\xf0\x9f", |c| output.push(c));
        parser.push(b"b\xff\x1b]", |c| output.push(c));
        for _ in 0..10_000 {
            parser.push(&[b'x'; 1024], |c| output.push(c));
        }
        parser.push(b"\x1b\\tail\xe2", |c| output.push(c));
        parser.finish(|c| output.push(c));
        assert_eq!(output, "a�b�tail�");
        assert!(parser.length <= 4);
        assert_eq!(bounded("head\x1b[31", 1024), "head");
    }
}
