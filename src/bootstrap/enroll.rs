//! the enroll step: the contract's questions, the machine's files and its
//! own secrets, committed once on the branch. found done by the enroll
//! commit's trailer.

use std::path::Path;

use eyre::{Result, WrapErr, bail, eyre};
use zeroize::Zeroizing;

use super::{
    Blocker, Bootstrap, SshKey, Step,
    branch::{self, Branch},
    contract::{self, Contract, Enrollment, Generator, Input, Question, RepoPath},
    remote::{Facts, Ssh},
    run::Shown,
    secrets,
    world::Machine,
};
use crate::{
    cli::{Console, Unanswerable},
    common::model::Hostname,
    ui::{
        ask::{Answer, Prompt},
        checklist::Mark,
    },
};

/// the machine on the branch, with what the contract said about it
pub struct Enrolled {
    pub host: Hostname,
    pub branch: Branch,
    pub contract: Contract,
    pub enrollment: Enrollment,
    pub host_key: SshKey,
    pub facts: Facts,
}

pub async fn enroll<M: Machine>(
    bootstrap: &mut Bootstrap,
    machine: &M,
    nix_version: Option<String>,
    console: &Console,
) -> Result<Shown<Enrolled>> {
    let host = bootstrap.host.clone();
    let repo = bootstrap.saved.repo.clone().ok_or_else(|| {
        eyre!("coeus does not know the configuration repository: say where with --repo")
    })?;
    console.mark(
        Step::Enroll,
        Mark::Active,
        "fetching origin into the bootstrap's clone",
    );
    let branch = Branch::open(
        &bootstrap.state,
        &repo,
        &host,
        bootstrap.saved.base.as_deref(),
    )
    .await?;
    let contract = Contract::open(branch.clone_dir(), Path::new(contract::DEFAULT_PATH)).await?;
    let found = branch.enrolled().await?;

    let host_key = Ssh::pinned(&bootstrap.state)?.ok_or_else(|| {
        eyre!(
            "no host key is pinned for {host}: `coeus bootstrap {host} --to <destination>` reads it"
        )
    })?;
    let facts = match (&found, &bootstrap.saved.facts) {
        (Some(_), Some(facts)) => facts.clone(),
        _ => {
            let mut facts = machine.probed(bootstrap, console).await?.facts;
            if nix_version.is_some() {
                facts.nix_version = nix_version;
            }
            bootstrap.saved.facts = Some(facts.clone());
            bootstrap.state.save(&bootstrap.saved)?;
            facts
        }
    };

    let mut input = Input {
        name: host.clone(),
        host_key: host_key.clone(),
        facts,
        answers: bootstrap.saved.answers.clone(),
    };
    ask(&contract, &mut input, bootstrap, console).await?;
    let enrollment = contract.enroll(&input).await?;

    let (commit, mark) = match found {
        Some(commit) => (commit, Mark::Kept),
        None => {
            let commit = commit(&branch, &contract, &enrollment, &host, console).await?;
            (commit, Mark::Done)
        }
    };
    Ok(Shown {
        detail: summary(&input.answers, &commit),
        mark,
        value: Enrolled {
            host,
            branch,
            contract,
            enrollment,
            host_key,
            facts: input.facts,
        },
    })
}

/// the files and the machine's own secrets written, and committed with the
/// enroll trailer. a file the default branch has already means the machine
/// is enrolled there.
async fn commit(
    branch: &Branch,
    contract: &Contract,
    enrollment: &Enrollment,
    host: &Hostname,
    console: &Console,
) -> Result<branch::Enrolled> {
    let declared: Vec<RepoPath> = enrollment
        .files
        .keys()
        .chain(enrollment.secrets.keys())
        .cloned()
        .collect();
    let enrolled_there = branch.on_base(&declared.iter().collect::<Vec<_>>()).await?;
    if !enrolled_there.is_empty() {
        return Err(Blocker::Enrolled {
            files: enrolled_there,
        }
        .into());
    }

    console.mark(
        Step::Enroll,
        Mark::Active,
        format!("writing {host}'s files"),
    );
    branch.write(&enrollment.files).await?;
    make_secrets(branch, contract, enrollment, host, console).await?;
    branch
        .commit_enroll(&enrollment.messages.enroll, &declared)
        .await
}

/// each question the contract asks and the saved answers do not hold, or
/// an `--answer` changes, asked in turn and saved at once; the questions
/// are evaluated again after each answer, since one may depend on another
async fn ask(
    contract: &Contract,
    input: &mut Input,
    bootstrap: &mut Bootstrap,
    console: &Console,
) -> Result<()> {
    let given = bootstrap.options.answers.clone();
    loop {
        let questions = contract.questions(input).await?;
        let next = questions.iter().find_map(|question| {
            let saved = input.answers.get(&question.id);
            match given.get(&question.id) {
                Some(raw) => match question.accept(raw) {
                    Ok(value) if saved == Some(value.as_str()) => None,
                    Ok(value) => Some(Ok((question, value))),
                    Err(why) => Some(Err(eyre!("--answer {}: {why}", question.id))),
                },
                None if saved.is_none() => Some(Ok((question, String::new()))),
                None => None,
            }
        });
        let Some(next) = next else {
            if let Some(id) = given
                .keys()
                .find(|id| !questions.iter().any(|question| &question.id == *id))
            {
                bail!("the repository asks no question `{id}`, which --answer answers");
            }
            return Ok(());
        };

        let (question, value) = next?;
        let value = match given.contains_key(&question.id) {
            true => value,
            false => answer(question, console).await?,
        };
        input.answers.insert(question.id.clone(), value);
        bootstrap.saved.answers = input.answers.clone();
        bootstrap.state.save(&bootstrap.saved)?;
    }
}

/// one question asked under the enroll row, until an answer is accepted
async fn answer(question: &Question, console: &Console) -> Result<String> {
    let mut prompt = Prompt::question(question)?;
    loop {
        let answer = console.ask(prompt.clone()).await.map_err(|error| {
            match error.downcast_ref::<Unanswerable>() {
                Some(_) => Blocker::Unanswered {
                    id: question.id.clone(),
                }
                .into(),
                None => error,
            }
        })?;
        match answer {
            Answer::Value(raw) => match question.accept(&raw) {
                Ok(value) => return Ok(value),
                Err(why) => prompt.error = Some(why),
            },
            Answer::Secret(_) => {
                return Err(eyre!("question {} answered with a secret", question.id));
            }
            Answer::Cancelled => {
                return Err(eyre!(
                    "cancelled while the repository asked {}",
                    question.id
                ));
            }
        }
    }
}

/// the machine's own secrets that are not there yet, encrypted to their
/// recipients straight from memory. the files are written first, since the
/// recipients of a new machine's secret may name its own key.
async fn make_secrets(
    branch: &Branch,
    contract: &Contract,
    enrollment: &Enrollment,
    host: &Hostname,
    console: &Console,
) -> Result<()> {
    let missing: Vec<(&RepoPath, &Generator)> = enrollment
        .secrets
        .iter()
        .filter(|(path, _)| branch.path(path).symlink_metadata().is_err())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let paths: Vec<RepoPath> = missing.iter().map(|(path, _)| (*path).clone()).collect();
    let rules = contract.recipients(&paths).await?;

    for (path, generator) in missing {
        let recipients = rules
            .get(path)
            .cloned()
            .flatten()
            .ok_or_else(|| eyre!("the contract names nobody who opens {path}"))?;
        console.mark(Step::Enroll, Mark::Active, format!("making {path}"));
        let typed = typed(path, generator, host, console).await?;
        let made = secrets::generate(generator, typed)?;
        let file = branch.path(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        secrets::create(&file, &made.plaintext, &recipients)
            .wrap_err_with(|| format!("could not make {path}"))?;
    }
    Ok(())
}

/// what a person types for a password or a text secret
async fn typed(
    path: &RepoPath,
    generator: &Generator,
    host: &Hostname,
    console: &Console,
) -> Result<Option<Zeroizing<String>>> {
    let (prompt, twice) = match generator {
        Generator::Password { prompt } => (prompt, true),
        Generator::Text { prompt } => (prompt, false),
        Generator::SshEd25519 { .. } | Generator::Random { .. } => return Ok(None),
    };
    let question = prompt
        .clone()
        .unwrap_or_else(|| format!("{path} for {host}"));
    let answer = console
        .ask(Prompt::secret(question, twice))
        .await
        .map_err(|error| match error.downcast_ref::<Unanswerable>() {
            Some(_) => Blocker::NeedsTerminal {
                secret: path.clone(),
            }
            .into(),
            None => error,
        })?;
    match answer {
        Answer::Secret(typed) => Ok(Some(typed)),
        Answer::Value(text) => Ok(Some(Zeroizing::new(text))),
        Answer::Cancelled => Err(eyre!("cancelled while asking for {path}")),
    }
}

/// `server · 4 files (a1b2c3d)`: the answers, then the commit
fn summary(answers: &contract::Answers, commit: &branch::Enrolled) -> String {
    let mut parts: Vec<String> = answers
        .iter()
        .filter(|(_, value)| !matches!(*value, "true" | "false"))
        .map(|(_, value)| value.to_owned())
        .collect();
    let files = commit.files.iter().filter(|path| !path.is_secret()).count();
    let secrets = commit.files.len() - files;
    parts.push(format!(
        "{} and {} ({})",
        count(files, "file"),
        count(secrets, "secret"),
        commit.commit.short()
    ));
    parts.join(" · ")
}

pub fn count(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        count => format!("{count} {noun}s"),
    }
}
