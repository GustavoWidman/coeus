use super::{ANSWERS, Fleet, git, has_nix, host};
use crate::{
    bootstrap::{Blocker, Finale, Step, branch::PullState, state::StateDir},
    ui::checklist::Mark,
};

use super::fake::Kill;

#[tokio::test]
async fn a_first_run_takes_the_machine_into_the_fleet_and_a_rerun_finds_it_there() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;

    let first = fleet.run(&Kill::default()).await.unwrap();
    fleet.assert_in_the_fleet(&first);
    assert_eq!(
        first.marks(),
        [
            (Step::Reach, Mark::Done),
            (Step::Nix, Mark::Skipped),
            (Step::Enroll, Mark::Done),
            (Step::Rekey, Mark::Done),
            (Step::Push, Mark::Done),
            (Step::Switch, Mark::Done),
            (Step::Answer, Mark::Done),
            (Step::Reboot, Mark::Done),
            (Step::Review, Mark::Done),
            (Step::Cleanup, Mark::Done),
        ]
    );
    {
        let world = fleet.world.lock().unwrap();
        assert_eq!(world.jobs.len(), 1);
        assert!(world.releases[0].install_bootloader);
        assert!(world.releases[0].label.ends_with(&format!(
            ":coeus/bootstrap/nova@{}",
            &fleet.origin_head().unwrap()[..7]
        )));
    }
    let saved = fleet.state().load().unwrap();
    assert_eq!(saved.answers.get("type"), Some("server"));
    assert_eq!(saved.facts.as_ref().map(|facts| facts.cpus), Some(8));

    let head = fleet.origin_head();
    let body = fleet.pulls.lock().unwrap().open[0].1.body.clone();
    let rerun = fleet.run(&Kill::default()).await.unwrap();
    fleet.assert_in_the_fleet(&rerun);
    assert_eq!(
        rerun.marks(),
        [
            (Step::Reach, Mark::Kept),
            (Step::Nix, Mark::Skipped),
            (Step::Enroll, Mark::Kept),
            (Step::Rekey, Mark::Kept),
            (Step::Push, Mark::Kept),
            (Step::Switch, Mark::Kept),
            (Step::Answer, Mark::Kept),
            (Step::Reboot, Mark::Skipped),
            (Step::Review, Mark::Kept),
            (Step::Cleanup, Mark::Kept),
        ]
    );
    assert_eq!(fleet.origin_head(), head);
    assert_eq!(fleet.pulls.lock().unwrap().open[0].1.body, body);
    let world = fleet.world.lock().unwrap();
    assert_eq!(world.jobs.len(), 1);
    assert_eq!(world.ssh, 1, "coeusd answers, so ssh is not opened again");
}

#[tokio::test]
async fn the_base_moving_after_the_switch_rebases_rekeys_and_switches_the_new_head() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.assert_in_the_fleet(&fleet.run(&Kill::default()).await.unwrap());

    fleet.seal_on_main(
        "secrets/later.age",
        &[fleet.admin.clone(), fleet.old.clone()],
    );
    let rerun = fleet.run(&Kill::default()).await.unwrap();

    fleet.assert_in_the_fleet(&rerun);
    assert_eq!(rerun.mark(Step::Rekey), Mark::Done);
    assert_eq!(rerun.mark(Step::Push), Mark::Done);
    assert_eq!(rerun.mark(Step::Switch), Mark::Done);
    assert_eq!(rerun.mark(Step::Review), Mark::Done);
    assert!(fleet.opens("secrets/later.age", &fleet.nova));
    let world = fleet.world.lock().unwrap();
    assert_eq!(world.jobs.len(), 2, "the rebased head is switched to");
    assert_eq!(world.ssh, 1);
}

#[tokio::test]
async fn a_failed_switch_waits_for_retry() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.world.lock().unwrap().breaks = true;

    let failed = fleet.run(&Kill::default()).await.unwrap();
    let Finale::Blocked {
        at: Step::Switch,
        blocker: Blocker::SwitchFailed { .. },
    } = failed.finale
    else {
        panic!("{:?}", failed.finale);
    };
    assert_eq!(failed.mark(Step::Switch), Mark::Blocked);
    assert!(fleet.pulls.lock().unwrap().open[0].1.draft);

    fleet.world.lock().unwrap().breaks = false;
    let again = fleet.run(&Kill::default()).await.unwrap();
    assert!(matches!(again.finale, Finale::Blocked { .. }));
    assert_eq!(fleet.world.lock().unwrap().jobs.len(), 1);

    let retried = fleet
        .run_with(&Kill::default(), ANSWERS, true)
        .await
        .unwrap();
    fleet.assert_in_the_fleet(&retried);
    assert_eq!(fleet.world.lock().unwrap().jobs.len(), 2);
}

#[tokio::test]
async fn a_switch_nixos_refuses_is_booted_and_rebooted_into() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.world.lock().unwrap().refuses = true;

    let ran = fleet.run(&Kill::default()).await.unwrap();
    fleet.assert_in_the_fleet(&ran);
    assert_eq!(ran.mark(Step::Switch), Mark::Done);
    assert_eq!(
        ran.mark(Step::Reboot),
        Mark::Skipped,
        "it booted what it runs"
    );
    assert!(
        ran.rows.notes.iter().any(|note| note
            .starts_with("NixOS asks for a reboot into this system instead of a live switch")
            && note.contains("Pre-switch check 'switchInhibitors' failed")),
        "{:?}",
        ran.rows.notes
    );
    fleet.assert_booted_into_the_fleet();

    let rerun = fleet.run(&Kill::default()).await.unwrap();
    fleet.assert_in_the_fleet(&rerun);
    assert_eq!(rerun.mark(Step::Switch), Mark::Kept);
    fleet.assert_booted_into_the_fleet();
}

#[tokio::test]
async fn a_question_without_a_terminal_or_an_answer_blocks_before_anything_is_written() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;

    let ran = fleet
        .run_with(&Kill::default(), &[("type", "server")], false)
        .await
        .unwrap();
    let Finale::Blocked {
        at: Step::Enroll,
        blocker: Blocker::Unanswered { id },
    } = &ran.finale
    else {
        panic!("{:?}", ran.finale);
    };
    assert_eq!(id, "public");
    assert_eq!(fleet.subjects(), Vec::<String>::new());
    assert_eq!(ran.saved.answers.get("type"), Some("server"));

    let wrong = fleet
        .run_with(&Kill::default(), &[("type", "laptop")], false)
        .await
        .unwrap();
    assert!(matches!(
        wrong.finale,
        Finale::Failed {
            at: Step::Enroll,
            ..
        }
    ));
    let unknown = fleet
        .run_with(
            &Kill::default(),
            &[("public", "true"), ("colour", "red")],
            false,
        )
        .await
        .unwrap();
    assert!(matches!(
        unknown.finale,
        Finale::Failed {
            at: Step::Enroll,
            ..
        }
    ));
}

#[tokio::test]
async fn a_signed_out_gh_blocks_the_push_and_a_non_admin_the_rekey() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.pulls.lock().unwrap().signed_out = true;
    let ran = fleet.run(&Kill::default()).await.unwrap();
    assert!(matches!(
        ran.finale,
        Finale::Blocked {
            at: Step::Push,
            blocker: Blocker::Forge(_)
        }
    ));

    let other = Fleet::new().await;
    let (private, _, line) = super::keypair("root@stranger");
    std::fs::write(&other.identity, private).unwrap();
    std::fs::write(other.identity.with_extension("pub"), line).unwrap();
    let ran = other.run(&Kill::default()).await.unwrap();
    let Finale::Blocked {
        at: Step::Rekey,
        blocker: Blocker::NotAdmin { files },
    } = &ran.finale
    else {
        panic!("{:?}", ran.finale);
    };
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(other.world.lock().unwrap().jobs.is_empty());
}

#[tokio::test]
async fn a_second_run_is_refused_while_one_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let state = StateDir::under(dir.path(), &host());
    let _held = state.lock().await.unwrap();

    assert!(state.locked().unwrap());
    let error = state.lock().await.err().unwrap();
    assert!(
        format!("{error}").contains(&format!("pid {}", std::process::id())),
        "{error}"
    );
    let listed = StateDir::all_in(dir.path()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, host());
}

#[tokio::test]
async fn a_rekey_stopped_before_its_commit_is_finished_and_an_edit_blocks() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.assert_in_the_fleet(&fleet.run(&Kill::default()).await.unwrap());

    // the rekey commit never made: its secrets rewritten, nothing committed
    // or pushed
    git(&fleet.clone(), &["reset", "--quiet", "--mixed", "HEAD~1"]);
    super::git(
        &fleet.clone(),
        &[
            "push",
            "--quiet",
            "--force",
            "origin",
            "HEAD:coeus/bootstrap/nova",
        ],
    );
    let finished = fleet.run(&Kill::default()).await.unwrap();
    fleet.assert_in_the_fleet(&finished);
    assert_eq!(finished.mark(Step::Rekey), Mark::Done);

    // a person's `agenix -e`: the same recipients, another secret
    let shared = fleet.clone().join("secrets/shared.age");
    let edited = fleet.root.join("edited.age");
    let everyone = [fleet.admin.clone(), fleet.old.clone(), fleet.nova.clone()];
    crate::bootstrap::secrets::create(&edited, b"edited", &everyone).unwrap();
    std::fs::rename(&edited, &shared).unwrap();
    let ran = fleet.run(&Kill::default()).await.unwrap();

    let Finale::Blocked {
        at: Step::Rekey,
        blocker: Blocker::Uncommitted { files, .. },
    } = &ran.finale
    else {
        panic!("{:?}", ran.finale);
    };
    assert_eq!(files, &["secrets/shared.age"]);
}

#[tokio::test]
async fn a_pull_request_closed_without_merging_is_left_and_a_new_one_opened() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.assert_in_the_fleet(&fleet.run(&Kill::default()).await.unwrap());
    let closed = {
        let mut pulls = fleet.pulls.lock().unwrap();
        pulls.open[0].1.state = PullState::Closed;
        pulls.open[0].1.clone()
    };

    let ran = fleet.run(&Kill::default()).await.unwrap();

    let Finale::Done(evidence) = &ran.finale else {
        panic!("{:?}", ran.finale);
    };
    assert_ne!(evidence.pull_request.number, closed.number);
    assert_eq!(evidence.pull_request.state, PullState::Open);
    assert!(!evidence.pull_request.draft);
    let pulls = fleet.pulls.lock().unwrap();
    assert_eq!(pulls.open.len(), 2);
    assert_eq!(pulls.open[0].1, closed, "the closed one is left as it was");
}

#[tokio::test]
async fn a_pull_request_of_another_commit_is_not_reviewed() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.assert_in_the_fleet(&fleet.run(&Kill::default()).await.unwrap());
    let elsewhere = git(&fleet.clone(), &["rev-parse", "HEAD~1"]);
    {
        let mut pulls = fleet.pulls.lock().unwrap();
        let pull = &mut pulls.open[0].1;
        pull.state = PullState::Merged;
        pull.head = elsewhere.parse().unwrap();
        pull.body = String::new();
    }

    let ran = fleet.run(&Kill::default()).await.unwrap();

    assert!(
        matches!(
            ran.finale,
            Finale::Failed {
                at: Step::Review,
                ..
            }
        ),
        "{:?}",
        ran.finale
    );
    assert_eq!(fleet.pulls.lock().unwrap().open[0].1.body, "");
}

#[tokio::test]
async fn an_answer_changed_after_the_enrollment_blocks_instead_of_being_ignored() {
    if !has_nix() {
        return;
    }
    let fleet = Fleet::new().await;
    fleet.assert_in_the_fleet(&fleet.run(&Kill::default()).await.unwrap());
    let head = fleet.origin_head();

    let ran = fleet
        .run_with(&Kill::default(), &[("type", "desktop")], false)
        .await
        .unwrap();

    let Finale::Blocked {
        at: Step::Enroll,
        blocker: Blocker::Reanswered { files },
    } = &ran.finale
    else {
        panic!("{:?}", ran.finale);
    };
    assert_eq!(files, &["hosts/nova/default.nix".parse().unwrap()]);
    assert_eq!(fleet.origin_head(), head);
    assert_eq!(fleet.world.lock().unwrap().jobs.len(), 1);

    let again = fleet
        .run_with(&Kill::default(), &[("type", "server")], false)
        .await
        .unwrap();
    fleet.assert_in_the_fleet(&again);
    assert_eq!(again.mark(Step::Enroll), Mark::Kept);
}
