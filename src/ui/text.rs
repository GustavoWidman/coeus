use std::borrow::Cow;

use unicode_width::UnicodeWidthChar;

const ESC: char = '\u{1b}';
const RESET: &str = "\u{1b}[0m";

/// the columns `text` takes on a terminal, escape sequences excluded
pub fn width(text: &str) -> usize {
    let mut columns = 0;
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        match char {
            ESC => skip_sequence(&mut chars, |_| {}),
            char => columns += char.width().unwrap_or(0),
        }
    }
    columns
}

/// `text` cut to at most `max` columns, ending in `…` when cut. colors
/// survive the cut: escape sequences are kept and a reset closes the line.
pub fn fit(text: &str, max: usize) -> Cow<'_, str> {
    if width(text) <= max {
        return Cow::Borrowed(text);
    }

    let budget = max.saturating_sub(1);
    let mut out = String::with_capacity(text.len());
    let mut columns = 0;
    let mut chars = text.chars();

    while let Some(char) = chars.next() {
        if char == ESC {
            out.push(ESC);
            skip_sequence(&mut chars, |char| out.push(char));
            continue;
        }

        let char_width = char.width().unwrap_or(0);
        if columns + char_width > budget {
            break;
        }
        columns += char_width;
        out.push(char);
    }

    if max > 0 {
        out.push('…');
    }
    out.push_str(RESET);
    Cow::Owned(out)
}

/// `text` followed by spaces up to `columns`
pub fn pad(text: &str, columns: usize) -> String {
    let fill = columns.saturating_sub(width(text));
    let mut out = String::with_capacity(text.len() + fill);
    out.push_str(text);
    out.extend(std::iter::repeat_n(' ', fill));
    out
}

/// `text` preceded by spaces up to `columns`
pub fn pad_left(text: &str, columns: usize) -> String {
    let fill = columns.saturating_sub(width(text));
    let mut out = String::with_capacity(text.len() + fill);
    out.extend(std::iter::repeat_n(' ', fill));
    out.push_str(text);
    out
}

/// consumes one CSI (`[ … final`) or a two-character escape after `ESC`,
/// handing each consumed char to `keep`
fn skip_sequence(chars: &mut std::str::Chars<'_>, mut keep: impl FnMut(char)) {
    match chars.next() {
        Some('[') => {
            keep('[');
            for char in chars.by_ref() {
                keep(char);
                if ('\u{40}'..='\u{7e}').contains(&char) {
                    break;
                }
            }
        }
        Some(char) => keep(char),
        None => {}
    }
}
