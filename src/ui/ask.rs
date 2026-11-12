//! questions drawn as part of the frame: keys in, lines out

// the bootstrap's ui unit fills this module in; until then its
// signatures have unused parameters
#![allow(unused_variables)]

use crossterm::event::KeyEvent;
use zeroize::Zeroizing;

use crate::bootstrap::contract::{Choice, Question};

pub struct Prompt {
    pub question: String,
    pub kind: PromptKind,
    /// what was wrong with the last answer, shown under it
    pub error: Option<String>,
}

pub enum PromptKind {
    /// ↑ ↓ to move, enter to pick
    Choice {
        choices: Vec<Choice>,
        at: usize,
    },
    Confirm {
        default: bool,
    },
    Text {
        input: String,
        default: Option<String>,
        pattern: Option<regex_lite::Regex>,
    },
    /// shown as dots, asked twice when `twice`
    Secret {
        input: Zeroizing<String>,
        first: Option<Zeroizing<String>>,
        twice: bool,
    },
}

pub enum Answer {
    /// a choice's value, `true`/`false`, or the text
    Value(String),
    Secret(Zeroizing<String>),
    /// ctrl-c while the prompt was open
    Cancelled,
}

impl Prompt {
    /// a contract question
    pub fn question(question: &Question) -> eyre::Result<Self> {
        todo!()
    }

    pub fn confirm(question: String, default: bool) -> Self {
        todo!()
    }

    pub fn secret(question: String, twice: bool) -> Self {
        todo!()
    }

    /// the answer once the person gave a valid one
    pub fn key(&mut self, key: KeyEvent) -> Option<Answer> {
        todo!()
    }

    pub fn lines(&self, width: usize) -> Vec<String> {
        todo!()
    }
}
