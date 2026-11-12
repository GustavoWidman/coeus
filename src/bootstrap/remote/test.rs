use serde_json::json;

use super::{BootLoader, Facts, Platform, System};
use crate::common::model;

pub(crate) fn nixos() -> Facts {
    Facts {
        platform: Platform::Nixos,
        system: System::X86_64Linux,
        os_version: "26.05".into(),
        state_version: Some("25.11".into()),
        hostname: "nixos".into(),
        efi: true,
        boot_loader: Some(BootLoader::SystemdBoot),
        boot_device: None,
        hardware_config: Some("{ ... }: { }\n".into()),
        time_zone: Some("America/Sao_Paulo".into()),
        login_user: "nixos".into(),
        main_user: Some("nixos".into()),
        cpus: 8,
        memory_mib: 16_384,
        nix_version: Some("2.31.2".into()),
    }
}

#[test]
fn facts_are_the_contracts_json_exactly() {
    assert_eq!(
        serde_json::to_value(nixos()).unwrap(),
        json!({
            "platform": "nixos",
            "system": "x86_64-linux",
            "osVersion": "26.05",
            "stateVersion": "25.11",
            "hostname": "nixos",
            "efi": true,
            "bootLoader": "systemd-boot",
            "bootDevice": null,
            "hardwareConfig": "{ ... }: { }\n",
            "timeZone": "America/Sao_Paulo",
            "loginUser": "nixos",
            "mainUser": "nixos",
            "cpus": 8,
            "memoryMib": 16384,
            "nixVersion": "2.31.2",
        })
    );
}

#[test]
fn a_mac_without_nix_says_null_where_nixos_would_answer() {
    let mac = Facts {
        platform: Platform::Darwin,
        system: System::Aarch64Darwin,
        os_version: "26.6.2".into(),
        state_version: None,
        hostname: "lumen".into(),
        efi: false,
        boot_loader: None,
        hardware_config: None,
        time_zone: None,
        login_user: "r3dlust".into(),
        main_user: Some("r3dlust".into()),
        nix_version: None,
        ..nixos()
    };
    let value = serde_json::to_value(&mac).unwrap();

    assert_eq!(value["platform"], "darwin");
    assert_eq!(value["system"], "aarch64-darwin");
    for null in ["stateVersion", "bootLoader", "hardwareConfig", "nixVersion"] {
        assert!(value[null].is_null(), "{null}");
    }
    assert_eq!(serde_json::from_value::<Facts>(value).unwrap(), mac);
    assert_eq!(model::Platform::from(mac.platform), model::Platform::Darwin);
}

#[test]
fn facts_with_a_field_the_contract_does_not_know_are_refused() {
    let mut value = serde_json::to_value(nixos()).unwrap();
    value["gpu"] = json!("none");
    assert!(serde_json::from_value::<Facts>(value).is_err());

    for loader in ["grub", "limine"] {
        let mut value = serde_json::to_value(nixos()).unwrap();
        value["bootLoader"] = json!(loader);
        assert!(serde_json::from_value::<Facts>(value).is_ok(), "{loader}");
    }
}
