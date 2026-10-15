//! Isolated state plus injected build/activation/SSH/health boundaries. No host commands.
use super::*;
use super::{
    build::{BuildExecutor, BuiltSystem, build_bootstrap},
    remote::{RemoteExecutor, remote_with},
    switch::{activate_deadline, activation_commands, prepare_built},
};
use eyre::{Result, bail, ensure};
use futures_util::future::BoxFuture;
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

const CLOSURE: &str = "/nix/store/00000000000000000000000000000000-system";
fn service(platform: &str) -> ServiceIdentity {
    ServiceIdentity {
        host: "fixture".into(),
        platform: platform.into(),
        daemon: "/nix/store/00000000000000000000000000000000-coeus/bin/coeusd".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: crate::common::proto::PROTOCOL_VERSION,
        user: "fixture".into(),
        account: crate::activation::AccountPolicy {
            managed: false,
            uid: None,
            gid: None,
        },
        state_directory: "/fixture/state".into(),
        config_file: "/fixture/config.toml".into(),
        runner: "/nix/store/00000000000000000000000000000000-runner".into(),
        elevation_adapter: "/nix/store/00000000000000000000000000000000-adapter".into(),
        nix: "/nix/store/00000000000000000000000000000000-nix/bin/nix".into(),
        service_definition: "fixture unit".into(),
    }
}
fn store(directory: &Path) -> Result<ReceiptStore> {
    ReceiptStore::open(&directory.join("activation"))
}
fn receipt(store: &ReceiptStore, manual: bool) -> Result<Receipt> {
    let expected = service("linux");
    let mut old = expected.clone();
    if manual {
        old.runner = "/nix/store/11111111111111111111111111111111-old-runner".into();
    }
    prepare_built(store, 42, CLOSURE.into(), expected, Some(old), Some(123))
}
#[derive(Default)]
struct Fake {
    switched: AtomicUsize,
    healthy: AtomicBool,
    fail: AtomicBool,
    hang: AtomicBool,
}
impl ActivationExecutor for Fake {
    fn current(&self) -> Result<PathBuf> {
        Ok(CLOSURE.into())
    }
    fn validate<'a>(&'a self, _: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn switch<'a>(&'a self, _: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.switched.fetch_add(1, Ordering::SeqCst);
            if self.hang.load(Ordering::SeqCst) {
                futures_util::future::pending::<()>().await;
            }
            ensure!(
                !self.fail.load(Ordering::SeqCst),
                "simulated lost connection/activation failure"
            );
            self.healthy.store(true, Ordering::SeqCst);
            Ok(())
        })
    }
    fn verify<'a>(&'a self, _: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(
                self.healthy.load(Ordering::SeqCst),
                "wrong closure/daemon/version/identity"
            );
            Ok(())
        })
    }
}
#[tokio::test]
async fn normal_automatic_activation_and_duplicate_switch_do_not_replay() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path())?;
    assert!(!receipt(&store, false)?.manual);
    let fake = Fake::default();
    assert_eq!(
        activate(&store, 42, &fake).await?.state,
        ActivationState::Verified
    );
    activate(&store, 42, &fake).await?;
    assert_eq!(fake.switched.load(Ordering::SeqCst), 1);
    Ok(())
}
#[test]
fn detection_covers_executable_runner_and_full_service_definition() -> Result<()> {
    for field in ["daemon", "runner", "unit"] {
        let dir = tempfile::tempdir()?;
        let store = store(dir.path())?;
        let old = service("linux");
        let mut changed = old.clone();
        match field {
            "daemon" => {
                changed.daemon =
                    "/nix/store/11111111111111111111111111111111-coeus/bin/coeusd".into()
            }
            "runner" => {
                changed.runner = "/nix/store/11111111111111111111111111111111-runner".into()
            }
            _ => changed.service_definition.push_str(" override"),
        }
        assert!(prepare_built(&store, 42, CLOSURE.into(), changed, Some(old), Some(1))?.manual);
    }
    Ok(())
}
#[tokio::test]
async fn failure_is_durable_and_reconciliation_only_observes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path())?;
    receipt(&store, true)?;
    let fake = Fake::default();
    fake.fail.store(true, Ordering::SeqCst);
    assert!(activate(&store, 42, &fake).await.is_err());
    assert_eq!(store.load(42)?.state, ActivationState::Uncertain);
    assert!(activate(&store, 42, &fake).await.is_err());
    assert!(reconcile(&store, 42, &fake).await.is_err());
    fake.healthy.store(true, Ordering::SeqCst);
    assert_eq!(
        reconcile(&store, 42, &fake).await?.state,
        ActivationState::Verified
    );
    assert_eq!(fake.switched.load(Ordering::SeqCst), 1);
    Ok(())
}
#[tokio::test]
async fn activation_timeout_never_replays_after_restart() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let directory = dir.path().join("activation");
    let store = ReceiptStore::open(&directory)?;
    receipt(&store, true)?;
    let fake = Fake::default();
    fake.hang.store(true, Ordering::SeqCst);
    assert!(
        activate_deadline(&store, 42, &fake, Duration::from_millis(5))
            .await
            .is_err()
    );
    assert_eq!(store.load(42)?.state, ActivationState::Uncertain);
    drop(store);
    let store = ReceiptStore::open(&directory)?;
    assert!(activate(&store, 42, &fake).await.is_err());
    fake.healthy.store(true, Ordering::SeqCst);
    reconcile(&store, 42, &fake).await?;
    assert_eq!(fake.switched.load(Ordering::SeqCst), 1);
    Ok(())
}
#[test]
fn receipt_lock_permissions_and_symlink_ownership_are_enforced() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir()?;
    let directory = dir.path().join("activation");
    let store = ReceiptStore::open(&directory)?;
    receipt(&store, true)?;
    assert!(ReceiptStore::open(&directory).is_err());
    let real = directory.join("42.toml");
    let renamed = directory.join("original.toml");
    std::fs::rename(&real, &renamed)?;
    symlink(&renamed, &real)?;
    assert!(store.load(42).is_err());
    std::fs::remove_file(real)?;
    drop(store);
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))?;
    assert!(ReceiptStore::open(&directory).is_err());
    Ok(())
}
#[test]
fn account_state_and_host_migrations_fail_before_receipt() -> Result<()> {
    for field in ["user", "state", "host"] {
        let dir = tempfile::tempdir()?;
        let store = store(dir.path())?;
        let old = service("linux");
        let mut changed = old.clone();
        match field {
            "user" => changed.user = "other".into(),
            "state" => changed.state_directory = "/different/state".into(),
            _ => changed.host = "other".into(),
        }
        assert!(prepare_built(&store, 42, CLOSURE.into(), changed, Some(old), None).is_err());
        assert!(store.load(42).is_err());
    }
    Ok(())
}
#[test]
fn platform_commands_preserve_exact_closure_and_pinned_nh_privilege_contract() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path())?;
    let mut r = receipt(&store, true)?;
    let linux = activation_commands(&r);
    assert_eq!(
        linux[0],
        [
            format!("{CLOSURE}/bin/switch-to-configuration"),
            "test".into()
        ]
    );
    assert_eq!(linux[1][0], r.expected.nix.to_string_lossy());
    assert_eq!(
        linux[1][1..],
        [
            "build",
            "--no-link",
            "--profile",
            "/nix/var/nix/profiles/system",
            CLOSURE
        ]
    );
    assert_eq!(linux[2][1], "boot");
    assert_eq!(linux[3][1..], ["restart", "coeusd.service"]);
    r.expected.platform = "darwin".into();
    let darwin = activation_commands(&r);
    assert_eq!(darwin.len(), 2);
    assert_eq!(
        darwin[1],
        [
            format!("{CLOSURE}/sw/bin/darwin-rebuild"),
            "activate".into()
        ]
    );
    assert!(
        darwin
            .iter()
            .flatten()
            .all(|arg| arg != "." && arg != "switch")
    );
    Ok(())
}
struct FakeBuild {
    calls: Mutex<Vec<String>>,
    platform: String,
}
impl BuildExecutor for FakeBuild {
    fn build<'a>(&'a self, attr: &'a str, _: &'a Path) -> BoxFuture<'a, Result<BuiltSystem>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(attr.into());
            Ok(BuiltSystem {
                closure: CLOSURE.into(),
                identity: service(&self.platform),
            })
        })
    }
}
#[tokio::test]
async fn first_install_builds_then_uses_same_activation_receipt_and_engine() -> Result<()> {
    for platform in ["linux", "darwin"] {
        let dir = tempfile::tempdir()?;
        let store = store(dir.path())?;
        let builder = FakeBuild {
            calls: Mutex::default(),
            platform: platform.into(),
        };
        let receipt = build_bootstrap(
            &store,
            42,
            "github:owner/repo/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "fixture",
            platform,
            &builder,
        )
        .await?;
        assert!(receipt.manual && receipt.previous.is_none());
        assert_eq!(receipt.state, ActivationState::Ready);
        let fake = Fake::default();
        activate(&store, 42, &fake).await?;
        assert!(
            build_bootstrap(
                &store,
                42,
                "github:owner/repo/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "fixture",
                platform,
                &builder
            )
            .await
            .is_err()
        );
        assert_eq!(builder.calls.lock().unwrap().len(), 1);
        let expected = if platform == "darwin" {
            "github:owner/repo/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa#darwinConfigurations.fixture.system"
        } else {
            "github:owner/repo/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa#nixosConfigurations.fixture.config.system.build.toplevel"
        };
        assert_eq!(builder.calls.lock().unwrap()[0], expected);
        assert_eq!(fake.switched.load(Ordering::SeqCst), 1);
    }
    Ok(())
}
#[derive(Default)]
struct FakeSsh {
    calls: Mutex<Vec<String>>,
    bad_identity: AtomicBool,
    lost_connection: AtomicBool,
}
impl RemoteExecutor for FakeSsh {
    fn script<'a>(&'a self, _: &'a str, script: String) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(script.clone());
            if script.contains("mktemp") {
                return Ok("Linux\nx86_64\n/tmp/coeus.fixture123\n".into());
            }
            if script.contains(" identity") {
                let mut identity = BinaryIdentity::current();
                identity.os = "linux".into();
                identity.arch = "x86_64".into();
                if self.bad_identity.load(Ordering::SeqCst) {
                    identity.protocol += 1;
                }
                return Ok(toml::to_string(&identity)?);
            }
            if script.contains("activate-local") && self.lost_connection.load(Ordering::SeqCst) {
                bail!("lost SSH connection; outcome uncertain");
            }
            Ok(String::new())
        })
    }
    fn copy<'a>(
        &'a self,
        host: &'a str,
        _: &'a Path,
        directory: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(format!("copy {host} {directory}"));
            Ok(())
        })
    }
}
fn remote(binary: &Path) -> RemoteArgs {
    RemoteArgs {
        ssh: "laptop".into(),
        activation_directory: "/fixture/private dir/a'quote".into(),
        peer_config: "/fixture/config.toml".into(),
        copy_binary: Some(binary.into()),
        operator_sudo: true,
    }
}
#[tokio::test]
async fn ssh_bootstrap_and_switch_are_independent_and_quote_input_without_latest_fallback()
-> Result<()> {
    let binary = tempfile::NamedTempFile::new()?;
    let args = remote(binary.path());
    let ssh = FakeSsh::default();
    remote_with(
        &ssh,
        &args,
        42,
        Some((
            "github:owner/repo/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "fixture",
            Path::new("/nix/var/nix/profiles/default/bin/nix"),
        )),
        false,
    )
    .await?;
    let calls = ssh.calls.lock().unwrap();
    let invocation = calls
        .iter()
        .find(|s| s.contains("bootstrap-local"))
        .unwrap();
    assert!(invocation.contains("--activation-directory '/fixture/private dir/a'\\''quote'"));
    assert!(calls.last().unwrap().starts_with("rm -f /tmp/coeus."));
    assert!(!calls.iter().any(|s| s.contains("latest")
        || s.contains("StrictHostKeyChecking=no")
        || s.contains("nix-channel")));
    assert!(
        calls
            .iter()
            .any(|s| s.contains("--nix-binary '/nix/var/nix/profiles/default/bin/nix'"))
    );
    assert!(super::remote::ssh_arguments("laptop").contains(&"StrictHostKeyChecking=yes"));
    Ok(())
}
#[tokio::test]
async fn binary_identity_mismatch_never_launches_switch_and_lost_connection_never_retries()
-> Result<()> {
    let binary = tempfile::NamedTempFile::new()?;
    let args = remote(binary.path());
    let ssh = FakeSsh::default();
    ssh.bad_identity.store(true, Ordering::SeqCst);
    assert!(remote_with(&ssh, &args, 42, None, false).await.is_err());
    assert!(
        !ssh.calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("activate-local"))
    );
    ssh.bad_identity.store(false, Ordering::SeqCst);
    ssh.lost_connection.store(true, Ordering::SeqCst);
    assert!(remote_with(&ssh, &args, 42, None, false).await.is_err());
    assert_eq!(
        ssh.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.contains("activate-local"))
            .count(),
        1
    );
    Ok(())
}
#[tokio::test]
async fn unsafe_ssh_target_or_floating_bootstrap_fails_before_transport() -> Result<()> {
    let binary = tempfile::NamedTempFile::new()?;
    let mut args = remote(binary.path());
    let ssh = FakeSsh::default();
    args.ssh = "-oProxyCommand=touch /tmp/bad".into();
    assert!(remote_with(&ssh, &args, 42, None, false).await.is_err());
    args.ssh = "laptop".into();
    assert!(
        remote_with(
            &ssh,
            &args,
            42,
            Some((
                "github:owner/repo/main",
                "fixture",
                Path::new("/nix/var/nix/profiles/default/bin/nix")
            )),
            false
        )
        .await
        .is_err()
    );
    assert!(ssh.calls.lock().unwrap().is_empty());
    Ok(())
}
#[test]
fn service_reader_includes_actual_unit_overrides_not_just_source_or_package() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("coeus-service.toml"),
        toml::to_string(&service("linux"))?,
    )?;
    std::fs::create_dir_all(dir.path().join("etc/systemd/system"))?;
    let unit = dir.path().join("etc/systemd/system/coeusd.service");
    std::fs::write(&unit, "initial")?;
    let before = ServiceIdentity::read(dir.path())?;
    std::fs::write(unit, "changed environment override")?;
    assert_ne!(before, ServiceIdentity::read(dir.path())?);
    Ok(())
}

#[test]
fn post_switch_checks_reject_old_pid_wrong_executable_host_version_protocol_and_platform()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path())?;
    let r = receipt(&store, true)?;
    let good = DaemonIdentity {
        binary: BinaryIdentity::current(),
        host: r.expected.host.clone(),
        executable: r.expected.daemon.clone(),
        runner: r.expected.runner.clone(),
        process_id: 456,
    };
    let mut good = good;
    good.binary.os = "linux".into();
    super::switch::verify_daemon_identity(&r, &good, &r.expected.daemon)?;
    for field in [
        "pid", "exe", "host", "version", "protocol", "os", "arch", "runner",
    ] {
        let mut bad = good.clone();
        match field {
            "pid" => bad.process_id = 123,
            "exe" => bad.executable = "/other".into(),
            "host" => bad.host = "other".into(),
            "version" => bad.binary.version = "wrong".into(),
            "protocol" => bad.binary.protocol = 1,
            "os" => bad.binary.os = "other".into(),
            "arch" => bad.binary.arch = "other".into(),
            _ => bad.runner = "/other".into(),
        }
        assert!(
            super::switch::verify_daemon_identity(&r, &bad, &r.expected.daemon).is_err(),
            "accepted wrong {field}"
        );
    }
    Ok(())
}
#[tokio::test]
async fn build_only_cancelled_receipt_never_activates() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path())?;
    let mut r = receipt(&store, true)?;
    r.state = ActivationState::Cancelled;
    store.save(&r)?;
    let fake = Fake::default();
    assert!(activate(&store, 42, &fake).await.is_err());
    assert_eq!(fake.switched.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn concurrent_operator_calls_in_same_process_are_excluded() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = std::sync::Arc::new(store(dir.path())?);
    receipt(&store, true)?;
    let fake = std::sync::Arc::new(Fake::default());
    fake.hang.store(true, Ordering::SeqCst);
    let owner_store = store.clone();
    let owner_fake = fake.clone();
    let owner = tokio::spawn(async move { activate(&owner_store, 42, owner_fake.as_ref()).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while fake.switched.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(activate(&store, 42, fake.as_ref()).await.is_err());
    assert!(reconcile(&store, 42, fake.as_ref()).await.is_err());
    owner.abort();
    let _ = owner.await;
    assert_eq!(store.load(42)?.state, ActivationState::Activating);
    assert!(activate(&store, 42, fake.as_ref()).await.is_err());
    fake.healthy.store(true, Ordering::SeqCst);
    reconcile(&store, 42, fake.as_ref()).await?;
    assert_eq!(fake.switched.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn bootstrap_nix_binary_requires_explicit_existing_executable_without_path_lookup() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    let binary = temp.path().join("nix with spaces");
    assert!(super::build::NativeBuild::new(Path::new("nix")).is_err());
    assert!(super::build::NativeBuild::new(&binary).is_err());
    std::fs::write(&binary, "fixture, never executed")?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o600))?;
    assert!(super::build::NativeBuild::new(&binary).is_err());
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    super::build::NativeBuild::new(&binary)?;
    assert!(super::build::NativeBuild::new(temp.path()).is_err());
    Ok(())
}

#[test]
fn managed_account_uid_gid_and_lifecycle_migrations_fail_before_receipt() -> Result<()> {
    for field in ["uid", "gid", "managed"] {
        let temp = tempfile::tempdir()?;
        let store = store(temp.path())?;
        let mut old = service("darwin");
        old.account = AccountPolicy {
            managed: true,
            uid: Some(498),
            gid: Some(20),
        };
        let mut changed = old.clone();
        match field {
            "uid" => changed.account.uid = Some(499),
            "gid" => changed.account.gid = Some(21),
            _ => {
                changed.account = AccountPolicy {
                    managed: false,
                    uid: None,
                    gid: None,
                }
            }
        }
        assert!(prepare_built(&store, 42, CLOSURE.into(), changed, Some(old), Some(123)).is_err());
        assert!(!store.directory().join("42.toml").exists());
    }
    Ok(())
}
