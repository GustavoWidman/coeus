//! what the repository wants to know about a machine, and the answers

use std::collections::BTreeMap;

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
        // TODO: by `kind`: a choice needs a non-empty `choices` with distinct
        // values and a default among them; a confirm a bool default; a text a
        // string default and a pattern regex_lite compiles. a field another
        // kind uses (a text's `choices`) is an error naming the question id
        todo!()
    }
}

impl Question {
    /// an answer from a prompt or from `--answer id=value`, as the contract
    /// receives it: a choice's value, `true`/`false`, or the text matching
    /// the pattern. the error is said to the person under the prompt.
    pub fn accept(&self, raw: &str) -> Result<String, String> {
        todo!()
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
        todo!()
    }
}
