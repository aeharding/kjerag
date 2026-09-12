#!/usr/bin/env python3
"""Build the sealed 6339..6399 owner three-panel review artifact."""

from __future__ import annotations

import argparse
from contextlib import ExitStack
import ctypes
import errno
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
from typing import Any
import re
import uuid


ROOT = Path(__file__).resolve().parent.parent
KJERAG_SCHEMA = "kjerag.playback-consecutive-range.v2"
KJERAG_CLAIM = "exact consecutive displayed production frames from one causal frame-zero run"
STUDIO_SCHEMA = "kjerag.studio-projected-interval.v1"
TRACE_SCHEMA = "kjerag.playback-output-seam-trace.v2"
TRACE_CLAIM = "actual selected-alpha 0.5 four-neighbour crossings over authenticated Kjerag output PNGs"
OUTPUT_SCHEMA = "kjerag.owner-three-panel-review.v2"
OUTPUT_RECEIPT = "three-panel-review-receipt.json"
NATIVE = "riser-three-panel-native.mp4"
QUARTER = "riser-three-panel-quarter-speed.mp4"
START, END, COUNT = 6339, 6399, 61
RATE = Fraction(30_000, 1_001)
TIME_BASE = Fraction(1, 30_000)
PTS_STEP = 1_001
NANOS = 1_000_000_000
WIDTH, HEIGHT = 3840, 2160
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
FONT_SHA256 = "89c3c497f618fdaa0b2d1e98fef93582f28c71debd2c4a8cdf41f190ced2909d"
OWNER_VIEW = {
    "yaw_radians": 1.241452693939209,
    "pitch_radians": -0.24417155981063843,
    "fov_radians": 1.011418342590332,
    "yaw_degrees": 71.12999725341797,
    "pitch_degrees": -13.989999771118164,
    "fov_degrees": 57.95000457763672,
    "horizon_locked": True,
    "readout": "file",
    "sampling": "Sharp",
    "seam_band": True,
    "exposure_tone": True,
    "render_format": "rgba8unorm",
    "render_width": 2560,
    "render_height": 1440,
    "capture_width": WIDTH,
    "capture_height": HEIGHT,
}
OWNER_SOURCES = (
    {"basename": "VID_20251018_191318_00_002.insv", "bytes": 1_730_601_828,
     "sha256": "d690e7889345d8d8441fbc4fdb9f4ad3cf170003c01811b91d297ae6fe6545ef",
     "decoder_lane": 0, "picked": True},
    {"basename": "VID_20251018_191318_10_002.insv", "bytes": 1_719_664_640,
     "sha256": "fd9bb5865576a76a5c8e52a7536e7acdcefd90b2224f5065488f5d0001f09ca6",
     "decoder_lane": 1, "picked": False},
)
CANONICAL_PROJECTOR_SHA256 = "4833b3520b60ba5f2a4e4ddd1d719fe044a61b2d44d3a6944e0a747d57a01243"
CANONICAL_VERIFIER_SHA256 = "67452d3e3acea5421dacd03745a1cd567a8be03c92aa7c62a66b2cc163ab13f3"
CANONICAL_CONTRACT_SHA256 = "eda588746c1d15347bae2064b3de2175eb1f1222d159a53c362f900d409e2374"
CANONICAL_AUTH = {
    "contract": {
        "name": "studio-video-oracle-602.json",
        "schema_version": 1,
        "sha256": "eda588746c1d15347bae2064b3de2175eb1f1222d159a53c362f900d409e2374",
    },
    "producer": {
        "application": "Insta360 Studio",
        "export_route": "ordinary UI, 360 Video, Match Source 5760x2880, Match Source Frame Rate 30000/1001, H.265",
        "version": "6.0.2",
    },
    "verifier": {
        "name": "verify-studio-video-oracle.py",
        "sha256": "fa1fe64deaf4b4ff77640527f0d72265e1de39e5d0728be855fd6a600241c12f",
    },
}
CANONICAL_DERIVED_AUTH = json.loads(json.dumps(CANONICAL_AUTH))
CANONICAL_DERIVED_AUTH["verifier"]["sha256"] = CANONICAL_VERIFIER_SHA256
CANONICAL_STUDIO_SOURCE = {
    "flow": "on",
    "optical_flow": True,
    "packet_grid": {
        "canonical_csv_sha256": "7e306d3b12f468ac52501b6460811ffb835622dbd7dd9e60aa1e1367359e0d65",
        "count": 8202,
        "duration": 1001,
        "first_index": 0,
        "last_index": 8201,
        "last_pts": 8209201,
        "step": 1001,
    },
    "project": {
        "bytes": 12830,
        "name": "flow-on-footage_project.insprj",
        "sha256": "d6878b13b53fadf82e644ffbf36be4baeff44fe193590bab237e44b3b83a4d5f",
    },
    "video": {
        "bytes": 3431316800,
        "name": "studio602-flow-on-360-native-source-rate.mp4",
        "sha256": "ab7b6ec972767a4fa53d71cd6ff069dab885057cbd70f55c4c123f4ef44f633d",
    },
}
FOOTER = "Not spatially aligned. Compare riser continuity inside each panel, not screen position."
RENAME_NOREPLACE = 1


class Refusal(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Refusal(message)


def obj(value: Any, name: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{name} must be an object")
    return value


def array(value: Any, name: str) -> list[Any]:
    require(isinstance(value, list), f"{name} must be an array")
    return value


def integer(value: Any, name: str) -> int:
    require(isinstance(value, int) and not isinstance(value, bool), f"{name} must be an integer")
    return value


def lower_sha(value: Any, name: str) -> str:
    require(isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value),
            f"{name} must be a lowercase SHA-256")
    return value


def lower_hex(value: Any, length: int, name: str) -> str:
    require(isinstance(value, str) and len(value) == length and
            all(c in "0123456789abcdef" for c in value), f"{name} must be {length} lowercase hex characters")
    return value


def sha256_bytes(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def sha256_path(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(8 * 1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def read_fd(descriptor: int, size: int) -> bytes:
    blocks, offset = [], 0
    while offset < size:
        block = os.pread(descriptor, min(1024 * 1024, size - offset), offset)
        require(bool(block), "pinned input ended before its retained size")
        blocks.append(block)
        offset += len(block)
    return b"".join(blocks)


class PinnedFile:
    def __init__(self, path: Path, expected: str | None, label: str):
        require(path.is_absolute(), f"{label} path must be absolute")
        self.fd = -1
        self.path, self.label = path, label
        try:
            self.fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
            self.state = os.fstat(self.fd)
            require(stat.S_ISREG(self.state.st_mode), f"{label} is not a regular file")
            self.raw = read_fd(self.fd, self.state.st_size)
            self.sha256 = sha256_bytes(self.raw)
            if expected is not None:
                require(self.sha256 == lower_sha(expected, f"{label} expected SHA-256"),
                        f"{label} SHA-256 changed: expected {expected}, got {self.sha256}")
        except (OSError, Refusal) as error:
            self.close()
            raise Refusal(f"cannot pin {label} {path}: {error}") from error

    def materialize(self, destination: Path, mode: int = 0o500) -> dict[str, Any]:
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor = -1
        try:
            descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), mode)
            offset = 0
            while offset < len(self.raw):
                written = os.write(descriptor, self.raw[offset:])
                require(written > 0, f"short write while materializing {self.label}")
                offset += written
            os.fsync(descriptor)
        finally:
            if descriptor >= 0:
                os.close(descriptor)
        return self.identity()

    def verify(self) -> None:
        require(self.fd >= 0, f"{self.label} descriptor is closed")
        try:
            current = os.stat(self.path, follow_symlinks=False)
        except OSError as error:
            raise Refusal(f"{self.label} path cannot be reverified: {error}") from error
        require(stat.S_ISREG(current.st_mode), f"{self.label} path is no longer a regular file")
        require((current.st_dev, current.st_ino, current.st_size) ==
                (self.state.st_dev, self.state.st_ino, self.state.st_size),
                f"{self.label} path changed during the build")
        require(sha256_bytes(read_fd(self.fd, self.state.st_size)) == self.sha256,
                f"{self.label} bytes changed during the build")

    def identity(self) -> dict[str, Any]:
        return {"path": os.fspath(self.path), "bytes": len(self.raw), "sha256": self.sha256,
                "device": self.state.st_dev, "inode": self.state.st_ino}

    def close(self) -> None:
        if getattr(self, "fd", -1) >= 0:
            os.close(self.fd)
            self.fd = -1

    def __enter__(self) -> "PinnedFile":
        return self

    def __exit__(self, *_unused: Any) -> None:
        self.close()


def validate_build(build: dict[str, Any], expected_commit: str, expected_tree: str,
                   expected_executable_sha256: str, label: str) -> None:
    expected_commit = lower_hex(expected_commit, 40, "expected Git commit")
    expected_tree = lower_hex(expected_tree, 40, "expected Git tree")
    expected_executable_sha256 = lower_sha(expected_executable_sha256, f"expected {label} SHA-256")
    require(build.get("embedded_git_commit") == expected_commit and
            build.get("runtime_git_commit") == expected_commit and
            build.get("embedded_git_tree") == expected_tree and
            build.get("runtime_git_tree") == expected_tree,
            f"{label} build commit/tree differs from the externally expected shipping build")
    require(build.get("embedded_git_dirty") == "false" and build.get("dirty_at_build") is False and
            build.get("runtime_tracked_tree_clean") is True,
            f"{label} does not bind a fully clean production build")
    executable = obj(build.get("executable"), f"{label} executable")
    require(executable.get("sha256") == expected_executable_sha256,
            f"{label} receipt executable differs from the externally expected binary")
    integer(executable.get("bytes"), f"{label} executable bytes")
    stable = obj(executable.get("stable_identity"), f"{label} executable stable identity")
    for key in ("device", "inode", "mode", "links", "uid", "mtime_seconds", "mtime_nanoseconds",
                "ctime_seconds", "ctime_nanoseconds"):
        integer(stable.get(key), f"{label} executable stable_identity.{key}")


def copy_canonical(path: Path, expected_sha: str, destination: Path, label: str) -> dict[str, Any]:
    pinned = PinnedFile(path, expected_sha, label)
    try:
        identity = pinned.materialize(destination, 0o400)
        pinned.verify()
        return identity
    finally:
        pinned.close()


class PinnedReceipt(PinnedFile):
    def __init__(self, path: Path, expected: str, label: str):
        super().__init__(path, expected, label)
        try:
            self.value = obj(json.loads(self.raw.decode("utf-8")), label)
        except (UnicodeError, json.JSONDecodeError, Refusal) as error:
            self.close()
            raise Refusal(f"cannot parse {label}: {error}") from error


def leaf_name(value: Any, label: str) -> str:
    require(isinstance(value, str) and value not in ("", ".", "..") and Path(value).name == value,
            f"{label} must be a basename")
    return value


def copy_leaf(directory: Path, declared: dict[str, Any], destination: Path, label: str) -> dict[str, Any]:
    name = leaf_name(declared.get("file", declared.get("png")), f"{label} filename")
    expected_bytes = integer(declared.get("bytes"), f"{label}.bytes")
    expected_sha = lower_sha(declared.get("sha256"), f"{label}.sha256")
    source_path = directory / name
    fd = -1
    try:
        fd = os.open(source_path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        state = os.fstat(fd)
        require(stat.S_ISREG(state.st_mode), f"{label} is not a regular file")
        require(state.st_size == expected_bytes, f"{label} size changed")
        digest = hashlib.sha256()
        with destination.open("xb") as output:
            offset = 0
            while block := os.pread(fd, 8 * 1024 * 1024, offset):
                digest.update(block)
                output.write(block)
                offset += len(block)
            output.flush()
            os.fsync(output.fileno())
    except (OSError, Refusal) as error:
        raise Refusal(f"cannot authenticate {label} {source_path}: {error}") from error
    finally:
        if fd >= 0:
            os.close(fd)
    actual = digest.hexdigest()
    require(actual == expected_sha, f"{label} SHA-256 changed: expected {expected_sha}, got {actual}")
    return {"file": name, "bytes": expected_bytes, "sha256": actual}


def require_png(path: Path, label: str) -> None:
    with path.open("rb") as image:
        header = image.read(24)
    require(len(header) == 24 and header[:8] == PNG_SIGNATURE and header[12:16] == b"IHDR",
            f"{label} is not a PNG with an IHDR header")
    require(struct.unpack(">II", header[16:24]) == (WIDTH, HEIGHT), f"{label} is not 3840x2160")


def exact_interval(start: int, count: int, end: int, label: str) -> None:
    require((start, count, end) == (START, COUNT, END),
            f"{label} must be the exact {START}..{END} interval")


def validate_kjerag(receipt: PinnedReceipt, stage: Path, expected_commit: str,
                    expected_tree: str, expected_playback_sha256: str) -> dict[str, Any]:
    value = receipt.value
    require(value.get("schema") == KJERAG_SCHEMA and value.get("claim") == KJERAG_CLAIM,
            "Kjerag receipt schema or claim changed")
    request = obj(value.get("request"), "Kjerag request")
    exact_interval(integer(request.get("start"), "start"), integer(request.get("count"), "count"),
                   integer(request.get("end_inclusive"), "end"), "Kjerag receipt")
    for key, expected in (("no_seek", True), ("cold_start_at_range_boundary", False),
                          ("every_source_frame_consumed", True), ("captured_map_substitution", False)):
        require(request.get(key) is expected, f"Kjerag request.{key} changed")
    view = obj(value.get("view"), "Kjerag view")
    require(view == OWNER_VIEW, "Kjerag receipt does not bind the exact fixed owner view and settings")
    run = obj(value.get("run"), "Kjerag run")
    require(run.get("presented") == END + 1 and run.get("dropped") == 0 and run.get("starved") == 0,
            "Kjerag receipt does not prove uninterrupted frame-zero causal consumption")
    require(run.get("gpu_pis_transactions") == run.get("presented") and
            run.get("cpu_pis_transactions") == 0,
            "Kjerag receipt does not prove every committed transaction used GPU PIS with no CPU fallback")
    source, build = array(value.get("source"), "Kjerag source"), obj(value.get("build"), "Kjerag build")
    require(len(source) == 2, "Kjerag receipt must bind two source files")
    for item, expected in zip(source, OWNER_SOURCES, strict=True):
        record = obj(item, "Kjerag source record")
        require(Path(record.get("path", "")).name == expected["basename"] and
                all(record.get(key) == expected[key] for key in ("bytes", "sha256", "decoder_lane", "picked")),
                f"Kjerag decoder lane {expected['decoder_lane']} is not the canonical owner source")
        stable = obj(record.get("stable_identity"), "Kjerag source stable identity")
        for key in ("device", "inode", "mode", "links", "uid", "mtime_seconds", "mtime_nanoseconds",
                    "ctime_seconds", "ctime_nanoseconds"):
            integer(stable.get(key), f"Kjerag source stable_identity.{key}")
    require(source[0]["stable_identity"]["device"] != source[1]["stable_identity"]["device"] or
            source[0]["stable_identity"]["inode"] != source[1]["stable_identity"]["inode"],
            "Kjerag canonical sources resolve to one historical file identity")
    validate_build(build, expected_commit, expected_tree, expected_playback_sha256, "Kjerag playback")
    frames = array(value.get("frames"), "Kjerag frames")
    require(len(frames) == COUNT, "Kjerag receipt does not inventory 61 frames")
    out = []
    for ordinal, raw in enumerate(frames):
        frame = obj(raw, "Kjerag frame")
        index = START + ordinal
        require(frame.get("index") == index, f"Kjerag frame ordinal {ordinal} is not index {index}")
        image = obj(frame.get("image"), f"Kjerag frame {index} image")
        require((image.get("width"), image.get("height")) == (WIDTH, HEIGHT), f"Kjerag frame {index} dimensions changed")
        destination = stage / f"frame-{index:010d}.png"
        authenticated = copy_leaf(receipt.path.parent, image, destination, f"Kjerag frame {index}")
        require_png(destination, f"Kjerag frame {index}")
        require(authenticated["file"] == destination.name, f"Kjerag frame {index} name changed")
        production = obj(frame.get("production_map"), f"Kjerag frame {index} production map")
        require(production.get("pis_backend") == "gpu",
                f"Kjerag frame {index} does not authenticate a GPU PIS committed map")
        packed_path = stage / f"frame-{index:010d}.packed-f32le.bin"
        alpha_path = stage / f"frame-{index:010d}.alpha-f32le.bin"
        packed = copy_leaf(receipt.path.parent, obj(production.get("packed"), "packed map"), packed_path,
                           f"Kjerag frame {index} packed map")
        alpha = copy_leaf(receipt.path.parent, obj(production.get("alpha"), "alpha map"), alpha_path,
                          f"Kjerag frame {index} alpha map")
        require(packed["file"] == packed_path.name and packed["bytes"] == 320_000,
                f"Kjerag frame {index} packed map binding changed")
        require(alpha["file"] == alpha_path.name and alpha["bytes"] == 80_000,
                f"Kjerag frame {index} alpha map binding changed")
        out.append({"index": index, "timestamp_seconds": frame.get("timestamp_seconds"),
                    "timestamp_nanoseconds": frame.get("timestamp_nanoseconds"), **authenticated,
                    "packed": packed, "alpha": alpha})
    return {"request": request, "view": view, "source": source, "build": build, "run": run, "frames": out}


def validate_studio(receipt: PinnedReceipt, stage: Path, kjerag: dict[str, Any],
                    expected_authentication: dict[str, Any] | None = None) -> dict[str, Any]:
    value = receipt.value
    require(value.get("schema") == STUDIO_SCHEMA, "Studio projected interval schema changed")
    authentication = value.get("authentication")
    if expected_authentication is None:
        require(authentication in (CANONICAL_AUTH, CANONICAL_DERIVED_AUTH),
                "Studio projected interval does not carry a recognized canonical 6.0.2 oracle authentication")
    else:
        require(authentication == expected_authentication,
                "Studio projected interval does not carry the current canonical 6.0.2 oracle authentication")
    require(value.get("source") == CANONICAL_STUDIO_SOURCE,
            "Studio projected interval does not bind the canonical Flow-On oracle source")
    projection = obj(value.get("projection"), "Studio projection")
    require(projection.get("input") == "equirect" and projection.get("output") == "flat",
            "Studio interval is not the canonical flat projection")
    require((projection.get("width"), projection.get("height")) == (WIDTH, HEIGHT), "Studio projection is not 3840x2160")
    view = kjerag["view"]
    require(projection.get("yaw") == view.get("yaw_degrees") and
            projection.get("pitch") == view.get("pitch_degrees") and
            projection.get("horizontal_fov") == view.get("fov_degrees"),
            "Studio projection does not carry the exact Kjerag view binding")
    interval = obj(value.get("interval"), "Studio interval")
    exact_interval(integer(interval.get("start_frame"), "Studio start"), integer(interval.get("count"), "Studio count"),
                   integer(interval.get("end_frame"), "Studio end"), "Studio receipt")
    frames = array(interval.get("frames"), "Studio frames")
    require(len(frames) == COUNT, "Studio receipt does not inventory 61 frames")
    out = []
    for ordinal, raw in enumerate(frames):
        frame = obj(raw, "Studio frame")
        index = START + ordinal
        require(frame.get("index") == index and frame.get("pts") == index * PTS_STEP and
                Fraction(frame.get("time_base")) == TIME_BASE, f"Studio frame {index} grid binding changed")
        kframe = kjerag["frames"][ordinal]
        nanos = integer(kframe["timestamp_seconds"], "Kjerag timestamp seconds") * NANOS + integer(kframe["timestamp_nanoseconds"], "Kjerag timestamp nanos")
        require(nanos == index * PTS_STEP * NANOS // 30_000, f"frame {index} Kjerag and Studio timestamps differ")
        declared = {"file": frame.get("png"), "bytes": frame.get("bytes"), "sha256": frame.get("sha256")}
        destination = stage / f"frame-{index:08d}.png"
        authenticated = copy_leaf(receipt.path.parent, declared, destination, f"Studio frame {index}")
        require_png(destination, f"Studio frame {index}")
        require(authenticated["file"] == destination.name, f"Studio frame {index} name changed")
        out.append({"index": index, "pts": frame.get("pts"), "time_base": frame.get("time_base"), **authenticated})
    extraction = obj(value.get("extraction"), "Studio extraction")
    require(extraction.get("temporal_interpolation") is False, "Studio projection used temporal interpolation")
    return {"authentication": authentication, "source": CANONICAL_STUDIO_SOURCE,
            "projection": projection, "extraction": extraction, "frames": out}


def derive_studio(oracle_dir: Path, private_root: Path, kjerag: dict[str, Any],
                  private_bin: Path, clean_env: dict[str, str]) -> tuple[PinnedReceipt, dict[str, Any]]:
    require(oracle_dir.is_absolute() and oracle_dir.is_dir() and not oracle_dir.is_symlink(),
            "oracle-dir must be an absolute non-symlink directory")
    chain = {
        "projector": copy_canonical(ROOT / "scripts/project-studio-video-interval.py", CANONICAL_PROJECTOR_SHA256,
                                    private_root / "scripts/project-studio-video-interval.py", "canonical Studio projector"),
        "verifier": copy_canonical(ROOT / "scripts/verify-studio-video-oracle.py", CANONICAL_VERIFIER_SHA256,
                                   private_root / "scripts/verify-studio-video-oracle.py", "canonical Studio verifier"),
        "contract": copy_canonical(ROOT / "docs/research/studio-video-oracle-602.json", CANONICAL_CONTRACT_SHA256,
                                   private_root / "docs/research/studio-video-oracle-602.json", "canonical Studio contract"),
    }
    output = private_root / "derived"
    view = kjerag["view"]
    command = [sys.executable, os.fspath(private_root / "scripts/project-studio-video-interval.py"),
               "--contract", os.fspath(private_root / "docs/research/studio-video-oracle-602.json"),
               "--oracle-dir", os.fspath(oracle_dir), "--flow", "on", "--start-frame", str(START),
               "--count", str(COUNT), "--yaw", repr(view["yaw_degrees"]),
               "--pitch", repr(view["pitch_degrees"]), "--horizontal-fov", repr(view["fov_degrees"]),
               "--width", str(WIDTH), "--height", str(HEIGHT), "--output-dir", os.fspath(output)]
    run(command, env={**clean_env, "PATH": os.fspath(private_bin)})
    derived_path = output / "projection-receipt.json"
    derived = PinnedReceipt(derived_path, sha256_path(derived_path), "privately derived Studio receipt")
    return derived, {"chain": chain, "command": command, "receipt": derived.identity()}


def compare_studio_derivation(supplied: dict[str, Any], derived: dict[str, Any]) -> None:
    require(supplied["projection"] == derived["projection"], "supplied Studio projection differs from canonical private derivation")
    require(len(supplied["frames"]) == len(derived["frames"]) == COUNT, "Studio derivation inventory length changed")
    for expected, actual in zip(derived["frames"], supplied["frames"], strict=True):
        require((actual["index"], actual["pts"], actual["time_base"], actual["bytes"], actual["sha256"]) ==
                (expected["index"], expected["pts"], expected["time_base"], expected["bytes"], expected["sha256"]),
                f"supplied Studio frame {actual['index']} differs from canonical private derivation")


def normalized_source(source: list[Any]) -> list[dict[str, Any]]:
    return [{k: v for k, v in obj(item, "source record").items() if k != "picked"} for item in source]


def validate_trace(receipt: PinnedReceipt, stage: Path, kjerag_receipt: PinnedReceipt,
                   kjerag: dict[str, Any], expected_commit: str, expected_tree: str,
                   expected_trace_sha256: str) -> dict[str, Any]:
    value = receipt.value
    require(value.get("schema") == TRACE_SCHEMA and value.get("claim") == TRACE_CLAIM,
            "range-trace receipt schema or claim changed")
    input_receipt = obj(value.get("input_receipt"), "trace input receipt")
    require(input_receipt.get("sha256") == kjerag_receipt.sha256 and
            input_receipt.get("schema") == KJERAG_SCHEMA and input_receipt.get("claim") == KJERAG_CLAIM,
            "range-trace receipt does not bind the supplied Kjerag causal receipt")
    require(value.get("request") == kjerag["request"], "range-trace request does not exactly bind the Kjerag request")
    trace_view = obj(value.get("view"), "trace view")
    require(trace_view.get("seam") == "factory", "range trace is not the factory-seam trace")
    for key in ("yaw_radians", "pitch_radians", "fov_radians", "yaw_degrees", "pitch_degrees",
                "fov_degrees", "horizon_locked", "readout", "sampling", "seam_band", "exposure_tone"):
        require(trace_view.get(key) == kjerag["view"].get(key), f"range-trace view binding differs at {key}")
    require((trace_view.get("width"), trace_view.get("height")) == (WIDTH, HEIGHT), "trace dimensions changed")
    require(normalized_source(array(value.get("source"), "trace source")) == normalized_source(kjerag["source"]),
            "range-trace source binding does not exactly match the Kjerag source")
    require(value.get("input_run") == kjerag["run"], "range-trace input-run binding differs from the Kjerag run")
    selected = array(value.get("selected_frames"), "selected trace frames")
    require(selected == list(range(START, END + 1)), "range trace does not select exact indices 6339..6399")
    frames = array(value.get("frames"), "trace frames")
    require(len(frames) == COUNT, "range-trace receipt does not inventory 61 frames")
    out = []
    for ordinal, raw in enumerate(frames):
        frame = obj(raw, "trace frame")
        index = START + ordinal
        kframe = kjerag["frames"][ordinal]
        require(frame.get("index") == index and frame.get("timestamp_seconds") == kframe["timestamp_seconds"] and
                frame.get("timestamp_nanoseconds") == kframe["timestamp_nanoseconds"],
                f"trace frame {index} index or timestamp binding changed")
        require(frame.get("base_png_sha256") == kframe["sha256"], f"trace frame {index} does not bind the Kjerag pixels")
        require(frame.get("packed_sha256") == kframe["packed"]["sha256"] and
                frame.get("alpha_sha256") == kframe["alpha"]["sha256"],
                f"trace frame {index} does not bind the Kjerag production map")
        destination = stage / f"frame-{index:010d}-trace.png"
        authenticated = copy_leaf(receipt.path.parent, obj(frame.get("trace"), "trace leaf"), destination, f"trace frame {index}")
        require_png(destination, f"trace frame {index}")
        require(authenticated["file"] == destination.name, f"trace frame {index} name changed")
        out.append({"index": index, "timestamp_seconds": frame.get("timestamp_seconds"),
                    "timestamp_nanoseconds": frame.get("timestamp_nanoseconds"), **authenticated})
    build = obj(value.get("build"), "trace build")
    validate_build(build, expected_commit, expected_tree, expected_trace_sha256, "range-trace")
    return {"input_receipt": input_receipt, "request": value.get("request"), "view": trace_view,
            "source": value.get("source"), "input_run": value.get("input_run"),
            "selected_frames": selected, "build": build, "frames": out}


def compare_trace_derivation(supplied: dict[str, Any], derived: dict[str, Any]) -> None:
    supplied_input, derived_input = dict(supplied["input_receipt"]), dict(derived["input_receipt"])
    supplied_input.pop("path", None); derived_input.pop("path", None)
    require(supplied_input == derived_input, "supplied range-trace input_receipt differs from private derivation")
    for key in ("request", "view", "source", "input_run", "selected_frames"):
        require(supplied[key] == derived[key], f"supplied range-trace {key} differs from private derivation")
    def normalized_build(value: dict[str, Any]) -> dict[str, Any]:
        result = json.loads(json.dumps(value))
        executable = obj(result.get("executable"), "trace executable")
        executable.pop("stable_identity", None)
        executable.pop("file", None)
        return result
    require(normalized_build(supplied["build"]) == normalized_build(derived["build"]),
            "supplied range-trace build differs from private derivation")
    require(len(supplied["frames"]) == len(derived["frames"]) == COUNT, "trace derivation inventory length changed")
    for expected, actual in zip(derived["frames"], supplied["frames"], strict=True):
        require(actual == expected, f"supplied trace frame {actual['index']} differs from private derivation")


def derive_trace(trace_binary: Path, private_root: Path, kjerag_receipt: PinnedReceipt,
                 kjerag: dict[str, Any], expected_commit: str, expected_tree: str,
                 expected_trace_sha256: str, clean_env: dict[str, str]) -> tuple[PinnedReceipt, dict[str, Any]]:
    private_root.mkdir(mode=0o700)
    output = private_root / "derived"
    command = [os.fspath(trace_binary), os.fspath(kjerag_receipt.path),
               f"--receipt-sha256={kjerag_receipt.sha256}", f"out-dir={output}", "frames=all"]
    run(command, env=clean_env)
    path = output / "range-trace-receipt.json"
    derived = PinnedReceipt(path, sha256_path(path), "privately derived range-trace receipt")
    return derived, {"command": command, "receipt": derived.identity(), "expected_commit": expected_commit,
                     "expected_tree": expected_tree, "expected_executable_sha256": expected_trace_sha256}


def run(arguments: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    try:
        result = subprocess.run(arguments, capture_output=True, check=False, env=env)
    except OSError as error:
        raise Refusal(f"cannot run {arguments[0]}: {error}") from error
    if result.returncode:
        raise Refusal(f"{' '.join(arguments)} failed ({result.returncode}): {result.stderr.decode(errors='replace').strip()}")
    return result


def executable_identity(path: Path) -> dict[str, Any]:
    return {"path": os.fspath(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}


def probe_video(ffprobe: str, path: Path, expected_frames: int, expected_duration: str,
                env: dict[str, str]) -> dict[str, Any]:
    command = [ffprobe, "-v", "error", "-select_streams", "v:0", "-show_entries",
               "stream=codec_name,profile,width,height,pix_fmt,r_frame_rate,time_base,nb_frames,duration",
               "-of", "json", os.fspath(path)]
    try:
        value = json.loads(run(command, env=env).stdout)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise Refusal(f"ffprobe returned invalid JSON for {path}: {error}") from error
    streams = array(value.get("streams"), f"ffprobe streams for {path.name}")
    require(len(streams) == 1, f"{path.name} does not contain exactly one selected video stream")
    stream = obj(streams[0], f"ffprobe stream for {path.name}")
    expected = {"codec_name": "h264", "profile": "High", "width": 3000, "height": 1144,
                "pix_fmt": "yuv420p", "r_frame_rate": "30000/1001", "time_base": "1/30000",
                "nb_frames": str(expected_frames), "duration": expected_duration}
    require(all(stream.get(key) == item for key, item in expected.items()),
            f"{path.name} stream contract changed: expected {expected}, got {stream}")
    return {"command": command, "stream": stream}


def clean_environment() -> dict[str, str]:
    allowed = ("HOME", "TMPDIR", "USER", "LOGNAME")
    result = {key: os.environ[key] for key in allowed if key in os.environ}
    result.update({"LC_ALL": "C", "LANG": "C", "PATH": "/nonexistent"})
    return result


def private_path_environment(clean_env: dict[str, str], private_bin: Path) -> dict[str, str]:
    require(private_bin.is_absolute(), "private executable directory must be absolute")
    return {**clean_env, "PATH": os.fspath(private_bin)}


def x264_core(output: bytes, label: str) -> str:
    matches = re.findall(rb"264 - core ([0-9]+ r[0-9]+ [0-9a-f]+)", output)
    require(matches, f"{label} did not report the loaded libx264 core identity")
    identities = {item.decode("ascii") for item in matches}
    require(len(identities) == 1, f"{label} reported conflicting libx264 identities")
    return identities.pop()


def package_identity(package: str, env: dict[str, str]) -> dict[str, str]:
    require(package and all(character.isalnum() or character in "+-.:" for character in package),
            "libx264 package name contains unsupported characters")
    query = run(["/usr/bin/dpkg-query", "-W", "-f=${binary:Package}\t${Version}\n", package], env=env)
    fields = query.stdout.decode("utf-8", errors="strict").strip().split("\t")
    require(len(fields) == 2 and fields[0] and fields[1], "dpkg-query did not return exact libx264 package/version")
    return {"package": fields[0], "version": fields[1]}


def filter_graph(font_path: Path) -> str:
    escaped = os.fspath(font_path).replace("\\", "\\\\").replace(":", "\\:").replace("'", "\\'")
    fontarg = f"fontfile='{escaped}':"
    return (
        f"[0:v]crop=2000:2160:900:0,scale=1000:1080:flags=lanczos,drawtext={fontarg}text='Kjerag':x=18:y=18:fontcolor=white:fontsize=42:box=1:boxcolor=black@0.70:boxborderw=12[k];"
        f"[1:v]crop=2000:2160:1840:0,scale=1000:1080:flags=lanczos,drawtext={fontarg}text='Studio Flow On':x=18:y=18:fontcolor=white:fontsize=42:box=1:boxcolor=black@0.70:boxborderw=12[s];"
        f"[2:v]crop=2000:2160:900:0,scale=1000:1080:flags=lanczos,drawtext={fontarg}text='Kjerag with computed seam':x=18:y=18:fontcolor=white:fontsize=42:box=1:boxcolor=black@0.70:boxborderw=12[t];"
        f"[k][s][t]hstack=inputs=3,pad=3000:1144:0:0:black,drawtext={fontarg}text='{FOOTER}':x=(w-text_w)/2:y=1096:fontcolor=white:fontsize=32"
    )


def commands(ffmpeg: str, inputs: Path, outputs: Path, font_path: Path) -> tuple[list[str], list[str]]:
    common = ["-an", "-c:v", "libx264", "-preset", "medium", "-crf", "14", "-pix_fmt", "yuv420p",
              "-r", "30000/1001", "-video_track_timescale", "30000", "-movflags", "+faststart"]
    native = [ffmpeg, "-hide_banner", "-loglevel", "info",
              "-framerate", "30000/1001", "-start_number", str(START), "-i", os.fspath(inputs / "kjerag" / "frame-%010d.png"),
              "-framerate", "30000/1001", "-start_number", str(START), "-i", os.fspath(inputs / "studio" / "frame-%08d.png"),
              "-framerate", "30000/1001", "-start_number", str(START), "-i", os.fspath(inputs / "trace" / "frame-%010d-trace.png"),
              "-filter_complex", filter_graph(font_path), "-frames:v", str(COUNT), *common, os.fspath(outputs / NATIVE)]
    quarter = [ffmpeg, "-hide_banner", "-loglevel", "info", "-i", os.fspath(outputs / NATIVE),
               "-vf", "setpts=4*PTS,fps=30000/1001", *common, os.fspath(outputs / QUARTER)]
    return native, quarter


def file_identity(path: Path) -> dict[str, Any]:
    return {"file": path.name, "bytes": path.stat().st_size, "sha256": sha256_path(path)}


def sync_file(path: Path) -> None:
    with path.open("rb") as leaf: os.fsync(leaf.fileno())


def sync_dir(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def rename_noreplace(parent_fd: int, source_name: str, destination_name: str) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    libc.renameat2.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    libc.renameat2.restype = ctypes.c_int
    result = libc.renameat2(parent_fd, os.fsencode(source_name), parent_fd,
                            os.fsencode(destination_name), RENAME_NOREPLACE)
    if result != 0:
        error = ctypes.get_errno()
        if error in (errno.EEXIST, errno.ENOTEMPTY):
            raise Refusal(f"destination already exists: {destination_name}")
        raise Refusal(f"renameat2 no-replace publication failed: {os.strerror(error)}")


def publish(parent_fd: int, destination_name: str, source: Path) -> None:
    stage_name = f".{destination_name}.publish-{os.getpid()}-{uuid.uuid4().hex}"
    stage_fd = -1
    try:
        os.mkdir(stage_name, 0o700, dir_fd=parent_fd)
        stage_fd = os.open(stage_name, os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0), dir_fd=parent_fd)
        for name in (NATIVE, QUARTER, OUTPUT_RECEIPT):
            source_fd = destination_fd = -1
            try:
                source_fd = os.open(source / name, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
                destination_fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                                         getattr(os, "O_NOFOLLOW", 0), 0o600, dir_fd=stage_fd)
                offset = 0
                size = os.fstat(source_fd).st_size
                while offset < size:
                    block = os.pread(source_fd, min(8 * 1024 * 1024, size - offset), offset)
                    require(bool(block), f"publication source {name} ended early")
                    written = os.write(destination_fd, block)
                    require(written == len(block), f"short publication write for {name}")
                    offset += written
                os.fsync(destination_fd)
            finally:
                if destination_fd >= 0: os.close(destination_fd)
                if source_fd >= 0: os.close(source_fd)
        os.fsync(stage_fd)
        rename_noreplace(parent_fd, stage_name, destination_name)
        os.fsync(parent_fd)
        stage_name = ""
    finally:
        if stage_fd >= 0: os.close(stage_fd)
        if stage_name:
            cleanup_fd = -1
            try:
                cleanup_fd = os.open(stage_name, os.O_RDONLY | os.O_DIRECTORY |
                                     getattr(os, "O_NOFOLLOW", 0), dir_fd=parent_fd)
            except OSError:
                pass
            for name in (NATIVE, QUARTER, OUTPUT_RECEIPT):
                try:
                    if cleanup_fd >= 0: os.unlink(name, dir_fd=cleanup_fd)
                except OSError: pass
            if cleanup_fd >= 0: os.close(cleanup_fd)
            try: os.rmdir(stage_name, dir_fd=parent_fd)
            except OSError: pass


def build(args: argparse.Namespace) -> Path:
    destination = args.out_dir
    require(destination.is_absolute(), "out-dir must be absolute")
    require(destination.name not in ("", ".", ".."), "out-dir must name one destination basename")
    require(destination.parent.exists(), "out-dir parent must already exist")
    try:
        parent_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise Refusal(f"cannot pin output parent {destination.parent}: {error}") from error
    clean_env = clean_environment()
    with ExitStack() as stack:
        stack.callback(os.close, parent_fd)
        builder = stack.enter_context(PinnedFile(Path(__file__), None, "builder"))
        kjerag_receipt = stack.enter_context(PinnedReceipt(args.kjerag_receipt, args.kjerag_sha256, "Kjerag receipt"))
        studio_receipt = stack.enter_context(PinnedReceipt(args.studio_receipt, args.studio_sha256, "Studio receipt"))
        trace_receipt = stack.enter_context(PinnedReceipt(args.trace_receipt, args.trace_sha256, "range-trace receipt"))
        playback = stack.enter_context(PinnedFile(args.playback_bin, args.expected_playback_sha256, "playback binary"))
        trace_binary = stack.enter_context(PinnedFile(args.range_trace_bin, args.expected_range_trace_sha256,
                                                       "range-trace binary"))
        ffmpeg_input = stack.enter_context(PinnedFile(args.ffmpeg, args.ffmpeg_sha256, "ffmpeg"))
        ffprobe_input = stack.enter_context(PinnedFile(args.ffprobe, args.ffprobe_sha256, "ffprobe"))
        font_input = stack.enter_context(PinnedFile(args.font, FONT_SHA256, "historical Noto Sans font"))
        x264_input = stack.enter_context(PinnedFile(args.libx264, args.libx264_sha256, "libx264 runtime"))
        git_input = stack.enter_context(PinnedFile(args.git, args.git_sha256, "git executable"))
        with tempfile.TemporaryDirectory(prefix=".three-panel-private-") as temporary:
            inputs = Path(temporary)
            for name in ("kjerag", "studio", "studio-supplied", "trace", "trace-supplied", "bin", "lib", "rendered"):
                (inputs / name).mkdir()
            private_ffmpeg, private_ffprobe = inputs / "bin/ffmpeg", inputs / "bin/ffprobe"
            private_trace, private_font = inputs / "bin/range-trace", inputs / "font.ttf"
            private_x264 = inputs / "lib/libx264.so.164"
            private_git = inputs / "bin/git"
            ffmpeg_input.materialize(private_ffmpeg, 0o500)
            ffprobe_input.materialize(private_ffprobe, 0o500)
            trace_binary.materialize(private_trace, 0o500)
            font_input.materialize(private_font, 0o400)
            x264_input.materialize(private_x264, 0o400)
            git_input.materialize(private_git, 0o500)
            private_copies = [
                stack.enter_context(PinnedFile(private_ffmpeg, ffmpeg_input.sha256, "private ffmpeg")),
                stack.enter_context(PinnedFile(private_ffprobe, ffprobe_input.sha256, "private ffprobe")),
                stack.enter_context(PinnedFile(private_trace, trace_binary.sha256, "private range-trace")),
                stack.enter_context(PinnedFile(private_font, font_input.sha256, "private font")),
                stack.enter_context(PinnedFile(private_x264, x264_input.sha256, "private libx264")),
                stack.enter_context(PinnedFile(private_git, git_input.sha256, "private git")),
            ]
            kjerag = validate_kjerag(kjerag_receipt, inputs / "kjerag", args.expected_commit,
                                      args.expected_tree, args.expected_playback_sha256)
            require(playback.sha256 == obj(kjerag["build"].get("executable"), "playback executable").get("sha256"),
                    "supplied playback binary differs from Kjerag receipt")
            require(playback.state.st_size == kjerag["build"]["executable"]["bytes"],
                    "supplied playback binary size differs from Kjerag receipt")
            private_k_path = inputs / "kjerag/range-receipt.json"
            kjerag_receipt.materialize(private_k_path, 0o400)
            private_k = stack.enter_context(PinnedReceipt(private_k_path, kjerag_receipt.sha256, "private Kjerag receipt"))
            studio = validate_studio(studio_receipt, inputs / "studio-supplied", kjerag)
            private_tool_env = private_path_environment(clean_env, inputs / "bin")
            derived_receipt, derivation = derive_studio(args.oracle_dir, inputs / "canonical-projector", kjerag,
                                                         inputs / "bin", clean_env)
            try:
                derived = validate_studio(derived_receipt, inputs / "studio", kjerag, CANONICAL_DERIVED_AUTH)
                compare_studio_derivation(studio, derived)
                derived_receipt.verify()
            finally:
                derived_receipt.close()
            supplied_trace = validate_trace(trace_receipt, inputs / "trace-supplied", kjerag_receipt, kjerag,
                                             args.expected_commit, args.expected_tree,
                                             args.expected_range_trace_sha256)
            require(trace_binary.state.st_size == supplied_trace["build"]["executable"]["bytes"],
                    "supplied range-trace binary size differs from trace receipt")
            private_trace_receipt, trace_derivation = derive_trace(private_trace, inputs / "canonical-trace", private_k,
                                                                    kjerag, args.expected_commit, args.expected_tree,
                                                                    args.expected_range_trace_sha256, private_tool_env)
            try:
                derived_trace = validate_trace(private_trace_receipt, inputs / "trace", private_k, kjerag,
                                               args.expected_commit, args.expected_tree,
                                               args.expected_range_trace_sha256)
                compare_trace_derivation(supplied_trace, derived_trace)
                private_trace_receipt.verify()
            finally:
                private_trace_receipt.close()
            encode_env = {**private_tool_env, "LD_PRELOAD": os.fspath(private_x264)}
            ffmpeg_version = run([os.fspath(private_ffmpeg), "-version"], env=encode_env).stdout.decode(errors="replace").splitlines()
            ffprobe_version = run([os.fspath(private_ffprobe), "-version"], env=clean_env).stdout.decode(errors="replace").splitlines()
            require(ffmpeg_version and ffprobe_version, "private ffmpeg tools did not print versions")
            encoders = run([os.fspath(private_ffmpeg), "-hide_banner", "-encoders"], env=encode_env).stdout
            require(b"libx264" in encoders, "private ffmpeg does not provide libx264")
            native, quarter = commands(os.fspath(private_ffmpeg), inputs, inputs / "rendered", private_font)
            native_run = run(native, env=encode_env)
            quarter_run = run(quarter, env=encode_env)
            native_core = x264_core(native_run.stderr, "native encode")
            quarter_core = x264_core(quarter_run.stderr, "quarter-speed encode")
            require(native_core == quarter_core, "native and quarter-speed encodes loaded different libx264 cores")
            require(native_core == args.expected_libx264_core,
                    f"loaded libx264 core differs from expected identity {args.expected_libx264_core}")
            x264_package = package_identity(args.libx264_package, clean_env)
            outputs = {
                "native": {**file_identity(inputs / "rendered" / NATIVE),
                           "ffprobe": probe_video(os.fspath(private_ffprobe), inputs / "rendered" / NATIVE,
                                                  COUNT, "2.035367", clean_env), "libx264_core": native_core},
                "quarter_speed": {**file_identity(inputs / "rendered" / QUARTER),
                                  "ffprobe": probe_video(os.fspath(private_ffprobe), inputs / "rendered" / QUARTER,
                                                         COUNT * 4, "8.141467", clean_env), "libx264_core": quarter_core},
            }
            for pinned in (builder, kjerag_receipt, studio_receipt, trace_receipt, playback, trace_binary,
                           ffmpeg_input, ffprobe_input, font_input, x264_input, git_input, *private_copies):
                pinned.verify()
            record = {
                "schema": OUTPUT_SCHEMA,
                "claim_boundary": "Owner review aid only. It authenticates exact inputs and construction, not Studio parity or an owner verdict.",
                "fixed_interval": {"start": START, "end_inclusive": END, "count": COUNT,
                                   "rate": "30000/1001", "native_frames": COUNT, "quarter_speed_frames": COUNT * 4},
                "footer": FOOTER, "registration_transform": None,
                "expected_shipping_build": {"commit": args.expected_commit, "tree": args.expected_tree,
                    "playback_sha256": args.expected_playback_sha256,
                    "range_trace_sha256": args.expected_range_trace_sha256},
                "inputs": {
                    "receipts": {"kjerag": kjerag_receipt.identity(), "studio": studio_receipt.identity(),
                                 "range_trace": trace_receipt.identity()},
                    "kjerag": {"source": kjerag["source"], "build": kjerag["build"], "run": kjerag["run"],
                               "view": kjerag["view"], "frames": kjerag["frames"]},
                    "studio": studio, "studio_private_derivation": derivation,
                    "range_trace": supplied_trace, "range_trace_private_derivation": trace_derivation,
                    "font": font_input.identity(),
                },
                "tools": {"ffmpeg": {**ffmpeg_input.identity(), "version": ffmpeg_version},
                          "ffprobe": {**ffprobe_input.identity(), "version": ffprobe_version},
                          "libx264": {**x264_input.identity(), "load_policy": "private LD_PRELOAD",
                                      "observed_core": native_core, **x264_package},
                          "playback": playback.identity(), "range_trace": trace_binary.identity(),
                          "git": git_input.identity(),
                          "python": {**executable_identity(Path(sys.executable)), "version": sys.version},
                          "builder": builder.identity()},
                "commands": {"studio_private_derivation": derivation["command"],
                             "range_trace_private_derivation": trace_derivation["command"],
                             "native": native, "quarter_speed": quarter},
                "outputs": outputs,
                "historical_reproduction": {
                    "native_sha256": "25f0e6bf385bc64a33977603c4a71140aa4a6e328d0d448f546ced911636b6e5",
                    "quarter_speed_sha256": "07413901c22a674975d2c65204c991041d34084c2eeb9f1c2d0c1d33a2cb8d9d",
                    "historical_derivatives_were_receipted": False, "explicit_font_added": True,
                    "exact_hashes_depend_on_recorded_ffmpeg_libx264_and_font_stack": True,
                    "native_matches": outputs["native"]["sha256"] == "25f0e6bf385bc64a33977603c4a71140aa4a6e328d0d448f546ced911636b6e5",
                    "quarter_speed_matches": outputs["quarter_speed"]["sha256"] == "07413901c22a674975d2c65204c991041d34084c2eeb9f1c2d0c1d33a2cb8d9d",
                },
            }
            receipt_path = inputs / "rendered" / OUTPUT_RECEIPT
            with receipt_path.open("xb") as output:
                output.write(json.dumps(record, indent=2, sort_keys=True).encode() + b"\n")
                output.flush(); os.fsync(output.fileno())
            publish(parent_fd, destination.name, inputs / "rendered")
            return destination / OUTPUT_RECEIPT


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--kjerag-receipt", type=Path, required=True)
    result.add_argument("--kjerag-sha256", required=True)
    result.add_argument("--studio-receipt", type=Path, required=True)
    result.add_argument("--studio-sha256", required=True)
    result.add_argument("--trace-receipt", type=Path, required=True)
    result.add_argument("--trace-sha256", required=True)
    result.add_argument("--oracle-dir", type=Path, required=True)
    result.add_argument("--expected-commit", required=True)
    result.add_argument("--expected-tree", required=True)
    result.add_argument("--playback-bin", type=Path, required=True)
    result.add_argument("--expected-playback-sha256", required=True)
    result.add_argument("--range-trace-bin", type=Path, required=True)
    result.add_argument("--expected-range-trace-sha256", required=True)
    result.add_argument("--font", type=Path, default=Path("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"))
    result.add_argument("--ffmpeg", type=Path, required=True)
    result.add_argument("--ffmpeg-sha256", required=True)
    result.add_argument("--ffprobe", type=Path, required=True)
    result.add_argument("--ffprobe-sha256", required=True)
    result.add_argument("--libx264", type=Path, required=True)
    result.add_argument("--libx264-sha256", required=True)
    result.add_argument("--libx264-package", required=True)
    result.add_argument("--expected-libx264-core", required=True)
    result.add_argument("--git", type=Path, required=True)
    result.add_argument("--git-sha256", required=True)
    result.add_argument("--out-dir", type=Path, required=True)
    return result


def main() -> int:
    try:
        receipt = build(parser().parse_args())
        print(f"receipt: {receipt}")
        return 0
    except Refusal as error:
        print(f"refused: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
