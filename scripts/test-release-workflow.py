#!/usr/bin/env python3
"""Execute release context/signer steps and check dispatch publication guards.

Uses disposable keys only. This checks the checked-in workflow, not GitHub's
event evaluator, an actual native build, artifact transport or publication.
Requires PyYAML and a caller-selected durable KJERAG_RELEASE_TEST_ROOT.
"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest

import yaml


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/release.yml"
TAG = "github.event_name == 'push' && github.ref_type == 'tag'"
DISPATCH = "github.event_name == 'workflow_dispatch'"


class ReleaseWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        parent = os.environ.get("KJERAG_RELEASE_TEST_ROOT")
        if not parent:
            raise RuntimeError("KJERAG_RELEASE_TEST_ROOT must name a scratch directory")
        parent = Path(parent).resolve()
        parent.mkdir(parents=True, exist_ok=True)
        cls.suite = Path(tempfile.mkdtemp(prefix="release-workflow-", dir=parent))
        cls.workflow = yaml.load(WORKFLOW.read_text(), Loader=yaml.BaseLoader)
        cls.jobs = cls.workflow["jobs"]
        cls.source = next(step for step in cls.jobs["context"]["steps"] if step.get("id") == "source")
        cls.head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=10
        ).strip()
        cls.version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]

    @classmethod
    def tearDownClass(cls):
        print(f"release workflow fixtures retained at {cls.suite}")

    def setUp(self):
        self.case = Path(tempfile.mkdtemp(prefix="case-", dir=self.suite))

    def run_script(self, script, extra=None, cwd=ROOT):
        env = os.environ.copy()
        env.update({
            "GITHUB_SHA": self.head,
            "GITHUB_OUTPUT": str(self.case / "outputs"),
            "GITHUB_ENV": str(self.case / "environment"),
            "RUNNER_TEMP": str(self.case),
            "GITHUB_RUN_ID": "12345",
            "GITHUB_RUN_ATTEMPT": "1",
            "EVENT_NAME": "workflow_dispatch",
            "REF_TYPE": "branch",
            "REF_NAME": "validation-branch",
            "DEFAULT_BRANCH": "main",
            "PRODUCTION_FINGERPRINT": "",
        })
        env.pop("GNUPGHOME", None)
        env.update(extra or {})
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-euo", "pipefail", "-c", script],
            cwd=cwd, env=env, capture_output=True, text=True, timeout=45,
        )

    def test_dispatch_selects_cargo_version_and_validation_artifacts(self):
        result = self.run_script(self.source["run"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.case / "outputs").read_text(),
                         f"version={self.version}\nartifact-prefix=validation\n")

    def test_tag_requires_exact_version_and_selects_release_artifacts(self):
        result = self.run_script(self.source["run"], {
            "EVENT_NAME": "push", "REF_TYPE": "tag", "REF_NAME": self.version,
        })
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.case / "outputs").read_text(),
                         f"version={self.version}\nartifact-prefix=release\n")

    def test_invalid_contexts_fail_before_emitting_outputs(self):
        for extra in (
            {"GITHUB_SHA": "f" * 40},
            {"EVENT_NAME": "push", "REF_TYPE": "tag", "REF_NAME": "0.0.0-wrong"},
            {"EVENT_NAME": "push", "REF_NAME": self.version},
            {"REF_TYPE": "tag", "REF_NAME": self.version},
            {"REF_NAME": "main"},
            {"EVENT_NAME": "pull_request"},
        ):
            with self.subTest(extra=extra):
                result = self.run_script(self.source["run"], extra)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.case / "outputs").exists())

    def test_publication_permissions_and_production_secrets_are_tag_only(self):
        self.assertEqual(self.workflow["permissions"], {"contents": "read"})
        self.assertEqual(set(self.workflow["on"]), {"push", "workflow_dispatch"})
        writers = set()
        imports = []
        guarded_references = 0
        for name, job in self.jobs.items():
            self.assertNotIn("secrets", job)
            permissions = job.get("permissions", {})
            self.assertIsInstance(permissions, dict)
            if "write" in permissions.values():
                writers.add(name)
                self.assertEqual(job.get("if"), TAG)
            for step in job.get("steps", []):
                if "secrets." in yaml.dump(step):
                    self.assertEqual(step.get("if"), TAG)
                    self.assertEqual(step.get("uses"), "crazy-max/ghaction-import-gpg@v7")
                    self.assertEqual(step["with"], {
                        "gpg_private_key": "${{ secrets.GPG_PRIVATE_KEY }}",
                        "passphrase": "${{ secrets.GPG_PASSPHRASE }}",
                    })
                    guarded_references += yaml.dump(step).count("secrets.")
                    imports.append(name)
        self.assertEqual(writers, {"github-release", "pages"})
        self.assertEqual(imports, ["build", "assemble"])
        self.assertEqual(yaml.dump(self.workflow).count("secrets."), guarded_references)
        verifier = self.jobs["validate-handoff"]
        self.assertEqual(verifier["if"], DISPATCH)
        self.assertEqual(verifier["needs"], ["context", "assemble"])
        self.assertEqual(verifier["steps"][1]["with"]["name"], "validation-publication")
        self.assertIn("release-artifacts.sh verify", verifier["steps"][-1]["run"])

    def test_disposable_key_stays_outside_repository_and_bundle_artifacts(self):
        uploads = []
        for name, job in self.jobs.items():
            for step in job.get("steps", []):
                if step.get("uses") == "actions/upload-artifact@v4":
                    uploads.append((name, step))
        self.assertEqual([name for name, _ in uploads], ["context", "build", "assemble"])
        key_upload = uploads[0][1]
        self.assertEqual(key_upload["if"], DISPATCH)
        self.assertEqual(key_upload["with"]["path"], "validation-key/disposable.asc")
        self.assertEqual(key_upload["with"]["retention-days"], "1")
        self.assertEqual(uploads[1][1]["with"]["path"], "release-${{ matrix.arch }}/")
        self.assertEqual(uploads[2][1]["with"]["path"].splitlines(), [
            "assembled/repository/", "assembled/bundles/", "assembled/commits.txt",
            "assembled/release.txt", "assembled/payload.sha256", "assembled/payload.sig",
        ])
        for _, step in uploads[1:]:
            self.assertIn("needs.context.outputs.artifact-prefix", step["with"]["name"])
        for name in ("build", "assemble"):
            download = next(step for step in self.jobs[name]["steps"]
                            if step.get("with", {}).get("name") == "validation-key")
            self.assertEqual(download["if"], DISPATCH)
            self.assertEqual(download["with"]["path"], "${{ runner.temp }}/validation-key")

    def test_shared_disposable_key_imports_into_both_signing_jobs(self):
        generation = next(step for step in self.jobs["context"]["steps"]
                          if step.get("name") == "Create a disposable validation signer")
        self.assertEqual(generation["if"], DISPATCH)
        fingerprints = []
        # GPG's Unix sockets need a short path, like Actions' RUNNER_TEMP.
        # Only disposable keyrings live here; the exported fixture key and
        # fingerprint outputs remain in the caller's durable scratch directory.
        with tempfile.TemporaryDirectory(prefix="kw-") as key_temp:
            result = self.run_script(generation["run"], {"RUNNER_TEMP": key_temp}, cwd=self.case)
            self.assertEqual(result.returncode, 0, result.stderr)
            # Mimic the artifact download outside the checkout, which the
            # Flatpak dir source would otherwise copy into its build/cache.
            shutil.copytree(self.case / "validation-key", Path(key_temp) / "validation-key")
            for name in ("build", "assemble"):
                signer = next(step for step in self.jobs[name]["steps"] if step.get("id") == "signer")
                output, environment = self.case / f"{name}-outputs", self.case / f"{name}-env"
                result = self.run_script(signer["run"], {
                    "GITHUB_OUTPUT": str(output), "GITHUB_ENV": str(environment),
                    "RUNNER_TEMP": key_temp,
                }, cwd=self.case)
                try:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    line = output.read_text().strip()
                    self.assertRegex(line, r"^fingerprint=[0-9A-F]{40}$")
                    fingerprints.append(line)
                finally:
                    if environment.exists():
                        home = environment.read_text().strip().removeprefix("GNUPGHOME=")
                        self.assertTrue(Path(home).is_relative_to(key_temp))
                        subprocess.run(["gpgconf", "--homedir", home, "--kill", "gpg-agent"],
                                       check=True, timeout=10)
        self.assertEqual(fingerprints[0], fingerprints[1])

    def test_all_shell_steps_parse_as_bash(self):
        self.assertEqual(self.workflow["defaults"]["run"]["shell"], "bash")
        for name, job in self.jobs.items():
            for step in job.get("steps", []):
                if "run" in step:
                    with self.subTest(job=name, step=step.get("name", step.get("id"))):
                        result = subprocess.run(["bash", "-n"], input=step["run"],
                                                capture_output=True, text=True, timeout=10)
                        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
