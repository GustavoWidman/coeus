use tokio::sync::mpsc;

use super::rollout::{Rollout, Step};
use crate::{
    client::Fleet,
    common::model::{Admission, Hostname, Order, Rejection},
    ui::{Board, board::Ordering},
};

/// sends one order to the fleet wave by wave, as the rollout allows
pub struct Dispatch {
    fleet: Fleet,
    order: Order,
    rollout: Rollout,
    sender: mpsc::UnboundedSender<(Hostname, Ordering)>,
    pub placed: mpsc::UnboundedReceiver<(Hostname, Ordering)>,
}

impl Dispatch {
    pub fn new(fleet: Fleet, order: Order, rollout: Rollout) -> Self {
        let (sender, placed) = mpsc::unbounded_channel();
        Self {
            fleet,
            order,
            rollout,
            sender,
            placed,
        }
    }

    /// numbers every machine's wave on the board
    pub fn mark(&self, board: &mut Board) {
        for (at, wave) in self.rollout.waves().iter().enumerate() {
            for host in wave {
                board.order(host, Ordering::Pending { wave: at + 1 });
            }
        }
    }

    /// orders the next wave once the board allows it
    pub fn step(&mut self, board: &mut Board) {
        match self.rollout.advance(board) {
            Step::Order { hosts, .. } => {
                for host in hosts {
                    board.order(&host, Ordering::Sending);
                    let (fleet, order, sender) =
                        (self.fleet.clone(), self.order.clone(), self.sender.clone());
                    tokio::spawn(async move {
                        let answer = place(&fleet, &host, order).await;
                        let _ = sender.send((host, ordering(answer)));
                    });
                }
            }
            Step::Stop(hosts) => {
                for host in hosts {
                    board.order(&host, Ordering::Held);
                }
            }
            Step::Wait | Step::Done => {}
        }
    }
}

/// a machine's answer to an order: taken, refused, or never heard
pub type Answer = Result<Result<Admission, Rejection>, String>;

pub async fn place(fleet: &Fleet, host: &Hostname, order: Order) -> Answer {
    let mut client = fleet
        .connect(host)
        .await
        .map_err(|error| error.to_string())?;
    client
        .deploy(order)
        .await
        .map_err(|error| error.to_string())
}

pub fn ordering(answer: Answer) -> Ordering {
    match answer {
        Ok(Ok(admission)) => Ordering::Admitted(admission),
        Ok(Err(rejection)) => Ordering::Rejected(rejection),
        Err(reason) => Ordering::Unreachable(reason),
    }
}
