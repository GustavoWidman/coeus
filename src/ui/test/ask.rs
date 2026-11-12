use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::plain;
use crate::{
    bootstrap::contract::{Choice, Question, QuestionKind},
    ui::ask::{Answer, Prompt},
};

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(char: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(char), KeyModifiers::CONTROL)
}

fn typed(text: &str) -> Vec<KeyEvent> {
    text.chars()
        .map(|char| press(KeyCode::Char(char)))
        .collect()
}

/// every key in turn; the answer the last one gave, if any
fn keys(prompt: &mut Prompt, keys: impl IntoIterator<Item = KeyEvent>) -> Option<Answer> {
    let mut answer = None;
    for key in keys {
        assert!(answer.is_none(), "a key after the answer");
        answer = prompt.key(key);
    }
    answer
}

fn value(answer: Option<Answer>) -> String {
    match answer {
        Some(Answer::Value(value)) => value,
        Some(Answer::Secret(_)) => panic!("a secret"),
        Some(Answer::Cancelled) => panic!("cancelled"),
        None => panic!("no answer"),
    }
}

fn kind() -> Question {
    let choice = |value: &str, label: &str, help: &str| Choice {
        value: value.into(),
        label: label.into(),
        help: Some(help.into()),
    };
    Question {
        id: "type".into(),
        prompt: "what kind of machine is nova?".into(),
        kind: QuestionKind::Choice {
            choices: vec![
                choice("server", "server", "always on, no screen"),
                choice("dev-server", "dev-server", "a server you also develop on"),
                choice("desktop", "desktop", "a workstation with a screen"),
            ],
            default: Some("dev-server".into()),
        },
    }
}

fn text(default: Option<&str>, pattern: Option<&str>) -> Prompt {
    Prompt::question(&Question {
        id: "timezone".into(),
        prompt: "nova's time zone".into(),
        kind: QuestionKind::Text {
            default: default.map(str::to_owned),
            pattern: pattern.map(str::to_owned),
        },
    })
    .unwrap()
}

#[test]
fn a_choice_starts_on_its_default_and_moves_with_arrows() {
    let mut prompt = Prompt::question(&kind()).unwrap();
    let lines = plain(&prompt.lines(80));
    assert_eq!(lines[0], "what kind of machine is nova?");
    assert_eq!(lines[1], "  server       always on, no screen");
    assert_eq!(lines[2], "› dev-server   a server you also develop on");
    assert_eq!(lines[4], "↑↓ to choose, enter to pick");

    let answer = keys(
        &mut prompt,
        [
            press(KeyCode::Down),
            press(KeyCode::Down),
            press(KeyCode::Up),
        ],
    );
    assert!(answer.is_none());
    assert!(plain(&prompt.lines(80))[3].starts_with("› desktop"));
    assert_eq!(value(prompt.key(press(KeyCode::Enter))), "desktop");
}

#[test]
fn a_choice_wraps_around_and_jumps_by_digit() {
    let mut prompt = Prompt::question(&kind()).unwrap();
    keys(&mut prompt, [press(KeyCode::Down), press(KeyCode::Down)]);
    assert!(plain(&prompt.lines(80))[1].starts_with("› server"));

    keys(&mut prompt, typed("9"));
    assert!(
        plain(&prompt.lines(80))[1].starts_with("› server"),
        "no ninth choice"
    );
    let answer = keys(
        &mut prompt,
        [press(KeyCode::Char('3')), press(KeyCode::Enter)],
    );
    assert_eq!(value(answer), "desktop");
}

#[test]
fn a_confirm_answers_on_y_or_n_and_enter_takes_the_default() {
    let mut prompt = Prompt::confirm(
        "ed25519 SHA256:3fA9xq\ntrust it and enroll it as nova's key?".into(),
        true,
    );
    let lines = plain(&prompt.lines(80));
    assert_eq!(
        lines,
        [
            "ed25519 SHA256:3fA9xq",
            "› trust it and enroll it as nova's key? [Y/n]"
        ]
    );

    assert!(prompt.key(press(KeyCode::Char('x'))).is_none());
    assert_eq!(value(prompt.key(press(KeyCode::Char('n')))), "false");
    assert_eq!(value(prompt.key(press(KeyCode::Enter))), "true");
    let mut no = Prompt::confirm("reboot now?".into(), false);
    assert!(plain(&no.lines(80))[0].ends_with("[y/N]"));
    assert_eq!(value(no.key(press(KeyCode::Enter))), "false");
}

#[test]
fn a_text_keeps_its_default_on_enter() {
    let mut prompt = text(Some("America/Sao_Paulo"), None);
    assert_eq!(plain(&prompt.lines(80))[1], "›  America/Sao_Paulo");
    assert_eq!(
        value(prompt.key(press(KeyCode::Enter))),
        "America/Sao_Paulo"
    );
}

#[test]
fn a_text_that_misses_its_pattern_says_so_and_stays_open() {
    let mut prompt = text(None, Some("[a-z]+/[A-Za-z_]+"));
    assert!(keys(&mut prompt, [press(KeyCode::Enter)]).is_none());
    assert!(plain(&prompt.lines(80))[2].contains("type an answer"));

    let answer = keys(
        &mut prompt,
        typed("utc").into_iter().chain([press(KeyCode::Enter)]),
    );
    assert!(answer.is_none());
    let lines = plain(&prompt.lines(80));
    assert_eq!(lines[1], "› utc ");
    assert!(
        lines[2].contains("\"utc\" does not match [a-z]+/[A-Za-z_]+"),
        "{lines:?}"
    );

    // the pattern matches whole: a prefix that fits is not enough
    keys(&mut prompt, [ctrl('u')]);
    assert!(
        plain(&prompt.lines(80)).len() == 2,
        "typing clears the error"
    );
    let answer = keys(
        &mut prompt,
        typed("america/Sao_Paulo!")
            .into_iter()
            .chain([press(KeyCode::Enter)]),
    );
    assert!(answer.is_none());

    let answer = keys(
        &mut prompt,
        [press(KeyCode::Backspace), press(KeyCode::Enter)],
    );
    assert_eq!(value(answer), "america/Sao_Paulo");
}

#[test]
fn a_secret_shows_dots_and_is_asked_twice() {
    let mut prompt = Prompt::secret("password for root and r3dlust on nova".into(), true);
    keys(&mut prompt, typed("hunter2"));
    let lines = plain(&prompt.lines(80));
    assert_eq!(lines[1], "› password ••••••• ");
    assert!(!lines.concat().contains("hunter2"));

    keys(&mut prompt, [press(KeyCode::Enter)]);
    keys(&mut prompt, typed("hunter3"));
    let lines = plain(&prompt.lines(80));
    assert_eq!(lines[1], "› password •••••••");
    assert_eq!(lines[2], "› again    ••••••• ");

    assert!(prompt.key(press(KeyCode::Enter)).is_none());
    let lines = plain(&prompt.lines(80));
    assert!(lines[2].contains("the two did not match"), "{lines:?}");
    assert_eq!(lines[1], "› password  ", "asked again from the first");

    let answer = keys(
        &mut prompt,
        typed("hunter2")
            .into_iter()
            .chain([press(KeyCode::Enter)])
            .chain(typed("hunter2"))
            .chain([press(KeyCode::Enter)]),
    );
    match answer {
        Some(Answer::Secret(secret)) => assert_eq!(secret.as_str(), "hunter2"),
        _ => panic!("no secret"),
    }
}

#[test]
fn an_empty_secret_is_refused() {
    let mut prompt = Prompt::secret("the cache token".into(), false);
    assert!(prompt.key(press(KeyCode::Enter)).is_none());
    assert!(plain(&prompt.lines(80))[2].contains("cannot be empty"));
}

#[test]
fn ctrl_c_cancels_any_prompt() {
    for mut prompt in [
        Prompt::question(&kind()).unwrap(),
        Prompt::confirm("sure?".into(), true),
        text(None, None),
        Prompt::secret("password".into(), true),
    ] {
        assert!(matches!(prompt.key(ctrl('c')), Some(Answer::Cancelled)));
    }
}

#[test]
fn a_question_carries_its_id_and_refuses_a_bad_pattern() {
    assert_eq!(text(None, None).id.as_deref(), Some("timezone"));
    let broken = Question {
        id: "name".into(),
        prompt: "name".into(),
        kind: QuestionKind::Text {
            default: None,
            pattern: Some("(".into()),
        },
    };
    let error = Prompt::question(&broken).err().unwrap();
    assert!(format!("{error:#}").contains("question name's pattern"));
}

#[test]
fn a_long_question_wraps_instead_of_hiding_its_options() {
    let prompt = Prompt::confirm(
        "ssh://nixos@192.168.1.40:22 presents a host key coeus has never seen\n\
         trust it and enroll it as nova's key, the one agenix will encrypt its secrets to?"
            .into(),
        true,
    );
    let lines = plain(&prompt.lines(40));
    assert!(lines.iter().all(|line| !line.contains('…')), "{lines:#?}");
    assert!(lines.last().unwrap().ends_with("[Y/n]"), "{lines:#?}");
    assert!(
        lines.iter().any(|line| line.starts_with("› trust it")),
        "{lines:#?}"
    );
}
