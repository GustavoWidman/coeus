//! how commands become root on the machine. before the session key is
//! installed, a stock user needs sudo, maybe with a password: asked once per
//! run in the checklist, kept in memory, written to `sudo -S` on each call
//! and never saved.

use eyre::{Result, bail, eyre};
use zeroize::Zeroizing;

use super::{Call, Ssh, exec};
use crate::{
    bootstrap::Step,
    cli::Console,
    ui::{
        ask::{Answer, Prompt},
        checklist::Mark,
    },
};

pub enum Privilege {
    /// root already: the session key, or a root login
    Root,
    /// `sudo -n` works
    Sudo,
    /// `sudo -S -p ''`, fed this password
    SudoPassword(Zeroizing<String>),
}

impl Privilege {
    /// how many passwords sudo gets before the run gives up
    const TRIES: usize = 3;

    /// finds out which, asking for the password only when sudo needs one,
    /// and checking it with `sudo -S -v` before keeping it
    pub async fn establish(ssh: &Ssh, login: &str, console: &Console) -> Result<Self> {
        if ssh.is_root() || login == "root" {
            return Ok(Self::Root);
        }
        if exec::raw(ssh, &words(&["sudo", "-n", "true"]), None)
            .await?
            .status
            .success()
        {
            return Ok(Self::Sudo);
        }

        let mut error = None;
        for _ in 0..Self::TRIES {
            console.mark(
                Step::Reach,
                Mark::Active,
                format!("sudo asks for {login}'s password"),
            );
            let mut prompt = Prompt::secret(
                format!("sudo password for {login}; kept in memory until this run ends"),
                false,
            );
            prompt.error = error.take();
            let password = match console.ask(prompt).await? {
                Answer::Secret(password) => password,
                Answer::Value(text) => Zeroizing::new(text),
                Answer::Cancelled => {
                    return Err(eyre!("cancelled while sudo asked for a password"));
                }
            };

            let privilege = Self::SudoPassword(password);
            let check = words(&["sudo", "-k", "-S", "-p", "", "-v"]);
            let output = exec::raw(ssh, &check, privilege.password()).await?;
            if output.status.success() {
                return Ok(privilege);
            }
            if Ssh::lost(output.status) {
                bail!("the connection dropped while checking the sudo password");
            }
            error = Some("sudo refused it; try again".into());
        }
        Err(eyre!(
            "sudo refused {login}'s password {} times",
            Self::TRIES
        ))
    }

    /// `call`'s argv as root, and the line to write first on its stdin
    pub fn wrap(&self, call: &Call) -> (Vec<String>, Option<&str>) {
        match call.privileged() {
            true => (self.prefix(call.argv()), self.password()),
            false => (call.argv(), None),
        }
    }

    fn password(&self) -> Option<&str> {
        match self {
            Self::SudoPassword(password) => Some(password.as_str()),
            _ => None,
        }
    }

    fn prefix(&self, argv: Vec<String>) -> Vec<String> {
        // `-k` makes sudo read the password even with a cached ticket, so
        // the line written for it is never left for the command to read
        let sudo: &[&str] = match self {
            Self::Root => &[],
            Self::Sudo => &["sudo", "-H", "-n", "--"],
            Self::SudoPassword(_) => &["sudo", "-H", "-k", "-S", "-p", "", "--"],
        };
        words(sudo).into_iter().chain(argv).collect()
    }
}

fn words(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}
