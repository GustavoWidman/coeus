use std::{future::pending, time::Duration};

use colored::Colorize;
use eyre::Result;
use tokio::time::{MissedTickBehavior, interval};

use super::dispatch::Dispatch;
use crate::{
    client::Fleet,
    common::model::Timestamp,
    ui::{Board, Frame, Plain, Screen, status, text::fit},
};

/// what a live terminal shows of the board
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// a row per machine with its job's progress
    Board,
    /// the status table
    Status,
}

/// follows the fleet until every ordered machine settles, or until ctrl-c
pub struct Follow {
    pub board: Board,
    out: Out,
}

enum Out {
    Live { screen: Screen, view: View },
    Plain(Plain),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Settled,
    Detached,
}

impl Follow {
    const TICK: Duration = Duration::from_millis(100);

    pub fn new(board: Board, live: bool, view: View) -> Result<Self> {
        let out = match live {
            true => Out::Live {
                screen: Screen::new()?,
                view,
            },
            false => Out::Plain(Plain::default()),
        };
        Ok(Self { board, out })
    }

    pub async fn run(&mut self, fleet: &Fleet, mut dispatch: Option<Dispatch>) -> Result<Ending> {
        let mut views = fleet.watch();
        let mut ticks = interval(Self::TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let interrupt = tokio::signal::ctrl_c();
        tokio::pin!(interrupt);

        if let Some(dispatch) = &dispatch {
            dispatch.mark(&mut self.board);
        }

        let ending = loop {
            if let Some(dispatch) = &mut dispatch {
                dispatch.step(&mut self.board);
            }
            if self.board.is_settled() {
                break Ending::Settled;
            }

            let placed = async {
                match &mut dispatch {
                    Some(dispatch) => dispatch.placed.recv().await,
                    None => pending().await,
                }
            };

            tokio::select! {
                _ = &mut interrupt => break Ending::Detached,
                Some(event) = views.recv() => {
                    self.board.apply(event);
                    if matches!(self.out, Out::Plain(_)) {
                        self.draw()?;
                    }
                }
                Some((host, ordering)) = placed => self.board.order(&host, ordering),
                _ = ticks.tick() => self.draw()?,
            }
        };

        self.draw()?;
        Ok(ending)
    }

    fn draw(&mut self) -> Result<()> {
        let now = Timestamp::now();
        match &mut self.out {
            Out::Live { screen, view } => {
                let (width, height) = Screen::size();
                let frame = Frame { now, width, height };
                let lines = match view {
                    View::Board => self.board.lines(frame),
                    View::Status => status_lines(&self.board, frame),
                };
                screen.draw(&lines)?;
            }
            Out::Plain(plain) => {
                for line in plain.observe(&self.board, now) {
                    println!("{line}");
                }
            }
        }
        Ok(())
    }
}

fn status_lines(board: &Board, frame: Frame) -> Vec<String> {
    let mut lines = vec![
        format!("{} {}", "coeus".bold(), "status".bold()),
        String::new(),
    ];
    lines.extend(status::lines(board, frame.now));
    lines.push(String::new());
    lines.push("ctrl-c stops watching".dimmed().to_string());
    lines.truncate(frame.height.saturating_sub(1).max(1));

    lines
        .iter()
        .map(|line| fit(line, frame.width).into_owned())
        .collect()
}
