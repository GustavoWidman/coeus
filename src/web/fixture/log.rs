//! a simulated job's log, written as a runner writes one: coeus's own
//! events beside nix's internal JSON

use serde_json::json;

use crate::common::model::{AttachFrame, JobEvent, Stamped, Timestamp};

/// the log, in the order it was written; a frame's index is its offset
#[derive(Clone, Debug, Default)]
pub struct Log {
    pub frames: Vec<AttachFrame>,
    /// the last nix activity started
    activity: u64,
}

pub const CACHE: &str = "https://cache.nixos.org";

impl Log {
    pub fn event(&mut self, at: i64, event: JobEvent) {
        self.frames.push(AttachFrame::Event(Stamped {
            at: Timestamp(at),
            event,
        }));
    }

    /// a line a program printed outside nix
    pub fn output(&mut self, text: &str) {
        self.frames
            .push(AttachFrame::Nix(format!("{text}\n").into_bytes()));
    }

    /// a message of nix's own; level 0 is an error, 3 information
    pub fn message(&mut self, level: u8, text: &str) {
        self.nix(json!({ "action": "msg", "level": level, "msg": text }));
    }

    /// nix fetches the flake's source
    pub fn fetch(&mut self, url: &str) {
        let id = self.start(112, &format!("fetching {url}"), json!([url]));
        self.stop(id);
    }

    /// nix copies a path from the binary cache
    pub fn copy(&mut self, path: &str) {
        let text = format!("copying path '{path}' from '{CACHE}'");
        let id = self.start(100, &text, json!([path, CACHE, "local"]));
        self.stop(id);
    }

    /// nix starts building a derivation; its builder's lines go to the id
    pub fn build(&mut self, drv: &str) -> u64 {
        self.start(105, &format!("building '{drv}'"), json!([drv, "", 1, 1]))
    }

    /// a line the builder of activity `id` printed
    pub fn builder(&mut self, id: u64, text: &str) {
        self.nix(json!({ "action": "result", "id": id, "type": 101, "fields": [text] }));
    }

    pub fn stop(&mut self, id: u64) {
        self.nix(json!({ "action": "stop", "id": id }));
    }

    /// spreads coeus's events evenly from `start` to `end`, nix's lines
    /// falling between them
    pub fn spread(&mut self, start: i64, end: i64) {
        let events = self
            .frames
            .iter()
            .filter(|frame| matches!(frame, AttachFrame::Event(_)))
            .count();
        let gaps = events.saturating_sub(1).max(1) as i64;
        let stamps = self.frames.iter_mut().filter_map(|frame| match frame {
            AttachFrame::Event(stamped) => Some(stamped),
            AttachFrame::Nix(_) => None,
        });
        for (at, stamped) in stamps.enumerate() {
            stamped.at = Timestamp(start + (end - start) * at as i64 / gaps);
        }
    }

    fn start(&mut self, activity: u64, text: &str, fields: serde_json::Value) -> u64 {
        self.activity += 1;
        let id = self.activity;
        self.nix(json!({
            "action": "start",
            "id": id,
            "level": 3,
            "parent": 0,
            "type": activity,
            "text": text,
            "fields": fields,
        }));
        id
    }

    fn nix(&mut self, event: serde_json::Value) {
        self.frames
            .push(AttachFrame::Nix(format!("@nix {event}\n").into_bytes()));
    }
}
