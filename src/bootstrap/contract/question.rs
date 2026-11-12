//! what the repository wants to know about a machine, and the answers

use std::collections::{BTreeMap, BTreeSet};

use eyre::{bail, eyre};
use regex_lite::Regex;
use serde::{Deserialize, Serialize};

use super::wire::{self, Choice};

/// one question, checked: a choice has choices, a pattern compiles
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub prompt: String,
    pub kind: QuestionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionKind {
    Choice {
        choices: Vec<Choice>,
        /// the value picked when the person just presses enter
        default: Option<String>,
    },
    Confirm {
        default: Option<bool>,
    },
    Text {
        default: Option<String>,
        /// a regular expression the answer must match whole
        pattern: Option<String>,
    },
}

impl TryFrom<wire::Question> for Question {
    type Error = eyre::Report;

    fn try_from(question: wire::Question) -> eyre::Result<Self> {
        let wire::Question {
            id,
            prompt,
            kind,
            choices,
            default,
            pattern,
        } = question;
        if id.is_empty() {
            bail!("the contract asks a question without an id");
        }
        let unused =
            |field: &str| eyre!("question {id:?} is a {} and has no {field}", kind.as_str());

        let kind = match kind {
            wire::QuestionKind::Choice => {
                if pattern.is_some() {
                    return Err(unused("pattern"));
                }
                let choices = choices.unwrap_or_default();
                if choices.is_empty() {
                    bail!("question {id:?} is a choice without choices");
                }
                let mut values = BTreeSet::new();
                if let Some(twice) = choices.iter().find(|c| !values.insert(&c.value)) {
                    bail!("question {id:?} offers {:?} twice", twice.value);
                }
                let default = match default {
                    None => None,
                    Some(wire::Default::Text(value)) if values.contains(&value) => Some(value),
                    Some(other) => {
                        return Err(eyre!(
                            "question {id:?} defaults to {}, which is not one of its choices",
                            shown(&other)
                        ));
                    }
                };
                QuestionKind::Choice { choices, default }
            }
            wire::QuestionKind::Confirm => {
                if choices.is_some() {
                    return Err(unused("choices"));
                }
                if pattern.is_some() {
                    return Err(unused("pattern"));
                }
                // answers are strings, so a default may be one too
                let default = match default {
                    None => None,
                    Some(wire::Default::Bool(value)) => Some(value),
                    Some(wire::Default::Text(text)) => {
                        Some(confirmation(&text).ok_or_else(|| {
                            eyre!("question {id:?} defaults to {text:?}, not true or false")
                        })?)
                    }
                };
                QuestionKind::Confirm { default }
            }
            wire::QuestionKind::Text => {
                if choices.is_some() {
                    return Err(unused("choices"));
                }
                if let Some(pattern) = &pattern {
                    whole(pattern).map_err(|error| {
                        eyre!("question {id:?} has a pattern that does not compile: {error}")
                    })?;
                }
                let default = match default {
                    None => None,
                    Some(wire::Default::Text(text)) => Some(text),
                    Some(other) => {
                        return Err(eyre!(
                            "question {id:?} is text and defaults to {}",
                            shown(&other)
                        ));
                    }
                };
                QuestionKind::Text { default, pattern }
            }
        };

        Ok(Self { id, prompt, kind })
    }
}

impl Question {
    /// an answer from a prompt or from `--answer id=value`, as the contract
    /// receives it: a choice's value, `true`/`false`, or the text matching
    /// the pattern. an empty answer takes the default. the error is said to
    /// the person under the prompt.
    pub fn accept(&self, raw: &str) -> Result<String, String> {
        match &self.kind {
            QuestionKind::Choice { choices, default } => {
                let raw = raw.trim();
                if raw.is_empty()
                    && let Some(default) = default
                {
                    return Ok(default.clone());
                }
                match choices.iter().find(|choice| choice.value == raw) {
                    Some(choice) => Ok(choice.value.clone()),
                    None => Err(format!(
                        "pick one of {}",
                        choices
                            .iter()
                            .map(|choice| choice.value.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                }
            }
            QuestionKind::Confirm { default } => {
                let raw = raw.trim();
                let answer = match (raw.is_empty(), default) {
                    (true, Some(default)) => Some(*default),
                    _ => confirmation(raw),
                };
                answer
                    .map(|yes| yes.to_string())
                    .ok_or_else(|| "answer yes or no".to_owned())
            }
            QuestionKind::Text { default, pattern } => {
                let answer = match (raw.is_empty(), default) {
                    (true, Some(default)) => default.as_str(),
                    _ => raw,
                };
                // without a pattern, any answer but none
                match pattern {
                    Some(pattern) if !whole(pattern).is_ok_and(|regex| regex.is_match(answer)) => {
                        Err(format!("the answer must match {pattern}"))
                    }
                    None if answer.is_empty() => Err("an answer is needed".to_owned()),
                    _ => Ok(answer.to_owned()),
                }
            }
        }
    }
}

/// `pattern` anchored at both ends, so it matches a whole answer
fn whole(pattern: &str) -> Result<Regex, regex_lite::Error> {
    Regex::new(&format!("^(?:{pattern})$"))
}

/// a confirm's answer: `true`/`false`, or yes/no as a person types it
fn confirmation(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "y" | "yes" => Some(true),
        "false" | "n" | "no" => Some(false),
        _ => None,
    }
}

fn shown(default: &wire::Default) -> String {
    match default {
        wire::Default::Bool(value) => value.to_string(),
        wire::Default::Text(text) => format!("{text:?}"),
    }
}

/// answers by question id, every value a string, as `answers` in the
/// contract's input
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Answers(BTreeMap<String, String>);

impl Answers {
    pub fn get(&self, id: &str) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }

    pub fn insert(&mut self, id: String, value: String) {
        self.0.insert(id, value);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(id, value)| (id.as_str(), value.as_str()))
    }

    /// `--answer type=server` arguments by id; a malformed one is an error
    /// naming it
    pub fn given(raw: &[String]) -> eyre::Result<BTreeMap<String, String>> {
        let mut given = BTreeMap::new();
        for answer in raw {
            let Some((id, value)) = answer.split_once('=') else {
                bail!("--answer {answer:?} is not id=value");
            };
            let id = id.trim();
            if id.is_empty() {
                bail!("--answer {answer:?} names no question");
            }
            if given.insert(id.to_owned(), value.to_owned()).is_some() {
                bail!("--answer {id} is given twice");
            }
        }
        Ok(given)
    }
}
