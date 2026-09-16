#!/usr/bin/env python3
"""Refuse a stale or downgraded replacement of the published channel."""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import NoReturn


SOURCE_RE = re.compile(r"[0-9a-f]{40}")
VERSION_RE = re.compile(
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)
REFS = tuple(
    ref
    for arch in ("x86_64", "aarch64")
    for ref in (
        f"app/dev.harding.Kjerag/{arch}/stable",
        f"runtime/dev.harding.Kjerag.Debug/{arch}/stable",
    )
)


def fail(message: str) -> NoReturn:
    raise RuntimeError(message)


@dataclass(frozen=True)
class Version:
    core: tuple[int, int, int]
    prerelease: tuple[str, ...] | None

    @classmethod
    def parse(cls, text: str) -> "Version":
        match = VERSION_RE.fullmatch(text)
        if match is None:
            fail("invalid release version")
        prerelease = None if match[4] is None else tuple(match[4].split("."))
        if prerelease is not None and any(
            part.isdigit() and len(part) > 1 and part.startswith("0")
            for part in prerelease
        ):
            fail("invalid numeric prerelease identifier")
        return cls(tuple(int(match[index]) for index in range(1, 4)), prerelease)

    def compare(self, other: "Version") -> int:
        if self.core != other.core:
            return (self.core > other.core) - (self.core < other.core)
        if self.prerelease is None or other.prerelease is None:
            return (self.prerelease is None) - (other.prerelease is None)
        for left, right in zip(self.prerelease, other.prerelease):
            if left == right:
                continue
            left_number, right_number = left.isdigit(), right.isdigit()
            if left_number and right_number:
                return (int(left) > int(right)) - (int(left) < int(right))
            if left_number != right_number:
                return -1 if left_number else 1
            return (left > right) - (left < right)
        return (len(self.prerelease) > len(other.prerelease)) - (
            len(self.prerelease) < len(other.prerelease)
        )


@dataclass(frozen=True)
class Record:
    source: str
    version: Version


def read_record(path: Path) -> tuple[bytes, Record]:
    try:
        raw = path.read_bytes()
        text = raw.decode("ascii")
    except (OSError, UnicodeError) as error:
        fail(f"cannot read release record {path}: {error}")
    lines = text.splitlines(keepends=True)
    if len(lines) != 7 or any(not line.endswith("\n") for line in lines):
        fail(f"release record is not canonical: {path}")
    if lines[0] != "format=1\n":
        fail(f"unsupported release record format: {path}")
    if not lines[1].startswith("source=") or not lines[2].startswith("version="):
        fail(f"release record fields are not canonical: {path}")
    source = lines[1][len("source=") : -1]
    version_text = lines[2][len("version=") : -1]
    if SOURCE_RE.fullmatch(source) is None:
        fail(f"invalid source revision in {path}")
    version = Version.parse(version_text)
    for line, expected_ref in zip(lines[3:], REFS):
        match = re.fullmatch(r"([^ ]+) ([0-9a-f]{64})\n", line)
        if match is None or match.group(1) != expected_ref:
            fail(f"release commit fields are not canonical: {path}")
    return raw, Record(source, version)


def check(published_path: Path, candidate_path: Path) -> None:
    candidate_raw, candidate = read_record(candidate_path)
    # A genuinely absent marker is the one-time bootstrap. A broken symlink or
    # unreadable existing path is not absence and must fail in read_record.
    if not os.path.lexists(published_path):
        return
    published_raw, published = read_record(published_path)
    if published.source == candidate.source:
        if published_raw != candidate_raw:
            fail("the same source revision has a different release record")
        return
    if candidate.version.compare(published.version) <= 0:
        fail("candidate version does not advance the published version")
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", published.source, candidate.source],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=30,
        check=False,
    )
    if result.returncode == 1:
        fail("candidate source does not descend from the published source")
    if result.returncode != 0:
        detail = result.stderr.strip() or "git could not check release ancestry"
        fail(detail)


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(
            "usage: check-release-channel.py PUBLISHED_RECORD CANDIDATE_RECORD",
            file=sys.stderr,
        )
        return 2
    try:
        check(Path(argv[1]), Path(argv[2]))
    except (RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"check-release-channel: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
