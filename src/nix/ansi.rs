use std::borrow::Cow;

const ESC: char = '\u{1b}';

/// `text` without terminal escape sequences, borrowed when it has none
pub fn strip_ansi(text: &str) -> Cow<'_, str> {
    if text.contains(ESC) {
        let mut plain = String::with_capacity(text.len());
        push_plain(&mut plain, text);
        Cow::Owned(plain)
    } else {
        Cow::Borrowed(text)
    }
}

/// appends `text` to `out` without its CSI (`ESC [ … final`) and OSC
/// (`ESC ] … BEL` or `ESC ] … ESC \`) sequences
fn push_plain(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(start) = rest.find(ESC) {
        out.push_str(&rest[..start]);
        rest = skip_sequence(&rest[start + ESC.len_utf8()..]);
    }
    out.push_str(rest);
}

fn skip_sequence(after_esc: &str) -> &str {
    let bytes = after_esc.as_bytes();
    let end = match bytes.first() {
        Some(b'[') => bytes[1..]
            .iter()
            .position(|byte| (0x40..=0x7e).contains(byte))
            .map_or(bytes.len(), |at| at + 2),
        Some(b']') => (1..bytes.len())
            .find_map(|at| match bytes[at] {
                0x07 => Some(at + 1),
                0x1b if bytes.get(at + 1) == Some(&b'\\') => Some(at + 2),
                _ => None,
            })
            .unwrap_or(bytes.len()),
        _ => after_esc.chars().next().map_or(0, char::len_utf8),
    };
    // each sequence ends on an ASCII byte or at the end of the text
    &after_esc[end..]
}
