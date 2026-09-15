#!/usr/bin/env python3
"""Publish one already assembled, signed release payload through gh."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import NoReturn


REPOSITORY = "aeharding/kjerag"
ARCHITECTURES = ("x86_64", "aarch64")
VERSION_RE = re.compile(
    r"[0-9]+\.[0-9]+\.[0-9]+(?:[+-][A-Za-z0-9.+-]+)?"
)
SOURCE_RE = re.compile(r"[0-9a-f]{40}")
SHA256_RE = re.compile(r"([0-9a-f]{64})  ([^/\n]+)\n")
MAX_TAG_OBJECTS = 4


def fail(message: str) -> NoReturn:
    raise RuntimeError(message)


def is_prerelease(version: str) -> bool:
    return "-" in version.partition("+")[0]


def run_gh(
    arguments: list[str], *, cwd: Path | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["gh", *arguments],
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=120,
        check=False,
    )


def gh_json(arguments: list[str], *, allow_404: bool = False) -> dict | None:
    result = run_gh(arguments)
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError:
        value = None
    if result.returncode:
        if allow_404 and isinstance(value, dict) and str(value.get("status")) == "404":
            return None
        detail = result.stderr.strip() or result.stdout.strip() or "no error text"
        fail(f"gh failed: {detail}")
    if not isinstance(value, dict):
        fail("gh returned invalid JSON")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def expected_files(version: str, directory: Path) -> dict[str, Path]:
    names = [
        name
        for arch in ARCHITECTURES
        for name in (
            f"kjerag-{version}-{arch}.flatpak",
            f"kjerag-{version}-{arch}.flatpak.sha256",
        )
    ]
    if not directory.is_dir():
        fail("bundles directory does not exist")
    actual = {entry.name for entry in directory.iterdir()}
    if actual != set(names) or any(not (directory / name).is_file() for name in names):
        fail("bundles directory must contain exactly the four release files")

    for arch in ARCHITECTURES:
        bundle_name = f"kjerag-{version}-{arch}.flatpak"
        checksum_name = f"{bundle_name}.sha256"
        checksum_text = (directory / checksum_name).read_text(encoding="ascii")
        match = SHA256_RE.fullmatch(checksum_text)
        if not match or match.group(2) != bundle_name:
            fail(f"invalid checksum file: {checksum_name}")
        if match.group(1) != sha256(directory / bundle_name):
            fail(f"checksum does not match bundle: {bundle_name}")
    return {name: directory / name for name in names}


def verify_tag(version: str, source: str, repository: str) -> None:
    value = gh_json(["api", f"repos/{repository}/git/ref/tags/{version}"])
    if value is None:
        fail("tag lookup unexpectedly returned not found")
    object_value = value.get("object")
    peeled = 0
    while True:
        if not isinstance(object_value, dict):
            fail("tag lookup returned no object")
        kind, revision = object_value.get("type"), object_value.get("sha")
        if not isinstance(revision, str) or SOURCE_RE.fullmatch(revision) is None:
            fail("tag lookup returned no revision")
        if kind == "commit":
            if revision != source:
                fail(f"tag {version} resolves to {revision}, not {source}")
            return
        if kind != "tag":
            fail(f"tag {version} resolves to unsupported object type {kind!r}")
        if peeled == MAX_TAG_OBJECTS:
            fail(
                f"tag {version} has more than {MAX_TAG_OBJECTS} annotated tag objects"
            )
        peeled += 1
        value = gh_json(["api", f"repos/{repository}/git/tags/{revision}"])
        if value is None:
            fail("annotated tag lookup unexpectedly returned not found")
        object_value = value.get("object")


def release(version: str, repository: str) -> dict | None:
    return gh_json(
        ["api", f"repos/{repository}/releases/tags/{version}"], allow_404=True
    )


def release_assets(
    value: dict, version: str, expected: set[str]
) -> tuple[dict[str, dict], bool]:
    if value.get("tag_name") != version:
        fail("release lookup returned a different tag")
    draft = value.get("draft")
    if not isinstance(draft, bool):
        fail("release lookup returned invalid draft state")
    prerelease = value.get("prerelease")
    if not isinstance(prerelease, bool) or prerelease != is_prerelease(version):
        fail("release prerelease state does not match its tag")
    assets = value.get("assets")
    if not isinstance(assets, list):
        fail("release lookup returned invalid assets")
    by_name: dict[str, dict] = {}
    for asset in assets:
        if not isinstance(asset, dict) or not isinstance(asset.get("name"), str):
            fail("release lookup returned an invalid asset")
        name = asset["name"]
        if name in by_name:
            fail(f"release has duplicate asset: {name}")
        by_name[name] = asset
    unexpected = set(by_name).difference(expected)
    if unexpected:
        fail(f"release has unexpected asset: {sorted(unexpected)[0]}")
    return by_name, draft


def checked_gh(arguments: list[str], *, cwd: Path | None = None) -> None:
    result = run_gh(arguments, cwd=cwd)
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or "no error text"
        fail(f"gh failed: {detail}")


def verify_remote_assets(
    version: str,
    repository: str,
    local_hashes: dict[str, str],
    names: set[str],
    workdir: Path,
) -> None:
    for name in sorted(names):
        destination = workdir / name
        if destination.exists():
            destination.unlink()
        checked_gh(
            [
                "release",
                "download",
                version,
                "--repo",
                repository,
                "--pattern",
                name,
                "--dir",
                ".",
            ],
            cwd=workdir,
        )
        if not destination.is_file() or sha256(destination) != local_hashes[name]:
            fail(f"release asset differs from assembled payload: {name}")


def publish(version: str, source: str, directory: Path, repository: str) -> None:
    files = expected_files(version, directory)
    local_hashes = {name: sha256(path) for name, path in files.items()}
    verify_tag(version, source, repository)

    current = release(version, repository)
    if current is None:
        arguments = [
            "release",
            "create",
            version,
            "--repo",
            repository,
            "--verify-tag",
            "--draft",
            "--generate-notes",
        ]
        if is_prerelease(version):
            arguments.append("--prerelease")
        checked_gh(arguments)
        current = release(version, repository)
        if current is None:
            fail("created release cannot be found")

    expected = set(files)
    with tempfile.TemporaryDirectory(prefix="kjerag-release-") as temporary:
        workdir = Path(temporary)
        present, draft = release_assets(current, version, expected)
        present_expected = expected.intersection(present)
        verify_remote_assets(
            version, repository, local_hashes, present_expected, workdir
        )

        missing = expected.difference(present_expected)
        if not draft:
            if missing:
                fail("public release is missing expected assets")
            return
        if missing:
            checked_gh(
                [
                    "release",
                    "upload",
                    version,
                    *(str(files[name]) for name in sorted(missing)),
                    "--repo",
                    repository,
                ]
            )

        final = release(version, repository)
        if final is None:
            fail("release disappeared after upload")
        final_assets, final_draft = release_assets(final, version, expected)
        if not final_draft:
            fail("release became public before verification")
        if not expected.issubset(final_assets):
            fail("draft release is missing expected assets")
        verify_remote_assets(version, repository, local_hashes, expected, workdir)
        checked_gh(
            ["release", "edit", version, "--repo", repository, "--draft=false"]
        )


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(
            "usage: publish-release-assets.py VERSION SOURCE BUNDLES_DIR",
            file=sys.stderr,
        )
        return 2
    version, source, directory_text = argv[1:]
    repository = os.environ.get("GH_REPO")
    try:
        if VERSION_RE.fullmatch(version) is None:
            fail("invalid version")
        if SOURCE_RE.fullmatch(source) is None:
            fail("invalid source revision")
        if repository != REPOSITORY:
            fail(f"GH_REPO must be {REPOSITORY}")
        publish(version, source, Path(directory_text).resolve(), repository)
    except (OSError, UnicodeError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"publish-release-assets: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
