//! questions drawn as part of the frame: keys in, lines out

use colored::Colorize;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use eyre::{Context, eyre};
use regex_lite::Regex;
use zeroize::Zeroizing;

use super::text::{fit, pad};
use crate::bootstrap::contract::{Choice, Question, QuestionKind};

/// not `Debug`: a secret's input must never reach a log
#[derive(Clone)]
pub struct Prompt {
    /// the contract question's id, named when nobody is there to answer
    pub id: Option<String>,
    /// lines before the last are context; the last is what is asked
    pub question: String,
    pub kind: PromptKind,
    /// what was wrong with the last answer, shown under it
    pub error: Option<String>,
}

#[derive(Clone)]
pub enum PromptKind {
    /// ↑ ↓ to move, a digit to jump, enter to pick
    Choice { choices: Vec<Choice>, at: usize },
    /// y or n answers at once; enter takes the default
    Confirm { default: bool },
    Text {
        input: String,
        default: Option<String>,
        /// anchored, so the whole answer must match
        pattern: Option<Regex>,
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
    /// a contract question, its default preselected
    pub fn question(question: &Question) -> eyre::Result<Self> {
        let kind = match &question.kind {
            QuestionKind::Choice { choices, default } => {
                if choices.is_empty() {
                    return Err(eyre!("question {} has no choices", question.id));
                }
                let at = default
                    .as_ref()
                    .and_then(|default| choices.iter().position(|c| &c.value == default))
                    .unwrap_or(0);
                PromptKind::Choice {
                    choices: choices.clone(),
                    at,
                }
            }
            QuestionKind::Confirm { default } => PromptKind::Confirm {
                default: default.unwrap_or(false),
            },
            QuestionKind::Text { default, pattern } => PromptKind::Text {
                input: String::new(),
                default: default.clone(),
                pattern: pattern
                    .as_deref()
                    .map(|pattern| Regex::new(&format!("^(?:{pattern})$")))
                    .transpose()
                    .wrap_err_with(|| format!("question {}'s pattern", question.id))?,
            },
        };

        Ok(Self {
            id: Some(question.id.clone()),
            question: question.prompt.clone(),
            kind,
            error: None,
        })
    }

    pub fn confirm(question: String, default: bool) -> Self {
        Self::new(question, PromptKind::Confirm { default })
    }

    pub fn secret(question: String, twice: bool) -> Self {
        Self::new(
            question,
            PromptKind::Secret {
                input: blank(),
                first: None,
                twice,
            },
        )
    }

    fn new(question: String, kind: PromptKind) -> Self {
        Self {
            id: None,
            question,
            kind,
            error: None,
        }
    }

    /// the answer once the person gave a valid one
    pub fn key(&mut self, key: KeyEvent) -> Option<Answer> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('c') {
            return Some(Answer::Cancelled);
        }

        let answered = match &mut self.kind {
            PromptKind::Choice { choices, at } => choice(choices, at, key.code),
            PromptKind::Confirm { default } => confirm(*default, key.code),
            PromptKind::Text {
                input,
                default,
                pattern,
            } => text(input, default.as_deref(), pattern.as_ref(), key, control),
            PromptKind::Secret {
                input,
                first,
                twice,
            } => secret(input, first, *twice, key, control),
        };

        match answered {
            Step::Typing => {
                self.error = None;
                None
            }
            Step::Wrong(error) => {
                self.error = Some(error);
                None
            }
            Step::Answered(answer) => Some(answer),
        }
    }

    pub fn lines(&self, width: usize) -> Vec<String> {
        let (context, asked) = match self.question.rsplit_once('\n') {
            Some((context, asked)) => (context.lines().collect(), asked),
            None => (Vec::new(), self.question.as_str()),
        };
        let mut lines: Vec<String> = context.into_iter().map(str::to_owned).collect();
        let pointer = "›".cyan().bold();

        match &self.kind {
            PromptKind::Choice { choices, at } => {
                lines.push(asked.bold().to_string());
                let label_width = choices.iter().map(|c| c.label.len()).max().unwrap_or(0);
                for (index, choice) in choices.iter().enumerate() {
                    let (mark, label) = match index == *at {
                        true => (pointer.to_string(), choice.label.bold().to_string()),
                        false => (" ".to_owned(), choice.label.clone()),
                    };
                    let help = choice.help.as_deref().unwrap_or_default().dimmed();
                    lines.push(
                        format!("{mark} {}   {help}", pad(&label, label_width))
                            .trim_end()
                            .to_owned(),
                    );
                }
                lines.push("↑↓ to choose, enter to pick".dimmed().to_string());
            }
            PromptKind::Confirm { default } => {
                let options = match default {
                    true => "[Y/n]",
                    false => "[y/N]",
                };
                lines.push(format!("{pointer} {asked} {}", options.dimmed()));
            }
            PromptKind::Text { input, default, .. } => {
                lines.push(asked.bold().to_string());
                let shown = match (input.is_empty(), default) {
                    (true, Some(default)) => format!("{CURSOR}{}", default.dimmed()),
                    _ => format!("{input}{CURSOR}"),
                };
                lines.push(format!("{pointer} {shown}"));
            }
            PromptKind::Secret {
                input,
                first,
                twice,
            } => {
                lines.push(asked.bold().to_string());
                let dots = |text: &str| "•".repeat(text.chars().count());
                match (twice, first) {
                    (false, _) => lines.push(format!("{pointer} {}{CURSOR}", dots(input))),
                    (true, None) => {
                        lines.push(format!("{pointer} password {}{CURSOR}", dots(input)))
                    }
                    (true, Some(first)) => {
                        lines.push(format!("{pointer} password {}", dots(first)));
                        lines.push(format!("{pointer} again    {}{CURSOR}", dots(input)));
                    }
                }
            }
        }

        if let Some(error) = &self.error {
            lines.push(format!("  {}", error.red()));
        }

        lines
            .into_iter()
            .map(|line| fit(&line, width).into_owned())
            .collect()
    }
}

/// where the cursor is: drawn, since the screen hides the real one
const CURSOR: &str = "\u{1b}[7m \u{1b}[27m";

/// what one key did to a prompt
enum Step {
    Typing,
    Wrong(String),
    Answered(Answer),
}

fn choice(choices: &[Choice], at: &mut usize, code: KeyCode) -> Step {
    let last = choices.len() - 1;
    match code {
        KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
            *at = at.checked_sub(1).unwrap_or(last)
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
            *at = if *at == last { 0 } else { *at + 1 }
        }
        KeyCode::Home => *at = 0,
        KeyCode::End => *at = last,
        KeyCode::Char(digit @ '1'..='9') => {
            let index = digit as usize - '1' as usize;
            if index <= last {
                *at = index;
            }
        }
        KeyCode::Enter => return Step::Answered(Answer::Value(choices[*at].value.clone())),
        _ => {}
    }
    Step::Typing
}

fn confirm(default: bool, code: KeyCode) -> Step {
    let yes = match code {
        KeyCode::Char('y' | 'Y') => true,
        KeyCode::Char('n' | 'N') => false,
        KeyCode::Enter => default,
        _ => return Step::Typing,
    };
    Step::Answered(Answer::Value(yes.to_string()))
}

fn text(
    input: &mut String,
    default: Option<&str>,
    pattern: Option<&Regex>,
    key: KeyEvent,
    control: bool,
) -> Step {
    match key.code {
        KeyCode::Enter => {
            let value = match (input.is_empty(), default) {
                (true, Some(default)) => default.to_owned(),
                _ => input.trim().to_owned(),
            };
            if value.is_empty() {
                return Step::Wrong("type an answer".into());
            }
            match pattern {
                Some(pattern) if !pattern.is_match(&value) => Step::Wrong(format!(
                    "{value:?} does not match {}",
                    unanchored(pattern.as_str())
                )),
                _ => Step::Answered(Answer::Value(value)),
            }
        }
        code => {
            edit(input, code, control);
            Step::Typing
        }
    }
}

fn secret(
    input: &mut Zeroizing<String>,
    first: &mut Option<Zeroizing<String>>,
    twice: bool,
    key: KeyEvent,
    control: bool,
) -> Step {
    if key.code != KeyCode::Enter {
        edit(input, key.code, control);
        return Step::Typing;
    }
    if input.is_empty() {
        return Step::Wrong("it cannot be empty".into());
    }

    let typed = std::mem::replace(input, blank());
    match (twice, first.take()) {
        (false, _) => Step::Answered(Answer::Secret(typed)),
        (true, None) => {
            *first = Some(typed);
            Step::Typing
        }
        (true, Some(first)) if *first == *typed => Step::Answered(Answer::Secret(typed)),
        (true, Some(_)) => Step::Wrong("the two did not match; type it again".into()),
    }
}

/// room for a long passphrase, so typing never moves the secret and leaves
/// a copy behind
fn blank() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(256))
}

/// a printable key typed, backspace, or ctrl-u clearing the line
fn edit(input: &mut String, code: KeyCode, control: bool) {
    match code {
        KeyCode::Char('u') if control => input.clear(),
        KeyCode::Char(_) if control => {}
        KeyCode::Char(char) => input.push(char),
        KeyCode::Backspace => {
            input.pop();
        }
        _ => {}
    }
}

/// the pattern as the repository wrote it
fn unanchored(pattern: &str) -> &str {
    pattern
        .strip_prefix("^(?:")
        .and_then(|pattern| pattern.strip_suffix(")$"))
        .unwrap_or(pattern)
}
