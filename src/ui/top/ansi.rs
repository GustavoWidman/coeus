use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

const ESC: char = '\u{1b}';

/// a line styled with SGR escapes, as the rest of the CLI prints it, read
/// into ratatui's styled text. other escape sequences are dropped.
pub fn line(text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut style = Style::default();
    let mut run = String::new();
    let mut rest = text;

    while let Some(start) = rest.find(ESC) {
        run.push_str(&rest[..start]);
        let (sequence, after) = Sequence::read(&rest[start + ESC.len_utf8()..]);
        if let Sequence::Sgr(params) = sequence {
            if !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), style));
            }
            style = sgr(style, params);
        }
        rest = after;
    }
    run.push_str(rest);
    if !run.is_empty() {
        spans.push(Span::styled(run, style));
    }

    Line::from(spans)
}

enum Sequence<'a> {
    /// `ESC [ params m`
    Sgr(&'a str),
    Other,
}

impl<'a> Sequence<'a> {
    /// the sequence after an `ESC`, and the text after it
    fn read(after_esc: &'a str) -> (Self, &'a str) {
        let bytes = after_esc.as_bytes();
        match bytes.first() {
            Some(b'[') => {
                let Some(end) = bytes[1..]
                    .iter()
                    .position(|byte| (0x40..=0x7e).contains(byte))
                    .map(|at| at + 1)
                else {
                    return (Self::Other, "");
                };
                let sequence = match bytes[end] {
                    b'm' => Self::Sgr(&after_esc[1..end]),
                    _ => Self::Other,
                };
                (sequence, &after_esc[end + 1..])
            }
            Some(b']') => {
                let end = (1..bytes.len())
                    .find_map(|at| match bytes[at] {
                        0x07 => Some(at + 1),
                        0x1b if bytes.get(at + 1) == Some(&b'\\') => Some(at + 2),
                        _ => None,
                    })
                    .unwrap_or(bytes.len());
                (Self::Other, &after_esc[end..])
            }
            _ => {
                let end = after_esc.chars().next().map_or(0, char::len_utf8);
                (Self::Other, &after_esc[end..])
            }
        }
    }
}

/// `style` after the SGR parameters `params`, e.g. `1;36` or `38;5;208`
fn sgr(mut style: Style, params: &str) -> Style {
    let mut codes = params
        .split([';', ':'])
        .map(|code| code.parse::<u16>().unwrap_or(0));

    while let Some(code) = codes.next() {
        style = match code {
            0 => Style::default(),
            1 => style.add_modifier(Modifier::BOLD),
            2 => style.add_modifier(Modifier::DIM),
            3 => style.add_modifier(Modifier::ITALIC),
            4 => style.add_modifier(Modifier::UNDERLINED),
            5 => style.add_modifier(Modifier::SLOW_BLINK),
            7 => style.add_modifier(Modifier::REVERSED),
            9 => style.add_modifier(Modifier::CROSSED_OUT),
            22 => style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => style.remove_modifier(Modifier::ITALIC),
            24 => style.remove_modifier(Modifier::UNDERLINED),
            25 => style.remove_modifier(Modifier::SLOW_BLINK),
            27 => style.remove_modifier(Modifier::REVERSED),
            29 => style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => style.fg(basic(code - 30)),
            90..=97 => style.fg(bright(code - 90)),
            40..=47 => style.bg(basic(code - 40)),
            100..=107 => style.bg(bright(code - 100)),
            39 => style.fg(Color::Reset),
            49 => style.bg(Color::Reset),
            38 => match extended(&mut codes) {
                Some(color) => style.fg(color),
                None => style,
            },
            48 => match extended(&mut codes) {
                Some(color) => style.bg(color),
                None => style,
            },
            _ => style,
        };
    }
    style
}

/// `5;n` or `2;r;g;b` after a `38` or `48`
fn extended(codes: &mut impl Iterator<Item = u16>) -> Option<Color> {
    let byte = |code: Option<u16>| code.and_then(|code| u8::try_from(code).ok());
    match codes.next()? {
        5 => byte(codes.next()).map(Color::Indexed),
        2 => Some(Color::Rgb(
            byte(codes.next())?,
            byte(codes.next())?,
            byte(codes.next())?,
        )),
        _ => None,
    }
}

const fn basic(index: u16) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

const fn bright(index: u16) -> Color {
    match index {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}
