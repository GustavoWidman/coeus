mod act;
mod screen;
mod tail;

use std::{collections::HashMap, io::IsTerminal, time::Duration};

use clap::Args;
use crossterm::{
    event::{Event, EventStream, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use eyre::{Result, bail};
use futures_util::StreamExt;
use tokio::{
    sync::mpsc,
    time::{MissedTickBehavior, interval},
};

use self::{screen::Fullscreen, tail::Follower};
use crate::{
    cli::{
        Context,
        attach::{Attach, Mode},
        exit::Exit,
    },
    client::Fleet,
    common::model::{Hostname, JobSelector, Timestamp},
    ui::{
        say,
        top::{self, Effect, Top},
    },
};

#[derive(Args, Debug)]
pub struct TopArgs {
    /// Machines to show [default: @all]
    #[arg(value_name = "TARGETS")]
    pub targets: Vec<String>,
}

impl TopArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        if !context.live || !std::io::stdin().is_terminal() {
            bail!("top needs a terminal; `coeus watch` and `coeus status` work anywhere");
        }

        let fleet = context.fleet(context.targets_or_all(&self.targets)?)?;
        let tags = context
            .settings
            .hosts
            .iter()
            .map(|(host, entry)| (host.clone(), entry.tags.clone()))
            .collect();

        Dashboard::new(fleet, tags).run().await?;
        Ok(Exit::Success)
    }
}

/// the dashboard on the terminal: feeds the fleet's views, the followed
/// log, the keyboard and the machines' answers to [Top], and performs what
/// it asks for
struct Dashboard {
    fleet: Fleet,
    top: Top,
}

impl Dashboard {
    const TICK: Duration = Duration::from_millis(100);

    fn new(fleet: Fleet, tags: HashMap<Hostname, Vec<String>>) -> Self {
        let top = Top::new(
            fleet.hosts(),
            tags,
            env!("CARGO_PKG_VERSION"),
            Timestamp::now(),
        );
        Self { fleet, top }
    }

    async fn run(mut self) -> Result<()> {
        // log lines would tear through the alternate screen
        let level = log::max_level();
        log::set_max_level(log::LevelFilter::Off);
        let ran = self.show().await;
        log::set_max_level(level);
        ran
    }

    async fn show(&mut self) -> Result<()> {
        let mut views = self.fleet.watch();
        let (mut follower, mut batches) = Follower::new(self.fleet.clone());
        let (answers, mut answered) = mpsc::unbounded_channel();
        let mut ticks = interval(Self::TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut screen = Fullscreen::enter()?;
        let mut keys = EventStream::new();

        loop {
            if let Some(retarget) = self.top.retarget() {
                follower.retarget(retarget);
            }
            let now = Timestamp::now();
            if self.top.frame_due(now) {
                screen.draw(|frame| top::draw(frame, &self.top, now))?;
                self.top.drawn(now);
            }

            tokio::select! {
                Some(event) = views.recv() => self.top.apply(event),
                Some(batch) = batches.recv() => {
                    self.top.tail_lines(&batch.host, batch.job, batch.lines, batch.ended);
                }
                Some(answer) = answered.recv() => self.top.notify(answer, Timestamp::now()),
                event = keys.next() => {
                    let effect = match event {
                        Some(Ok(Event::Key(key))) => self.top.key(key, Timestamp::now()),
                        Some(Ok(Event::Resize(..))) => {
                            self.top.touch();
                            None
                        }
                        Some(Ok(_)) => None,
                        Some(Err(error)) => return Err(error.into()),
                        None => return Ok(()),
                    };

                    match effect {
                        Some(Effect::Quit) => return Ok(()),
                        Some(Effect::Attach(host)) => {
                            screen.suspend()?;
                            self.attach(&host, &mut keys).await?;
                            screen.resume()?;
                            self.top.touch();
                        }
                        Some(Effect::Run(request)) => {
                            self.top.notify(say::info(request.pending()), Timestamp::now());
                            let (fleet, answers) = (self.fleet.clone(), answers.clone());
                            tokio::spawn(async move {
                                let _ = answers.send(act::perform(fleet, request).await);
                            });
                        }
                        None => {}
                    }
                }
                _ = ticks.tick() => {}
            }
        }
    }

    /// the machine's current job, drawn by its nom on the normal screen. once
    /// the job ends its outcome stays up until a key is pressed; a ctrl-c
    /// detach comes straight back.
    async fn attach(&self, host: &Hostname, keys: &mut EventStream) -> Result<()> {
        let attached = Attach {
            fleet: &self.fleet,
            host,
            job: JobSelector::Current,
            mode: Mode::Live,
            idle: None,
            diff_limit: Some(12),
        }
        .run()
        .await;

        let linger = match attached {
            Ok(Some(attached)) => attached.outcome.is_some(),
            Ok(None) => true,
            Err(error) => {
                println!("{}", say::error(format!("{error:#}")));
                true
            }
        };
        if !linger {
            return Ok(());
        }

        println!("\n{}", say::aside("press any key to go back to coeus top"));
        enable_raw_mode()?;
        while let Some(event) = keys.next().await {
            if let Event::Key(key) = event?
                && key.kind == KeyEventKind::Press
            {
                break;
            }
        }
        disable_raw_mode()?;
        Ok(())
    }
}
