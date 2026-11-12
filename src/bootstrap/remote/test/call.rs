use zeroize::Zeroizing;

use crate::{
    bootstrap::{
        SshKey,
        remote::{AdoptCall, Call, Privilege},
    },
    common::model::OrderId,
};

const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBqyOKVkbPDCemEVQJa2aQiocqctG5AenH2AeYZEHd/L";
const COEUS: &str = "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0";
const SOURCE: &str = "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-source";
const NIX_PATH: &str =
    r#"PATH="/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH" exec "$@""#;

fn adopt(install_bootloader: bool, follow: Option<u64>) -> Call {
    Call::Adopt(AdoptCall {
        coeus: COEUS.parse().unwrap(),
        source: SOURCE.into(),
        nar_hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            .parse()
            .unwrap(),
        label: "altar:coeus/bootstrap/nova@a1b2c3d".into(),
        hostname: "nova".parse().unwrap(),
        order: OrderId(0x5f0e_0000_0000_00ff),
        install_bootloader,
        follow,
    })
}

#[test]
fn the_probe_reads_its_script_on_stdin_and_is_told_the_login() {
    let call = Call::Probe {
        login_user: "nixos".into(),
    };
    assert_eq!(call.argv(), ["sh", "-s", "--", "nixos"]);
    assert_eq!(call.stdin(), Some(include_bytes!("../probe.sh").as_slice()));
    assert!(!call.privileged());
}

#[test]
fn adopt_passes_every_flag_and_only_asked_ones() {
    let prefix = [
        "sh",
        "-c",
        NIX_PATH,
        "sh",
        &format!("{COEUS}/bin/coeusd"),
        "adopt",
    ];
    let flags = [
        "--source",
        SOURCE,
        "--nar-hash",
        "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "--label",
        "altar:coeus/bootstrap/nova@a1b2c3d",
        "--hostname",
        "nova",
        "--order",
        "5f0e0000000000ff",
    ];
    let plain: Vec<&str> = prefix.iter().chain(&flags).copied().collect();
    assert_eq!(adopt(false, None).argv(), plain);

    let mut full = plain.clone();
    full.extend(["--install-bootloader", "--follow", "214"]);
    assert_eq!(adopt(true, Some(214)).argv(), full);
    assert!(adopt(true, None).privileged());
}

#[test]
fn the_fleets_coeus_is_built_from_the_shipped_source_with_its_substituters() {
    let call = Call::BuildCoeus {
        source: SOURCE.parse().unwrap(),
        attribute: "nixosConfigurations.nova.config.services.coeus.package".into(),
    };
    let argv = call.argv();
    assert_eq!(argv[..4], ["sh", "-c", NIX_PATH, "sh"]);
    assert_eq!(argv[4], "nix");
    for flag in ["--no-write-lock-file", "--json", "--no-link"] {
        assert!(argv.contains(&flag.to_owned()), "{flag}");
    }
    assert!(
        argv.windows(3)
            .any(|w| w == ["--option", "accept-flake-config", "true"])
    );
    assert!(
        argv.windows(2)
            .any(|w| w == ["--log-format", "internal-json"])
    );
    assert_eq!(
        argv.last().unwrap(),
        &format!("path:{SOURCE}#nixosConfigurations.nova.config.services.coeus.package")
    );
    assert!(call.privileged());
}

#[test]
fn the_session_key_line_carries_its_comment_and_removal_matches_the_key() {
    let key = SshKey::parse(KEY).unwrap();
    let install = Call::InstallSessionKey {
        key: key.clone(),
        host: "nova".parse().unwrap(),
    };
    let argv = install.argv();
    assert_eq!(argv[..2], ["sh", "-c"]);
    assert_eq!(
        argv[3..],
        ["sh", &format!("{KEY} coeus-bootstrap@nova"), KEY]
    );

    let remove = Call::RemoveSessionKey { key }.argv();
    assert_eq!(remove[3..], ["sh", KEY]);
    assert!(install.privileged());
}

#[test]
fn the_rest_are_pinned() {
    assert_eq!(
        Call::RepairNix.argv(),
        ["/nix/nix-installer", "repair", "--no-confirm"]
    );
    assert!(Call::InstallNix.argv()[2].ends_with("| sh -s -- install --no-confirm"));
    assert!(Call::InstallNix.argv()[2].contains("https://install.determinate.systems/nix"));
    assert_eq!(
        Call::HardwareConfig.argv()[4..],
        ["nixos-generate-config", "--show-hardware-config"]
    );
    let status = Call::AdoptStatus {
        coeus: COEUS.parse().unwrap(),
        nar_hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            .parse()
            .unwrap(),
    };
    assert_eq!(
        status.argv()[5..],
        [
            "adopt",
            "--status",
            "--nar-hash",
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ]
    );
    assert!(status.privileged());
    assert!(Call::MoveEtcAside.privileged());
    assert!(!Call::Tailscale.privileged());
}

#[test]
fn sudo_wraps_only_privileged_calls_and_reads_its_password_first() {
    let probe = Call::Probe {
        login_user: "nixos".into(),
    };
    let password = Privilege::SudoPassword(Zeroizing::new("hunter2".into()));

    for privilege in [Privilege::Root, Privilege::Sudo, password] {
        assert_eq!(privilege.wrap(&probe), (probe.argv(), None));
    }

    let call = Call::HardwareConfig;
    assert_eq!(Privilege::Root.wrap(&call), (call.argv(), None));

    let (argv, line) = Privilege::Sudo.wrap(&call);
    assert_eq!(argv[..4], ["sudo", "-H", "-n", "--"]);
    assert_eq!(argv[4..], call.argv());
    assert_eq!(line, None);

    let password = Privilege::SudoPassword(Zeroizing::new("hunter2".into()));
    let (argv, line) = password.wrap(&call);
    assert_eq!(argv[..7], ["sudo", "-H", "-k", "-S", "-p", "", "--"]);
    assert_eq!(argv[7..], call.argv());
    assert_eq!(line, Some("hunter2"));
}

/// what the CLI runs on a machine must be what that machine's coeusd takes
#[test]
fn coeusd_parses_every_adopt_it_is_sent() {
    use clap::Parser;

    use crate::cli::daemon::DaemonCLIArgs;

    let status = Call::AdoptStatus {
        coeus: COEUS.parse().unwrap(),
        nar_hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            .parse()
            .unwrap(),
    };
    let calls = [adopt(false, None), adopt(true, Some(214)), status];
    for call in calls {
        let argv = call.argv();
        let coeusd = argv
            .iter()
            .position(|word| word.ends_with("/bin/coeusd"))
            .expect("the call runs coeusd");
        DaemonCLIArgs::try_parse_from(&argv[coeusd..])
            .unwrap_or_else(|error| panic!("coeusd refuses {:?}: {error}", &argv[coeusd..]));
    }
}

#[test]
fn the_full_disk_access_pane_opens_for_whoever_sits_at_the_mac() {
    let call = Call::OpenFullDiskAccess {
        launcher: "/Library/Application Support/coeus/coeus-launcher".into(),
    };
    let argv = call.argv();
    assert!(call.privileged(), "launchctl asuser needs root");
    assert!(argv[2].contains("Privacy_AllFiles"), "{argv:?}");
    assert_eq!(
        argv.last().unwrap(),
        "/Library/Application Support/coeus/coeus-launcher"
    );
}
