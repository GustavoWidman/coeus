use crate::bootstrap::{
    SshKey,
    remote::{BootLoader, NixState, Platform, Probed, System},
};

/// probe.sh's output on a stock NixOS 26.05 EFI install
const NIXOS: &str = "\
kernel=Linux
machine=x86_64
hostname=nixos
loginUser=nixos
hostKey=ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDiNyjv0Q6QPc9pIbDabhan4An9PCIK6b6Ws6iI2cyhc root@nixos
timeZone=
nixVersion=nix (Nix) 2.31.2+2
nixReceipt=
osId=nixos
osName=NixOS 26.05 (Yarara)
osVersion=26.05
stateVersion=25.11
efi=yes
bootLoader=systemd-boot
bootDevice=
cpus=4
memoryMib=5923
";

/// probe.sh's output on a vanilla macOS 26 without nix
const MAC: &str = "\
kernel=Darwin
machine=arm64
hostname=Manageds-Virtual-Machine.local
loginUser=admin
hostKey=ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIN6qrrQQLLhqDXbTqS9/5GFBII1ZRonUIIRyuUd6f8l7 
timeZone=UTC
nixVersion=
nixReceipt=
osVersion=26.6.2
consoleUser=admin
cpus=4
memoryMib=8192
";

const HARDWARE: &str = "{ ... }: { boot.initrd.availableKernelModules = [ \"virtio_pci\" ]; }\n";

#[test]
fn a_stock_nixos_becomes_the_contracts_facts() {
    let probed = Probed::parse(NIXOS, Some(HARDWARE.into())).unwrap();
    let facts = &probed.facts;

    assert_eq!(facts.platform, Platform::Nixos);
    assert_eq!(facts.system, System::X86_64Linux);
    assert_eq!(facts.os_version, "26.05");
    assert_eq!(facts.state_version.as_deref(), Some("25.11"));
    assert_eq!(facts.hostname, "nixos");
    assert!(facts.efi);
    assert_eq!(facts.boot_loader, Some(BootLoader::SystemdBoot));
    assert_eq!(facts.boot_device, None);
    assert_eq!(facts.hardware_config.as_deref(), Some(HARDWARE));
    assert_eq!(facts.time_zone, None);
    assert_eq!(facts.login_user, "nixos");
    assert_eq!(facts.main_user.as_deref(), Some("nixos"));
    assert_eq!((facts.cpus, facts.memory_mib), (4, 5923));
    assert_eq!(facts.nix_version.as_deref(), Some("2.31.2+2"));

    assert_eq!(
        probed.host_key,
        SshKey::parse(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDiNyjv0Q6QPc9pIbDabhan4An9PCIK6b6Ws6iI2cyhc"
        )
        .unwrap()
    );
    assert_eq!(
        probed.nix,
        NixState::Installed {
            version: "2.31.2+2".into()
        }
    );
    assert_eq!(
        facts.describe(),
        "NixOS 26.05 · x86_64-linux · EFI · 5.8 GiB · 4 cpus"
    );
}

#[test]
fn a_mac_without_nix_has_no_nixos_facts() {
    let probed = Probed::parse(MAC, None).unwrap();
    let facts = &probed.facts;

    assert_eq!(facts.platform, Platform::Darwin);
    assert_eq!(facts.system, System::Aarch64Darwin);
    assert_eq!(facts.os_version, "26.6.2");
    assert_eq!(facts.hostname, "Manageds-Virtual-Machine");
    assert_eq!(facts.state_version, None);
    assert_eq!(facts.hardware_config, None);
    assert_eq!(facts.boot_loader, None);
    assert!(!facts.efi);
    assert_eq!(facts.time_zone.as_deref(), Some("UTC"));
    assert_eq!(facts.main_user.as_deref(), Some("admin"));
    assert_eq!(facts.nix_version, None);
    assert_eq!(probed.nix, NixState::Missing);
    assert_eq!(
        facts.describe(),
        "macOS 26.6.2 · aarch64-darwin · 8.0 GiB · 4 cpus · no nix yet"
    );
}

#[test]
fn a_mac_at_the_login_window_names_the_login_as_its_main_user() {
    let probed =
        Probed::parse(&MAC.replace("consoleUser=admin", "consoleUser=root"), None).unwrap();
    assert_eq!(probed.facts.main_user.as_deref(), Some("admin"));
}

#[test]
fn a_receipt_without_a_working_nix_is_a_broken_install() {
    let probed = Probed::parse(&MAC.replace("nixReceipt=", "nixReceipt=yes"), None).unwrap();
    assert_eq!(probed.nix, NixState::Broken);

    let determinate = MAC.replace(
        "nixVersion=",
        "nixVersion=nix (Determinate Nix 3.11.2) 2.31.2",
    );
    let probed = Probed::parse(&determinate, None).unwrap();
    assert_eq!(probed.facts.nix_version.as_deref(), Some("2.31.2"));
}

#[test]
fn nixos_reached_as_root_has_no_main_user() {
    let probed = Probed::parse(
        &NIXOS.replace("loginUser=nixos", "loginUser=root"),
        Some(HARDWARE.into()),
    )
    .unwrap();
    assert_eq!(probed.facts.login_user, "root");
    assert_eq!(probed.facts.main_user, None);
}

#[test]
fn a_linux_that_is_not_nixos_is_refused_by_name() {
    let ubuntu = NIXOS
        .replace("osId=nixos", "osId=ubuntu")
        .replace("osName=NixOS 26.05 (Yarara)", "osName=Ubuntu 24.04.1 LTS");
    let error = Probed::parse(&ubuntu, None).unwrap_err().to_string();
    assert!(error.contains("Ubuntu 24.04.1 LTS"), "{error}");
    assert!(error.contains("not NixOS"), "{error}");
}

#[test]
fn what_the_probe_could_not_learn_is_named() {
    let error = Probed::parse(&NIXOS.replace("cpus=4", "cpus="), Some(HARDWARE.into()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("cpus"), "{error}");

    let error = Probed::parse(NIXOS, None).unwrap_err().to_string();
    assert!(error.contains("hardware configuration"), "{error}");

    let error = Probed::parse(
        &NIXOS.replace("bootLoader=systemd-boot", "bootLoader=lilo"),
        Some(HARDWARE.into()),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("lilo"), "{error}");
}

#[test]
fn a_grub_machine_names_its_device() {
    let grub = NIXOS
        .replace("bootLoader=systemd-boot", "bootLoader=grub")
        .replace("bootDevice=", "bootDevice=/dev/sda")
        .replace("efi=yes", "efi=");
    let probed = Probed::parse(&grub, Some(HARDWARE.into())).unwrap();
    assert_eq!(probed.facts.boot_loader, Some(BootLoader::Grub));
    assert_eq!(probed.facts.boot_device.as_deref(), Some("/dev/sda"));
    assert!(!probed.facts.efi);
}
