#!/usr/bin/env python3
"""Verify the ordinary Studio 6.0.2 native-rate 360 video oracle."""

from __future__ import annotations

import argparse
import copy
from fractions import Fraction
import hashlib
import json
import math
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONTRACT = ROOT / "docs/research/studio-video-oracle-602.json"
DEFAULT_ARTIFACTS = ROOT / "scratch/studio-video-oracle/ordinary-360"
CANONICAL_CONTRACT_SHA256 = "eda588746c1d15347bae2064b3de2175eb1f1222d159a53c362f900d409e2374"
TARGET_PICTURE_HELPER = ROOT / "scripts/verify-studio-target-picture.py"
CANONICAL_TARGET_PICTURE_HELPER_SHA256 = "85b4eea0cc89985bf63ae15fcbe4a4dab002e5d08c5daa676bfdd5e8771251b1"


class Refusal(Exception):
    """An oracle precondition or receipt did not match exactly."""


class PinnedFile:
    """One authenticated regular file retained by descriptor."""

    def __init__(
        self,
        path: Path,
        descriptor: int,
        state: os.stat_result,
        sha256: str,
    ) -> None:
        self.path = path
        self.descriptor = descriptor
        self.state = state
        self.sha256 = sha256

    def close(self) -> None:
        if self.descriptor >= 0:
            os.close(self.descriptor)
            self.descriptor = -1


def strictly_equal(actual: Any, expected: Any) -> bool:
    if type(actual) is not type(expected):
        return False
    if isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(
            strictly_equal(actual[key], value) for key, value in expected.items()
        )
    if isinstance(expected, (list, tuple)):
        return len(actual) == len(expected) and all(
            strictly_equal(left, right) for left, right in zip(actual, expected)
        )
    return bool(actual == expected)


def same(what: str, actual: Any, expected: Any) -> None:
    if not strictly_equal(actual, expected):
        raise Refusal(f"{what} changed: expected {expected!r}, got {actual!r}")


def load_json_bytes(raw: bytes, path: Path) -> Any:
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise Refusal(f"cannot read JSON {path}: {error}") from error


def same_file_state(before: os.stat_result, after: os.stat_result) -> bool:
    return (
        before.st_dev,
        before.st_ino,
        before.st_mode,
        before.st_size,
        before.st_mtime_ns,
        before.st_ctime_ns,
    ) == (
        after.st_dev,
        after.st_ino,
        after.st_mode,
        after.st_size,
        after.st_mtime_ns,
        after.st_ctime_ns,
    )


def sha256_descriptor(descriptor: int) -> str:
    digest = hashlib.sha256()
    offset = 0
    try:
        while block := os.pread(descriptor, 8 * 1024 * 1024, offset):
            digest.update(block)
            offset += len(block)
    except OSError as error:
        raise Refusal(f"cannot hash pinned input descriptor: {error}") from error
    return digest.hexdigest()


def read_descriptor(descriptor: int) -> bytes:
    blocks = []
    offset = 0
    try:
        while block := os.pread(descriptor, 1024 * 1024, offset):
            blocks.append(block)
            offset += len(block)
    except OSError as error:
        raise Refusal(f"cannot read pinned input descriptor: {error}") from error
    return b"".join(blocks)


def pin_regular_file(
    path: Path,
    what: str,
    *,
    expected_bytes: int | None = None,
    expected_sha256: str | None = None,
) -> PinnedFile:
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise Refusal(f"{what} is not a regular file: {path}")
        if expected_bytes is not None:
            same(f"{what} byte size", before.st_size, expected_bytes)
        digest = sha256_descriptor(descriptor)
        if expected_sha256 is not None:
            same(f"{what} SHA-256", digest, expected_sha256)
        after = os.fstat(descriptor)
        named = os.stat(path, follow_symlinks=False)
        if not same_file_state(before, after) or not same_file_state(before, named):
            raise Refusal(f"{what} changed while it was authenticated: {path}")
        return PinnedFile(path, descriptor, before, digest)
    except (OSError, Refusal) as error:
        if descriptor >= 0:
            os.close(descriptor)
        if isinstance(error, Refusal):
            raise
        raise Refusal(f"cannot pin {what} {path}: {error}") from error


def pin_receipt_file(directory: Path, receipt: dict[str, Any]) -> PinnedFile:
    name = receipt["name"]
    if Path(name).name != name:
        raise Refusal(f"contract artifact name is not a basename: {name!r}")
    pinned = pin_regular_file(
        directory / name,
        f"oracle artifact {name}",
        expected_bytes=receipt["bytes"],
        expected_sha256=receipt["sha256"],
    )
    print(f"verified file: {receipt['name']}")
    return pinned


def reverify_file(pinned: PinnedFile, expected_sha256: str) -> None:
    try:
        descriptor_state = os.fstat(pinned.descriptor)
        named_state = os.stat(pinned.path, follow_symlinks=False)
    except OSError as error:
        raise Refusal(
            f"cannot reinspect pinned oracle artifact {pinned.path}: {error}"
        ) from error
    if not same_file_state(pinned.state, descriptor_state):
        raise Refusal(f"pinned oracle artifact changed during verification: {pinned.path}")
    if not same_file_state(pinned.state, named_state):
        raise Refusal(f"pinned oracle artifact path changed during verification: {pinned.path}")
    same(
        f"{pinned.path.name} final SHA-256",
        sha256_descriptor(pinned.descriptor),
        expected_sha256,
    )


def load_canonical_contract(path: Path) -> tuple[dict[str, Any], PinnedFile]:
    pinned = pin_regular_file(
        path,
        "oracle contract",
        expected_sha256=CANONICAL_CONTRACT_SHA256,
    )
    try:
        contract = load_json_bytes(read_descriptor(pinned.descriptor), pinned.path)
    except Exception:
        pinned.close()
        raise
    return contract, pinned


def materialize_pinned(
    pinned: PinnedFile, destination: Path, expected_sha256: str
) -> PinnedFile:
    descriptor = -1
    digest = hashlib.sha256()
    try:
        descriptor = os.open(
            destination,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            0o400,
        )
        offset = 0
        while block := os.pread(pinned.descriptor, 1024 * 1024, offset):
            digest.update(block)
            offset += len(block)
            written_offset = 0
            while written_offset < len(block):
                written = os.write(descriptor, block[written_offset:])
                if written <= 0:
                    raise Refusal(f"short write while materializing {destination}")
                written_offset += written
        os.fsync(descriptor)
    except OSError as error:
        raise Refusal(f"cannot materialize authenticated input {destination}: {error}") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    same(f"materialized {destination.name} SHA-256", digest.hexdigest(), expected_sha256)
    return pin_regular_file(
        destination,
        f"private materialized input {destination.name}",
        expected_sha256=expected_sha256,
    )


def command(
    args: list[str],
    *,
    pass_fds: tuple[int, ...] = (),
    env: dict[str, str] | None = None,
) -> bytes:
    try:
        process = subprocess.run(
            args,
            check=False,
            capture_output=True,
            pass_fds=pass_fds,
            env=env,
        )
    except OSError as error:
        raise Refusal(f"cannot run {args[0]}: {error}") from error
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise Refusal(f"{' '.join(args)} failed ({process.returncode}): {detail}")
    return process.stdout


def ffprobe(pinned: PinnedFile) -> dict[str, Any]:
    descriptor_path = f"/proc/self/fd/{pinned.descriptor}"
    raw = command(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            descriptor_path,
        ],
        pass_fds=(pinned.descriptor,),
    )
    try:
        return json.loads(raw)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise Refusal(f"ffprobe returned invalid JSON for {pinned.path}: {error}") from error


def one_stream(probe: dict[str, Any], kind: str) -> dict[str, Any]:
    streams = [stream for stream in probe["streams"] if stream.get("codec_type") == kind]
    same(f"{kind} stream count", len(streams), 1)
    return streams[0]


def verify_fields(what: str, actual: dict[str, Any], expected: dict[str, Any]) -> None:
    for key, value in expected.items():
        same(f"{what} {key}", actual.get(key), value)


def side_data(stream: dict[str, Any], kind: str) -> dict[str, Any]:
    matches = [
        entry
        for entry in stream.get("side_data_list", [])
        if entry.get("side_data_type") == kind
    ]
    same(f"{kind} side-data count", len(matches), 1)
    return matches[0]


def verify_media(
    pinned: PinnedFile,
    contract: dict[str, Any],
    grid: dict[str, Any],
) -> str:
    probe = ffprobe(pinned)
    same("total stream count", len(probe["streams"]), contract["stream_count"])
    video = one_stream(probe, "video")
    audio = one_stream(probe, "audio")
    verify_fields("video", video, contract["video"])
    verify_fields("audio", audio, contract["audio"])

    expected_format = contract["format"]
    actual_format = probe["format"]
    for key in ("format_name", "start_time", "duration"):
        same(f"format {key}", actual_format.get(key), expected_format[key])
    tags = actual_format.get("tags", {})
    same("format major_brand", tags.get("major_brand"), expected_format["major_brand"])
    same("format encoder", tags.get("encoder"), expected_format["encoder"])

    verify_fields(
        "Spherical Mapping",
        side_data(video, "Spherical Mapping"),
        contract["spherical_mapping"],
    )
    verify_fields("Stereo 3D", side_data(video, "Stereo 3D"), contract["stereo_3d"])

    packet_csv = command(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_packets",
            "-show_entries",
            "packet=pts,dts,duration",
            "-of",
            "csv=p=0",
            f"/proc/self/fd/{pinned.descriptor}",
        ],
        pass_fds=(pinned.descriptor,),
    )
    rows = packet_csv.decode("ascii").splitlines()
    same("video packet count", len(rows), grid["count"])
    same(
        "packet-grid coordinate count",
        grid["last_index"] - grid["first_index"] + 1,
        grid["count"],
    )
    canonical_rows = []
    for ordinal, row in enumerate(rows):
        try:
            actual = tuple(int(value) for value in row.split(","))
        except ValueError as error:
            raise Refusal(f"video packet {ordinal} is not an integer triple: {row!r}") from error
        index = grid["first_index"] + ordinal
        expected_pts = index * grid["step"]
        expected = (expected_pts, expected_pts, grid["duration"])
        same(f"video packet {ordinal}", actual, expected)
        canonical_rows.append(f"{expected_pts},{expected_pts},{grid['duration']}\n")
    same(
        "last video packet PTS",
        grid["last_index"] * grid["step"],
        grid["last_pts"],
    )
    digest = hashlib.sha256("".join(canonical_rows).encode("ascii")).hexdigest()
    same("canonical video packet grid SHA-256", digest, grid["canonical_csv_sha256"])
    print(f"verified media and packet grid: {pinned.path.name}")
    return digest


def project_payload(
    pinned: PinnedFile, enabled: bool, contract: dict[str, Any]
) -> dict[str, Any]:
    path = pinned.path
    document = load_json_bytes(read_descriptor(pinned.descriptor), path)
    same(f"{path.name} project count", len(document.get("projects", [])), 1)
    project = document["projects"][0]
    clip = project["clip"]
    endpoint = contract["inclusive_endpoint"]
    for key in ("sourceFrames", "source_total_frames"):
        same(f"{path.name} clip {key}", clip.get(key), endpoint)
    same(f"{path.name} startFrame", clip.get("startFrame"), 0)
    same(f"{path.name} leftTrim", clip.get("leftTrim"), 0)
    same(f"{path.name} rightTrim", clip.get("rightTrim"), 0)
    same(f"{path.name} source_left_trim", clip.get("source_left_trim"), 0)
    same(f"{path.name} source_right_trim", clip.get("source_right_trim"), 0)
    same(f"{path.name} roughTrimmed", clip.get("roughTrimmed"), False)
    fps = clip.get("fps")
    if not isinstance(fps, (int, float)) or not math.isclose(
        fps, contract["fps"], rel_tol=0.0, abs_tol=1e-13
    ):
        raise Refusal(
            f"{path.name} clip fps changed: expected {contract['fps']!r}, got {fps!r}"
        )
    roughcut = project.get("roughcut")
    same(
        f"{path.name} roughcut",
        roughcut,
        {"leftTrim": 0, "rightTrim": 0, "totalFrames": endpoint},
    )
    flow = clip["stitching_optimization"].get("enableOpticalFlowStitching")
    same(f"{path.name} optical-flow state", flow, enabled)
    return document


def verify_projects(
    flow_on_path: Path,
    flow_off_path: Path,
    flow_on_enabled: bool,
    flow_off_enabled: bool,
    contract: dict[str, Any],
) -> None:
    same("Flow On artifact declaration", flow_on_enabled, True)
    same("Flow Off artifact declaration", flow_off_enabled, False)
    flow_on = project_payload(flow_on_path, flow_on_enabled, contract)
    flow_off = project_payload(flow_off_path, flow_off_enabled, contract)
    normalized_off = copy.deepcopy(flow_off)
    normalized_off["projects"][0]["clip"]["stitching_optimization"][
        "enableOpticalFlowStitching"
    ] = True
    normalized_off["projects"][0]["modifiedTime"] = flow_on["projects"][0]["modifiedTime"]
    same("Flow On/Off project data outside the two allowed fields", normalized_off, flow_on)
    same(
        "declared project difference allowlist",
        contract["allowed_flow_on_off_differences"],
        [
            "$.projects[0].clip.stitching_optimization.enableOpticalFlowStitching",
            "$.projects[0].modifiedTime",
        ],
    )
    print("verified projects: only optical-flow state and modifiedTime differ")


def parse() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Verify the ordinary Studio 6.0.2 native-rate 360 video oracle.",
        allow_abbrev=False,
    )
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--artifact-dir", type=Path, default=DEFAULT_ARTIFACTS)
    parser.add_argument(
        "--source-dir",
        type=Path,
        help="also verify the two original .insv source halves",
    )
    parser.add_argument(
        "--picture-dir",
        type=Path,
        help="rerun the target-picture receipt from prederived hash-checked images",
    )
    parser.add_argument(
        "--regenerate-pictures",
        action="store_true",
        help=(
            "build the authenticated reframe receipt and regenerate target pictures; "
            "requires --source-dir"
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse()
    pins: list[PinnedFile] = []
    source_pins: list[PinnedFile] = []
    try:
        contract, contract_pin = load_canonical_contract(args.contract)
        pins.append(contract_pin)
        same("contract schema version", contract.get("schema_version"), 1)
        same(
            "producer declaration",
            contract.get("producer"),
            {
                "application": "Insta360 Studio",
                "version": "6.0.2",
                "export_route": (
                    "ordinary UI, 360 Video, Match Source 5760x2880, "
                    "Match Source Frame Rate 30000/1001, H.265"
                ),
            },
        )
        artifacts = contract["artifacts"]
        paths: dict[str, dict[str, PinnedFile]] = {}
        for state in ("flow_on", "flow_off"):
            paths[state] = {}
            for kind in ("video", "project"):
                pinned = pin_receipt_file(args.artifact_dir, artifacts[state][kind])
                paths[state][kind] = pinned
                pins.append(pinned)

        if args.source_dir is not None:
            for receipt in contract["source"]["files"]:
                pinned = pin_receipt_file(args.source_dir, receipt)
                source_pins.append(pinned)
                pins.append(pinned)

        grids = [
            verify_media(
                paths[state]["video"],
                contract["media_contract"],
                contract["packet_grid"],
            )
            for state in ("flow_on", "flow_off")
        ]
        same("Flow On/Off packet grids", grids[0], grids[1])
        verify_projects(
            paths["flow_on"]["project"],
            paths["flow_off"]["project"],
            artifacts["flow_on"]["optical_flow"],
            artifacts["flow_off"]["optical_flow"],
            contract["project_contract"],
        )

        target = contract["source"]["authenticated_target"]
        grid = contract["packet_grid"]
        if not grid["first_index"] <= target["frame_index"] <= grid["last_index"]:
            raise Refusal(
                f"owner frame {target['frame_index']} lies outside "
                f"{grid['first_index']}..{grid['last_index']}"
            )
        same(
            "owner target time base",
            target["time_base"],
            contract["media_contract"]["video"]["time_base"],
        )
        same(
            "target-picture receipt frame",
            contract["target_picture_authentication"]["studio_frame_index"],
            target["frame_index"],
        )
        same(
            "target-picture receipt winner",
            contract["target_picture_authentication"]["unique_source_winner"],
            target["frame_index"],
        )
        expected_pts = target["frame_index"] * grid["step"]
        same("owner output-frame PTS", expected_pts, target["pts"])
        seconds = f"{float(target['pts'] * Fraction(target['time_base'])):.6f}"
        same("owner output-frame seconds", seconds, target["seconds"])
        print(
            "verified owner grid point: "
            f"frame {target['frame_index']} PTS {target['pts']} * "
            f"{target['time_base']} = {seconds} seconds"
        )
        if args.picture_dir is not None and args.regenerate_pictures:
            raise Refusal("--picture-dir and --regenerate-pictures are mutually exclusive")
        if args.regenerate_pictures and args.source_dir is None:
            raise Refusal("--regenerate-pictures requires --source-dir")
        if args.picture_dir is not None or args.regenerate_pictures:
            helper_pin = pin_regular_file(
                TARGET_PICTURE_HELPER,
                "target-picture verifier",
                expected_sha256=CANONICAL_TARGET_PICTURE_HELPER_SHA256,
            )
            pins.append(helper_pin)
            with tempfile.TemporaryDirectory(
                prefix="kjerag-target-helper-"
            ) as temporary:
                private = Path(temporary)
                helper_copy = materialize_pinned(
                    helper_pin,
                    private / TARGET_PICTURE_HELPER.name,
                    CANONICAL_TARGET_PICTURE_HELPER_SHA256,
                )
                contract_copy = materialize_pinned(
                    contract_pin,
                    private / DEFAULT_CONTRACT.name,
                    CANONICAL_CONTRACT_SHA256,
                )
                try:
                    picture_args = [
                        sys.executable,
                        f"/proc/self/fd/{helper_copy.descriptor}",
                        "--contract",
                        os.fspath(contract_copy.path),
                    ]
                    if args.regenerate_pictures:
                        picture_args.extend(
                            [
                                "--artifact-dir",
                                os.fspath(args.artifact_dir),
                                "--source-dir",
                                os.fspath(args.source_dir),
                            ]
                        )
                    else:
                        picture_args.extend(
                            ["--picture-dir", os.fspath(args.picture_dir)]
                        )
                    environment = os.environ.copy()
                    environment["KJERAG_TARGET_HELPER_ROOT"] = os.fspath(ROOT)
                    output = command(
                        picture_args,
                        pass_fds=(helper_copy.descriptor,),
                        env=environment,
                    )
                    reverify_file(
                        helper_copy, CANONICAL_TARGET_PICTURE_HELPER_SHA256
                    )
                    reverify_file(contract_copy, CANONICAL_CONTRACT_SHA256)
                finally:
                    contract_copy.close()
                    helper_copy.close()
            reverify_file(helper_pin, CANONICAL_TARGET_PICTURE_HELPER_SHA256)
            print(output.decode("utf-8").rstrip())
            picture_boundary = (
                "target pictures regenerated and decoys rerun"
                if args.regenerate_pictures
                else "prederived target-picture decoys rerun"
            )
        else:
            picture_boundary = "target-picture decoys not rerun (pass --picture-dir)"
        for state in ("flow_on", "flow_off"):
            for kind in ("video", "project"):
                reverify_file(paths[state][kind], artifacts[state][kind]["sha256"])
        if args.source_dir is not None:
            for pinned, receipt in zip(
                source_pins, contract["source"]["files"], strict=True
            ):
                reverify_file(pinned, receipt["sha256"])
        reverify_file(contract_pin, CANONICAL_CONTRACT_SHA256)
        print(
            "claim boundary: exact artifacts, settings, media metadata and output sample grid; "
            f"{picture_boundary}; no Studio parent-clock semantics, unfitted owner-view "
            "registration, seam parity or Kjerag fix"
        )
        return 0
    except (KeyError, TypeError, ValueError, Refusal) as error:
        print(f"REFUSED: {error}", file=sys.stderr)
        return 1
    finally:
        for pinned in reversed(pins):
            pinned.close()


if __name__ == "__main__":
    raise SystemExit(main())
