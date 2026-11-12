use std::process::Command;

use crate::{
    common::model::{NarHash, Platform, Source, StorePath},
    engine::Snapshots,
    job::Target,
    runner::Recipe,
};

const NIX: [&str; 2] = ["--extra-experimental-features", "nix-command flakes"];

/// a flake added to the nix store as `nix flake archive` would, with its
/// NAR hash, or `None` where there is no usable nix (a build sandbox)
fn archived(dir: &std::path::Path) -> Option<(StorePath, NarHash)> {
    std::fs::write(
        dir.join("flake.nix"),
        "{ outputs = _: { answer = 42; }; }\n",
    )
    .unwrap();
    let nix = |args: &[&str]| -> Option<String> {
        let output = Command::new("nix").args(NIX).args(args).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap().trim().to_owned())
    };

    let path = nix(&["store", "add-path", "--name", "source", dir.to_str()?])?;
    let hash = nix(&["hash", "path", "--type", "sha256", "--sri", &path])?;
    Some((path.parse().unwrap(), hash.parse().unwrap()))
}

#[tokio::test]
async fn a_placed_source_evaluates_as_the_snapshot_it_names() {
    let tree = tempfile::tempdir().unwrap();
    let Some((source, hash)) = archived(tree.path()) else {
        eprintln!("skipped: no nix store to add a source to");
        return;
    };

    let state = tempfile::tempdir().unwrap();
    let snapshots = Snapshots::new(state.path());
    snapshots.place(&source, &hash).await.unwrap();
    assert!(snapshots.has(&hash));
    snapshots.place(&source, &hash).await.unwrap();

    let target = Target {
        hostname: "lab".parse().unwrap(),
        platform: Platform::NixOs,
        flake: "git+https://example.com/nix".into(),
        sources: state.path().join("sources"),
    };
    let snapshot = Source::Snapshot {
        nar_hash: hash.clone(),
        label: "admin:coeus/bootstrap/lab@a1b2c3d".into(),
    };
    let Recipe::Flake(flake) = target.recipe(&snapshot) else {
        panic!("a snapshot builds from a flake");
    };

    let metadata = Command::new("nix")
        .args(NIX)
        .args(["flake", "metadata", "--json", &flake])
        .output()
        .unwrap();
    assert!(
        metadata.status.success(),
        "nix refused {flake}: {}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    assert_eq!(metadata["locked"]["narHash"], hash.as_str());
}

#[tokio::test]
async fn a_source_that_is_not_the_snapshot_is_refused() {
    let tree = tempfile::tempdir().unwrap();
    let Some((source, _)) = archived(tree.path()) else {
        eprintln!("skipped: no nix store to add a source to");
        return;
    };

    let state = tempfile::tempdir().unwrap();
    let snapshots = Snapshots::new(state.path());
    let other: NarHash = format!("sha256-{}=", "A".repeat(43)).parse().unwrap();

    let refused = snapshots.place(&source, &other).await.unwrap_err();
    assert!(refused.to_string().contains("hashes to"), "{refused:#}");
    assert!(!snapshots.has(&other));
    assert_eq!(
        std::fs::read_dir(state.path().join("sources"))
            .unwrap()
            .count(),
        0,
        "nothing is left half placed"
    );
}

#[tokio::test]
async fn sources_placed_at_once_each_unpack_on_their_own() {
    let tree = tempfile::tempdir().unwrap();
    let Some((source, hash)) = archived(tree.path()) else {
        eprintln!("skipped: no nix store to add a source to");
        return;
    };

    let state = tempfile::tempdir().unwrap();
    let snapshots = Snapshots::new(state.path());
    let placing = (0..4).map(|_| snapshots.place(&source, &hash));
    for placed in futures_util::future::join_all(placing).await {
        placed.unwrap();
    }

    assert!(snapshots.has(&hash));
    let entries: Vec<String> = std::fs::read_dir(state.path().join("sources"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, [hash.file_name()], "nothing is left half placed");
}
