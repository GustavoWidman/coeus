//! Shared argv for flake-evaluating builds, never privileged exact-closure activation.
use std::{ffi::OsString, path::Path};

pub(crate) fn flake_build_args(attribute: &str, root: &Path) -> [OsString; 7] {
    [
        "build".into(),
        attribute.into(),
        "--out-link".into(),
        root.as_os_str().into(),
        "--extra-experimental-features".into(),
        "nix-command flakes".into(),
        "--accept-flake-config".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_flake_builds_accept_config_for_both_platforms() {
        for attribute in [
            ".#nixosConfigurations.fixture.config.system.build.toplevel",
            ".#darwinConfigurations.fixture.system",
        ] {
            assert_eq!(
                flake_build_args(attribute, Path::new("/private/state with spaces/root")),
                [
                    "build",
                    attribute,
                    "--out-link",
                    "/private/state with spaces/root",
                    "--extra-experimental-features",
                    "nix-command flakes",
                    "--accept-flake-config",
                ]
                .map(OsString::from)
            );
        }
    }

    #[test]
    fn bootstrap_flake_build_accepts_config_without_splitting_installable() {
        let attribute = "github:owner/repo/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa#darwinConfigurations.fixture.system";
        let args = flake_build_args(attribute, Path::new("/private/activation/1-root"));
        assert_eq!(args[1], attribute);
        assert_eq!(args[6], "--accept-flake-config");
        assert_eq!(args.len(), 7);
    }
}
