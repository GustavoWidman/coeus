use crate::{
    client::Targets,
    config::{ClientSettings, HostEntry},
};

fn inventory() -> ClientSettings {
    let mut settings = ClientSettings::default();
    for (name, tags) in [
        ("altar", vec!["darwin"]),
        ("lab", vec!["nixos", "canary"]),
        ("oracle-1", vec!["nixos", "server"]),
        ("oracle-2", vec!["nixos", "server"]),
    ] {
        settings.hosts.insert(
            name.parse().unwrap(),
            HostEntry {
                tags: tags.into_iter().map(str::to_owned).collect(),
                ..Default::default()
            },
        );
    }
    settings
}

fn select(expressions: &[&str]) -> eyre::Result<Vec<String>> {
    let expressions: Vec<String> = expressions.iter().map(|value| value.to_string()).collect();
    Ok(Targets::select(&expressions, &inventory())?
        .hosts()
        .iter()
        .map(ToString::to_string)
        .collect())
}

#[test]
fn selects_names_groups_and_exclusions() {
    assert_eq!(
        select(&["@all"]).unwrap(),
        ["altar", "lab", "oracle-1", "oracle-2"]
    );
    assert_eq!(select(&["@server"]).unwrap(), ["oracle-1", "oracle-2"]);
    assert_eq!(
        select(&["@nixos", "!oracle-2"]).unwrap(),
        ["lab", "oracle-1"]
    );
    assert_eq!(select(&["!@nixos"]).unwrap(), ["altar"]);
    assert_eq!(select(&["lab", "altar", "lab"]).unwrap(), ["lab", "altar"]);
}

#[test]
fn reaches_names_outside_the_inventory_and_rejects_unknown_groups() {
    assert_eq!(select(&["desktop-nixos"]).unwrap(), ["desktop-nixos"]);
    assert!(select(&["@nothing"]).is_err());
    assert!(select(&["!@all"]).is_err());
}

mod doctor {
    use crate::{
        client::{Checkup, Severity},
        common::model::{
            Action, DaemonState, Failure, FdaStatus, HostInfo, JobId, JobState, JobSummary,
            MachineView, OrderId, Origin, Outcome, Phase, Platform, Source, SystemInfo, Timestamp,
        },
    };

    fn view(platform: Platform) -> MachineView {
        MachineView {
            host: HostInfo {
                hostname: "altar".parse().unwrap(),
                platform,
                system: "aarch64-darwin".into(),
                version: "0.2.0".into(),
            },
            system: SystemInfo {
                current: Some(
                    "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-darwin-system-26.11"
                        .parse()
                        .unwrap(),
                ),
                generation: Some(10),
                pending_boot: false,
                reboot_required: false,
                deployed: None,
                follow: None,
                fda: FdaStatus::Granted,
                checked_at: Timestamp(0),
            },
            daemon: DaemonState::Serving,
            running: None,
            next: None,
            recent: Vec::new(),
        }
    }

    #[test]
    fn a_healthy_machine_says_so() {
        let checkup = Checkup::of(&view(Platform::Darwin), "0.2.0");
        assert_eq!(checkup.severity(), Severity::Ok);
    }

    #[test]
    fn missing_full_disk_access_warns_with_the_fix() {
        let mut view = view(Platform::Darwin);
        view.system.fda = FdaStatus::Missing;

        let checkup = Checkup::of(&view, "0.2.0");
        assert_eq!(checkup.severity(), Severity::Warn);
        assert!(
            checkup.findings[0]
                .fix
                .as_ref()
                .unwrap()
                .contains("coeus-launcher")
        );
    }

    #[test]
    fn version_skew_reboots_and_failures_are_warnings_worst_first() {
        let mut view = view(Platform::NixOs);
        view.system.reboot_required = true;
        view.system.current = None;
        view.recent.push(JobSummary {
            id: JobId(7),
            order: OrderId(1),
            source: Source::Generation(3),
            action: Action::Switch,
            origin: Origin::Follow,
            state: JobState::Finished(Outcome::Failed(Failure::new(Phase::Building, "boom"))),
            ordered_at: Timestamp(0),
            started_at: None,
            finished_at: None,
            toplevel: None,
            attempt: 1,
        });

        let checkup = Checkup::of(&view, "0.3.0");
        let titles: Vec<&str> = checkup
            .findings
            .iter()
            .map(|finding| finding.title.as_str())
            .collect();

        assert_eq!(checkup.findings[0].severity, Severity::Fail);
        assert!(titles.iter().any(|title| title.contains("0.2.0")));
        assert!(titles.iter().any(|title| title.contains("kernel")));
        assert!(
            titles
                .iter()
                .any(|title| title.contains("#7 failed building: boom"))
        );
    }
}
