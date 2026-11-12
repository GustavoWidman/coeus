use std::io::Write;

use colored::Colorize;
use eyre::Result;
use futures_util::future::join_all;

use super::{
    attach::{Attach, Mode},
    dispatch::{Answer, Dispatch, Shipment},
    exit::Exit,
    follow::{Ending, Follow, View},
    notify::notify,
    rollout::Rollout,
};
use crate::{
    client::Fleet,
    common::model::{Action, Admission, Hostname, JobSelector, Placement, Timestamp},
    nix::strip_ansi,
    ui::{Board, Heading, Summary, Theme, board::Verdict, describe, diff, say},
};

/// one order, sent to its machines and followed the way the command asked
pub struct Deployment {
    pub fleet: Fleet,
    pub shipment: Shipment,
    pub rollout: Rollout,
    pub following: Following,
    pub notify: bool,
    pub skip_unreachable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Following {
    /// print each machine's answer and leave
    Detach,
    /// one machine: its job's log through nom
    Attach,
    /// the board, live on a terminal, line by line elsewhere
    Board { live: bool },
}

impl Deployment {
    pub async fn run(self) -> Result<Exit> {
        match self.following {
            Following::Detach => Ok(self.detach().await),
            Following::Attach => self.attach().await,
            Following::Board { live } => self.follow(live).await,
        }
    }

    async fn detach(self) -> Exit {
        let (fleet, shipment) = (&self.fleet, &self.shipment);
        let placements = join_all(
            fleet
                .hosts()
                .iter()
                .map(|host| async move { (host, shipment.place(fleet, host).await) }),
        )
        .await;

        let verdicts = placements.iter().map(|(host, answer)| {
            println!("{}", placement(host, answer));
            match answer {
                Ok(Ok(_)) => Verdict::Done,
                Ok(Err(_)) => Verdict::Rejected,
                Err(_) => Verdict::Unreachable,
            }
        });
        let exit = Exit::of(verdicts.collect::<Vec<_>>(), self.skip_unreachable);

        println!(
            "{}",
            say::aside(format!("follow along with `coeus watch {}`", self.hosts()).dimmed())
        );
        exit
    }

    async fn attach(self) -> Result<Exit> {
        let host = &self.fleet.hosts()[0];
        let answer = self.shipment.place(&self.fleet, host).await;
        println!("{}", placement(host, &answer));

        let admission = match answer {
            Ok(Ok(admission)) => admission,
            Ok(Err(_)) => return Ok(Exit::Unreached),
            Err(_) => return Ok(Exit::of([Verdict::Unreachable], self.skip_unreachable)),
        };

        let attached = Attach {
            fleet: &self.fleet,
            host,
            job: JobSelector::Id(admission.job),
            mode: Mode::Nom,
            idle: None,
            diff_limit: self.diff_limit(),
        }
        .run()
        .await?;

        let Some(outcome) = attached.and_then(|attached| attached.outcome) else {
            return Ok(Exit::Detached);
        };

        if self.notify {
            notify(
                &format!("coeus {}", self.shipment.order.action),
                &format!("{host}: {}", Theme::outcome_label(&outcome)),
            );
        }
        Ok(match outcome.is_success() {
            true => Exit::Success,
            false => Exit::Failed,
        })
    }

    async fn follow(self, live: bool) -> Result<Exit> {
        let heading = Heading::Order {
            id: self.shipment.order.id,
            source: self.shipment.order.source.clone(),
            action: self.shipment.order.action,
            since: Timestamp::now(),
        };
        let board = Board::new(heading, self.fleet.hosts());
        let dispatch = Dispatch::new(
            self.fleet.clone(),
            self.shipment.clone(),
            self.rollout.clone(),
        );

        let mut follow = Follow::new(board, live, View::Board)?;
        let ending = follow.run(&self.fleet, Some(dispatch)).await?;
        let board = follow.board;

        if ending == Ending::Detached {
            println!(
                "\n{}",
                say::aside(format!(
                    "detached, the deployments keep running. follow again with `coeus watch {}`",
                    self.hosts()
                ))
            );
            return Ok(Exit::Detached);
        }

        let summary = Summary::new(&board, Timestamp::now());
        println!();
        match live {
            true => println!("{}", summary.verdict()),
            false => summary.lines().iter().for_each(|line| println!("{line}")),
        }
        step_summary(&summary);

        if self.shipment.order.action == Action::Build {
            self.previews(&board).await;
        }
        if self.notify {
            let verdict = summary.verdict();
            let verdict = strip_ansi(&verdict);
            notify(
                &format!("coeus {}", self.shipment.order.action),
                verdict.trim_start_matches(['✓', '✗', ' ']),
            );
        }

        Ok(Exit::of(board.verdicts(), self.skip_unreachable))
    }

    /// the closure diff of every machine that built, as a build preview
    async fn previews(&self, board: &Board) {
        for row in &board.rows {
            let (Some(Verdict::Done), Some(job)) = (row.verdict(), row.summary()) else {
                continue;
            };

            let attached = Attach {
                fleet: &self.fleet,
                host: &row.host,
                job: JobSelector::Id(job.id),
                mode: Mode::Silent,
                idle: None,
                diff_limit: None,
            }
            .run()
            .await;

            println!(
                "\n{} {}",
                Theme::host(row.host.as_str()),
                format!("job {}", job.id).dimmed()
            );
            match attached {
                Ok(Some(attached)) => match attached.diff {
                    Some(changes) => diff::lines(&changes, None)
                        .iter()
                        .for_each(|line| println!("  {line}")),
                    None => println!("  {}", "no closure diff recorded".dimmed()),
                },
                Ok(None) => println!("  {}", "the job's log ended early".dimmed()),
                Err(error) => println!(
                    "  {}",
                    say::warn(format!("could not read the diff: {error}"))
                ),
            }
        }
    }

    const fn diff_limit(&self) -> Option<usize> {
        match self.shipment.order.action {
            Action::Build => None,
            _ => Some(12),
        }
    }

    fn hosts(&self) -> String {
        self.fleet
            .hosts()
            .iter()
            .map(Hostname::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `[✓] altar took the order as job #12, running now`
fn placement(host: &Hostname, answer: &Answer) -> String {
    let host_name = Theme::host(host.as_str());
    match answer {
        Ok(Ok(Admission { job, placement })) => {
            let job = format!("job {job}").bold();
            match placement {
                Placement::Started => say::success(format!("{host_name} runs the order as {job}")),
                Placement::Queued { behind, replaced } => {
                    let replaced = replaced
                        .map(|replaced| format!(", replacing {replaced}"))
                        .unwrap_or_default();
                    say::success(format!(
                        "{host_name} queued {job} behind {behind}{replaced}"
                    ))
                }
                Placement::Preempted { replaced } => say::success(format!(
                    "{host_name} runs the order as {job}, replacing {replaced}"
                )),
                Placement::Duplicate => {
                    say::aside(format!("{host_name} already took this order as {job}"))
                }
            }
        }
        Ok(Err(rejection)) => say::error(format!(
            "{host_name} rejected the order: {}",
            describe::rejection(rejection)
        )),
        Err(reason) => say::error(format!("{host_name} is unreachable: {reason}")),
    }
}

/// appends the summary to the GitHub Actions job page, when running there
fn step_summary(summary: &Summary<'_>) {
    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return;
    };

    let written = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .and_then(|mut file| file.write_all(summary.markdown().as_bytes()));
    if let Err(error) = written {
        eprintln!(
            "{}",
            say::warn(format!("could not write the step summary: {error}"))
        );
    }
}
