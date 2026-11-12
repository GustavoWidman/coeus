use super::path;
use crate::bootstrap::branch::body::{END, Evidence, START, section, splice};

fn evidence() -> Evidence {
    Evidence {
        host: "nova".into(),
        base: "main".into(),
        machine: "NixOS 26.05 · x86_64-linux · EFI · 16 GiB".into(),
        host_key: "ssh-ed25519 AAAA".into(),
        fingerprint: "SHA256:3fA9xQw".into(),
        enrolled_by: "altar".into(),
        files: vec![path("hosts/nova/flake.nix"), path("hosts/nova/host.pub")],
        created: vec![path("hosts/nova/password.age")],
        rekeyed: vec![path("secrets/cache.age"), path("secrets/github.age")],
        switch: Some("job #1 · generation 2 · `nixos-system-nova-26.05`".into()),
        coeusd: Some("0.2.0 at nova:45729".into()),
        boot: Some("the new system".into()),
    }
}

#[test]
fn the_section_says_what_happened_to_the_machine() {
    let expected = "\
<!-- coeus:bootstrap -->
### nova joins the fleet

| | |
|---|---|
| platform | NixOS 26.05 · x86_64-linux · EFI · 16 GiB |
| host key | `SHA256:3fA9xQw` |
| enrolled by | altar |
| first switch | job #1 · generation 2 · `nixos-system-nova-26.05` |
| coeusd | 0.2.0 at nova:45729 |
| boot | the new system |

**files**
- `hosts/nova/flake.nix`
- `hosts/nova/host.pub`

**secrets created**
- `hosts/nova/password.age`

<details>
<summary>2 secrets rekeyed so nova opens them</summary>

- `secrets/cache.age`
- `secrets/github.age`

</details>

merging tells every machine about nova; run `coeus bootstrap nova` again before merging if `main` moved
<!-- /coeus:bootstrap -->";

    assert_eq!(section(&evidence()), expected);
}

#[test]
fn what_has_not_happened_yet_is_said_so() {
    let evidence = Evidence {
        switch: None,
        coeusd: None,
        boot: None,
        created: Vec::new(),
        rekeyed: Vec::new(),
        ..evidence()
    };

    let section = section(&evidence);

    assert!(
        section.contains("| first switch | _not yet_ |"),
        "{section}"
    );
    assert!(
        section.contains("| coeusd | _not answering yet_ |"),
        "{section}"
    );
    assert!(
        section.contains("| boot | _not checked yet_ |"),
        "{section}"
    );
    assert!(section.contains("**secrets created**: none"), "{section}");
    assert!(
        section.contains("no other secret needed rekeying"),
        "{section}"
    );
    assert!(!section.contains("<details>"), "{section}");
}

#[test]
fn a_pipe_does_not_break_the_table() {
    let evidence = Evidence {
        enrolled_by: "a|b\nc".into(),
        ..evidence()
    };

    assert!(section(&evidence).contains("| enrolled by | a\\|b c |"));
}

#[test]
fn a_body_without_a_section_gets_it_appended() {
    let spliced = splice("adds nova to the fleet\n", "SECTION");

    assert_eq!(spliced, "adds nova to the fleet\n\nSECTION\n");
    assert_eq!(splice("", "SECTION"), "SECTION\n");
}

#[test]
fn only_the_section_is_rewritten() {
    let body = format!("intro\n\n{START}\nold\n{END}\n\nwritten by a person\r\n");

    let spliced = splice(&body, &format!("{START}\nnew\n{END}"));

    assert_eq!(
        spliced,
        format!("intro\n\n{START}\nnew\n{END}\n\nwritten by a person\r\n")
    );
}

#[test]
fn splicing_the_same_section_twice_changes_nothing() {
    let section = section(&evidence());
    let once = splice("adds nova\n", &section);

    assert_eq!(splice(&once, &section), once);
}

#[test]
fn a_section_missing_its_end_marker_runs_to_the_end() {
    let body = format!("intro\n{START}\nold, its end deleted\n");

    assert_eq!(splice(&body, "SECTION"), "intro\nSECTION");
}
