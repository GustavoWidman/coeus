use std::time::Duration;

use tokio::{io::AsyncWriteExt, time::timeout};

use super::{Feed, Nom};
use crate::{
    common::{
        model::{JobEvent, Phase, Progress, Stamped},
        proto::TerminalSize,
    },
    job::LogItem,
    nix::strip_ansi,
};

fn fed(item: &LogItem) -> String {
    strip_ansi(std::str::from_utf8(&Feed::bytes(item)).unwrap()).into_owned()
}

#[test]
fn nom_reads_nix_lines_verbatim() {
    let lines = b"@nix {\"action\":\"stop\",\"id\":1}\n@nix {\"action\":\"stop\",\"id\":2}\n";
    assert_eq!(Feed::bytes(&LogItem::Nix(lines.to_vec())).as_ref(), lines);
}

#[test]
fn nom_never_reads_activities_it_does_not_know() {
    let standard = "@nix {\"action\":\"start\",\"id\":7,\"level\":3,\"type\":102,\"text\":\"\",\"parent\":231928233984000,\"fields\":[]}\n@nix {\"action\":\"stop\",\"id\":7}\n";
    let determinate = concat!(
        "@nix {\"action\":\"start\",\"id\":231928233984000,\"level\":4,\"name\":\"BuildInstallables\",\"parent\":0,\"payload\":{},\"sid\":\"01a1cfdd-77a3-720f-8078-c6aa31765611\",\"text\":\"\",\"type\":10113}\n",
        "@nix {\"action\":\"start\",\"id\":231941118885888,\"level\":6,\"name\":\"QueryMissing\",\"parent\":0,\"payload\":{},\"sid\":\"01a1cfdd-77a3-720f-8078-c6aa31765611\",\"text\":\"querying info about missing paths\",\"type\":10113}\n",
    );

    let mixed = format!("{determinate}{standard}");
    assert_eq!(fed(&LogItem::Nix(mixed.into_bytes())), standard);
    assert_eq!(fed(&LogItem::Nix(determinate.as_bytes().to_vec())), "");
}

#[test]
fn nom_reads_events_as_the_lines_attach_shows() {
    let phase = LogItem::Event(Stamped::now(JobEvent::Phase(Phase::Building)));
    assert_eq!(fed(&phase), "[›] building\n");

    let progress = LogItem::Event(Stamped::now(JobEvent::Progress(Progress::default())));
    assert_eq!(fed(&progress), "");
}

/// what the terminal shows once `expected` appears in it
async fn until(nom: &mut Nom, expected: &str) -> String {
    let mut shown = String::new();
    timeout(Duration::from_secs(5), async {
        while !shown.contains(expected) {
            let bytes = nom.output().await.expect("the terminal closed early");
            shown.push_str(&String::from_utf8_lossy(&bytes));
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expected:?} never showed, only {shown:?}"));
    shown
}

#[tokio::test]
async fn draws_on_a_terminal_of_the_requested_size_and_follows_resizes() {
    let size = |columns, rows| TerminalSize { columns, rows };
    let (mut nom, mut input) = Nom::spawn_program(
        "/bin/sh",
        &["-c", "while read line; do stty size </dev/tty; done"],
        size(90, 30),
    )
    .unwrap();

    input.write_all(b"\n").await.unwrap();
    until(&mut nom, "30 90\r\n").await;

    nom.resize(size(140, 40)).unwrap();
    input.write_all(b"\n").await.unwrap();
    until(&mut nom, "40 140\r\n").await;

    drop(input);
    let ended = timeout(Duration::from_secs(5), async {
        while nom.output().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "the output ends once the program exits");
    assert!(nom.wait().await.unwrap().success());
}
