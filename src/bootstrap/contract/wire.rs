//! the contract's outputs exactly as its JSON says them. anything the schema
//! does not name is refused, so a typo in a repository's contract fails at
//! the boundary instead of being ignored. [super::question] and
//! [super::enrollment] turn these into checked types.

use std::collections::BTreeMap;

use serde::Deserialize;

/// `{ version; identity; }`, read before anything else
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Head {
    pub version: u32,
    /// where an admin keeps the identity that opens every secret
    pub identity: String,
}

/// one element of `questions input`
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub prompt: String,
    pub kind: QuestionKind,
    pub choices: Option<Vec<Choice>>,
    pub default: Option<Default>,
    /// a regular expression a text answer must match whole
    pub pattern: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    Choice,
    Confirm,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// what the answer is
    pub value: String,
    /// what the person reads
    pub label: String,
    pub help: Option<String>,
}

/// a choice's value or a text's default is a string, a confirm's a bool
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Default {
    Bool(bool),
    Text(String),
}

/// `enroll input`
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Enrollment {
    /// repository path to text
    pub files: BTreeMap<String, String>,
    /// repository path of a `.age` file to how coeus makes it
    pub secrets: BTreeMap<String, Secret>,
    pub first_switch: FirstSwitch,
    pub messages: Messages,
    /// what only a person can do, said in the closing summary
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Secret {
    pub kind: SecretKind,
    /// what the person is asked, for `password` and `text`
    pub prompt: Option<String>,
    /// the key's comment, for `ssh-ed25519`
    pub comment: Option<String>,
    /// how many bytes, for `random`
    #[serde(default = "Secret::default_bytes")]
    pub bytes: usize,
}

impl Secret {
    const fn default_bytes() -> usize {
        32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretKind {
    Password,
    SshEd25519,
    Random,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirstSwitch {
    /// `NIXOS_INSTALL_BOOTLOADER=1` on the first activation
    pub install_bootloader: bool,
}

/// what the commits and the pull request say
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Messages {
    /// the enroll commit's subject
    pub enroll: String,
    /// the rekey commit's subject
    pub rekey: String,
    /// the pull request's title
    pub title: String,
    /// the pull request's body above coeus's own section
    pub body: String,
}

/// `recipients path` for every path asked about at once: the keys that must
/// open it, or `None` when it is not a secret coeus manages
pub type Recipients = BTreeMap<String, Option<Vec<String>>>;
