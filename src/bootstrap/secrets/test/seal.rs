use std::{collections::BTreeMap, fs, path::Path};

use tempfile::tempdir;

use super::{admin, keypair, seal};
use crate::bootstrap::{
    Blocker, SshKey,
    contract::{RepoPath, Rules},
    secrets::{self, AdminIdentity, Header, Verdict},
};

fn path(text: &str) -> RepoPath {
    text.parse().unwrap()
}

fn put(root: &Path, file: &str, bytes: &[u8]) {
    let on_disk = root.join(file);
    fs::create_dir_all(on_disk.parent().unwrap()).unwrap();
    fs::write(on_disk, bytes).unwrap();
}

fn open(identity: &AdminIdentity, root: &Path, file: &str) -> Vec<u8> {
    identity
        .decrypt(&fs::read(root.join(file)).unwrap())
        .unwrap()
        .to_vec()
}

/// a clone holding one current, two stale and one unmanaged secret: the fleet
/// gains `host`, which `admin` and `peer` already open everything for
struct Fleet {
    root: tempfile::TempDir,
    admin: AdminIdentity,
    peer: SshKey,
    host: SshKey,
    rules: Rules,
}

async fn fleet() -> Fleet {
    let root = tempdir().unwrap();
    let keys = tempdir().unwrap();
    let admin = admin(keys.path(), "altar").await;
    let (_, peer) = keypair("peer");
    let (_, host) = keypair("host");
    let me = admin.key().clone();

    put(
        root.path(),
        "shared/current.age",
        &seal(b"current", &[&me, &peer], &[], false),
    );
    put(
        root.path(),
        "shared/binary.age",
        &seal(b"binary", &[&me], &[], false),
    );
    put(
        root.path(),
        "hosts/host/armored.age",
        &seal(b"armored", &[&peer, &me], &[], true),
    );
    put(root.path(), "other/unmanaged.age", b"anything");

    let rules = BTreeMap::from([
        (
            path("shared/current.age"),
            Some(vec![peer.clone(), me.clone()]),
        ),
        (
            path("shared/binary.age"),
            Some(vec![me.clone(), host.clone()]),
        ),
        (
            path("hosts/host/armored.age"),
            Some(vec![host.clone(), me.clone(), host.clone()]),
        ),
        (path("other/unmanaged.age"), None),
    ]);
    Fleet {
        root,
        admin,
        peer,
        host,
        rules,
    }
}

fn stale(plan: &secrets::Plan) -> Vec<&str> {
    plan.stale().map(|file| file.path.as_str()).collect()
}

#[tokio::test]
async fn the_plan_lists_what_disagrees_with_its_recipients() {
    let fleet = fleet().await;
    let plan = secrets::plan(fleet.root.path(), &fleet.rules).unwrap();

    assert_eq!(
        stale(&plan),
        ["hosts/host/armored.age", "shared/binary.age"]
    );
    assert_eq!(plan.unmanaged, [path("other/unmanaged.age")]);
    let verdicts: BTreeMap<&str, &Verdict> = plan
        .files
        .iter()
        .map(|file| (file.path.as_str(), &file.verdict))
        .collect();
    assert_eq!(verdicts["shared/current.age"], &Verdict::Current);
    assert_eq!(
        verdicts["hosts/host/armored.age"],
        &Verdict::Stale {
            missing: 1,
            extra: 1
        }
    );
}

#[tokio::test]
async fn apply_rekeys_the_stale_files_and_leaves_the_rest_alone() {
    let fleet = fleet().await;
    let root = fleet.root.path();
    let current = fs::read(root.join("shared/current.age")).unwrap();
    let unmanaged = fs::read(root.join("other/unmanaged.age")).unwrap();

    let plan = secrets::plan(root, &fleet.rules).unwrap();
    let mut seen = Vec::new();
    let rewritten = plan
        .apply(&fleet.admin, |index, total, file| {
            seen.push((index, total, file.clone()))
        })
        .unwrap();

    assert_eq!(
        rewritten,
        [path("hosts/host/armored.age"), path("shared/binary.age")]
    );
    let steps: Vec<(usize, usize)> = seen
        .iter()
        .map(|(index, total, _)| (*index, *total))
        .collect();
    assert_eq!(steps, [(0, 2), (1, 2)]);
    assert!(
        secrets::plan(root, &fleet.rules)
            .unwrap()
            .stale()
            .next()
            .is_none()
    );
    assert_eq!(fs::read(root.join("shared/current.age")).unwrap(), current);
    assert_eq!(
        fs::read(root.join("other/unmanaged.age")).unwrap(),
        unmanaged
    );

    let header = |file: &str| Header::read(&root.join(file)).unwrap();
    assert!(header("hosts/host/armored.age").armored);
    assert!(!header("shared/binary.age").armored);
    assert!(header("shared/binary.age").names(&fleet.host));
    assert!(!header("hosts/host/armored.age").names(&fleet.peer));

    assert_eq!(open(&fleet.admin, root, "shared/binary.age"), b"binary");
    assert_eq!(
        open(&fleet.admin, root, "hosts/host/armored.age"),
        b"armored"
    );

    // nothing is left beside the secrets
    let names: Vec<_> = fs::read_dir(root.join("shared"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[tokio::test]
async fn the_new_host_opens_what_was_rekeyed_for_it() {
    let fleet = fleet().await;
    let keys = tempdir().unwrap();
    let host = admin(keys.path(), "host").await;
    let mut rules = fleet.rules.clone();
    rules.insert(
        path("shared/binary.age"),
        Some(vec![fleet.admin.key().clone(), host.key().clone()]),
    );

    secrets::plan(fleet.root.path(), &rules)
        .unwrap()
        .apply(&fleet.admin, |_, _, _| {})
        .unwrap();
    assert_eq!(
        open(&host, fleet.root.path(), "shared/binary.age"),
        b"binary"
    );
}

#[tokio::test]
async fn an_admin_absent_from_a_stale_file_is_refused_before_any_key_is_needed() {
    let fleet = fleet().await;
    let root = fleet.root.path();
    let (_, outsider) = keypair("outsider");
    put(
        root,
        "shared/foreign.age",
        &seal(b"foreign", &[&fleet.peer], &[], false),
    );
    let mut rules = fleet.rules.clone();
    rules.insert(
        path("shared/foreign.age"),
        Some(vec![fleet.peer.clone(), fleet.host.clone()]),
    );
    let plan = secrets::plan(root, &rules).unwrap();

    assert_eq!(
        plan.unopenable_by(fleet.admin.key()),
        [path("shared/foreign.age")]
    );
    let Err(Blocker::NotAdmin { files }) = plan.require_admin(fleet.admin.key()) else {
        panic!("the admin check passed");
    };
    assert_eq!(files, [path("shared/foreign.age")]);
    let Err(Blocker::NotAdmin { files }) = plan.require_admin(&outsider) else {
        panic!("the admin check passed");
    };
    assert_eq!(
        files,
        [
            path("hosts/host/armored.age"),
            path("shared/binary.age"),
            path("shared/foreign.age")
        ]
    );

    let before = fs::read(root.join("shared/binary.age")).unwrap();
    assert!(plan.apply(&fleet.admin, |_, _, _| {}).is_err());
    assert_eq!(fs::read(root.join("shared/binary.age")).unwrap(), before);
}

#[tokio::test]
async fn a_missing_or_broken_file_is_unreadable_not_stale() {
    let fleet = fleet().await;
    let root = fleet.root.path();
    put(root, "shared/broken.age", b"not age\n");
    let mut rules = fleet.rules.clone();
    for file in ["shared/broken.age", "shared/missing.age"] {
        rules.insert(path(file), Some(vec![fleet.host.clone()]));
    }
    let plan = secrets::plan(root, &rules).unwrap();

    let unreadable: Vec<&str> = plan
        .files
        .iter()
        .filter(|file| matches!(file.verdict, Verdict::Unreadable(_)))
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(unreadable, ["shared/broken.age", "shared/missing.age"]);
    assert_eq!(
        stale(&plan),
        ["hosts/host/armored.age", "shared/binary.age"]
    );
}

#[tokio::test]
async fn create_encrypts_a_new_secret_and_never_overwrites() {
    let fleet = fleet().await;
    let root = fleet.root.path();
    let recipients = [fleet.admin.key().clone(), fleet.host.clone()];

    let file = root.join("hosts/host/new.age");
    secrets::create(&file, b"fresh", &recipients).unwrap();
    let header = Header::read(&file).unwrap();
    assert!(!header.armored);
    assert_eq!(header.verdict(&recipients), Verdict::Current);
    assert_eq!(open(&fleet.admin, root, "hosts/host/new.age"), b"fresh");

    let existing = fs::read(root.join("shared/current.age")).unwrap();
    assert!(secrets::create(&root.join("shared/current.age"), b"other", &recipients).is_err());
    assert_eq!(fs::read(root.join("shared/current.age")).unwrap(), existing);
}
