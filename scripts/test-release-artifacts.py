#!/usr/bin/env python3
"""Test the signed release handoff with tiny synthetic Flatpak repositories.

No application or runtime is executed; only a disposable synthetic app is
installed. The caller must select a durable scratch parent explicitly, for example:

    KJERAG_RELEASE_TEST_ROOT=scratch/release-artifact-tests \
        python3 scripts/test-release-artifacts.py
"""

from __future__ import annotations

import base64
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().with_name("release-artifacts.sh")
APP_ID = "dev.harding.Kjerag"
VERSION, OTHER_VERSION = "9.8.7-test1", "9.8.8-test1"
SOURCE, OTHER_SOURCE = "a" * 40, "b" * 40
ARCHES = ("x86_64", "aarch64")
SHORT_TIMEOUT, LONG_TIMEOUT = 45, 120


def app_ref(arch: str) -> str:
    return f"app/{APP_ID}/{arch}/stable"


def debug_ref(arch: str) -> str:
    return f"runtime/{APP_ID}.Debug/{arch}/stable"


class ReleaseArtifactsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        requested = os.environ.get("KJERAG_RELEASE_TEST_ROOT")
        if not requested:
            raise RuntimeError("KJERAG_RELEASE_TEST_ROOT must name a scratch directory")
        root = Path(requested).expanduser()
        root = root.resolve() if root.is_absolute() else (Path.cwd() / root).resolve()
        root.mkdir(parents=True, exist_ok=True)
        cls.suite = Path(tempfile.mkdtemp(prefix="release-artifacts-", dir=root))

        missing = [
            tool
            for tool in ("flatpak", "gpg", "gpgconf", "gpgv", "ostree")
            if shutil.which(tool) is None
        ]
        if missing:
            raise RuntimeError(f"required test tools are missing: {', '.join(missing)}")
        if not SCRIPT.is_file():
            raise RuntimeError(f"release artifact CLI is missing: {SCRIPT}")

        cls.env = os.environ.copy()
        cls.env.pop("GPG_AGENT_INFO", None)
        cls.env.update(
            {
                "HOME": str(cls.suite / "home"),
                "GNUPGHOME": str(cls.suite / "gnupg"),
                "XDG_CACHE_HOME": str(cls.suite / "xdg-cache"),
                "XDG_CONFIG_HOME": str(cls.suite / "xdg-config"),
                "XDG_DATA_HOME": str(cls.suite / "xdg-data"),
                "XDG_STATE_HOME": str(cls.suite / "xdg-state"),
                "XDG_RUNTIME_DIR": str(cls.suite / "xdg-runtime"),
                "FLATPAK_USER_DIR": str(cls.suite / "flatpak-user"),
            }
        )
        for variable in (
            "HOME",
            "GNUPGHOME",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "FLATPAK_USER_DIR",
        ):
            Path(cls.env[variable]).mkdir(mode=0o700)
        cls.addClassCleanup(cls._report_suite)
        cls.addClassCleanup(cls._kill_agent)

        cls.signer = cls._make_key("Release Test", "release@example.invalid")
        cls.other_signer = cls._make_key("Wrong Release Test", "wrong@example.invalid")
        base = cls.suite / "base-repositories"
        base.mkdir()
        cls.repositories = {
            arch: cls._make_repository(base / arch, arch, cls.signer) for arch in ARCHES
        }

    @classmethod
    def _kill_agent(cls) -> None:
        subprocess.run(
            ["gpgconf", "--homedir", cls.env["GNUPGHOME"], "--kill", "gpg-agent"],
            env=cls.env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=SHORT_TIMEOUT,
            check=False,
        )

    @classmethod
    def _report_suite(cls) -> None:
        print(f"release artifact fixtures retained at {cls.suite}")

    @classmethod
    def _raw_run(
        cls, argv: list[str | Path], timeout: int = SHORT_TIMEOUT
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(word) for word in argv],
            env=cls.env,
            cwd=cls.suite,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )

    @classmethod
    def _checked_run(cls, argv: list[str | Path], timeout: int = SHORT_TIMEOUT) -> str:
        result = cls._raw_run(argv, timeout)
        if result.returncode:
            raise RuntimeError(
                f"command failed ({result.returncode}): {argv!r}\n"
                f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
        return result.stdout

    @classmethod
    def _make_key(cls, name: str, email: str) -> str:
        cls._checked_run(
            [
                "gpg",
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                f"{name} <{email}>",
                "rsa2048",
                "sign",
                "0",
            ]
        )
        listing = cls._checked_run(
            ["gpg", "--batch", "--with-colons", "--list-secret-keys", email]
        )
        fingerprints = [
            line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:")
        ]
        if not fingerprints:
            raise RuntimeError(f"no fingerprint generated for {email}")
        return fingerprints[0]

    @classmethod
    def _make_repository(
        cls,
        repository: Path,
        arch: str,
        signer: str,
        *,
        sign_app: bool = True,
        sign_debug: bool = True,
        debug_signer: str | None = None,
        debug_payload: str = "initial",
        payload: str = "initial",
        release_version: str = VERSION,
    ) -> dict[str, str | Path]:
        repository.mkdir(parents=True)
        cls._checked_run(["ostree", f"--repo={repository}", "init", "--mode=archive-z2"])

        app_metadata = (
            "[Application]\n"
            f"name={APP_ID}\n"
            f"runtime=org.freedesktop.Platform/{arch}/25.08\n"
            f"sdk=org.freedesktop.Sdk/{arch}/25.08\n"
            "command=kjerag\n\n[Context]\n"
        )
        app_tree = repository.parent / f"{repository.name}-{arch}-app-tree"
        binary = app_tree / "files/bin/kjerag"
        binary.parent.mkdir(parents=True)
        binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        binary.chmod(0o755)
        (app_tree / "metadata").write_text(app_metadata, encoding="utf-8")
        payload_file = app_tree / "files/share/kjerag-release-test/payload.txt"
        payload_file.parent.mkdir(parents=True)
        payload_file.write_text(f"{payload}\n", encoding="utf-8")
        license_file = app_tree / f"files/share/licenses/{APP_ID}/kjerag/LICENSE"
        license_file.parent.mkdir(parents=True)
        shutil.copyfile(SCRIPT.parent.parent / "LICENSE", license_file)
        metainfo = app_tree / f"files/share/metainfo/{APP_ID}.metainfo.xml"
        metainfo.parent.mkdir(parents=True)
        metainfo.write_text(
            f"""<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>{APP_ID}</id><name>Kjerag release test</name>
  <summary>Synthetic release artifact</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>AGPL-3.0-only</project_license>
  <description><p>Exercises repository handoff only.</p></description>
  <launchable type="desktop-id">{APP_ID}.desktop</launchable>
  <releases><release version="{release_version}" date="2026-09-14"/></releases>
</component>
""",
            encoding="utf-8",
        )
        desktop = app_tree / f"files/share/applications/{APP_ID}.desktop"
        desktop.parent.mkdir(parents=True)
        desktop.write_text(
            "[Desktop Entry]\nType=Application\nName=Kjerag release test\n"
            f"Exec=kjerag\nIcon={APP_ID}\n",
            encoding="utf-8",
        )
        icon = app_tree / f"files/share/icons/hicolor/64x64/apps/{APP_ID}.png"
        icon.parent.mkdir(parents=True)
        icon.write_bytes(
            base64.b64decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/"
                "Wn1+AAAAAElFTkSuQmCC"
            )
        )

        debug_metadata = (
            f"[Runtime]\nname={APP_ID}.Debug\n"
            f"runtime={APP_ID}.Debug/{arch}/stable\n"
            f"sdk=org.freedesktop.Sdk/{arch}/25.08\n"
        )
        debug_tree = repository.parent / f"{repository.name}-{arch}-debug-tree"
        debug_file = debug_tree / "files/lib/debug/.build-id/00/synthetic.debug"
        debug_file.parent.mkdir(parents=True)
        debug_file.write_text(f"debug-{arch}-{debug_payload}\n", encoding="utf-8")
        (debug_tree / "metadata").write_text(debug_metadata, encoding="utf-8")

        commits: dict[str, str | Path] = {"repository": repository}
        inputs = (
            ("app", app_ref(arch), app_tree, app_metadata, sign_app, signer),
            (
                "debug",
                debug_ref(arch),
                debug_tree,
                debug_metadata,
                sign_debug,
                debug_signer or signer,
            ),
        )
        for kind, ref, tree, metadata, signed, commit_signer in inputs:
            command = [
                "ostree",
                f"--repo={repository}",
                "commit",
                f"--branch={ref}",
                f"--subject=synthetic {kind} {arch}",
                f"--tree=dir={tree}",
                f"--add-metadata-string=xa.ref={ref}",
                f"--add-metadata-string=xa.metadata={metadata}",
                "--timestamp=2026-09-14 12:00:00+00:00",
            ]
            if signed:
                command.append(f"--gpg-sign={commit_signer}")
            cls._checked_run(command)
            commits[kind] = cls._checked_run(
                ["ostree", f"--repo={repository}", "rev-parse", ref]
            ).strip()
        cls._checked_run(
            ["flatpak", "build-update-repo", f"--gpg-sign={signer}", repository],
            LONG_TIMEOUT,
        )
        return commits

    def setUp(self) -> None:
        label = self.id().rsplit(".", 1)[-1].replace("_", "-")
        self.case = Path(tempfile.mkdtemp(prefix=f"{label}-", dir=self.suite))

    def cli(self, arguments: list[str | Path], *, succeeds: bool = True) -> None:
        timeout = LONG_TIMEOUT if arguments[0] == "assemble" else SHORT_TIMEOUT
        result = self._raw_run(["bash", SCRIPT, *arguments], timeout)
        detail = f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        if succeeds:
            self.assertEqual(result.returncode, 0, detail)
        else:
            self.assertNotEqual(result.returncode, 0, detail)
            if arguments[0] == "assemble":
                output = Path(arguments[2])
                self.assertFalse(
                    (output / "payload.sig").exists(),
                    f"failed assembly created a publication seal under {output}",
                )

    def stage(
        self,
        staging: Path,
        arch: str,
        *,
        repository: Path | None = None,
        version: str = VERSION,
        source: str = SOURCE,
    ) -> Path:
        repository = repository or Path(self.repositories[arch]["repository"])
        output = staging / f"release-{arch}"
        self.cli(["stage", repository, output, arch, version, source, self.signer])
        return output

    def stage_pair(self, staging: Path) -> None:
        staging.mkdir()
        for arch in ARCHES:
            self.stage(staging, arch)

    def assemble(
        self, staging: Path, output: Path, *, signer: str | None = None, succeeds=True
    ) -> None:
        self.cli(
            ["assemble", staging, output, VERSION, SOURCE, signer or self.signer],
            succeeds=succeeds,
        )

    def seal(self, output: Path) -> None:
        self.cli(["seal", output, VERSION, SOURCE, self.signer])

    def verify(
        self, output: Path, *, source: str = SOURCE, succeeds: bool = True
    ) -> None:
        self.cli(["verify", output, VERSION, source, self.signer], succeeds=succeeds)

    def assembled_and_sealed(self) -> Path:
        staging, output = self.case / "staging", self.case / "assembled"
        self.stage_pair(staging)
        self.assemble(staging, output)
        self.seal(output)
        return output

    def resign(self, handoff: Path) -> None:
        signature = handoff / "provenance.sig"
        signature.unlink(missing_ok=True)
        self._checked_run(
            [
                "gpg",
                "--batch",
                "--local-user",
                self.signer,
                "--detach-sign",
                "--output",
                signature,
                handoff / "provenance.txt",
            ]
        )

    def rev(self, repository: Path, ref: str) -> str:
        return self._checked_run(
            ["ostree", f"--repo={repository}", "rev-parse", ref]
        ).strip()

    def assert_installed(
        self, arch: str, commit: str, origin: str, payload: str, sentinel: Path
    ) -> None:
        info = ["flatpak", "info", "--user", f"--arch={arch}"]
        self.assertEqual(
            self._checked_run([*info, "--show-commit", APP_ID]).strip(), commit
        )
        self.assertEqual(
            self._checked_run([*info, "--show-origin", APP_ID]).strip(), origin
        )
        location = Path(
            self._checked_run([*info, "--show-location", APP_ID]).strip()
        )
        self.assertEqual(
            (location / "files/share/kjerag-release-test/payload.txt").read_text(),
            f"{payload}\n",
        )
        self.assertEqual(
            (location / f"files/share/licenses/{APP_ID}/kjerag/LICENSE").read_bytes(),
            (SCRIPT.parent.parent / "LICENSE").read_bytes(),
        )
        self.assertEqual(sentinel.read_text(), "keep this setting\n")

    def test_happy_path_preserves_signed_bundle_commits(self) -> None:
        staging, output = self.case / "staging", self.case / "assembled"
        self.stage_pair(staging)
        self.assemble(staging, output)
        self._checked_run(
            ["bash", SCRIPT.with_name("pages-site.sh"), output / "repository", self.signer],
            LONG_TIMEOUT,
        )
        self.seal(output)
        self.verify(output)

        aggregate = output / "repository"
        summary_remote = "release"
        self._checked_run(
            [
                "flatpak",
                "remote-add",
                "--user",
                f"--gpg-import={output / 'signing-key.gpg'}",
                summary_remote,
                f"file://{aggregate}",
            ]
        )
        for arch in ARCHES:
            expected_app = str(self.repositories[arch]["app"])
            expected_debug = str(self.repositories[arch]["debug"])
            self.assertEqual(self.rev(aggregate, app_ref(arch)), expected_app)
            self.assertEqual(self.rev(aggregate, debug_ref(arch)), expected_debug)
            for ref, expected in (
                (app_ref(arch), expected_app),
                (debug_ref(arch), expected_debug),
            ):
                queried = self._checked_run(
                    [
                        "flatpak",
                        "remote-info",
                        "--user",
                        f"--arch={arch}",
                        "--show-commit",
                        summary_remote,
                        ref,
                    ]
                ).strip()
                self.assertEqual(queried, expected)

            # Debug stays repository-only. Prove both commit signatures survive
            # aggregation instead of relying on local ref/checksum equality.
            aggregate_verified = self.case / f"aggregate-verified-{arch}"
            aggregate_remote = f"aggregate-{arch}"
            self._checked_run(
                ["ostree", f"--repo={aggregate_verified}", "init", "--mode=archive-z2"]
            )
            self._checked_run(
                [
                    "ostree",
                    f"--repo={aggregate_verified}",
                    "remote",
                    "add",
                    f"--gpg-import={output / 'signing-key.gpg'}",
                    aggregate_remote,
                    f"file://{aggregate}",
                ]
            )
            self._checked_run(
                [
                    "ostree",
                    f"--repo={aggregate_verified}",
                    "pull-local",
                    "--untrusted",
                    "--gpg-verify",
                    f"--remote={aggregate_remote}",
                    aggregate,
                    app_ref(arch),
                    debug_ref(arch),
                ],
                LONG_TIMEOUT,
            )
            self.assertEqual(
                self.rev(aggregate_verified, f"{aggregate_remote}:{app_ref(arch)}"),
                expected_app,
            )
            self.assertEqual(
                self.rev(aggregate_verified, f"{aggregate_remote}:{debug_ref(arch)}"),
                expected_debug,
            )

            imported = self.case / f"bundle-import-{arch}"
            self._checked_run(["ostree", f"--repo={imported}", "init", "--mode=archive-z2"])
            bundle = output / "bundles" / f"kjerag-{VERSION}-{arch}.flatpak"
            self._checked_run(
                ["flatpak", "build-import-bundle", imported, bundle], LONG_TIMEOUT
            )
            self.assertEqual(self.rev(imported, app_ref(arch)), expected_app)

            # A second untrusted pull proves the detached commit signature,
            # rather than only its checksum, survived bundle export/import.
            verified, remote = self.case / f"bundle-verified-{arch}", f"bundle-{arch}"
            self._checked_run(["ostree", f"--repo={verified}", "init", "--mode=archive-z2"])
            self._checked_run(
                [
                    "ostree",
                    f"--repo={verified}",
                    "remote",
                    "add",
                    f"--gpg-import={output / 'signing-key.gpg'}",
                    remote,
                    f"file://{imported}",
                ]
            )
            self._checked_run(
                [
                    "ostree",
                    f"--repo={verified}",
                    "pull-local",
                    "--untrusted",
                    "--gpg-verify",
                    f"--remote={remote}",
                    imported,
                    app_ref(arch),
                ],
                LONG_TIMEOUT,
            )
            self.assertEqual(self.rev(verified, f"{remote}:{app_ref(arch)}"), expected_app)

        # A site-only republish must preserve all refs and the signed public
        # source/commit record, while being free to re-sign the summary.
        refs_before = {
            path.relative_to(aggregate): path.read_bytes()
            for path in (aggregate / "refs/heads").rglob("*")
            if path.is_file()
        }
        record_before = (aggregate / "kjerag-release.txt").read_bytes()
        self._checked_run(
            ["bash", SCRIPT.with_name("pages-site.sh"), aggregate, self.signer],
            LONG_TIMEOUT,
        )
        refs_after = {
            path.relative_to(aggregate): path.read_bytes()
            for path in (aggregate / "refs/heads").rglob("*")
            if path.is_file()
        }
        self.assertEqual(refs_after, refs_before)
        self.assertEqual((aggregate / "kjerag-release.txt").read_bytes(), record_before)
        self._checked_run(
            ["gpg", "--batch", "--verify", aggregate / "kjerag-release.sig", aggregate / "kjerag-release.txt"]
        )

    def test_isolated_native_install_bundle_reinstall_and_update(self) -> None:
        arch = platform.machine()
        if arch not in ARCHES:
            self.skipTest(f"no synthetic release architecture for host {arch}")
        staging, output = self.case / "staging", self.case / "assembled"
        self.stage_pair(staging)
        self.assemble(staging, output)

        remote = "kjerag-release-test"
        aggregate = output / "repository"
        self._checked_run(
            [
                "flatpak",
                "remote-add",
                "--user",
                "--if-not-exists",
                f"--gpg-import={output / 'signing-key.gpg'}",
                remote,
                f"file://{aggregate}",
            ]
        )
        self._checked_run(
            [
                "flatpak",
                "install",
                "--user",
                "--noninteractive",
                "--no-deps",
                "--no-related",
                remote,
                app_ref(arch),
            ],
            LONG_TIMEOUT,
        )
        sentinel = Path(self.env["HOME"]) / f".var/app/{APP_ID}/config/test-setting"
        sentinel.parent.mkdir(parents=True)
        sentinel.write_text("keep this setting\n")
        first_commit = str(self.repositories[arch]["app"])
        self.assert_installed(arch, first_commit, remote, "initial", sentinel)

        # build-bundle records the public channel URL. Retargeting only this
        # isolated fixture remote exercises origin matching without contacting
        # that URL; it is not live HTTPS or channel-publication evidence.
        self._checked_run(
            [
                "flatpak",
                "remote-modify",
                "--user",
                "--url=https://kjerag.harding.dev/",
                remote,
            ]
        )
        bundle = output / "bundles" / f"kjerag-{VERSION}-{arch}.flatpak"
        self._checked_run(
            [
                "flatpak",
                "install",
                "--user",
                "--noninteractive",
                "--no-deps",
                "--no-related",
                "--reinstall",
                bundle,
            ],
            LONG_TIMEOUT,
        )
        self.assert_installed(arch, first_commit, remote, "initial", sentinel)

        update = self._make_repository(
            self.case / f"update-{arch}",
            arch,
            self.signer,
            payload="updated",
            release_version=OTHER_VERSION,
        )
        update_commit = str(update["app"])
        self.assertNotEqual(update_commit, first_commit)
        self._checked_run(
            [
                "flatpak",
                "remote-modify",
                "--user",
                f"--url=file://{update['repository']}",
                remote,
            ]
        )
        self._checked_run(
            [
                "flatpak",
                "update",
                "--user",
                "--noninteractive",
                "--no-deps",
                "--no-related",
                app_ref(arch),
            ],
            LONG_TIMEOUT,
        )
        self.assert_installed(arch, update_commit, remote, "updated", sentinel)

    def test_fresh_native_app_only_bundle_install_records_official_origin(self) -> None:
        arch = platform.machine()
        if arch not in ARCHES:
            self.skipTest(f"no synthetic release architecture for host {arch}")
        output = self.assembled_and_sealed()
        fresh = {
            "HOME": str(self.case / "fresh-home"),
            "FLATPAK_USER_DIR": str(self.case / "fresh-flatpak-user"),
        }
        for directory in fresh.values():
            Path(directory).mkdir(mode=0o700)
        previous = {name: self.env.get(name) for name in fresh}
        self.env.update(fresh)
        try:
            self.assertEqual(
                self._checked_run(
                    ["flatpak", "list", "--user", "--columns=application"]
                ).strip(),
                "",
            )
            self.assertEqual(
                self._checked_run(
                    ["flatpak", "remotes", "--user", "--columns=name"]
                ).strip(),
                "",
            )
            bundle = output / "bundles" / f"kjerag-{VERSION}-{arch}.flatpak"
            self._checked_run(
                [
                    "flatpak",
                    "install",
                    "--user",
                    "--noninteractive",
                    "--no-deps",
                    "--no-related",
                    bundle,
                ],
                LONG_TIMEOUT,
            )
            info = ["flatpak", "info", "--user", f"--arch={arch}"]
            self.assertEqual(
                self._checked_run([*info, "--show-commit", APP_ID]).strip(),
                str(self.repositories[arch]["app"]),
            )
            origin = self._checked_run([*info, "--show-origin", APP_ID]).strip()
            location = Path(
                self._checked_run([*info, "--show-location", APP_ID]).strip()
            )
            executable = location / "files/bin/kjerag"
            self.assertEqual(
                executable.read_text(), "#!/bin/sh\nexit 0\n"
            )
            self.assertTrue(os.access(executable, os.X_OK))
            self.assertEqual(
                (location / f"files/share/licenses/{APP_ID}/kjerag/LICENSE").read_bytes(),
                (SCRIPT.parent.parent / "LICENSE").read_bytes(),
            )
            remotes = dict(
                line.split("\t", 1)
                for line in self._checked_run(
                    ["flatpak", "remotes", "--user", "--columns=name,url"]
                ).splitlines()
            )
            self.assertEqual(remotes[origin], "https://kjerag.harding.dev/")
            self.assertEqual(
                self._checked_run(
                    ["flatpak", "list", "--user", "--runtime", "--columns=application"]
                ).strip(),
                "",
            )
        finally:
            for name, value in previous.items():
                if value is None:
                    self.env.pop(name, None)
                else:
                    self.env[name] = value

    def test_missing_architecture_is_refused(self) -> None:
        staging = self.case / "staging"
        staging.mkdir()
        self.stage(staging, "x86_64")
        self.assemble(staging, self.case / "assembled", succeeds=False)

    def test_mixed_source_or_version_is_refused(self) -> None:
        for field, value in (("source", OTHER_SOURCE), ("version", OTHER_VERSION)):
            with self.subTest(field=field):
                staging = self.case / f"staging-{field}"
                staging.mkdir()
                self.stage(staging, "x86_64")
                self.stage(staging, "aarch64", **{field: value})
                self.assemble(staging, self.case / f"assembled-{field}", succeeds=False)

    def test_wrong_but_signed_architecture_is_refused(self) -> None:
        staging = self.case / "staging"
        self.stage_pair(staging)
        handoff = staging / "release-aarch64"
        provenance = handoff / "provenance.txt"
        provenance.write_text(
            provenance.read_text().replace("arch=aarch64\n", "arch=x86_64\n")
        )
        self.resign(handoff)
        self.assemble(staging, self.case / "assembled", succeeds=False)

    def test_tampered_or_unsigned_provenance_is_refused(self) -> None:
        for failure in ("tampered", "unsigned"):
            with self.subTest(failure=failure):
                staging = self.case / f"staging-{failure}"
                self.stage_pair(staging)
                handoff = staging / "release-aarch64"
                if failure == "tampered":
                    with (handoff / "provenance.txt").open("a") as stream:
                        stream.write("tampered=yes\n")
                else:
                    (handoff / "provenance.sig").unlink()
                self.assemble(staging, self.case / f"assembled-{failure}", succeeds=False)

    def test_ref_change_after_provenance_check_cannot_replace_commits(self) -> None:
        staging, output = self.case / "staging", self.case / "assembled"
        self.stage_pair(staging)
        handoff = staging / "release-x86_64/repository"
        original_app = str(self.repositories["x86_64"]["app"])
        original_debug = str(self.repositories["x86_64"]["debug"])
        successor = self._make_repository(
            self.case / "signed-successor-x86_64",
            "x86_64",
            self.signer,
            payload="successor",
            debug_payload="successor",
            release_version=OTHER_VERSION,
        )
        successor_app = str(successor["app"])
        successor_debug = str(successor["debug"])
        self.assertNotEqual(successor_app, original_app)
        self.assertNotEqual(successor_debug, original_debug)
        self._checked_run(
            [
                "ostree",
                f"--repo={handoff}",
                "pull-local",
                successor["repository"],
                successor_app,
                successor_debug,
            ]
        )

        real_ostree = shutil.which("ostree", path=self.env["PATH"])
        self.assertIsNotNone(real_ostree)
        wrappers = self.case / "ref-swap-wrappers"
        wrappers.mkdir()
        wrapper = wrappers / "ostree"
        wrapper.write_text(
            "#!/bin/sh\n"
            "for arg do\n"
            "  if [ \"$arg\" = pull-local ] && [ ! -e \"$REF_SWAP_MARKER\" ]; then\n"
            "    printf '%s\\n' \"$REF_SWAP_APP\" > \"$REF_SWAP_APP_PATH\"\n"
            "    printf '%s\\n' \"$REF_SWAP_DEBUG\" > \"$REF_SWAP_DEBUG_PATH\"\n"
            "    : > \"$REF_SWAP_MARKER\"\n"
            "    break\n"
            "  fi\n"
            "done\n"
            "exec \"$REAL_OSTREE\" \"$@\"\n"
        )
        wrapper.chmod(0o755)
        injected = {
            "PATH": f"{wrappers}:{self.env['PATH']}",
            "REAL_OSTREE": real_ostree,
            "REF_SWAP_MARKER": str(self.case / "ref-swap-fired"),
            "REF_SWAP_APP": successor_app,
            "REF_SWAP_DEBUG": successor_debug,
            "REF_SWAP_APP_PATH": str(handoff / "refs/heads" / app_ref("x86_64")),
            "REF_SWAP_DEBUG_PATH": str(handoff / "refs/heads" / debug_ref("x86_64")),
        }
        previous = {name: self.env.get(name) for name in injected}
        self.env.update(injected)
        try:
            self.assemble(staging, output)
        finally:
            for name, value in previous.items():
                if value is None:
                    self.env.pop(name, None)
                else:
                    self.env[name] = value

        self.assertTrue((self.case / "ref-swap-fired").is_file())
        self.assertEqual(self.rev(output / "repository", app_ref("x86_64")), original_app)
        self.assertEqual(
            self.rev(output / "repository", debug_ref("x86_64")), original_debug
        )
        self.assertEqual(
            (handoff / "refs/heads" / app_ref("x86_64")).read_text().strip(),
            successor_app,
        )
        self.assertEqual(
            (handoff / "refs/heads" / debug_ref("x86_64")).read_text().strip(),
            successor_debug,
        )

    def test_unsigned_commit_is_refused(self) -> None:
        unsigned = self._make_repository(
            self.case / "unsigned-aarch64", "aarch64", self.signer, sign_app=False
        )
        staging = self.case / "staging"
        staging.mkdir()
        self.stage(staging, "x86_64")
        self.stage(staging, "aarch64", repository=Path(unsigned["repository"]))
        self.assemble(staging, self.case / "assembled", succeeds=False)

    def test_unsigned_debug_commit_is_refused(self) -> None:
        unsigned = self._make_repository(
            self.case / "unsigned-debug-aarch64",
            "aarch64",
            self.signer,
            sign_debug=False,
        )
        staging = self.case / "staging"
        staging.mkdir()
        self.stage(staging, "x86_64")
        self.stage(staging, "aarch64", repository=Path(unsigned["repository"]))
        self.assemble(staging, self.case / "assembled", succeeds=False)

    def test_wrong_key_signed_debug_commit_is_refused(self) -> None:
        wrong_key = self._make_repository(
            self.case / "wrong-key-debug-aarch64",
            "aarch64",
            self.signer,
            debug_signer=self.other_signer,
        )
        staging = self.case / "staging"
        staging.mkdir()
        self.stage(staging, "x86_64")
        self.stage(staging, "aarch64", repository=Path(wrong_key["repository"]))
        self.assemble(staging, self.case / "assembled", succeeds=False)

    def test_wrong_trusted_key_is_refused(self) -> None:
        staging = self.case / "staging"
        self.stage_pair(staging)
        self.assemble(
            staging, self.case / "assembled", signer=self.other_signer, succeeds=False
        )

    def test_missing_or_corrupt_app_or_debug_commit_is_refused(self) -> None:
        for kind in ("app", "debug"):
            for failure in ("missing", "corrupt"):
                with self.subTest(kind=kind, failure=failure):
                    staging = self.case / f"staging-{kind}-{failure}"
                    self.stage_pair(staging)
                    commit = str(self.repositories["aarch64"][kind])
                    obj = (
                        staging
                        / "release-aarch64/repository/objects"
                        / commit[:2]
                        / f"{commit[2:]}.commit"
                    )
                    self.assertTrue(obj.is_file(), obj)
                    if failure == "missing":
                        obj.unlink()
                    else:
                        contents = obj.read_bytes()
                        obj.write_bytes(bytes([contents[0] ^ 0xFF]) + contents[1:])
                    self.assemble(
                        staging,
                        self.case / f"assembled-{kind}-{failure}",
                        succeeds=False,
                    )

    def test_assembly_output_inside_staging_is_refused(self) -> None:
        staging = self.case / "staging"
        self.stage_pair(staging)
        self.assemble(staging, staging / "assembled", succeeds=False)

    def test_verify_refuses_tampered_bundle(self) -> None:
        output = self.assembled_and_sealed()
        bundle = output / "bundles" / f"kjerag-{VERSION}-x86_64.flatpak"
        contents = bundle.read_bytes()
        bundle.write_bytes(bytes([contents[0] ^ 0xFF]) + contents[1:])
        self.verify(output, succeeds=False)

    def test_verify_refuses_changed_repository_ref(self) -> None:
        output = self.assembled_and_sealed()
        ref = output / "repository/refs/heads" / app_ref("x86_64")
        ref.write_text(f"{self.repositories['aarch64']['app']}\n")
        self.verify(output, succeeds=False)

    def test_verify_refuses_added_payload_file(self) -> None:
        output = self.assembled_and_sealed()
        (output / "bundles/unexpected.txt").write_text("not in the seal\n")
        self.verify(output, succeeds=False)

    def test_verify_refuses_payload_symlink(self) -> None:
        output = self.assembled_and_sealed()
        (output / "bundles/unexpected-link").symlink_to(output / "release.txt")
        self.verify(output, succeeds=False)

    def test_verify_refuses_wrong_expected_source(self) -> None:
        output = self.assembled_and_sealed()
        self.verify(output, source=OTHER_SOURCE, succeeds=False)

    def test_second_bundle_failure_leaves_no_publication_seal(self) -> None:
        staging, output = self.case / "staging", self.case / "assembled"
        self.stage_pair(staging)
        real_flatpak = shutil.which("flatpak", path=self.env["PATH"])
        self.assertIsNotNone(real_flatpak)
        wrappers = self.case / "wrappers"
        wrappers.mkdir()
        wrapper = wrappers / "flatpak"
        wrapper.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = build-bundle ]; then\n"
            "  for arg do\n"
            "    [ \"$arg\" = --arch=aarch64 ] && exit 23\n"
            "  done\n"
            "fi\n"
            f'exec "{real_flatpak}" "$@"\n'
        )
        wrapper.chmod(0o755)
        previous_path = self.env["PATH"]
        self.env["PATH"] = f"{wrappers}:{previous_path}"
        try:
            self.assemble(staging, output, succeeds=False)
        finally:
            self.env["PATH"] = previous_path
        self.assertTrue((output / f"bundles/kjerag-{VERSION}-x86_64.flatpak").is_file())
        self.assertFalse((output / f"bundles/kjerag-{VERSION}-aarch64.flatpak").exists())
        self.assertFalse((output / "payload.sig").exists())

    def test_existing_output_is_refused(self) -> None:
        staging = self.case / "staging"
        staging.mkdir()
        stage_output = self.stage(staging, "x86_64")
        self.cli(
            [
                "stage",
                self.repositories["x86_64"]["repository"],
                stage_output,
                "x86_64",
                VERSION,
                SOURCE,
                self.signer,
            ],
            succeeds=False,
        )
        self.assertFalse(any(stage_output.rglob("*.flatpak")))

        self.stage(staging, "aarch64")
        assembled = self.case / "assembled"
        assembled.mkdir()
        self.assemble(staging, assembled, succeeds=False)


if __name__ == "__main__":
    unittest.main(verbosity=2)
