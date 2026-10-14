"""Unprivileged policy tests; never execute a deployment or sudo."""
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("policy", sys.argv.pop(1))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.store = str(Path(self.temp.name) / "store")
        self.root = Path(self.store) / ("0" * 32 + "-system")
        for program in ("bin/switch-to-configuration", "sw/bin/darwin-rebuild"):
            target = self.root / program
            target.parent.mkdir(parents=True, exist_ok=True)
            target.touch()
            target.chmod(0o755)
        self.link = Path(self.temp.name) / "result"
        self.link.symlink_to(self.root)

    def validate(self, argv, platform="linux"):
        return policy.validate(argv, platform, "/nix/store/nix/bin/nix", self.store)

    def test_profile_only_realised_closure(self):
        args = ["nix", "build", "--no-link", "--profile", policy.PROFILE, str(self.link)]
        actual = self.validate(args)
        self.assertEqual(actual[0], "/nix/store/nix/bin/nix")
        self.assertEqual(actual[-1], str(self.root))
        for mutation in [args + ["--impure"], args[:-1] + ["github:owner/repo"],
                         args[:4] + ["/tmp/profile", str(self.root)],
                         ["sh", *args[1:]], args[:-1]]:
            with self.assertRaises((ValueError, OSError)):
                self.validate(mutation)

    def test_linux_exact_actions(self):
        command = str(self.link) + "/bin/switch-to-configuration"
        for action in ("test", "boot"):
            self.assertEqual(self.validate([command, action]), [str(self.root) + "/bin/switch-to-configuration", action])
        for action in ("switch", "dry-activate", "--help", "boot; sh"):
            with self.assertRaises(ValueError):
                self.validate([command, action])

    def test_darwin_preserves_closure_spelling(self):
        actual = self.validate([str(self.link) + "/sw/bin/darwin-rebuild", "activate"], "darwin")
        self.assertEqual(actual, [str(self.root) + "/sw/bin/darwin-rebuild", "activate"])
        with self.assertRaises(ValueError):
            self.validate([str(self.link) + "/sw/bin/darwin-rebuild", "switch"], "darwin")

    def test_reject_non_store_or_subdirectory(self):
        for target in [self.temp.name, str(self.root / "bin")]:
            with self.assertRaises(ValueError):
                self.validate(["nix", "build", "--no-link", "--profile", policy.PROFILE, target])

    def test_requires_system_marker(self):
        os.unlink(self.root / "bin/switch-to-configuration")
        with self.assertRaises(ValueError):
            self.validate([str(self.link) + "/bin/switch-to-configuration", "test"])

    def test_discards_environment_without_evaluation(self):
        self.assertEqual(policy.envelope(["env", "HOME=/other", "PATH=$(touch /tmp/no)", "NIX_CONFIG=anything", "nix", "build"]), ["nix", "build"])
        for args in [[], ["sudo", "env", "nix"], ["env", "HOME=private"], ["env", "A=" + "a" * 65537, "nix"], ["env"] * 257]:
            with self.assertRaises(ValueError):
                policy.envelope(args)

    def test_rejects_mutable_activation_symlink_and_nested_store_root(self):
        marker = self.root / "bin/switch-to-configuration"
        marker.unlink()
        outside = Path(self.temp.name) / "mutable-activation"
        outside.write_text("#!/bin/sh\nexit 0\n")
        outside.chmod(0o755)
        marker.symlink_to(outside)
        with self.assertRaises(ValueError):
            policy.closure(str(self.root), "linux", self.store)
        nested = self.root / ("1" * 32 + "-nested")
        nested.mkdir()
        with self.assertRaises(ValueError):
            policy.closure(str(nested), "linux", self.store)

    def test_nonroot_helper_never_executes(self):
        args = ["policy", "helper", "linux", "/nix", "/sudo", "/helper", "/trusted/bin", "anything"]
        with patch.object(sys, "argv", args), patch.object(policy.os, "geteuid", return_value=1000), \
             patch.object(policy.os, "execve") as execute, patch("sys.stderr"):
            with self.assertRaises(SystemExit):
                policy.main()
            execute.assert_not_called()

    def test_root_environment_is_reset(self):
        args = ["policy", "helper", "linux", "/configured/nix", "/sudo", "/helper", "/trusted/bin",
                "nix", "build", "--no-link", "--profile", policy.PROFILE, str(self.link)]
        with patch.object(sys, "argv", args), patch.object(policy.os, "geteuid", return_value=0), \
             patch.object(policy, "validate", return_value=["/configured/nix", "fixture"]), \
             patch.dict(os.environ, {"HOME": "/user", "NIX_CONFIG": "malicious", "SUDO_USER": "deploy"}, clear=True), \
             patch.object(policy.os, "execve") as execute:
            policy.main()
            execute.assert_called_once_with("/configured/nix", ["/configured/nix", "fixture"], {
                "HOME": "/root", "USER": "root", "LOGNAME": "root", "PATH": "/trusted/bin",
                "NIX_REMOTE": "daemon", "SUDO_USER": "deploy"})

    def test_adapter_uses_noninteractive_sudo_and_no_env(self):
        args = ["policy", "adapter", "linux", "/nix", "/sudo", "/helper", "/trusted/bin",
                "env", "HOME=/user", "PATH=unsafe", "nix", "build"]
        with patch.object(sys, "argv", args), patch.object(policy.os, "execv") as execute:
            policy.main()
            execute.assert_called_once_with("/sudo", ["/sudo", "-n", "--", "/helper", "nix", "build"])

    def test_dangling_and_traversal_paths(self):
        dangling = Path(self.temp.name) / "dangling"
        dangling.symlink_to(Path(self.temp.name) / "absent")
        for target in [str(dangling), str(self.root) + "/../outside", str(self.root) + "/bin/.."]:
            with self.assertRaises((ValueError, OSError)):
                policy.closure(target, "linux", self.store)



unittest.main()
