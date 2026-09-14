#!/usr/bin/env python3
"""CPU-only runner tests: fake manager/harness, no GPU, app, audio or D-Bus."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / "scripts/run-gpu-test-isolated.sh"
CASE = "draw_retirement::tests::fixture"


class IsolatedRunnerTests(unittest.TestCase):
    def setUp(self):
        # Synthetic fixtures use workspace scratch, then clean up after the case.
        (ROOT / "scratch").mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(
            prefix="isolated-runner-test.", dir=ROOT / "scratch"
        )
        self.addCleanup(temporary.cleanup)
        self.fixture = Path(temporary.name)
        self.calls = self.fixture / "calls.jsonl"
        self.fake = self.fixture / "systemd-run"
        self.fake.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "with open(os.environ['FAKE_CALLS'], 'a') as output:\n"
            "    output.write(json.dumps([os.getpid(), sys.argv[1:]]) + '\\n')\n"
            "mode = os.environ.get('FAKE_MODE', 'pass')\n"
            "if mode == 'manager-failure': sys.exit(77)\n"
            "if '--list' in sys.argv:\n"
            "    if mode != 'missing': print(os.environ['FAKE_CASE'] + ': test')\n"
            "    if mode == 'extra': print('another::test: test')\n"
            "    if mode == 'replace-before': Path(os.environ['FAKE_BINARY']).write_text('changed')\n"
            "elif mode == 'test-failure':\n"
            "    print('underlying test failure'); sys.exit(101)\n"
            "elif mode == 'ignored':\n"
            "    print('test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;')\n"
            "else:\n"
            "    print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s')\n"
            "    if mode == 'replace-after': Path(os.environ['FAKE_BINARY']).write_text('changed')\n"
        )
        self.fake.chmod(0o755)
        # Keep fake-run evidence inside this fixture, cleaned after each case.
        # Actual GPU-run evidence still uses the production durable directory.
        mktemp = self.fixture / "mktemp"
        mktemp.write_text(
            "#!/usr/bin/env python3\n"
            "import os, tempfile\n"
            "print(tempfile.mkdtemp(prefix='evidence.', dir=os.environ['FAKE_DIR']))\n"
        )
        mktemp.chmod(0o755)
        self.binary = self.fixture / "fake test binary"
        # The fake manager must intercept every attempted test-binary execution.
        self.binary.write_text("#!/bin/sh\nexit 99\n")
        self.binary.chmod(0o755)
        self.env = dict(
            os.environ,
            PATH=f"{self.fixture}:{os.environ['PATH']}",
            FAKE_CALLS=str(self.calls),
            FAKE_CASE=CASE,
            FAKE_DIR=str(self.fixture),
            FAKE_BINARY=str(self.binary),
        )

    def run_case(self, mode="pass", args=None):
        if args is None:
            args = ["--gpu-approved", str(self.binary), CASE]
        return subprocess.run(
            ["bash", str(RUNNER), *args],
            env=dict(self.env, FAKE_MODE=mode),
            text=True,
            capture_output=True,
            timeout=5,
            check=False,
        )

    def recorded(self):
        if not self.calls.exists():
            return []
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def test_no_approval_or_multiple_cases_runs_nothing(self):
        for args in (
            [str(self.binary), CASE],
            ["--gpu-approved", str(self.binary), CASE, "another::test"],
            ["--gpu-approved", str(self.binary), "--ignored"],
        ):
            self.assertEqual(self.run_case(args=args).returncode, 2)
        self.assertEqual(self.recorded(), [])

    def test_listing_and_case_are_separate_bounded_processes(self):
        result = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded()
        self.assertEqual(len(calls), 2)
        self.assertNotEqual(calls[0][0], calls[1][0])
        prefix = [
            "--user", "--scope", "--property=MemoryMax=4G",
            "--property=MemorySwapMax=0", "--property=CPUQuota=100%",
            "--property=TasksMax=128", "--property=RuntimeMaxSec=125s",
            "--property=TimeoutStopSec=5s", "--property=KillMode=control-group",
            "--property=OOMPolicy=kill", "prlimit", "--fsize=67108864:67108864", "--",
            "nice", "-n", "10", "timeout", "--signal=TERM", "--kill-after=5s", "120s",
            "env", "KJERAG_REQUIRE_GPU=1", "RUST_TEST_THREADS=1", str(self.binary),
        ]
        self.assertEqual(
            calls[0][1], prefix + ["--list", "--format", "terse", "--exact", CASE]
        )
        self.assertEqual(
            calls[1][1], prefix + ["--exact", CASE, "--nocapture", "--test-threads=1"]
        )

    def test_missing_or_extra_listing_stops_before_execution(self):
        for mode in ("missing", "extra"):
            with self.subTest(mode=mode):
                before = len(self.recorded())
                self.assertEqual(self.run_case(mode).returncode, 2)
                self.assertEqual(len(self.recorded()), before + 1)

    def test_manager_failure_has_no_unbounded_fallback(self):
        self.assertEqual(self.run_case("manager-failure").returncode, 77)
        self.assertEqual(len(self.recorded()), 1)

    def test_case_failure_or_ignore_is_not_a_pass(self):
        self.assertEqual(self.run_case("test-failure").returncode, 101)
        self.assertEqual(len(self.recorded()), 2)
        self.assertEqual(self.run_case("ignored").returncode, 2)
        self.assertEqual(len(self.recorded()), 4)

    def test_binary_replacement_before_execution_refuses_the_case(self):
        self.assertNotEqual(self.run_case("replace-before").returncode, 0)
        self.assertEqual(len(self.recorded()), 1)

    def test_binary_replacement_during_execution_invalidates_the_result(self):
        self.assertNotEqual(self.run_case("replace-after").returncode, 0)
        self.assertEqual(len(self.recorded()), 2)


if __name__ == "__main__":
    unittest.main()
