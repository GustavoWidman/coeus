use crate::{
    client::Targets,
    config::{ClientSettings, HostEntry},
};

#[tokio::test]
async fn stages_a_snapshot_beneath_no_symlink() {
    let tree = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(tree.path())
        .args(["init", "--quiet"])
        .status()
        .unwrap();
    assert!(status.success());

    let stage = crate::client::Snapshot::stage(tree.path()).await.unwrap();
    assert_eq!(
        stage.path(),
        std::fs::canonicalize(stage.path()).unwrap(),
        "nix 2.34 will not pack a path beneath a symlink"
    );
}

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

mod tailnet {
    use std::time::{Duration, Instant};

    use crate::{
        client::{Fleet, Offline, Tailnet, Targets},
        common::model::Timestamp,
        config::{ClientSettings, HostEntry, Key},
    };

    /// a tailscale in touch with its coordination server, which sees `gone`
    /// offline since 2026-11-15 23:41 UTC and `here` online
    const CURRENT: &[u8] = br#"{"BackendState":"Running",
        "Self":{"HostName":"altar","DNSName":"altar.tail1.ts.net.","TailscaleIPs":["100.64.0.1"],"Online":true},
        "Peer":{
            "nodekey:a":{"HostName":"desktop","DNSName":"gone.tail1.ts.net.","TailscaleIPs":["100.64.0.2","fd7a::2"],"Online":false,"LastSeen":"2026-11-15T23:41:00Z"},
            "nodekey:b":{"HostName":"here","DNSName":"here.tail1.ts.net.","TailscaleIPs":["100.64.0.3"],"Online":true,"LastSeen":"0001-01-01T00:00:00Z"},
            "nodekey:c":{"HostName":"never","DNSName":"never.tail1.ts.net.","TailscaleIPs":["100.64.0.4"],"Online":false,"LastSeen":"0001-01-01T00:00:00Z"}
        }}"#;

    fn current() -> Tailnet {
        Tailnet::parse(CURRENT).unwrap()
    }

    #[test]
    fn a_peer_is_found_by_its_magicdns_name() {
        let status = br#"{"MagicDNSSuffix":"tail1.ts.net","Peer":{
            "nodekey:a":{"HostName":"localhost","DNSName":"nova.tail1.ts.net.","TailscaleIPs":["100.99.40.45","fd7a::1"]},
            "nodekey:b":{"HostName":"nova","DNSName":"nova-1.tail1.ts.net.","TailscaleIPs":["100.1.2.3"]}
        }}"#;
        let tailnet = Tailnet::parse(status).unwrap();
        assert_eq!(
            tailnet.address("nova").as_deref(),
            Some("100.99.40.45"),
            "MagicDNS's name wins over the machine's own hostname"
        );
        assert_eq!(tailnet.address("lab"), None);
        assert!(Tailnet::parse(b"not json").is_none());
    }

    #[test]
    fn a_machine_the_coordination_server_sees_offline_is_named_by_any_address_coeus_dials() {
        let tailnet = current();
        let last_seen = Some(Timestamp(1_794_786_060_000));
        for host in [
            "gone",
            "GONE",
            "gone.tail1.ts.net",
            "gone.tail1.ts.net.",
            "100.64.0.2",
            "fd7a::2",
        ] {
            assert_eq!(tailnet.offline(host), Some(Offline { last_seen }), "{host}");
        }
        assert_eq!(
            tailnet.offline("never"),
            Some(Offline { last_seen: None }),
            "the zero time means it was never seen"
        );
        assert_eq!(tailnet.offline("here"), None, "online");
        assert_eq!(tailnet.offline("100.64.0.3"), None, "online");
        assert_eq!(tailnet.offline("lab"), None, "not in the tailnet");
        assert_eq!(tailnet.offline("127.0.0.1"), None, "not in the tailnet");
    }

    #[test]
    fn a_tailscale_out_of_touch_with_its_coordination_server_names_nobody_offline() {
        let status = String::from_utf8(CURRENT.to_vec()).unwrap();
        let cut_off = status.replacen(r#""Online":true}"#, r#""Online":false}"#, 1);
        let stopped = status.replace(r#""Running""#, r#""Stopped""#);
        for status in [cut_off, stopped] {
            let tailnet = Tailnet::parse(status.as_bytes()).unwrap();
            assert_eq!(tailnet.offline("gone"), None, "{status}");
        }
        assert_eq!(Tailnet::default().offline("gone"), None);
    }

    #[test]
    fn an_offline_machine_says_when_tailscale_last_saw_it() {
        let seen = Timestamp(1_794_786_060_000);
        let now = seen.after(Duration::from_secs(17 * 3600));
        assert_eq!(
            Offline {
                last_seen: Some(seen)
            }
            .describe(now),
            "offline, tailscale last saw it 17h ago"
        );
        assert_eq!(
            Offline { last_seen: None }.describe(now),
            "offline, as tailscale sees it"
        );
    }

    /// a fleet of one machine, reached at a port that accepts connections
    /// and never says a word, as a machine whose coeusd hangs
    async fn silent(
        address: &str,
        timeout: Duration,
    ) -> (
        Fleet,
        tokio::net::TcpListener,
        crate::common::model::Hostname,
    ) {
        let listener = tokio::net::TcpListener::bind((address, 0)).await.unwrap();
        let mut settings = ClientSettings {
            connect_timeout: timeout,
            ..ClientSettings::default()
        };
        let host = crate::common::model::Hostname::try_from("gone".to_owned()).unwrap();
        settings.hosts.insert(
            host.clone(),
            HostEntry {
                address: Some(address.to_owned()),
                port: Some(listener.local_addr().unwrap().port()),
                ..HostEntry::default()
            },
        );
        let targets = Targets::select(&["gone".to_owned()], &settings).unwrap();
        (
            Fleet::new(settings, Key::generate(), targets),
            listener,
            host,
        )
    }

    #[tokio::test]
    async fn an_offline_machine_fails_without_waiting_out_the_connect_timeout() {
        let (fleet, _listener, host) = silent("127.0.0.1", Duration::from_secs(30)).await;
        let status = String::from_utf8(CURRENT.to_vec())
            .unwrap()
            .replace("100.64.0.2", "127.0.0.1");
        let fleet = fleet.seeing(Tailnet::parse(status.as_bytes()).unwrap());

        let started = Instant::now();
        let error = fleet.connect(&host).await.err().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert!(
            error
                .to_string()
                .starts_with("offline, tailscale last saw it"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_machine_tailscale_sees_online_gets_the_whole_connect_timeout() {
        let (fleet, _listener, host) = silent("127.0.0.1", Duration::from_millis(300)).await;
        let status = String::from_utf8(CURRENT.to_vec())
            .unwrap()
            .replace("100.64.0.3", "127.0.0.1");
        let fleet = fleet.seeing(Tailnet::parse(status.as_bytes()).unwrap());

        let started = Instant::now();
        let error = fleet.connect(&host).await.err().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(error.to_string().starts_with("no answer within"), "{error}");
    }
}
