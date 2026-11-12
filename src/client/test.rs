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
