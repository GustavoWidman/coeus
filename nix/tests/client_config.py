"""Unprivileged runtime-copy tests; fixtures contain no real credentials."""
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest

SCRIPT, BASH, COREUTILS, STORE_FILE = sys.argv[1:]
sys.argv = sys.argv[:1]


class ClientConfiguration(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.source = self.root / "secret client.toml"
        self.source.write_text('[settings]\nkey="synthetic-only"\nfleet=["me"]\n')
        self.source.chmod(0o600)
        self.home = self.root / "xdg ' config"
        self.destination = self.home / "coeus" / "config.toml"

    def activate(self, source=None, dry=False):
        env = {"PATH": os.environ["PATH"], "source_config": str(source or self.source),
               "config_home": str(self.home), "coreutils": COREUTILS}
        if dry:
            env["DRY_RUN"] = "1"
        return subprocess.run([BASH, "-e", SCRIPT], env=env, capture_output=True, text=True,
                              timeout=10)

    def test_private_copy_and_atomic_refresh(self):
        self.assertEqual(self.activate().returncode, 0)
        self.assertEqual(self.destination.read_bytes(), self.source.read_bytes())
        self.assertFalse(self.destination.is_symlink())
        self.assertEqual(stat.S_IMODE(self.destination.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.destination.parent.stat().st_mode), 0o700)
        self.assertEqual(self.destination.stat().st_uid, os.getuid())
        self.source.write_text('new synthetic config\n')
        self.destination.chmod(0o644)
        self.assertEqual(self.activate().returncode, 0)
        self.assertEqual(self.destination.read_text(), 'new synthetic config\n')
        self.assertEqual(stat.S_IMODE(self.destination.stat().st_mode), 0o600)
        self.assertEqual(list(self.destination.parent.glob('.config.toml.*')), [])

    def test_existing_directory_special_bits_are_cleared(self):
        self.destination.parent.mkdir(parents=True)
        try:
            self.destination.parent.chmod(0o3777)
        except PermissionError:
            # Nix's syscall filter may prohibit setgid even on our own fixture.
            # Sticky-bit clearing remains exercised in the sandbox; ordinary
            # unprivileged runs also cover clearing inherited setgid.
            self.destination.parent.chmod(0o1777)
        self.assertEqual(self.activate().returncode, 0)
        self.assertEqual(stat.S_IMODE(self.destination.parent.stat().st_mode), 0o700)

    def test_runtime_secret_symlink(self):
        link = self.root / 'agenix-link'
        link.symlink_to(self.source)
        self.assertEqual(self.activate(link).returncode, 0)
        self.assertEqual(self.destination.read_bytes(), self.source.read_bytes())

    def test_resolved_store_source_rejected(self):
        link = self.root / 'store-link'
        link.symlink_to(STORE_FILE)
        result = self.activate(link)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Nix store', result.stderr)
        self.assertFalse(self.destination.exists())

    def test_missing_or_directory_source_rejected(self):
        for path in [self.root / 'missing', self.root]:
            self.assertNotEqual(self.activate(path).returncode, 0)
        self.assertFalse(self.destination.exists())

    @unittest.skipIf(os.geteuid() == 0, "root bypasses file permission checks")
    def test_unreadable_source_fails_without_mutation(self):
        self.source.chmod(0)
        self.assertNotEqual(self.activate().returncode, 0)
        self.assertFalse(self.destination.exists())

    def test_destination_directory_symlink_rejected(self):
        other = self.root / 'other'
        other.mkdir()
        self.home.mkdir()
        (self.home / 'coeus').symlink_to(other)
        self.assertNotEqual(self.activate().returncode, 0)
        self.assertEqual(list(other.iterdir()), [])

    def test_destination_file_symlink_rejected(self):
        other = self.root / 'other.toml'
        other.write_text('leave alone')
        self.destination.parent.mkdir(parents=True)
        self.destination.symlink_to(other)
        self.assertNotEqual(self.activate().returncode, 0)
        self.assertEqual(other.read_text(), 'leave alone')

    def test_dry_run_does_not_access_missing_source_or_create_destination(self):
        self.assertEqual(self.activate(self.root / 'missing', dry=True).returncode, 0)
        self.assertFalse(self.home.exists())

    def test_copy_is_private_even_with_public_fixture_source(self):
        self.source.chmod(0o644)
        result = self.activate()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(stat.S_IMODE(self.destination.stat().st_mode), 0o600)
        self.assertNotIn('synthetic-only', result.stdout + result.stderr)


unittest.main()
