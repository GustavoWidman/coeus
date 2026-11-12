use std::borrow::Cow;

use super::{BUILD, DOWNLOAD, FAILING};
use crate::nix::*;

fn event(raw: &str) -> Event<'_> {
    match Line::parse(raw) {
        Line::Nix(event) => event,
        Line::Output(text) => panic!("not an event: {text}"),
    }
}

#[test]
fn every_fixture_line_is_an_event() {
    for raw in [BUILD, DOWNLOAD, FAILING].into_iter().flat_map(str::lines) {
        assert!(Line::parse(raw).event().is_some(), "{raw}");
    }
}

#[test]
fn other_lines_are_output() {
    assert_eq!(Line::parse("building...\n"), Line::Output("building..."));
    assert_eq!(Line::parse("@nix {broken"), Line::Output("@nix {broken"));
    assert_eq!(
        Line::parse(r#"@nix {"action":"dance"}"#),
        Line::Output(r#"@nix {"action":"dance"}"#)
    );
}

#[test]
fn reads_a_build_start() {
    let raw = r#"@nix {"action":"start","fields":["/nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-hello-2.12.drv","",1,1],"id":17,"level":3,"parent":13,"text":"building 'hello'","type":105}"#;
    let Event::Start(start) = event(raw) else { panic!() };

    assert_eq!(start.id, ActivityId(17));
    assert_eq!(start.parent, Some(ActivityId(13)));
    assert_eq!(start.activity, Activity::Build);
    assert_eq!(start.level, Level::Info);
    assert!(matches!(start.text, Cow::Borrowed("building 'hello'")));
    assert_eq!(
        start.drv_path().as_deref(),
        Some("/nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-hello-2.12.drv")
    );
}

#[test]
fn keeps_unknown_activities_and_ignores_unknown_keys() {
    let raw = r#"@nix {"action":"start","id":5,"level":6,"name":"SubstitutionGoal","parent":0,"payload":{"nix.store.path":"/nix/store/x"},"sid":"01a1","text":"","type":10113}"#;
    let Event::Start(start) = event(raw) else { panic!() };

    assert_eq!(start.activity, Activity::Other(10113));
    assert_eq!(start.parent, None);
    assert_eq!(start.drv_path(), None);
}

#[test]
fn decodes_results_by_kind() {
    let report = |raw| match event(raw) {
        Event::Result(report) => report,
        other => panic!("{other:?}"),
    };

    let progress = report(r#"@nix {"action":"result","fields":[1,4,2,0],"id":3,"type":105}"#);
    assert_eq!(
        progress.progress(),
        Some(Counts {
            done: 1,
            expected: 4,
            running: 2,
            failed: 0
        })
    );
    assert_eq!(progress.expected(), None);

    let expected = report(r#"@nix {"action":"result","fields":[101,1323579],"id":2,"type":106}"#);
    assert_eq!(expected.expected(), Some((Activity::FileTransfer, 1_323_579)));

    let log = report(r#"@nix {"action":"result","fields":["say \"hi\""],"id":7,"type":101}"#);
    assert_eq!(log.log_line().as_deref(), Some("say \"hi\""));
    assert_eq!(log.progress(), None);

    let determinate = report(r#"@nix {"action":"result","id":8,"payload":{"bodySize":562},"type":10111}"#);
    assert_eq!(determinate.kind, ResultKind::Other(10111));
    assert_eq!(determinate.progress(), None);
}

#[test]
fn reads_messages_and_stops() {
    let Event::Msg(msg) = event(
        r#"@nix {"action":"msg","column":null,"file":null,"level":0,"line":null,"msg":"\u001b[31;1merror:\u001b[0m boom","raw_msg":"boom"}"#,
    ) else {
        panic!()
    };
    assert_eq!(msg.level, Level::Error);
    assert_eq!(msg.text, "\u{1b}[31;1merror:\u{1b}[0m boom");

    assert_eq!(
        event(r#"@nix {"action":"stop","id":42,"sid":"x"}"#),
        Event::Stop(ActivityId(42))
    );
}
