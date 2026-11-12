//! whole runs over a fake machine and a fake forge, with a real git origin,
//! a real contract evaluated by nix and real age secrets; skipped where
//! there is no nix

mod fake;
mod kill;
mod runs;

use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};
use tempfile::TempDir;

use fake::{FakeForge, FakeMachine, Kill, Pulls, World};

use crate::{
    bootstrap::{
        Bootstrap, Finale, Options, SshKey, Step,
        branch::Branch,
        remote,
        secrets::{self, Header},
        state::{Saved, StateDir},
    },
    cli::Console,
    common::model::Hostname,
    ui::checklist::{self, Checklist, Mark},
};

const CONTRACT: &str = include_str!("contract.nix");

fn has_nix() -> bool {
    let found = Command::new("nix")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipped: no nix on PATH");
    }
    found
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn host() -> Hostname {
    "nova".parse().unwrap()
}

/// an ed25519 key pair: the private key's text and the public line
fn keypair(comment: &str) -> (String, SshKey, String) {
    let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
    key.set_comment(comment);
    let line = key.public_key().to_openssh().unwrap();
    (
        key.to_openssh(LineEnding::LF).unwrap().to_string(),
        SshKey::parse(&line).unwrap(),
        line,
    )
}

/// a fleet of two machines, `altar` (the admin this test runs as) and
/// `old`, on a bare origin; the person's checkout of it; and nova, a stock
/// NixOS machine, as a fake that outlives every run
struct Fleet {
    _dir: TempDir,
    root: PathBuf,
    origin: PathBuf,
    checkout: PathBuf,
    /// altar's private host key, which opens every secret
    identity: PathBuf,
    admin: SshKey,
    old: SshKey,
    nova: SshKey,
    world: Arc<Mutex<World>>,
    pulls: Arc<Mutex<Pulls>>,
}

impl Fleet {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let origin = root.join("origin.git");
        let checkout = root.join("checkout");
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "checkout"]);

        let (private, admin, admin_line) = keypair("root@altar");
        let identity = root.join("altar").join("ssh_host_ed25519_key");
        std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
        std::fs::write(&identity, private).unwrap();
        std::fs::write(identity.with_extension("pub"), format!("{admin_line}\n")).unwrap();
        let (_, old, old_line) = keypair("root@old");
        let (_, nova, _) = keypair("root@nova");

        let fleet = Self {
            _dir: dir,
            world: Arc::new(Mutex::new(World::new(remote::test::nixos(), nova.clone()))),
            pulls: Arc::default(),
            root,
            origin,
            checkout,
            identity,
            admin,
            old,
            nova,
        };
        fleet.commit_on_main(&[
            (".coeus/bootstrap.nix", CONTRACT),
            ("hosts/altar/host.pub", &format!("{admin_line}\n")),
            ("hosts/altar/admin", ""),
            ("hosts/old/host.pub", &format!("{old_line}\n")),
        ]);
        let everyone = [fleet.admin.clone(), fleet.old.clone()];
        fleet.seal_on_main("secrets/shared.age", &everyone);
        fleet.seal_on_main("hosts/old/token.age", &everyone);
        fleet.prepare().await;
        fleet
    }

    fn commit_on_main(&self, files: &[(&str, &str)]) {
        for (name, text) in files {
            let file = self.checkout.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        git(&self.checkout, &["add", "--all"]);
        git(
            &self.checkout,
            &["commit", "--quiet", "-m", "chore: move main"],
        );
        git(&self.checkout, &["push", "--quiet", "origin", "HEAD:main"]);
    }

    /// a secret for `recipients`, committed on main
    fn seal_on_main(&self, path: &str, recipients: &[SshKey]) {
        let file = self.checkout.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        secrets::create(&file, b"sealed", recipients).unwrap();
        self.commit_on_main(&[]);
    }

    fn state(&self) -> StateDir {
        StateDir::under(&self.root.join("state"), &host())
    }

    /// the private clone made once, committing as the test without
    /// signing, before any run opens it
    async fn prepare(&self) {
        let branch = Branch::open(&self.state(), &self.checkout, &host(), None)
            .await
            .unwrap();
        let clone = branch.clone_dir();
        git(clone, &["config", "user.name", "altar"]);
        git(clone, &["config", "user.email", "altar@example.com"]);
        git(clone, &["config", "commit.gpgsign", "false"]);
    }

    fn bootstrap(&self, answers: &[(&str, &str)], retry: bool) -> Bootstrap {
        let state = self.state();
        let mut saved = state.load().unwrap();
        saved.destination = Some("nixos@192.168.1.40".into());
        saved.repo = Some(self.checkout.clone());
        Bootstrap {
            host: host(),
            settings: Default::default(),
            state,
            saved,
            options: Options {
                identity: Some(self.identity.clone()),
                host_key: None,
                retry,
                answers: answers
                    .iter()
                    .map(|(id, value)| ((*id).to_owned(), (*value).to_owned()))
                    .collect::<BTreeMap<_, _>>(),
            },
        }
    }

    /// one run, told every answer, stopped where `kill` says; `None` when
    /// it was stopped
    async fn run(&self, kill: &Kill) -> Option<Ran> {
        self.run_with(kill, ANSWERS, false).await
    }

    async fn run_with(&self, kill: &Kill, answers: &[(&str, &str)], retry: bool) -> Option<Ran> {
        self.run_polled(kill, answers, retry, None).await.0
    }

    /// a run dropped after `polls` of its polls found it waiting, when
    /// given; how many times it waited
    async fn run_polled(
        &self,
        kill: &Kill,
        answers: &[(&str, &str)],
        retry: bool,
        polls: Option<usize>,
    ) -> (Option<Ran>, usize) {
        let mut bootstrap = self.bootstrap(answers, retry);
        let checklist = Checklist::new(
            checklist::title(&host(), bootstrap.saved.destination.as_deref()),
            Vec::new(),
        );
        let console = Console::start(false, checklist).unwrap();
        let machine = FakeMachine {
            world: self.world.clone(),
            kill: kill.clone(),
        };
        let forge = FakeForge {
            pulls: self.pulls.clone(),
            kill: kill.clone(),
        };

        let mut run = StopAfter {
            inner: Box::pin(crate::bootstrap::run(
                &mut bootstrap,
                &machine,
                &forge,
                &console,
            )),
            polls,
            waited: 0,
        };
        let finale = tokio::select! {
            finale = &mut run => finale,
            () = kill.killed() => None,
        };
        let waited = run.waited;
        drop(run);
        let Some(finale) = finale else {
            return (None, waited);
        };
        let finale = finale.unwrap();
        let rows = console.checklist().await.unwrap();
        console.finish(&bootstrap.state, &finale).await.unwrap();
        let ran = Ran {
            finale,
            rows,
            saved: bootstrap.saved,
        };
        (Some(ran), waited)
    }

    fn origin_head(&self) -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.origin)
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                "refs/heads/coeus/bootstrap/nova",
            ])
            .output()
            .unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap().trim().to_owned())
    }

    fn clone(&self) -> PathBuf {
        self.state().clone_dir()
    }

    /// the subjects of the branch's own commits, oldest first
    fn subjects(&self) -> Vec<String> {
        let log = git(
            &self.clone(),
            &["log", "--reverse", "--format=%s", "origin/main..HEAD"],
        );
        log.lines().map(str::to_owned).collect()
    }

    fn opens(&self, path: &str, key: &SshKey) -> bool {
        Header::read(&self.clone().join(path)).unwrap().names(key)
    }

    /// everything a run that got everywhere leaves, however many runs and
    /// stops it took
    fn assert_in_the_fleet(&self, ran: &Ran) {
        let Finale::Done(evidence) = &ran.finale else {
            panic!("the run ended {:?}", ran.finale);
        };
        assert!(!evidence.pull_request.draft);
        assert!(
            evidence
                .pull_request
                .body
                .contains("### nova joins the fleet")
        );
        assert!(evidence.pull_request.body.contains("job #"));
        assert!(
            evidence
                .pull_request
                .body
                .starts_with("adds nova, a server")
        );

        let world = self.world.lock().unwrap();
        assert!(world.daemon);
        assert!(!world.authorized, "the session key is out of root's keys");
        assert!(!world.reboot_required);
        assert!(world.jobs.iter().all(|job| job.state.is_finished()));
        drop(world);
        assert!(!self.state().session_key().exists());

        assert_eq!(
            self.subjects(),
            [
                "feat(nova): join the fleet",
                "chore(secrets): rekey for nova"
            ]
        );
        assert_eq!(
            self.origin_head().as_deref(),
            Some(git(&self.clone(), &["rev-parse", "HEAD"]).as_str())
        );
        assert_eq!(git(&self.clone(), &["status", "--porcelain"]), "");
        assert!(self.opens("secrets/shared.age", &self.nova));
        assert!(self.opens("hosts/nova/token.age", &self.nova));
        assert!(self.opens("hosts/nova/ssh.age", &self.admin));
        assert!(!self.opens("hosts/old/token.age", &self.nova));
        assert_eq!(
            ran.saved
                .last_run
                .as_ref()
                .and_then(|last| last.ended.as_ref())
                .map(|ended| ended.how.as_str()),
            Some("in the fleet")
        );
        let pulls = self.pulls.lock().unwrap();
        assert_eq!(pulls.open.len(), 1, "one pull request, however many runs");
    }
}

impl Fleet {
    /// a refused switch, then one boot into the fleet's system and one
    /// reboot, which it runs now
    fn assert_booted_into_the_fleet(&self) {
        use crate::common::model::{Action, JobState, Outcome, Phase};

        let world = self.world.lock().unwrap();
        assert_eq!(world.jobs.len(), 2, "one refused switch, one boot");
        let JobState::Finished(Outcome::RolledBack(refused)) = &world.jobs[0].state else {
            panic!("{:?}", world.jobs[0].state);
        };
        assert_eq!(refused.phase, Phase::Activating);
        let boot = &world.jobs[1];
        assert_eq!(boot.action, Action::Boot);
        assert_eq!(boot.state, JobState::Finished(Outcome::Succeeded));
        assert_eq!(
            world.running, boot.toplevel,
            "it runs the boot job's system"
        );
        assert_eq!(world.reboots, 1, "rebooted once, however many runs");
    }
}

/// a future that ends with `None` once it was found waiting `polls` times
struct StopAfter<F> {
    inner: Pin<Box<F>>,
    polls: Option<usize>,
    waited: usize,
}

impl<F: Future> Future for StopAfter<F> {
    type Output = Option<F::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.polls == Some(self.waited) {
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll(cx) {
            Poll::Ready(output) => Poll::Ready(Some(output)),
            Poll::Pending => {
                self.waited += 1;
                if self.polls == Some(self.waited) {
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            }
        }
    }
}

const ANSWERS: &[(&str, &str)] = &[("type", "server"), ("public", "false")];

struct Ran {
    finale: Finale,
    rows: Checklist,
    saved: Saved,
}

impl Ran {
    fn mark(&self, step: Step) -> Mark {
        self.rows.row(step).map(|row| row.mark).unwrap()
    }

    fn marks(&self) -> Vec<(Step, Mark)> {
        Step::ALL
            .into_iter()
            .map(|step| (step, self.mark(step)))
            .collect()
    }
}
