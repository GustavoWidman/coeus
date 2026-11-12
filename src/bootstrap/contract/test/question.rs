use serde_json::json;

use crate::bootstrap::contract::{Answers, Question, QuestionKind, wire};

fn question(value: serde_json::Value) -> eyre::Result<Question> {
    serde_json::from_value::<wire::Question>(value)
        .unwrap()
        .try_into()
}

fn kind() -> Question {
    question(json!({
        "id": "type", "prompt": "what is it for?", "kind": "choice",
        "choices": [
            { "value": "server", "label": "server", "help": "always on" },
            { "value": "desktop", "label": "desktop" },
        ],
        "default": "server",
    }))
    .unwrap()
}

#[test]
fn a_choice_takes_one_of_its_values() {
    let kind = kind();
    assert_eq!(kind.accept("desktop"), Ok("desktop".into()));
    assert_eq!(kind.accept(""), Ok("server".into()));
    assert_eq!(
        kind.accept("laptop"),
        Err("pick one of server, desktop".into())
    );
}

#[test]
fn a_confirm_takes_true_or_false_and_its_default_may_be_a_string() {
    for default in [json!("false"), json!(false)] {
        let admin = question(json!({
            "id": "admin", "prompt": "an admin?", "kind": "confirm", "default": default,
        }))
        .unwrap();
        assert_eq!(
            admin.kind,
            QuestionKind::Confirm {
                default: Some(false)
            }
        );
        assert_eq!(admin.accept(""), Ok("false".into()));
        assert_eq!(admin.accept("true"), Ok("true".into()));
        assert_eq!(admin.accept("yes"), Ok("true".into()));
        assert_eq!(admin.accept("N"), Ok("false".into()));
        assert!(admin.accept("maybe").is_err());
    }

    let undecided = question(json!({ "id": "a", "prompt": "p", "kind": "confirm" })).unwrap();
    assert!(undecided.accept("").is_err());
}

#[test]
fn a_text_answer_matches_its_pattern_whole() {
    let zone = question(json!({
        "id": "zone", "prompt": "time zone", "kind": "text",
        "default": "UTC", "pattern": "[A-Za-z_]+(/[A-Za-z_]+)*",
    }))
    .unwrap();
    assert_eq!(
        zone.accept("America/Sao_Paulo"),
        Ok("America/Sao_Paulo".into())
    );
    assert_eq!(zone.accept(""), Ok("UTC".into()));
    assert!(zone.accept("America/Sao Paulo").is_err());
    assert!(zone.accept("x; rm -rf /").is_err());

    let free = question(json!({ "id": "note", "prompt": "a note", "kind": "text" })).unwrap();
    assert_eq!(free.accept("anything at all"), Ok("anything at all".into()));
    assert!(free.accept("").is_err());
}

#[test]
fn a_question_that_contradicts_its_kind_is_refused_by_id() {
    for bad in [
        json!({ "id": "a", "prompt": "p", "kind": "choice", "choices": [] }),
        json!({ "id": "a", "prompt": "p", "kind": "choice", "default": "x",
                "choices": [{ "value": "y", "label": "y" }] }),
        json!({ "id": "a", "prompt": "p", "kind": "choice",
                "choices": [{ "value": "y", "label": "y" }, { "value": "y", "label": "z" }] }),
        json!({ "id": "a", "prompt": "p", "kind": "choice", "pattern": ".*",
                "choices": [{ "value": "y", "label": "y" }] }),
        json!({ "id": "a", "prompt": "p", "kind": "confirm", "default": "perhaps" }),
        json!({ "id": "a", "prompt": "p", "kind": "confirm", "choices": [] }),
        json!({ "id": "a", "prompt": "p", "kind": "text", "pattern": "(" }),
        json!({ "id": "a", "prompt": "p", "kind": "text", "default": true }),
        json!({ "id": "a", "prompt": "p", "kind": "text",
                "choices": [{ "value": "y", "label": "y" }] }),
    ] {
        let error = question(bad.clone()).unwrap_err();
        assert!(error.to_string().contains("\"a\""), "{bad}: {error}");
    }
}

#[test]
fn answers_given_on_the_command_line_are_id_equals_value() {
    let given = Answers::given(&["type=server".into(), "motd=a=b".into()]).unwrap();
    assert_eq!(given["type"], "server");
    assert_eq!(given["motd"], "a=b");

    assert!(Answers::given(&["type".into()]).is_err());
    assert!(Answers::given(&["=server".into()]).is_err());
    assert!(Answers::given(&["type=a".into(), "type=b".into()]).is_err());
}
