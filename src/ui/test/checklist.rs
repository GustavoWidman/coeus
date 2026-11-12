use super::*;
use crate::{
    bootstrap::Step,
    ui::{
        Frame,
        ask::Prompt,
        checklist::{self, Checklist, Live, Mark, Row, title},
        text,
    },
};

fn frame(width: usize) -> Frame {
    Frame {
        now: NOW,
        width,
        height: 40,
    }
}

fn checklist() -> Checklist {
    Checklist::new(
        title(&host("nova"), Some("nixos@192.168.1.40")),
        vec!["~/GitHub/nix · coeus/bootstrap/nova".into()],
    )
}

fn line<'a>(lines: &'a [String], label: &str) -> &'a str {
    lines
        .iter()
        .find(|line| line.chars().skip(4).collect::<String>().starts_with(label))
        .unwrap_or_else(|| panic!("no {label} row in {lines:#?}"))
}

#[test]
fn a_first_run_lists_every_step_pending_but_nix() {
    let lines = plain(&checklist().lines(frame(100)));
    assert_eq!(lines[0], "coeus bootstrap nova ← nixos@192.168.1.40");
    assert_eq!(lines[1], "~/GitHub/nix · coeus/bootstrap/nova");
    let labels: Vec<&str> = lines[3..]
        .iter()
        .filter_map(|line| line.strip_prefix("  · "))
        .collect();
    assert_eq!(
        labels,
        [
            "reach", "enroll", "rekey", "push", "switch", "answer", "reboot", "review", "cleanup"
        ]
    );
    assert!(
        !lines.iter().any(|line| line.contains("ctrl-c")),
        "nothing runs yet"
    );
}

#[test]
fn rows_show_their_mark_and_detail_and_found_done_says_already() {
    let mut list = checklist();
    list.update(Row::new(
        Step::Reach,
        Mark::Kept,
        "NixOS 26.05 · x86_64-linux",
    ));
    list.update(Row::new(
        Step::Enroll,
        Mark::Kept,
        "server · 4 files, 2 new secrets (a1b2c3d)",
    ));
    list.update(Row::new(
        Step::Rekey,
        Mark::Done,
        "27 rekeyed for nova (e4f5a6b)",
    ));
    list.update(Row::new(
        Step::Push,
        Mark::Active,
        "pushing coeus/bootstrap/nova",
    ));

    let lines = plain(&list.lines(frame(100)));
    let reach = line(&lines, "reach");
    let enroll = line(&lines, "enroll");
    assert!(
        reach.starts_with("  ✓ reach    NixOS 26.05 · x86_64-linux"),
        "{reach}"
    );
    assert!(reach.ends_with("already") && enroll.ends_with("already"));
    assert_eq!(reach.find("already"), enroll.find("already"), "aligned");
    assert_eq!(
        line(&lines, "rekey"),
        "  ✓ rekey    27 rekeyed for nova (e4f5a6b)"
    );
    assert!(line(&lines, "push").ends_with("push     pushing coeus/bootstrap/nova"));
    assert_eq!(line(&lines, "answer"), "  · answer");
    assert!(
        lines
            .last()
            .unwrap()
            .starts_with("ctrl-c stops here, and running this again picks up")
    );
}

#[test]
fn a_mac_shows_nix_and_a_skipped_reboot_disappears() {
    let mut list = checklist();
    list.update(Row::new(
        Step::Nix,
        Mark::Active,
        "installing Determinate Nix",
    ));
    list.update(Row::new(Step::Reboot, Mark::Skipped, ""));
    let lines = plain(&list.lines(frame(100)));
    assert!(line(&lines, "nix").contains("installing Determinate Nix"));
    assert!(!lines.iter().any(|line| line.contains("reboot")));
}

#[test]
fn the_switch_row_draws_the_deploy_boards_progress_cells() {
    let mut list = checklist();
    let mut row = Row::new(Step::Switch, Mark::Active, "job #1 on nova");
    row.live = vec![Live::Job(Box::new(building(1)))];
    list.update(row);

    let lines = plain(&list.lines(frame(140)));
    let at = lines
        .iter()
        .position(|line| line.contains("job #1 on nova"))
        .unwrap();
    let progress = &lines[at + 1];
    assert!(
        progress.starts_with(&format!("{}building", " ".repeat(13))),
        "{progress}"
    );
    assert!(progress.contains("58%"), "{progress}");
    assert!(
        progress.contains("142/225 built · 12/40 fetched"),
        "{progress}"
    );
    assert!(progress.contains("~"), "an extrapolated eta: {progress}");
    assert_eq!(lines[at + 2].trim(), "firefox-121.0 +1");
    assert!(
        lines
            .last()
            .unwrap()
            .contains("the switch goes on on the machine")
    );
}

#[test]
fn live_lines_show_counts_waits_and_tails_only_while_unsettled() {
    let mut list = checklist();
    let mut rekey = Row::new(
        Step::Rekey,
        Mark::Active,
        "27 of 53 secrets must also open on nova",
    );
    rekey.live = vec![Live::Count {
        done: 9,
        total: 27,
        current: "secrets/github-token.age".into(),
    }];
    list.update(rekey);
    let mut answer = Row::new(Step::Answer, Mark::Blocked, "nova is not on the tailnet");
    answer.live = vec![Live::Waiting {
        since: ORDERED,
        on: "sign in to Tailscale on nova".into(),
    }];
    list.update(answer);
    let mut nix = Row::new(Step::Nix, Mark::Done, "Determinate Nix 3.8");
    nix.live = vec![Live::Tail(vec!["installed".into()])];
    list.update(nix);

    let lines = plain(&list.lines(frame(100)));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("9/27  secrets/github-token.age"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.trim() == "sign in to Tailscale on nova  10m00s"),
        "{lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("│ installed")),
        "done rows keep quiet"
    );

    let mut failed = Row::new(Step::Nix, Mark::Failed, "the installer stopped");
    failed.live = vec![Live::Tail((1..=9).map(|n| format!("line {n}")).collect())];
    list.update(failed);
    let lines = plain(&list.lines(frame(100)));
    let tail: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("│ line"))
        .collect();
    assert_eq!(tail.len(), 6);
    assert!(tail[5].ends_with("│ line 9"));
}

#[test]
fn a_prompt_opens_under_the_active_row() {
    let mut list = checklist();
    list.update(Row::new(Step::Reach, Mark::Done, "NixOS 26.05"));
    list.update(Row::new(Step::Enroll, Mark::Active, "~/GitHub/nix asks"));
    list.prompt = Some(Prompt::confirm("is nova an admin?".into(), false));

    let lines = plain(&list.lines(frame(100)));
    let at = lines
        .iter()
        .position(|line| line.contains("~/GitHub/nix asks"))
        .unwrap();
    assert_eq!(lines[at + 1], "");
    assert_eq!(lines[at + 2], "  › is nova an admin? [y/N]");
    assert_eq!(lines[at + 3], "");
    assert!(lines[at + 4].contains("rekey"));
}

#[test]
fn notes_and_the_hint_sit_at_the_bottom() {
    let mut list = checklist();
    list.update(Row::new(Step::Nix, Mark::Active, "installing"));
    list.notes
        .push("moved /etc/zshrc aside as /etc/zshrc.before-nix-darwin".into());
    let lines = plain(&list.lines(frame(100)));
    let n = lines.len();
    assert_eq!(
        lines[n - 3],
        "[•] moved /etc/zshrc aside as /etc/zshrc.before-nix-darwin"
    );
    assert_eq!(lines[n - 2], "");
    assert!(lines[n - 1].starts_with("ctrl-c"));
    assert!(
        !plain(&list.still(frame(100)))
            .iter()
            .any(|line| line.contains("ctrl-c"))
    );
}

#[test]
fn settling_stops_spinners_and_closes_the_prompt() {
    let mut list = checklist();
    let mut row = Row::new(Step::Switch, Mark::Active, "building");
    row.live = vec![Live::Job(Box::new(building(1)))];
    list.update(row);
    list.prompt = Some(Prompt::confirm("sure?".into(), true));
    list.settle();

    let lines = plain(&list.lines(frame(100)));
    assert_eq!(line(&lines, "switch"), "  · switch   building");
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("sure?") || line.contains("ctrl-c"))
    );
}

#[test]
fn every_line_fits_the_terminal_and_the_height() {
    let mut list = checklist();
    list.update(Row::new(
        Step::Reach,
        Mark::Kept,
        "NixOS 26.05 · x86_64-linux · EFI · 16 GiB · 8 cpus · host key pinned",
    ));
    let mut row = Row::new(Step::Switch, Mark::Active, "job #1 on nova");
    row.live = vec![Live::Job(Box::new(building(1)))];
    list.update(row);

    for width in [30, 60, 100] {
        for line in list.lines(frame(width)) {
            assert!(text::width(&line) <= width, "{width}: {line}");
        }
    }
    let short = list.lines(Frame {
        now: NOW,
        width: 80,
        height: 8,
    });
    assert_eq!(short.len(), 7);
    assert!(plain(&short).last().unwrap().starts_with("ctrl-c"));
}

#[test]
fn plain_prints_a_line_per_change() {
    let before = checklist();
    let mut after = before.clone();
    assert!(checklist::plain(&before, &after, NOW).is_empty());

    after.update(Row::new(Step::Reach, Mark::Active, "probing nova"));
    let lines = plain(&checklist::plain(&before, &after, NOW));
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with(" reach    ⠋ probing nova"), "{lines:?}");

    let before = after.clone();
    after.update(Row::new(Step::Reach, Mark::Kept, "NixOS 26.05"));
    after.notes.push("the host key is pinned".into());
    let lines = plain(&checklist::plain(&before, &after, NOW));
    assert!(
        lines[0].ends_with(" reach    ✓ NixOS 26.05  already"),
        "{lines:?}"
    );
    assert!(
        lines[1].ends_with(" [•] the host key is pinned"),
        "{lines:?}"
    );
}

#[test]
fn plain_prints_job_progress_by_tenths() {
    let mut before = checklist();
    let mut row = Row::new(Step::Switch, Mark::Active, "job #1 on nova");
    let mut job = building(1);
    row.live = vec![Live::Job(Box::new(job.clone()))];
    let mut after = before.clone();
    after.update(row.clone());

    let lines = plain(&checklist::plain(&before, &after, NOW));
    assert!(
        lines[1].ends_with("switch   building 58% · 142/225 built · 12/40 fetched · 1.2/3.4 GiB"),
        "{lines:?}"
    );

    before = after.clone();
    job.progress.builds.done += 1;
    row.live = vec![Live::Job(Box::new(job.clone()))];
    after.update(row.clone());
    assert!(
        checklist::plain(&before, &after, NOW).is_empty(),
        "same tenth"
    );

    before = after.clone();
    job.progress.builds.done = 160;
    row.live = vec![Live::Job(Box::new(job))];
    after.update(row);
    let lines = plain(&checklist::plain(&before, &after, NOW));
    assert!(lines[0].contains("building 64%"), "{lines:?}");
}

#[test]
fn plain_shows_a_failures_tail_once() {
    let before = checklist();
    let mut after = before.clone();
    let mut row = Row::new(Step::Nix, Mark::Failed, "the installer stopped");
    row.live = vec![Live::Tail(vec!["error: disk full".into()])];
    after.update(row);
    let lines = plain(&checklist::plain(&before, &after, NOW));
    assert!(
        lines[0].ends_with("nix      ✗ the installer stopped"),
        "{lines:?}"
    );
    assert!(
        lines[1].ends_with("nix        │ error: disk full"),
        "{lines:?}"
    );
    assert!(checklist::plain(&after, &after, NOW).is_empty());
}
