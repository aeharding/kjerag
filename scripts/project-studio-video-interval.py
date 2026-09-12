#!/usr/bin/env python3
"""Project an exact consecutive interval from the authenticated Studio oracle."""

from __future__ import annotations

import argparse
import ctypes
import errno
from fractions import Fraction
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONTRACT = ROOT / "docs/research/studio-video-oracle-602.json"
DEFAULT_ORACLE = ROOT / "scratch/studio-video-oracle/ordinary-360"
VERIFIER = ROOT / "scripts/verify-studio-video-oracle.py"
RECEIPT_NAME = "projection-receipt.json"
CANONICAL_CONTRACT_SHA256 = "eda588746c1d15347bae2064b3de2175eb1f1222d159a53c362f900d409e2374"
CANONICAL_VERIFIER_SHA256 = "67452d3e3acea5421dacd03745a1cd567a8be03c92aa7c62a66b2cc163ab13f3"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SHOWINFO_TIME_BASE = re.compile(r"config in time_base:\s*([^,\s]+)")
SHOWINFO_FRAME = re.compile(r"Parsed_showinfo_[^ ]+.*?\bn:\s*(\d+)\s+pts:\s*(-?\d+)")
AT_FDCWD = -100
RENAME_NOREPLACE = 1


class Refusal(Exception):
    """An input, oracle, extraction, or publication precondition failed."""


class PinnedFile:
    """Bytes and identity retained from one open regular file."""

    def __init__(
        self,
        path: Path,
        descriptor: int,
        state: os.stat_result,
        raw: bytes,
        sha256: str,
    ) -> None:
        self.path = path
        self.descriptor = descriptor
        self.state = state
        self.raw = raw
        self.sha256 = sha256

    def close(self) -> None:
        if self.descriptor >= 0:
            os.close(self.descriptor)
            self.descriptor = -1


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Refusal(message)


def load_json_bytes(raw: bytes, path: Path) -> Any:
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise Refusal(f"cannot read JSON {path}: {error}") from error


def sha256_path(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            while block := source.read(8 * 1024 * 1024):
                digest.update(block)
    except OSError as error:
        raise Refusal(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def sha256_descriptor(descriptor: int) -> str:
    digest = hashlib.sha256()
    offset = 0
    try:
        while block := os.pread(descriptor, 8 * 1024 * 1024, offset):
            digest.update(block)
            offset += len(block)
    except OSError as error:
        raise Refusal(f"cannot hash oracle video descriptor: {error}") from error
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


def finite(name: str, value: float) -> float:
    require(math.isfinite(value), f"{name} must be finite")
    return value


def vertical_fov(horizontal_fov: float, width: int, height: int) -> float:
    require(width > 0 and height > 0, "output width and height must be positive")
    require(
        math.isfinite(horizontal_fov) and 0.0 < horizontal_fov < 180.0,
        "horizontal FOV must be finite and between 0 and 180 degrees",
    )
    half_horizontal = math.radians(horizontal_fov) / 2.0
    return math.degrees(
        2.0 * math.atan(math.tan(half_horizontal) * height / width)
    )


def number(value: float) -> str:
    return format(value, ".15g")


def expected_frames(
    contract: dict[str, Any], start: int, count: int
) -> list[dict[str, Any]]:
    require(start >= 0, "start frame must not be negative")
    require(count > 0, "count must be greater than zero")
    grid = contract["packet_grid"]
    first = grid["first_index"]
    last = grid["last_index"]
    end = start + count - 1
    require(
        first <= start <= end <= last,
        f"requested frame interval {start}..{end} lies outside {first}..{last}",
    )
    time_base = Fraction(contract["media_contract"]["video"]["time_base"])
    frames = []
    for index in range(start, end + 1):
        pts = index * grid["step"]
        frames.append(
            {
                "index": index,
                "pts": pts,
                "time_base": str(time_base),
                "seconds": format(float(pts * time_base), ".9f"),
                "png": f"frame-{index:08d}.png",
            }
        )
    return frames


def run(args: list[str], **kwargs: Any) -> subprocess.CompletedProcess[bytes]:
    try:
        process = subprocess.run(args, check=False, capture_output=True, **kwargs)
    except OSError as error:
        raise Refusal(f"cannot run {args[0]}: {error}") from error
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise Refusal(f"{' '.join(args)} failed ({process.returncode}): {detail}")
    return process


def publish_leaf(path: Path, raw: bytes, mode: int) -> None:
    descriptor = -1
    try:
        descriptor = os.open(
            path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            mode,
        )
        offset = 0
        while offset < len(raw):
            written = os.write(descriptor, raw[offset:])
            require(written > 0, f"short write while publishing {path}")
            offset += written
        os.fsync(descriptor)
    except OSError as error:
        raise Refusal(f"cannot publish private input {path}: {error}") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def authenticate(
    verifier_raw: bytes,
    contract_raw: bytes,
    work: Path,
    oracle_dir: Path,
) -> bytes:
    verifier = work / ".pinned-verifier.py"
    contract = work / ".pinned-contract.json"
    publish_leaf(verifier, verifier_raw, 0o400)
    publish_leaf(contract, contract_raw, 0o400)
    verifier_descriptor = -1
    contract_descriptor = -1
    try:
        verifier_descriptor = os.open(
            verifier, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        )
        contract_descriptor = os.open(
            contract, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        )
        require(
            sha256_descriptor(verifier_descriptor)
            == hashlib.sha256(verifier_raw).hexdigest(),
            "private verifier bytes changed before execution",
        )
        require(
            sha256_descriptor(contract_descriptor)
            == hashlib.sha256(contract_raw).hexdigest(),
            "private contract bytes changed before verification",
        )
        process = run(
            [
                sys.executable,
                f"/proc/self/fd/{verifier_descriptor}",
                "--contract",
                os.fspath(contract),
                "--artifact-dir",
                os.fspath(oracle_dir),
            ],
            pass_fds=(verifier_descriptor,),
        )
        require(
            sha256_descriptor(verifier_descriptor)
            == hashlib.sha256(verifier_raw).hexdigest(),
            "private verifier bytes changed during execution",
        )
        require(
            sha256_descriptor(contract_descriptor)
            == hashlib.sha256(contract_raw).hexdigest(),
            "private contract bytes changed during verification",
        )
        return process.stdout
    finally:
        if verifier_descriptor >= 0:
            os.close(verifier_descriptor)
        if contract_descriptor >= 0:
            os.close(contract_descriptor)
        verifier.unlink(missing_ok=True)
        contract.unlink(missing_ok=True)


def parse_showinfo(
    stderr: bytes, expected: list[dict[str, Any]], expected_time_base: str
) -> None:
    text = stderr.decode("utf-8", errors="replace")
    bases = SHOWINFO_TIME_BASE.findall(text)
    require(bases, "ffmpeg did not report the projection input time base")
    require(
        all(Fraction(value) == Fraction(expected_time_base) for value in bases),
        f"ffmpeg projection time base changed: expected {expected_time_base}, got {bases}",
    )
    observed = [(int(n), int(pts)) for n, pts in SHOWINFO_FRAME.findall(text)]
    wanted = [(ordinal, frame["pts"]) for ordinal, frame in enumerate(expected)]
    require(
        observed == wanted,
        f"ffmpeg output frame/PTS sequence changed: expected {wanted}, got {observed}",
    )


def png_dimensions(path: Path) -> tuple[int, int]:
    try:
        with path.open("rb") as image:
            header = image.read(24)
    except OSError as error:
        raise Refusal(f"cannot read projected picture {path}: {error}") from error
    require(
        len(header) == 24 and header[:8] == PNG_SIGNATURE and header[12:16] == b"IHDR",
        f"projected picture is not a PNG with an IHDR header: {path}",
    )
    return struct.unpack(">II", header[16:24])


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    raw = (
        json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n"
    ).encode("utf-8")
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    descriptor = -1
    try:
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        offset = 0
        while offset < len(raw):
            written = os.write(descriptor, raw[offset:])
            require(written > 0, f"short write while publishing {path}")
            offset += written
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = -1
        os.replace(temporary, path)
        fsync_directory(path.parent)
    except OSError as error:
        raise Refusal(f"cannot publish receipt {path}: {error}") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass


def fsync_regular_file(path: Path) -> None:
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        state = os.fstat(descriptor)
        require(stat.S_ISREG(state.st_mode), f"leaf is not a regular file: {path}")
        os.fsync(descriptor)
    except OSError as error:
        raise Refusal(f"cannot sync leaf {path}: {error}") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def fsync_directory(path: Path) -> None:
    descriptor = -1
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY
            | getattr(os, "O_DIRECTORY", 0)
            | getattr(os, "O_NOFOLLOW", 0),
        )
        state = os.fstat(descriptor)
        require(stat.S_ISDIR(state.st_mode), f"path is not a directory: {path}")
        os.fsync(descriptor)
    except OSError as error:
        raise Refusal(f"cannot sync directory {path}: {error}") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def validate_contract(contract: dict[str, Any]) -> None:
    require(contract.get("schema_version") == 1, "oracle contract schema is not 1")
    require(
        contract.get("producer", {}).get("application") == "Insta360 Studio",
        "oracle producer is not Insta360 Studio",
    )
    grid = contract["packet_grid"]
    require(grid["first_index"] == 0, "oracle packet grid does not begin at frame zero")
    require(grid["step"] > 0, "oracle packet-grid step is not positive")
    require(grid["duration"] == grid["step"], "oracle packet duration differs from its step")


def open_authenticated_video(
    oracle_dir: Path, receipt: dict[str, Any]
) -> tuple[int, os.stat_result, Path]:
    name = receipt["name"]
    require(Path(name).name == name, f"oracle video name is not a basename: {name!r}")
    path = oracle_dir / name
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
        before = os.fstat(descriptor)
    except OSError as error:
        raise Refusal(f"cannot open oracle video {path}: {error}") from error
    try:
        require(stat.S_ISREG(before.st_mode), f"oracle video is not a regular file: {path}")
        require(before.st_size == receipt["bytes"], f"oracle video size changed: {path}")
        require(
            sha256_descriptor(descriptor) == receipt["sha256"],
            f"oracle video SHA-256 changed: {path}",
        )
    except Exception:
        os.close(descriptor)
        raise
    return descriptor, before, path


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


def pin_regular_file(path: Path, what: str) -> PinnedFile:
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        before = os.fstat(descriptor)
        require(stat.S_ISREG(before.st_mode), f"{what} is not a regular file: {path}")
        raw = read_descriptor(descriptor)
        after = os.fstat(descriptor)
        require(same_file_state(before, after), f"{what} changed while it was read: {path}")
        named = os.stat(path, follow_symlinks=False)
        require(same_file_state(before, named), f"{what} path changed while it was read: {path}")
        return PinnedFile(
            path=path,
            descriptor=descriptor,
            state=before,
            raw=raw,
            sha256=hashlib.sha256(raw).hexdigest(),
        )
    except (OSError, Refusal) as error:
        if descriptor >= 0:
            os.close(descriptor)
        if isinstance(error, Refusal):
            raise
        raise Refusal(f"cannot pin {what} {path}: {error}") from error


def require_pinned_unchanged(pinned: PinnedFile, what: str) -> None:
    try:
        descriptor_state = os.fstat(pinned.descriptor)
        named_state = os.stat(pinned.path, follow_symlinks=False)
    except OSError as error:
        raise Refusal(f"cannot reinspect pinned {what} {pinned.path}: {error}") from error
    require(
        same_file_state(pinned.state, descriptor_state),
        f"pinned {what} changed during projection: {pinned.path}",
    )
    require(
        same_file_state(pinned.state, named_state),
        f"pinned {what} path changed during projection: {pinned.path}",
    )


def require_canonical(pinned: PinnedFile, expected_sha256: str, what: str) -> None:
    require(
        pinned.sha256 == expected_sha256,
        f"{what} is not the canonical committed file: expected SHA-256 "
        f"{expected_sha256}, got {pinned.sha256}",
    )


def rename_noreplace(source: Path, destination: Path) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    try:
        operation = libc.renameat2
    except AttributeError as error:
        raise Refusal("this Linux system does not provide renameat2") from error
    operation.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    ]
    operation.restype = ctypes.c_int
    result = operation(
        AT_FDCWD,
        os.fsencode(source),
        AT_FDCWD,
        os.fsencode(destination),
        RENAME_NOREPLACE,
    )
    if result == 0:
        return
    code = ctypes.get_errno()
    if code == errno.EEXIST:
        raise Refusal(f"output directory already exists: {destination}")
    raise Refusal(
        f"cannot publish output directory {destination}: {os.strerror(code)}"
    )


def project(
    video_descriptor: int,
    destination: Path,
    expected: list[dict[str, Any]],
    yaw: float,
    pitch: float,
    horizontal: float,
    vertical: float,
    width: int,
    height: int,
    time_base: str,
) -> list[str]:
    first = expected[0]["index"]
    last = expected[-1]["index"]
    projection = (
        f"select=between(n\\,{first}\\,{last}),"
        f"v360=input=equirect:output=flat:yaw={number(yaw)}:pitch={number(pitch)}:"
        f"h_fov={number(horizontal)}:v_fov={number(vertical)}:w={width}:h={height},"
        "showinfo"
    )
    output = destination / "frame-%08d.png"
    arguments = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "info",
        "-nostdin",
        "-i",
        f"/proc/self/fd/{video_descriptor}",
        "-map",
        "0:v:0",
        "-an",
        "-vf",
        projection,
        "-frames:v",
        str(len(expected)),
        "-fps_mode",
        "passthrough",
        "-start_number",
        str(first),
        "-map_metadata",
        "-1",
        os.fspath(output),
    ]
    process = run(arguments, pass_fds=(video_descriptor,))
    parse_showinfo(process.stderr, expected, time_base)
    return arguments


def receipt_arguments(arguments: list[str], video_name: str) -> list[str]:
    result = arguments[1:].copy()
    input_index = result.index("-i") + 1
    result[input_index] = video_name
    result[-1] = "frame-%08d.png"
    return result


def parse() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Authenticate a Studio ordinary-360 oracle and project an exact "
            "consecutive output-frame interval."
        ),
        allow_abbrev=False,
    )
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--oracle-dir", type=Path, default=DEFAULT_ORACLE)
    parser.add_argument("--flow", choices=("on", "off"), default="on")
    parser.add_argument("--start-frame", type=int, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--yaw", type=float, required=True)
    parser.add_argument("--pitch", type=float, required=True)
    parser.add_argument("--horizontal-fov", type=float, required=True)
    parser.add_argument("--width", type=int, required=True)
    parser.add_argument("--height", type=int, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse()
    staging: Path | None = None
    descriptor = -1
    contract_pin: PinnedFile | None = None
    verifier_pin: PinnedFile | None = None
    try:
        require(
            not args.output_dir.exists() and not args.output_dir.is_symlink(),
            f"output directory already exists: {args.output_dir}",
        )
        parent = args.output_dir.parent
        require(
            parent.is_dir() and not parent.is_symlink(),
            f"output parent is not a regular nonsymlink directory: {parent}",
        )
        finite("yaw", args.yaw)
        finite("pitch", args.pitch)
        require(
            -90.0 <= args.pitch <= 90.0,
            "pitch must be between -90 and 90 degrees",
        )
        vertical = vertical_fov(args.horizontal_fov, args.width, args.height)

        contract_pin = pin_regular_file(args.contract, "oracle contract")
        verifier_pin = pin_regular_file(VERIFIER, "oracle verifier")
        require_canonical(
            contract_pin, CANONICAL_CONTRACT_SHA256, "oracle contract"
        )
        require_canonical(
            verifier_pin, CANONICAL_VERIFIER_SHA256, "oracle verifier"
        )
        contract = load_json_bytes(contract_pin.raw, contract_pin.path)
        validate_contract(contract)
        frames = expected_frames(contract, args.start_frame, args.count)
        state = f"flow_{args.flow}"
        artifact = contract["artifacts"][state]
        descriptor, before, video_path = open_authenticated_video(
            args.oracle_dir, artifact["video"]
        )
        staging = Path(
            tempfile.mkdtemp(prefix=f".{args.output_dir.name}.tmp-", dir=parent)
        )
        verifier_output = authenticate(
            verifier_pin.raw,
            contract_pin.raw,
            staging,
            args.oracle_dir,
        )
        sys.stdout.buffer.write(verifier_output)
        ffmpeg_arguments = project(
            descriptor,
            staging,
            frames,
            args.yaw,
            args.pitch,
            args.horizontal_fov,
            vertical,
            args.width,
            args.height,
            contract["media_contract"]["video"]["time_base"],
        )
        ffmpeg_version = run(["ffmpeg", "-version"]).stdout.decode(
            "utf-8", errors="replace"
        ).splitlines()[0]
        after = os.fstat(descriptor)
        require(same_file_state(before, after), "oracle video changed during projection")
        try:
            path_after = os.stat(video_path, follow_symlinks=False)
        except OSError as error:
            raise Refusal(f"cannot reinspect oracle video {video_path}: {error}") from error
        require(same_file_state(before, path_after), "oracle video path changed during projection")

        names = sorted(path.name for path in staging.iterdir())
        expected_names = [frame["png"] for frame in frames]
        require(
            names == expected_names,
            f"projected picture inventory changed: expected {expected_names}, got {names}",
        )
        for frame in frames:
            picture = staging / frame["png"]
            require(
                png_dimensions(picture) == (args.width, args.height),
                f"projected picture dimensions changed: {picture}",
            )
            frame["bytes"] = picture.stat().st_size
            frame["sha256"] = sha256_path(picture)
            fsync_regular_file(picture)

        receipt = {
            "schema": "kjerag.studio-projected-interval.v1",
            "authentication": {
                "contract": {
                    "name": DEFAULT_CONTRACT.name,
                    "schema_version": contract["schema_version"],
                    "sha256": contract_pin.sha256,
                },
                "producer": contract["producer"],
                "verifier": {
                    "name": VERIFIER.name,
                    "sha256": verifier_pin.sha256,
                },
            },
            "source": {
                "flow": args.flow,
                "optical_flow": artifact["optical_flow"],
                "video": artifact["video"],
                "project": artifact["project"],
                "packet_grid": contract["packet_grid"],
            },
            "projection": {
                "input": "equirect",
                "output": "flat",
                "yaw": args.yaw,
                "pitch": args.pitch,
                "horizontal_fov": args.horizontal_fov,
                "vertical_fov": vertical,
                "width": args.width,
                "height": args.height,
            },
            "interval": {
                "start_frame": args.start_frame,
                "count": args.count,
                "end_frame": frames[-1]["index"],
                "frames": frames,
            },
            "extraction": {
                "selection": "decoded frame number n on the authenticated CFR packet grid",
                "temporal_interpolation": False,
                "ffmpeg_version": ffmpeg_version,
                "ffmpeg_arguments": receipt_arguments(
                    ffmpeg_arguments, artifact["video"]["name"]
                ),
            },
            "claim_boundary": (
                "Authenticated Studio output pixels projected at exact consecutive output "
                "samples. This receipt does not establish Kjerag stitch or temporal parity."
            ),
        }
        atomic_json(staging / RECEIPT_NAME, receipt)
        fsync_regular_file(staging / RECEIPT_NAME)
        fsync_directory(staging)
        require_pinned_unchanged(contract_pin, "oracle contract")
        require_pinned_unchanged(verifier_pin, "oracle verifier")
        rename_noreplace(staging, args.output_dir)
        fsync_directory(parent)
        staging = None
        print(
            f"projected authenticated Studio Flow {args.flow.title()} frames "
            f"{args.start_frame}..{frames[-1]['index']} to {args.output_dir}"
        )
        print(f"receipt: {args.output_dir / RECEIPT_NAME}")
        return 0
    except (KeyError, TypeError, ValueError, OverflowError, Refusal, OSError) as error:
        print(f"REFUSED: {error}", file=sys.stderr)
        return 1
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        if contract_pin is not None:
            contract_pin.close()
        if verifier_pin is not None:
            verifier_pin.close()
        if staging is not None:
            shutil.rmtree(staging, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
