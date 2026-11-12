//! who an age file is encrypted to, read without any key:
//!
//! ```text
//! age-encryption.org/v1
//! -> ssh-ed25519 <tag> <share>      one stanza per recipient
//! <body>
//! -> X25519 <share>
//! <body>
//! -> 8d-grease …                    random stanzas age adds; not recipients
//! --- <mac>
//! <payload>
//! ```
//!
//! an armored file is the same behind `-----BEGIN AGE ENCRYPTED FILE-----`.

use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use age::armor::ArmoredReader;
use eyre::{WrapErr, bail, eyre};

use crate::bootstrap::SshKey;

/// the first line of every age file
const VERSION: &str = "age-encryption.org/v1";
/// what an armored file starts with
const ARMOR: &[u8] = b"-----BEGIN AGE ENCRYPTED FILE-----";
/// a header longer than this is not one age wrote; it bounds what a broken
/// file makes coeus read
const LONGEST: u64 = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub stanzas: Vec<Stanza>,
    pub armored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stanza {
    /// `ssh-ed25519` and `ssh-rsa` name their recipient by
    /// [SshKey::age_tag]
    Ssh { tag: String },
    /// X25519 names nobody: only how many there are can be compared
    X25519,
    /// grease, and stanza types coeus does not know
    Other,
}

/// how a file stands against its recipients
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// the ssh tags equal the recipients' as sets, and no X25519 stanza
    /// is left over
    Current,
    /// `missing` recipients must be added, `extra` stanzas dropped
    Stale { missing: usize, extra: usize },
    /// not an age file coeus can read the header of
    Unreadable(String),
}

impl Header {
    /// reads up to the `---` line only; the payload is never read
    pub fn read(path: &Path) -> eyre::Result<Self> {
        let file =
            File::open(path).wrap_err_with(|| format!("failed to open {}", path.display()))?;
        Self::from_reader(file)
            .wrap_err_with(|| format!("failed to read the age header of {}", path.display()))
    }

    /// the header at the start of `input`, binary or armored
    pub fn from_reader(input: impl Read) -> eyre::Result<Self> {
        let mut input = BufReader::new(input);
        let armored = input.fill_buf()?.trim_ascii_start().starts_with(ARMOR);
        let stanzas = if armored {
            stanzas(BufReader::new(ArmoredReader::new(input).take(LONGEST)))?
        } else {
            stanzas(input.take(LONGEST))?
        };
        Ok(Self { stanzas, armored })
    }

    /// against the recipients the contract gives. duplicates on either side
    /// collapse, since `[ host ] ++ admins` may name a key twice
    pub fn verdict(&self, recipients: &[SshKey]) -> Verdict {
        let wanted: BTreeSet<String> = recipients.iter().map(SshKey::age_tag).collect();
        let named = self.tags();
        let x25519 = self
            .stanzas
            .iter()
            .filter(|stanza| **stanza == Stanza::X25519)
            .count();

        let missing = wanted.difference(&named).count();
        let extra = named.difference(&wanted).count() + x25519;
        if missing == 0 && extra == 0 {
            Verdict::Current
        } else {
            Verdict::Stale { missing, extra }
        }
    }

    /// whether `key` is among the recipients, to check an admin before any
    /// sudo prompt
    pub fn names(&self, key: &SshKey) -> bool {
        self.tags().contains(&key.age_tag())
    }

    fn tags(&self) -> BTreeSet<String> {
        self.stanzas
            .iter()
            .filter_map(|stanza| match stanza {
                Stanza::Ssh { tag } => Some(tag.clone()),
                Stanza::X25519 | Stanza::Other => None,
            })
            .collect()
    }
}

/// the stanzas of a binary header, up to its `---` line
fn stanzas(mut input: impl BufRead) -> eyre::Result<Vec<Stanza>> {
    let mut line = Vec::new();
    if next_line(&mut input, &mut line)? != Some(VERSION) {
        bail!("not an age file: it does not start with {VERSION}");
    }

    let mut stanzas = Vec::new();
    loop {
        let Some(text) = next_line(&mut input, &mut line)? else {
            bail!("the age header ends before its `---` line");
        };
        if text == "---" || text.starts_with("--- ") {
            return Ok(stanzas);
        }
        // a line that is neither a stanza's start nor the end is the body of
        // the stanza before it
        if let Some(args) = text.strip_prefix("-> ") {
            stanzas.push(Stanza::parse(args)?);
        } else if stanzas.is_empty() {
            bail!("the age header has a line outside any stanza");
        }
    }
}

/// the next line without its newline, or `None` at the end
fn next_line<'a>(input: &mut impl BufRead, line: &'a mut Vec<u8>) -> eyre::Result<Option<&'a str>> {
    line.clear();
    if input.read_until(b'\n', line)? == 0 {
        return Ok(None);
    }
    let bytes = line.strip_suffix(b"\n").unwrap_or(line);
    let text = std::str::from_utf8(bytes).map_err(|_| eyre!("the age header is not text"))?;
    Ok(Some(text))
}

impl Stanza {
    /// `<type> <args…>`, what follows `-> `
    fn parse(args: &str) -> eyre::Result<Self> {
        let mut args = args.split(' ');
        Ok(match args.next() {
            Some("ssh-ed25519" | "ssh-rsa") => {
                let tag = args.next().filter(|tag| !tag.is_empty());
                let tag = tag.ok_or_else(|| eyre!("an ssh stanza of the age header has no tag"))?;
                Self::Ssh {
                    tag: tag.to_owned(),
                }
            }
            Some("X25519") => Self::X25519,
            _ => Self::Other,
        })
    }
}
