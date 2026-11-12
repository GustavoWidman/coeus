//! an argv as the one string ssh hands the remote shell. ssh joins its
//! arguments with spaces and the remote shell splits them again, so every
//! word is quoted here, and nowhere else.

/// one word for a POSIX shell: as it is when nothing in it is special,
/// else single-quoted, with each `'` written as `'\''`
pub fn word(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@%+=:,./_-".contains(&byte));
    match plain {
        true => text.to_owned(),
        false => format!("'{}'", text.replace('\'', r"'\''")),
    }
}

/// the words of `argv`, each quoted, joined by spaces
pub fn line(argv: &[String]) -> String {
    argv.iter()
        .map(|argument| word(argument))
        .collect::<Vec<_>>()
        .join(" ")
}
