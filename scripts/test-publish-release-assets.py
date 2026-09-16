#!/usr/bin/env python3
"""Unit tests for release publication using a stateful fake gh executable."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().with_name("publish-release-assets.py")
VERSION = "9.8.7-test1"
SOURCE = "a" * 40
REPOSITORY = "aeharding/kjerag"
NAMES = tuple(
    name
    for arch in ("x86_64", "aarch64")
    for name in (
        f"kjerag-{VERSION}-{arch}.flatpak",
        f"kjerag-{VERSION}-{arch}.flatpak.sha256",
    )
)


FAKE_GH = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import sys

root = Path(os.environ["FAKE_GH_ROOT"])
state_path = root / "state.json"
state = json.loads(state_path.read_text())
args = sys.argv[1:]
state["commands"].append(args)

def save():
    state_path.write_text(json.dumps(state))

def release_value():
    return {
        "tag_name": state["version"],
        "draft": state["draft"],
        "prerelease": state["prerelease"],
        "assets": [{"name": name} for name in state["assets"]],
    }

if args[:1] == ["api"]:
    endpoint = args[1]
    if "/git/ref/tags/" in endpoint:
        if state.get("annotated"):
            print(json.dumps({"object": {"type": "tag", "sha": "b" * 40}}))
        else:
            print(json.dumps({"object": {"type": "commit", "sha": state["source"]}}))
    elif "/git/tags/" in endpoint:
        print(json.dumps({"object": {"type": "commit", "sha": state["source"]}}))
    elif "/releases/tags/" in endpoint:
        if state.get("api_error"):
            print(json.dumps({"message": "broken", "status": state["api_error"]}))
            save()
            sys.exit(1)
        if not state["exists"]:
            print(json.dumps({"message": "Not Found", "status": "404"}))
            save()
            sys.exit(1)
        print(json.dumps(release_value()))
    else:
        raise SystemExit("unexpected api endpoint")
elif args[:2] == ["release", "create"]:
    state["exists"] = True
    state["draft"] = True
    state["prerelease"] = "--prerelease" in args
elif args[:2] == ["release", "upload"]:
    for argument in args[3:args.index("--repo")]:
        source = Path(argument)
        shutil.copyfile(source, root / "remote" / source.name)
        if state.get("corrupt_upload") == source.name:
            (root / "remote" / source.name).write_text("corrupt upload")
        state["assets"].append(source.name)
elif args[:2] == ["release", "download"]:
    name = args[args.index("--pattern") + 1]
    destination = Path(args[args.index("--dir") + 1]) / name
    shutil.copyfile(root / "remote" / name, destination)
elif args[:2] == ["release", "edit"]:
    state["draft"] = False
else:
    raise SystemExit("unexpected gh command: " + repr(args))
save()
'''


class PublishReleaseAssetsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        requested = os.environ.get("KJERAG_RELEASE_TEST_ROOT")
        if not requested:
            raise RuntimeError(
                "KJERAG_RELEASE_TEST_ROOT must name a durable scratch directory"
            )
        parent = Path(requested).expanduser()
        if not parent.is_absolute():
            parent = (Path.cwd() / parent).resolve()
        parent.mkdir(parents=True, exist_ok=True)
        cls.suite = Path(
            tempfile.mkdtemp(prefix="publish-release-assets-", dir=parent)
        )
        cls.addClassCleanup(cls._report_suite)

    @classmethod
    def _report_suite(cls) -> None:
        print(f"publish release fixtures retained at {cls.suite}")

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="case-", dir=self.suite))
        self.bin = self.root / "bin"
        self.remote = self.root / "remote"
        self.bundles = self.root / "bundles"
        self.bin.mkdir()
        self.remote.mkdir()
        self.bundles.mkdir()
        fake = self.bin / "gh"
        fake.write_text(FAKE_GH)
        fake.chmod(0o755)
        self.state = {
            "version": VERSION,
            "source": SOURCE,
            "exists": False,
            "draft": True,
            "prerelease": True,
            "assets": [],
            "commands": [],
        }
        for arch in ("x86_64", "aarch64"):
            name = f"kjerag-{VERSION}-{arch}.flatpak"
            content = f"signed {arch} payload\n".encode()
            (self.bundles / name).write_bytes(content)
            digest = hashlib.sha256(content).hexdigest()
            (self.bundles / f"{name}.sha256").write_text(f"{digest}  {name}\n")
        self._write_state()
        self.env = os.environ.copy()
        self.env.update(
            {
                "PATH": f"{self.bin}{os.pathsep}{self.env.get('PATH', '')}",
                "FAKE_GH_ROOT": str(self.root),
                "GH_REPO": REPOSITORY,
            }
        )

    def _write_state(self) -> None:
        (self.root / "state.json").write_text(json.dumps(self.state))

    def _read_state(self) -> dict:
        return json.loads((self.root / "state.json").read_text())

    def _run(
        self, *, version: str = VERSION, source: str = SOURCE
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), version, source, str(self.bundles)],
            env=self.env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=45,
            check=False,
        )

    def _install_remote(self, names=NAMES) -> None:
        self.state.update({"exists": True, "assets": list(names)})
        for name in names:
            (self.remote / name).write_bytes((self.bundles / name).read_bytes())
        self._write_state()

    def test_new_draft_uploads_all_then_publishes(self) -> None:
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stderr)
        state = self._read_state()
        self.assertFalse(state["draft"])
        self.assertEqual(set(state["assets"]), set(NAMES))
        create = next(
            command
            for command in state["commands"]
            if command[:2] == ["release", "create"]
        )
        self.assertIn("--verify-tag", create)
        self.assertIn("--draft", create)
        self.assertIn("--generate-notes", create)
        self.assertIn("--prerelease", create)
        self.assertNotIn("--clobber", sum(state["commands"], []))

    def test_identical_public_retry_does_not_rewrite(self) -> None:
        self.state["draft"] = False
        self._install_remote()
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self._read_state()["commands"]
        self.assertFalse(
            any(command[:2] == ["release", "upload"] for command in commands)
        )
        self.assertFalse(
            any(command[:2] == ["release", "edit"] for command in commands)
        )

    def test_partial_draft_resumes_only_missing_assets(self) -> None:
        present = NAMES[:2]
        self._install_remote(present)
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stderr)
        state = self._read_state()
        upload = next(
            command
            for command in state["commands"]
            if command[:2] == ["release", "upload"]
        )
        uploaded = {Path(item).name for item in upload[3:upload.index("--repo")]}
        self.assertEqual(uploaded, set(NAMES[2:]))
        self.assertFalse(state["draft"])

    def test_conflicting_asset_fails_before_upload(self) -> None:
        self._install_remote(NAMES[:1])
        (self.remote / NAMES[0]).write_text("different")
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        commands = self._read_state()["commands"]
        self.assertFalse(
            any(command[:2] == ["release", "upload"] for command in commands)
        )
        self.assertFalse(
            any(command[:2] == ["release", "edit"] for command in commands)
        )

    def test_unknown_remote_asset_fails_before_upload_or_publish(self) -> None:
        self._install_remote(NAMES)
        self.state = self._read_state()
        self.state["assets"].append("unexpected.txt")
        (self.remote / "unexpected.txt").write_text("not part of the payload")
        self._write_state()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        commands = self._read_state()["commands"]
        self.assertFalse(
            any(command[:2] == ["release", "upload"] for command in commands)
        )
        self.assertFalse(
            any(command[:2] == ["release", "edit"] for command in commands)
        )

    def test_wrong_prerelease_classification_is_refused(self) -> None:
        self._install_remote(NAMES)
        self.state = self._read_state()
        self.state["prerelease"] = False
        self._write_state()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        commands = self._read_state()["commands"]
        self.assertFalse(
            any(command[:2] == ["release", "upload"] for command in commands)
        )
        self.assertFalse(
            any(command[:2] == ["release", "edit"] for command in commands)
        )

    def test_non_404_api_failure_does_not_create(self) -> None:
        self.state["api_error"] = "500"
        self._write_state()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        commands = self._read_state()["commands"]
        self.assertFalse(
            any(command[:2] == ["release", "create"] for command in commands)
        )

    def test_wrong_tag_source_fails_before_release_write(self) -> None:
        self.state["source"] = "b" * 40
        self._write_state()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        commands = self._read_state()["commands"]
        self.assertFalse(any(command[:1] == ["release"] for command in commands))

    def test_annotated_tag_is_peeled(self) -> None:
        self.state["annotated"] = True
        self._write_state()
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self._read_state()["commands"]
        self.assertTrue(
            any(
                "/git/tags/" in command[1]
                for command in commands
                if command[:1] == ["api"]
            )
        )

    def test_verification_failure_never_publishes(self) -> None:
        self.state["corrupt_upload"] = NAMES[-1]
        self._write_state()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        state = self._read_state()
        self.assertTrue(state["draft"])
        self.assertFalse(
            any(
                command[:2] == ["release", "edit"]
                for command in state["commands"]
            )
        )

    def test_rejects_wrong_repository_and_extra_file(self) -> None:
        self.env["GH_REPO"] = "someone/kjerag"
        self.assertNotEqual(self._run().returncode, 0)
        self.env["GH_REPO"] = REPOSITORY
        (self.bundles / "notes.txt").write_text("not a release asset")
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self._read_state()["commands"], [])

    def test_rejects_bad_local_checksum_before_gh(self) -> None:
        checksum = self.bundles / NAMES[1]
        checksum.write_text("0" * 64 + "  wrong.flatpak\n")
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self._read_state()["commands"], [])

    def test_rejects_malformed_semver_before_gh(self) -> None:
        malformed = (
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-rc.01",
            "1.2.3-rc..1",
            "1.2.3+build..1",
            "1.2.3+first+second",
        )
        for version in malformed:
            with self.subTest(version=version):
                result = self._run(version=version)
                self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self._read_state()["commands"], [])


if __name__ == "__main__":
    unittest.main()
