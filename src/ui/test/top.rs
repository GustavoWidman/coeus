use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

use super::*;
use crate::{
    client::{FleetEvent, Link},
    common::model::{FdaStatus, Follow, JobId, JobState, Outcome, Pinned},
    ui::top::{
        Arrangement, Badge, Effect, Entry, Intent, Mode, Panes, Request, Retarget, Top, draw,
    },
};

fn top(names: &[&str]) -> Top {
    let hosts: Vec<Hostname> = names.iter().map(|name| host(name)).collect();
    let tags = HashMap::from([
        (host("lab"), vec!["nixos".to_owned(), "canary".to_owned()]),
        (host("altar"), vec!["darwin".to_owned()]),
    ]);
    Top::new(&hosts, tags, "0.1.0", ORDERED)
}

fn see(top: &mut Top, view: MachineView) {
    top.apply(FleetEvent::View {
        host: view.host.hostname.clone(),
        view: Box::new(view),
    });
}

fn press(top: &mut Top, code: KeyCode) -> Option<Effect> {
    top.key(KeyEvent::new(code, KeyModifiers::NONE), NOW)
}

fn type_text(top: &mut Top, text: &str) {
    for char in text.chars() {
        press(top, KeyCode::Char(char));
    }
}

fn selected(top: &Top) -> &str {
    top.selected().unwrap().host.as_str()
}

fn screen(top: &Top, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, top, NOW)).unwrap();

    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn styled_lines_keep_their_colors_and_drop_other_escapes() {
    use ratatui::style::{Color, Modifier};

    let line = crate::ui::top::ansi::line(
        "\u{1b}[1;36mbuilding\u{1b}[0m \u{1b}[38;5;208mfirefox\u{1b}[39m\u{1b}[K \u{1b}]8;;https://x\u{7}link",
    );
    let spans: Vec<_> = line
        .spans
        .iter()
        .map(|span| (span.content.as_ref(), span.style))
        .collect();

    assert_eq!(spans[0].0, "building");
    assert_eq!(spans[0].1.fg, Some(Color::Cyan));
    assert!(spans[0].1.add_modifier.contains(Modifier::BOLD));
    assert_eq!(spans[1].0, " ");
    assert_eq!(spans[1].1.fg, None);
    assert_eq!(spans[2].0, "firefox");
    assert_eq!(spans[2].1.fg, Some(Color::Indexed(208)));
    assert_eq!(
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        "building firefox link"
    );
}

#[test]
fn a_building_machine_shows_its_progress_and_how_long_it_runs() {
    let mut top = top(&["lab"]);
    see(&mut top, view("lab", Some(building(12)), Vec::new()));

    let entry = Entry::of(top.selected().unwrap(), NOW);
    let gauge = entry.gauge.unwrap();
    assert!((gauge - 154.0 / 265.0).abs() < 1e-9, "{gauge}");
    assert_eq!(entry.clock.as_deref(), Some("9m59s"));
    assert!(entry.badges.is_empty());
    assert!(entry.animates());
}

#[test]
fn a_finished_machine_shows_the_age_of_its_last_job() {
    let mut top = top(&["lab"]);
    see(&mut top, view("lab", None, vec![failed(11)]));

    let entry = Entry::of(top.selected().unwrap(), NOW);
    assert_eq!(entry.gauge, None);
    assert_eq!(entry.clock.as_deref(), Some("5m ago"));
    assert!(!entry.animates());
}

#[test]
fn badges_flag_what_needs_a_person() {
    let mut top = top(&["altar"]);
    let mut machine = view("altar", None, Vec::new());
    machine.system.reboot_required = true;
    machine.system.fda = FdaStatus::Missing;
    machine.system.follow = Some(Follow {
        git_ref: "deployed".into(),
        latest: Some(Pinned {
            git_ref: "deployed".into(),
            rev: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
            committed_at: ORDERED,
        }),
        paused: false,
    });
    see(&mut top, machine);
    top.apply(FleetEvent::Link {
        host: host("altar"),
        link: Link::Connecting,
    });

    let entry = Entry::of(top.selected().unwrap(), NOW);
    assert_eq!(
        entry.badges,
        [
            Badge::Reconnecting,
            Badge::Reboot,
            Badge::NoDiskAccess,
            Badge::Behind("deployed".into()),
        ]
    );
}

#[test]
fn wide_screens_put_the_fleet_beside_the_machine() {
    let panes = Panes::of(Rect::new(0, 0, 140, 40), 3);

    assert_eq!(panes.arrangement, Arrangement::Wide);
    assert_eq!(panes.fleet.y, panes.detail.y);
    assert_eq!(panes.fleet.right(), panes.detail.x);
    assert_eq!(panes.detail.right(), 140);
    assert_eq!(panes.log.y, panes.fleet.bottom());
    assert_eq!(panes.log.width, 140);
    assert_eq!(panes.footer.y, 39);
}

#[test]
fn narrow_screens_stack_the_panes() {
    let panes = Panes::of(Rect::new(0, 0, 80, 40), 3);

    assert_eq!(panes.arrangement, Arrangement::Narrow);
    assert_eq!(panes.fleet.height, 5, "three machines and the borders");
    assert_eq!(panes.detail.y, panes.fleet.bottom());
    assert_eq!(panes.log.y, panes.detail.bottom());
    for pane in [panes.fleet, panes.detail, panes.log] {
        assert_eq!(pane.width, 80);
    }

    let crowded = Panes::of(Rect::new(0, 0, 80, 40), 60);
    assert!(crowded.fleet.height <= 38 / 3, "{}", crowded.fleet.height);
}

#[test]
fn the_selection_moves_and_stops_at_the_ends() {
    let mut top = top(&["altar", "lab", "oracle"]);
    assert_eq!(selected(&top), "altar");

    press(&mut top, KeyCode::Char('k'));
    assert_eq!(selected(&top), "altar");
    press(&mut top, KeyCode::Char('j'));
    press(&mut top, KeyCode::Down);
    press(&mut top, KeyCode::Down);
    assert_eq!(selected(&top), "oracle");
    press(&mut top, KeyCode::Char('g'));
    assert_eq!(selected(&top), "altar");
    press(&mut top, KeyCode::Char('G'));
    assert_eq!(selected(&top), "oracle");
}

#[test]
fn the_filter_matches_names_and_tags_as_it_is_typed() {
    let mut top = top(&["altar", "lab", "oracle"]);

    press(&mut top, KeyCode::Char('/'));
    type_text(&mut top, "@can");
    assert_eq!(top.mode, Mode::Filter);
    assert_eq!(
        top.visible()
            .map(|row| row.host.as_str())
            .collect::<Vec<_>>(),
        ["lab"]
    );
    assert_eq!(selected(&top), "lab");

    press(&mut top, KeyCode::Enter);
    assert_eq!(top.mode, Mode::Browse);
    assert_eq!(top.filter, "@can");

    assert_eq!(
        press(&mut top, KeyCode::Esc),
        None,
        "esc clears the filter first"
    );
    assert_eq!(top.visible().count(), 3);
    assert_eq!(press(&mut top, KeyCode::Esc), Some(Effect::Quit));

    press(&mut top, KeyCode::Char('/'));
    type_text(&mut top, "ora");
    press(&mut top, KeyCode::Backspace);
    assert_eq!(top.filter, "or");
    assert_eq!(selected(&top), "oracle");
}

#[test]
fn cancelling_asks_first_and_runs_only_on_yes() {
    let mut top = top(&["lab"]);
    see(&mut top, view("lab", Some(building(12)), Vec::new()));

    assert_eq!(press(&mut top, KeyCode::Char('c')), None);
    let request = Request {
        host: host("lab"),
        intent: Intent::Cancel { job: JobId(12) },
    };
    assert_eq!(top.mode, Mode::Confirm(request.clone()));

    assert_eq!(press(&mut top, KeyCode::Char('x')), None, "other keys wait");
    assert_eq!(top.mode, Mode::Confirm(request.clone()));
    assert_eq!(press(&mut top, KeyCode::Char('n')), None);
    assert_eq!(top.mode, Mode::Browse);

    press(&mut top, KeyCode::Char('c'));
    assert_eq!(
        press(&mut top, KeyCode::Char('y')),
        Some(Effect::Run(request))
    );
    assert_eq!(top.mode, Mode::Browse);
}

#[test]
fn every_action_names_its_machine_and_target() {
    let mut top = top(&["lab"]);
    see(&mut top, view("lab", None, vec![failed(11)]));

    let mut confirmed = |key| {
        press(&mut top, KeyCode::Char(key));
        match press(&mut top, KeyCode::Enter) {
            Some(Effect::Run(request)) => request.intent,
            other => panic!("{key}: {other:?}"),
        }
    };

    assert_eq!(confirmed('r'), Intent::Rollback { from: Some(41) });
    assert_eq!(confirmed('R'), Intent::Reboot);
    assert_eq!(confirmed('d'), Intent::Deploy);
}

#[test]
fn impossible_actions_explain_themselves_instead_of_asking() {
    let mut top = top(&["altar", "lab"]);
    see(&mut top, view("lab", None, Vec::new()));

    assert_eq!(press(&mut top, KeyCode::Char('c')), None);
    assert_eq!(top.mode, Mode::Browse);
    let notice = plain(&[top.notice(NOW).unwrap().text.clone()]);
    assert!(notice[0].contains("altar is not connected"), "{notice:?}");

    press(&mut top, KeyCode::Char('j'));
    assert_eq!(press(&mut top, KeyCode::Char('c')), None);
    let notice = plain(&[top.notice(NOW).unwrap().text.clone()]);
    assert!(
        notice[0].contains("lab has nothing to cancel"),
        "{notice:?}"
    );
    assert_eq!(press(&mut top, KeyCode::Enter), None, "no job to attach to");

    see(&mut top, view("lab", None, vec![failed(11)]));
    assert_eq!(
        press(&mut top, KeyCode::Char('a')),
        Some(Effect::Attach(host("lab")))
    );
}

#[test]
fn help_closes_on_any_key_and_ctrl_c_always_quits() {
    let mut top = top(&["lab"]);

    press(&mut top, KeyCode::Char('?'));
    assert_eq!(top.mode, Mode::Help);
    assert_eq!(press(&mut top, KeyCode::Char('q')), None);
    assert_eq!(top.mode, Mode::Browse);

    press(&mut top, KeyCode::Char('/'));
    let quit = top.key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        NOW,
    );
    assert_eq!(quit, Some(Effect::Quit));
}

#[test]
fn the_log_follows_the_selected_machines_running_job() {
    let mut top = top(&["altar", "lab"]);
    see(&mut top, view("lab", Some(building(12)), Vec::new()));
    assert_eq!(top.retarget(), None, "altar runs nothing");

    press(&mut top, KeyCode::Char('j'));
    assert_eq!(
        top.retarget(),
        Some(Retarget::Follow {
            host: host("lab"),
            job: JobId(12)
        })
    );
    assert_eq!(top.retarget(), None, "already following");

    top.tail_lines(&host("lab"), JobId(12), vec!["building".into()], false);
    top.tail_lines(&host("lab"), JobId(9), vec!["stale".into()], false);
    let tail = top.tail.as_ref().unwrap();
    assert_eq!(tail.last(10).collect::<Vec<_>>(), ["building"]);

    see(
        &mut top,
        view(
            "lab",
            None,
            vec![summary(12, JobState::Finished(Outcome::Succeeded))],
        ),
    );
    assert_eq!(top.retarget(), None, "a finished job's log stays");
    assert!(top.tail.is_some());

    press(&mut top, KeyCode::Char('k'));
    assert_eq!(top.retarget(), Some(Retarget::Stop));
    assert!(top.tail.is_none());
}

#[test]
fn frames_come_only_when_something_changed_or_moves() {
    let mut top = top(&["lab"]);
    let at = |millis| Timestamp(NOW.0 + millis);

    assert!(top.frame_due(NOW));
    top.drawn(NOW);
    assert!(!top.frame_due(at(150)), "idle and unchanged");
    assert!(top.frame_due(at(1_000)), "clocks tick each second");

    see(&mut top, view("lab", Some(building(12)), Vec::new()));
    assert!(top.frame_due(at(10)), "a new view");
    top.drawn(at(10));
    assert!(!top.frame_due(at(50)));
    assert!(top.frame_due(at(110)), "the spinner turns");
}

#[test]
fn the_dashboard_draws_every_pane_wide_and_narrow() {
    let mut top = top(&["altar", "lab"]);
    see(&mut top, view("lab", Some(building(12)), vec![failed(11)]));
    press(&mut top, KeyCode::Char('j'));
    top.retarget();
    top.tail_lines(
        &host("lab"),
        JobId(12),
        vec!["12:00:01 [›] building".into()],
        false,
    );

    let wide = screen(&top, 140, 40).join("\n");
    for expected in [
        "coeus top",
        "machines 2",
        "connecting",
        "58%",
        "142/225 built",
        "job #12",
        "building 9m00s",
        "firefox-121.0",
        "doctor",
        "recent jobs",
        "log lab job #12",
        "[›] building",
        "? help",
    ] {
        assert!(wide.contains(expected), "{expected} missing from\n{wide}");
    }

    let narrow = screen(&top, 60, 30).join("\n");
    for expected in ["coeus top", "lab", "job #12", "log lab"] {
        assert!(
            narrow.contains(expected),
            "{expected} missing from\n{narrow}"
        );
    }
}

#[test]
fn a_confirmation_appears_over_the_dashboard() {
    let mut top = top(&["lab"]);
    see(&mut top, view("lab", Some(building(12)), Vec::new()));
    press(&mut top, KeyCode::Char('R'));

    let lines = screen(&top, 100, 30).join("\n");
    assert!(lines.contains("reboot lab now?"), "{lines}");
    assert!(lines.contains("y yes   n no"), "{lines}");
}
