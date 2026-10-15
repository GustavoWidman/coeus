"""Harmless unprivileged process fixtures; never sudo, nix, activation or live state."""
import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("policy", sys.argv.pop(1))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class SupervisionTests(unittest.TestCase):
    def fixture(self, script):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        marker = Path(temp.name) / "pid"
        read, write = os.pipe()
        self.addCleanup(lambda: os.close(write) if self.open_write else None)
        self.open_write = True
        # Separate helper interpreter, like root helper, owning a separate session
        # that the caller's parent process-group signalling would not reach.
        code = "import importlib.util,sys; s=importlib.util.spec_from_file_location('p',sys.argv[1]); p=importlib.util.module_from_spec(s); s.loader.exec_module(p); sys.exit(p.supervised(['/bin/sh','-c',sys.argv[2]],dict(__import__('os').environ),deadline=2))"
        env_script = script.replace("MARKER", str(marker))
        process = subprocess.Popen([sys.executable, "-c", code, policy.__file__, env_script],
                                   stdin=read, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        os.close(read)
        self.addCleanup(lambda: process.kill() if process.poll() is None else None)
        return process, write, marker

    def close(self, write):
        os.close(write)
        self.open_write = False

    def child_pid(self, marker):
        end = time.monotonic() + 2
        while time.monotonic() < end:
            try:
                return int(marker.read_text())
            except (FileNotFoundError, ValueError):
                time.sleep(.01)
        self.fail("child did not start")

    def assert_dead(self, pid):
        end = time.monotonic() + 2
        while time.monotonic() < end:
            try:
                # Linux reparented zombies are dead, even before PID1 reaps.
                text = Path(f"/proc/{pid}/stat").read_text()
                if text.split(") ", 1)[1].startswith("Z"):
                    return
            except FileNotFoundError:
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    return
            time.sleep(.01)
        self.fail("owned descendant is still alive")

    def test_eof_kills_descendant_in_separate_activation_group(self):
        process, write, marker = self.fixture("sleep 30 & echo $! > MARKER; wait")
        pid = self.child_pid(marker)
        self.close(write)
        _, error = process.communicate(timeout=4)
        self.assertEqual(process.returncode, 125, error)
        self.assert_dead(pid)

    def test_child_surviving_parent_is_killed_before_success(self):
        process, write, marker = self.fixture("sleep 30 & echo $! > MARKER; exit 0")
        pid = self.child_pid(marker)
        _, error = process.communicate(timeout=4)
        self.close(write)
        self.assertEqual(process.returncode, 0, error)
        self.assert_dead(pid)

    def test_supervisor_deadline_cleans_hang_even_if_client_retains_lease(self):
        process, write, marker = self.fixture("sleep 30 & echo $! > MARKER; wait")
        pid = self.child_pid(marker)
        _, error = process.communicate(timeout=4)
        self.close(write)
        self.assertEqual(process.returncode, 125, error)
        self.assert_dead(pid)

    def test_supervision_rejects_nonpipe_lease(self):
        with open(os.devnull) as control:
            with self.assertRaises(ValueError):
                policy.supervised(["/bin/true"], {}, control=control.fileno())


unittest.main()
