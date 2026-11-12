use std::borrow::Cow;

use crate::nix::strip_ansi;

#[test]
fn strips_colors_and_links() {
    assert_eq!(
        strip_ansi("\u{1b}[31;1merror:\u{1b}[0m Cannot build '\u{1b}[35;1mhello\u{1b}[0m'"),
        "error: Cannot build 'hello'"
    );
    assert_eq!(
        strip_ansi("see \u{1b}]8;;https://nixos.org\u{7}nixos\u{1b}]8;;\u{1b}\\ ❌"),
        "see nixos ❌"
    );
    assert_eq!(strip_ansi("cut \u{1b}[31"), "cut ");
    assert_eq!(strip_ansi("lone \u{1b}é"), "lone ");
}

#[test]
fn borrows_plain_text() {
    assert!(matches!(strip_ansi("plain"), Cow::Borrowed("plain")));
}
