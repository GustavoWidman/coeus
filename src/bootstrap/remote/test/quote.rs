use std::process::Command;

use crate::bootstrap::remote::quote::{line, word};

#[test]
fn plain_words_stay_as_they_are() {
    for plain in [
        "nix",
        "--no-link",
        "path:/nix/store/abc-source",
        "user@host",
        "a=b,c",
    ] {
        assert_eq!(word(plain), plain);
    }
}

#[test]
fn anything_a_shell_reads_is_single_quoted() {
    assert_eq!(word(""), "''");
    assert_eq!(word("two words"), "'two words'");
    assert_eq!(word("$HOME"), "'$HOME'");
    assert_eq!(word("flake#attribute"), "'flake#attribute'");
    assert_eq!(word("it's"), r"'it'\''s'");
}

#[test]
fn a_shell_splits_the_line_back_into_the_argv() {
    let argv: Vec<String> = [
        "printf",
        "%s\\n",
        "",
        "two words",
        "it's $HOME `id` \"quoted\"",
        "line\nbreak",
        "*",
        "a;b|c&d",
    ]
    .map(str::to_owned)
    .to_vec();

    let output = Command::new("sh")
        .args(["-c", &line(&argv)])
        .output()
        .unwrap();
    let printed = String::from_utf8(output.stdout).unwrap();
    let expected: String = argv[2..].iter().map(|word| format!("{word}\n")).collect();
    assert_eq!(printed, expected);
}
