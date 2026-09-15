#!/usr/bin/env python3
"""Tests for the release-channel ancestry and SemVer guard."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


CHECKER = Path(__file__).resolve().with_name("check-release-channel.py")
SOURCE = "1" * 40
DESCENDANT = "2" * 40
OTHER = "3" * 40
COMMITS = tuple(str(index) * 64 for index in range(4, 8))
REFS = tuple(
    ref
    for arch in ("x86_64", "aarch64")
    for ref in (
        f"app/dev.harding.Kjerag/{arch}/stable",
        f"runtime/dev.harding.Kjerag.Debug/{arch}/stable",
    )
)


def record(source: str, version: str, commits: tuple[str, ...] = COMMITS) -> str:
    refs = "".join(f"{ref} {commit}\n" for ref, commit in zip(REFS, commits))
    return f"format=1\nsource={source}\nversion={version}\n{refs}"


class ChannelGuardTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        requested = os.environ.get("KJERAG_RELEASE_TEST_ROOT")
        if not requested:
            raise RuntimeError("KJERAG_RELEASE_TEST_ROOT must name a scratch directory")
        root = Path(requested).expanduser()
        root = root.resolve() if root.is_absolute() else (Path.cwd() / root).resolve()
        root.mkdir(parents=True, exist_ok=True)
        cls.suite = Path(tempfile.mkdtemp(prefix="channel-guard-", dir=root))
        cls.addClassCleanup(
            lambda: print(f"channel guard fixtures retained at {cls.suite}")
        )

    def setUp(self) -> None:
        self.case = Path(tempfile.mkdtemp(prefix="case-", dir=self.suite))
        self.bin = self.case / "bin"
        self.bin.mkdir()
        self.log = self.case / "git.log"
        git = self.bin / "git"
        git.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' \"$*\" >> \"$FAKE_GIT_LOG\"\n"
            "exit \"${FAKE_GIT_STATUS:-0}\"\n",
            encoding="utf-8",
        )
        git.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(
            {
                "PATH": f"{self.bin}{os.pathsep}{self.env.get('PATH', '')}",
                "FAKE_GIT_LOG": str(self.log),
                "FAKE_GIT_STATUS": "0",
            }
        )

    def write(self, name: str, contents: str) -> Path:
        path = self.case / name
        path.write_text(contents, encoding="ascii")
        return path

    def run_guard(
        self, published: Path, candidate: Path, *, succeeds: bool
    ) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            ["python3", str(CHECKER), str(published), str(candidate)],
            env=self.env,
            cwd=self.case,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=30,
            check=False,
        )
        if succeeds:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stderr)
        return result

    def assert_git_not_called(self) -> None:
        self.assertFalse(self.log.exists(), self.log.read_text() if self.log.exists() else "")

    def test_missing_published_record_bootstraps(self) -> None:
        candidate = self.write("candidate", record(SOURCE, "1.0.0"))
        self.run_guard(self.case / "missing", candidate, succeeds=True)
        self.assert_git_not_called()

    def test_same_source_requires_identical_whole_record(self) -> None:
        published = self.write("published", record(SOURCE, "1.0.0"))
        identical = self.write("identical", record(SOURCE, "1.0.0"))
        self.run_guard(published, identical, succeeds=True)
        changed = (*COMMITS[:3], "8" * 64)
        candidate = self.write("candidate", record(SOURCE, "1.0.0", changed))
        self.run_guard(published, candidate, succeeds=False)
        self.assert_git_not_called()

    def test_descendant_with_newer_version_is_accepted(self) -> None:
        published = self.write("published", record(SOURCE, "1.2.3"))
        candidate = self.write("candidate", record(DESCENDANT, "1.3.0"))
        self.run_guard(published, candidate, succeeds=True)
        self.assertEqual(
            self.log.read_text(), f"merge-base --is-ancestor {SOURCE} {DESCENDANT}\n"
        )

    def test_older_or_equal_version_is_refused_before_git(self) -> None:
        published = self.write("published", record(SOURCE, "2.0.0"))
        for name, version in (("older", "1.9.9"), ("equal", "2.0.0")):
            with self.subTest(version=version):
                candidate = self.write(name, record(DESCENDANT, version))
                self.run_guard(published, candidate, succeeds=False)
        self.assert_git_not_called()

    def test_nonancestor_is_refused(self) -> None:
        self.env["FAKE_GIT_STATUS"] = "1"
        published = self.write("published", record(SOURCE, "1.0.0"))
        candidate = self.write("candidate", record(OTHER, "2.0.0"))
        self.run_guard(published, candidate, succeeds=False)
        self.assertTrue(self.log.exists())

    def test_prerelease_orders_before_stable(self) -> None:
        published = self.write("published-rc", record(SOURCE, "1.0.0-rc.2"))
        stable = self.write("stable", record(DESCENDANT, "1.0.0"))
        self.run_guard(published, stable, succeeds=True)

        self.log.unlink()
        published = self.write("published-stable", record(SOURCE, "1.0.0"))
        candidate = self.write("candidate-rc", record(DESCENDANT, "1.0.1-rc.1"))
        self.run_guard(published, candidate, succeeds=True)
        # A prerelease of the same core is lower than its stable release.
        same_core = self.write("same-core-rc", record(DESCENDANT, "1.0.0-rc.3"))
        self.run_guard(published, same_core, succeeds=False)

    def test_build_metadata_does_not_advance_version(self) -> None:
        published = self.write("published", record(SOURCE, "1.2.3+first"))
        candidate = self.write("candidate", record(DESCENDANT, "1.2.3+second"))
        self.run_guard(published, candidate, succeeds=False)
        self.assert_git_not_called()

    def test_numeric_prerelease_identifier_orders_before_text(self) -> None:
        published = self.write("published", record(SOURCE, "1.0.0-9"))
        candidate = self.write("candidate", record(DESCENDANT, "1.0.0-alpha"))
        self.run_guard(published, candidate, succeeds=True)

    def test_malformed_fields_are_rejected_before_git(self) -> None:
        published = self.write("published", record(SOURCE, "1.0.0"))
        malformed = (
            record("not-a-revision", "2.0.0"),
            record(DESCENDANT, "02.0.0"),
            record(DESCENDANT, "2.0.0").replace(REFS[0], "app/wrong/x86_64/stable"),
        )
        for index, contents in enumerate(malformed):
            with self.subTest(index=index):
                candidate = self.write(f"bad-{index}", contents)
                self.run_guard(published, candidate, succeeds=False)
        self.assert_git_not_called()


if __name__ == "__main__":
    unittest.main(verbosity=2)
