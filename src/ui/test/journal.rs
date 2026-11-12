use super::*;
use crate::{
    common::model::{AttachFrame, JobEvent, Outcome, Phase, Stamped},
    ui::journal,
};

fn stamped(event: JobEvent) -> Stamped<JobEvent> {
    Stamped { at: NOW, event }
}

#[test]
fn events_read_like_the_deploy_scripts() {
    let line = |event| strip_ansi(&journal::event(&stamped(event)).unwrap()).into_owned();

    assert_eq!(line(JobEvent::Phase(Phase::Building)), "[›] building");
    assert_eq!(
        line(JobEvent::Evaluated {
            drv: TOPLEVEL.parse().unwrap(),
            toplevel: TOPLEVEL.parse().unwrap(),
        }),
        "[›] system nixos-system-lab-26.11"
    );
    assert_eq!(
        line(JobEvent::Finished(Outcome::UpToDate)),
        "[✓] up to date"
    );
    assert!(journal::event(&stamped(JobEvent::Progress(Default::default()))).is_none());
}

#[test]
fn transcripts_keep_messages_and_builds_and_drop_the_rest() {
    let nix = concat!(
        "@nix {\"action\":\"msg\",\"level\":0,\"msg\":\"error: build failed\"}\n",
        "@nix {\"action\":\"start\",\"id\":5,\"level\":3,\"type\":105,\"text\":\"building\",",
        "\"fields\":[\"/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-hello-2.12.drv\",\"\",1,1]}\n",
        "@nix {\"action\":\"result\",\"id\":5,\"type\":105,\"fields\":[1,2,0,0]}\n",
        "plain output\n",
    );

    let lines = plain(&journal::frame(&AttachFrame::Nix(nix.as_bytes().to_vec())));
    assert_eq!(
        lines,
        [
            "error: build failed",
            "  building hello-2.12",
            "plain output"
        ]
    );

    let event = plain(&journal::frame(&AttachFrame::Event(stamped(
        JobEvent::Phase(Phase::Fetching),
    ))));
    assert!(event[0].ends_with(" [›] fetching"), "{event:?}");
}
