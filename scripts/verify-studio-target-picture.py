#!/usr/bin/env python3
"""Reproduce the Studio target-picture adjacent-frame decoy receipt.

Every candidate receives the same independent fit on one terrain band and is
scored only on the other band. These registrations identify source-frame
content only. They are not stitch, view, stabilization, or playback constants.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tarfile
import tempfile
from typing import Any

try:
    import cv2
    import numpy as np
except ImportError as error:
    print(f"REFUSED: target-picture verification needs OpenCV and NumPy: {error}", file=sys.stderr)
    raise SystemExit(1) from error


ROOT = Path(
    os.environ.get(
        "KJERAG_TARGET_HELPER_ROOT",
        os.fspath(Path(__file__).resolve().parent.parent),
    )
).resolve()
DEFAULT_CONTRACT = ROOT / "docs/research/studio-video-oracle-602.json"
CANONICAL_CONTRACT_SHA256 = "eda588746c1d15347bae2064b3de2175eb1f1222d159a53c362f900d409e2374"


class Refusal(Exception):
    """A target-picture receipt did not match exactly."""


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


def reverify_file(pinned: PinnedFile, expected_sha256: str) -> None:
    try:
        descriptor_state = os.fstat(pinned.descriptor)
        named_state = os.stat(pinned.path, follow_symlinks=False)
    except OSError as error:
        raise Refusal(f"cannot reinspect pinned input {pinned.path}: {error}") from error
    if not same_file_state(pinned.state, descriptor_state):
        raise Refusal(f"pinned input changed during use: {pinned.path}")
    if not same_file_state(pinned.state, named_state):
        raise Refusal(f"pinned input path changed during use: {pinned.path}")
    same(
        f"{pinned.path.name} final SHA-256",
        sha256_descriptor(pinned.descriptor),
        expected_sha256,
    )


def pin_receipt_file(directory: Path, receipt: dict[str, Any]) -> PinnedFile:
    name = receipt["name"]
    if Path(name).name != name:
        raise Refusal(f"receipt file name is not a basename: {name!r}")
    return pin_regular_file(
        directory / name,
        f"receipt file {name}",
        expected_bytes=receipt["bytes"],
        expected_sha256=receipt["sha256"],
    )


def load_canonical_contract(path: Path) -> dict[str, Any]:
    pinned = pin_regular_file(
        path,
        "oracle contract",
        expected_sha256=CANONICAL_CONTRACT_SHA256,
    )
    try:
        try:
            return json.loads(read_descriptor(pinned.descriptor))
        except (UnicodeError, json.JSONDecodeError) as error:
            raise Refusal(f"cannot read JSON {path}: {error}") from error
    finally:
        pinned.close()


def load_image(directory: Path, receipt: dict[str, Any]) -> np.ndarray:
    pinned = pin_receipt_file(directory, receipt)
    try:
        raw = read_descriptor(pinned.descriptor)
        reverify_file(pinned, receipt["sha256"])
    finally:
        pinned.close()
    encoded = np.frombuffer(raw, dtype=np.uint8)
    image = cv2.imdecode(encoded, cv2.IMREAD_GRAYSCALE)
    if image is None:
        raise Refusal(f"OpenCV cannot decode authenticated {receipt['name']}")
    return image


def command(
    args: list[str],
    *,
    cwd: Path = ROOT,
    env: dict[str, str] | None = None,
    pass_fds: tuple[int, ...] = (),
) -> bytes:
    try:
        process = subprocess.run(
            args,
            cwd=cwd,
            env=env,
            check=False,
            capture_output=True,
            pass_fds=pass_fds,
        )
    except OSError as error:
        raise Refusal(f"cannot run {args[0]}: {error}") from error
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise Refusal(f"{' '.join(args)} failed ({process.returncode}): {detail}")
    return process.stdout


def build_reframe(work: Path, contract: dict[str, Any]) -> Path:
    receipt = contract["target_picture_authentication"]["reframe_build"]
    same(
        "reframe build receipt",
        receipt,
        {
            "commit": "554a8d53dadfcee1485f87fe63f98bb334112207",
            "tree": "a4ba7b06f2710d6d0aec1e0b3538efcda90da271",
            "cargo_lock_sha256": "aba8c3a0dc15325afe7d64258ab707e071e3a4d5730a46b412618f0e0344b118",
            "command": [
                "cargo",
                "build",
                "--release",
                "--frozen",
                "-p",
                "kjerag-spike",
                "--bin",
                "reframe",
            ],
            "binary": "release/reframe",
        },
    )
    commit = receipt["commit"]
    resolved_commit = command(["git", "rev-parse", f"{commit}^{{commit}}"])
    same("reframe receipt commit", resolved_commit.decode("ascii").strip(), commit)
    resolved_tree = command(["git", "rev-parse", f"{commit}^{{tree}}"])
    same("reframe receipt tree", resolved_tree.decode("ascii").strip(), receipt["tree"])
    lock = command(["git", "show", f"{commit}:Cargo.lock"])
    same(
        "reframe receipt Cargo.lock SHA-256",
        hashlib.sha256(lock).hexdigest(),
        receipt["cargo_lock_sha256"],
    )

    source_root = work / "source"
    target_root = work / "target"
    try:
        source_root.mkdir()
        archive_bytes = command(["git", "archive", "--format=tar", commit])
        with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:") as archive:
            archive.extractall(source_root, filter="data")
    except (OSError, tarfile.TarError) as error:
        raise Refusal(f"cannot extract Kjerag receipt commit {commit}: {error}") from error
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = os.fspath(target_root)
    command(receipt["command"], cwd=source_root, env=environment)
    binary = target_root / receipt["binary"]
    try:
        if binary.is_symlink() or not binary.is_file():
            raise Refusal(f"built reframe is not a regular nonsymlink file: {binary}")
        if not os.access(binary, os.X_OK):
            raise Refusal(f"built reframe is not executable: {binary}")
    except OSError as error:
        raise Refusal(f"cannot inspect built reframe {binary}: {error}") from error
    print(f"built reframe from authenticated commit/tree: {commit} / {receipt['tree']}")
    return binary


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
        while block := os.pread(pinned.descriptor, 8 * 1024 * 1024, offset):
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


def regenerate(
    directory: Path,
    contract: dict[str, Any],
    artifact_dir: Path,
    source_dir: Path,
    reframe_bin: Path,
) -> None:
    receipt = contract["target_picture_authentication"]
    target = contract["source"]["authenticated_target"]
    projection = receipt["studio_projection"]
    shape = receipt["image_shape"]
    view = receipt["kjerag_view"]

    original_pins: list[tuple[PinnedFile, str]] = []
    private_pins: list[tuple[PinnedFile, str]] = []
    try:
        videos: dict[str, PinnedFile] = {}
        for state in ("flow_on", "flow_off"):
            video_receipt = contract["artifacts"][state]["video"]
            pinned = pin_receipt_file(artifact_dir, video_receipt)
            videos[state] = pinned
            original_pins.append((pinned, video_receipt["sha256"]))

        source_inputs = directory.parent / "authenticated-source"
        source_inputs.mkdir()
        sources = []
        for source_receipt in contract["source"]["files"]:
            original = pin_receipt_file(source_dir, source_receipt)
            original_pins.append((original, source_receipt["sha256"]))
            copied = materialize_pinned(
                original,
                source_inputs / source_receipt["name"],
                source_receipt["sha256"],
            )
            private_pins.append((copied, source_receipt["sha256"]))
            sources.append(copied.path)

        video_filter = (
            f"v360=input={projection['input']}:output={projection['output']}:"
            f"yaw={projection['yaw']}:pitch={projection['pitch']}:"
            f"h_fov={projection['horizontal_fov']}:v_fov={projection['vertical_fov']}:"
            f"w={shape[1]}:h={shape[0]}"
        )
        for state, role in (
            ("flow_on", "studio_flow_on"),
            ("flow_off", "studio_flow_off"),
        ):
            source = videos[state]
            output = directory / receipt["images"][role]["name"]
            command(
                [
                    "ffmpeg",
                    "-v",
                    "error",
                    "-y",
                    "-ss",
                    target["seconds"],
                    "-i",
                    f"/proc/self/fd/{source.descriptor}",
                    "-frames:v",
                    "1",
                    "-vf",
                    video_filter,
                    os.fspath(output),
                ],
                pass_fds=(source.descriptor,),
            )

        source = sources[0]
        for frame in receipt["source_candidates"]:
            output = directory / receipt["images"][f"source_{frame}"]["name"]
            command(
                [
                    os.fspath(reframe_bin.resolve()),
                    os.fspath(source),
                    f"frame={frame}",
                    f"yaw={view['yaw']}",
                    f"pitch={view['pitch']}",
                    f"fov={view['fov']}",
                    f"lock={view['lock']}",
                    f"seam={view['seam']}",
                    f"size={shape[1]}",
                    f"out={output}",
                ]
            )
        for pinned, digest in original_pins:
            reverify_file(pinned, digest)
        for pinned, digest in private_pins:
            reverify_file(pinned, digest)
    finally:
        for pinned, _digest in reversed(private_pins):
            pinned.close()
        for pinned, _digest in reversed(original_pins):
            pinned.close()


def mutual_ratio_matches(
    matcher: Any,
    candidate_desc: np.ndarray,
    studio_desc: np.ndarray,
    ratio: float,
) -> list[Any]:
    forward_pairs = matcher.knnMatch(candidate_desc, studio_desc, k=2)
    reverse_pairs = matcher.knnMatch(studio_desc, candidate_desc, k=2)
    forward = {
        first.queryIdx: first
        for first, second in forward_pairs
        if first.distance < ratio * second.distance
    }
    reverse = {
        first.queryIdx: first
        for first, second in reverse_pairs
        if first.distance < ratio * second.distance
    }
    return [
        match
        for candidate_index, match in forward.items()
        if match.trainIdx in reverse and reverse[match.trainIdx].trainIdx == candidate_index
    ]


def authenticate(directory: Path, contract: dict[str, Any]) -> None:
    receipt = contract["target_picture_authentication"]
    same("OpenCV version", cv2.__version__, receipt["opencv_version"])
    same(
        "target picture frame",
        receipt["studio_frame_index"],
        contract["source"]["authenticated_target"]["frame_index"],
    )
    same("source candidates", receipt["source_candidates"], [6368, 6369, 6370])
    same("source winner", receipt["unique_source_winner"], receipt["studio_frame_index"])
    same(
        "held-out method",
        receipt["method"],
        {
            "studio_x_half_open": [16, 700],
            "studio_y_bands_half_open": {
                "A_north": [32, 480],
                "B_south": [544, 992],
            },
            "sift_nfeatures": 12000,
            "sift_contrast_threshold": 0.01,
            "sift_edge_threshold": 12,
            "ratio_threshold": 0.7,
            "bidirectional_mutual_matches": True,
            "rng_seed": 20260830,
            "ransac_threshold": 2.0,
            "held_out_thresholds": [2.0, 3.0],
        },
    )

    images = {
        role: load_image(directory, image_receipt)
        for role, image_receipt in receipt["images"].items()
    }
    shape = tuple(receipt["image_shape"])
    for role, image in images.items():
        same(f"{role} image shape", image.shape, shape)

    method = receipt["method"]
    sift = cv2.SIFT_create(
        nfeatures=method["sift_nfeatures"],
        contrastThreshold=method["sift_contrast_threshold"],
        edgeThreshold=method["sift_edge_threshold"],
    )
    matcher = cv2.BFMatcher(cv2.NORM_L2)
    measured: dict[str, dict[str, dict[str, Any]]] = {}

    for flow in ("flow_on", "flow_off"):
        studio = images[f"studio_{flow}"]
        studio_mask = np.zeros_like(studio)
        x0, x1 = method["studio_x_half_open"]
        studio_mask[:, x0:x1] = 255
        studio_keys, studio_desc = sift.detectAndCompute(studio, studio_mask)
        if studio_desc is None:
            raise Refusal(f"{flow} produced no Studio descriptor set")
        measured[flow] = {}

        for frame in receipt["source_candidates"]:
            candidate_keys, candidate_desc = sift.detectAndCompute(images[f"source_{frame}"], None)
            if candidate_desc is None:
                raise Refusal(f"source {frame} produced no descriptor set")
            matches = mutual_ratio_matches(
                matcher,
                candidate_desc,
                studio_desc,
                method["ratio_threshold"],
            )
            source = np.float32([candidate_keys[match.queryIdx].pt for match in matches])
            target = np.float32([studio_keys[match.trainIdx].pt for match in matches])
            bands = {
                name: (target[:, 1] >= bounds[0]) & (target[:, 1] < bounds[1])
                for name, bounds in method["studio_y_bands_half_open"].items()
            }
            result: dict[str, Any] = {
                "mutual": len(matches),
                "band_matches": [int(bands["A_north"].sum()), int(bands["B_south"].sum())],
            }
            for name, train_name, held_name in (
                ("A_to_B", "A_north", "B_south"),
                ("B_to_A", "B_south", "A_north"),
            ):
                train = bands[train_name]
                held = bands[held_name]
                cv2.setRNGSeed(method["rng_seed"])
                matrix, inliers = cv2.findHomography(
                    source[train].reshape(-1, 1, 2),
                    target[train].reshape(-1, 1, 2),
                    cv2.RANSAC,
                    method["ransac_threshold"],
                    maxIters=10000,
                    confidence=0.999,
                )
                if matrix is None or inliers is None:
                    raise Refusal(f"{flow} source {frame} {name} did not produce a model")
                predicted = cv2.perspectiveTransform(
                    source[held].reshape(-1, 1, 2), matrix
                ).reshape(-1, 2)
                error = np.linalg.norm(predicted - target[held], axis=1)
                result[name] = [
                    int(inliers.sum()),
                    *(
                        int((error < threshold).sum())
                        for threshold in method["held_out_thresholds"]
                    ),
                ]
            measured[flow][f"source_{frame}"] = result

    same("held-out picture receipt", measured, receipt["results"])
    winner = receipt["unique_source_winner"]
    for flow in ("flow_on", "flow_off"):
        selected = measured[flow][f"source_{winner}"]
        for decoy in receipt["source_candidates"]:
            decoy_mutual = measured[flow][f"source_{decoy}"]["mutual"]
            if decoy != winner and not selected["mutual"] > decoy_mutual:
                raise Refusal(f"{flow} mutual-match count does not uniquely select source {winner}")
        for direction in ("A_to_B", "B_to_A"):
            for offset, threshold in enumerate(
                method["held_out_thresholds"], start=1
            ):
                selected_count = selected[direction][offset]
                for decoy in receipt["source_candidates"]:
                    decoy_count = measured[flow][f"source_{decoy}"][direction][offset]
                    if decoy != winner and not selected_count > decoy_count:
                        raise Refusal(
                            f"{flow} {direction} held-out count at {threshold}px "
                            f"does not uniquely select source {winner}"
                        )

    print(
        "verified target picture: candidate-symmetric disjoint-band fits uniquely select "
        "source frame 6369 against 6368/6370 in both Studio exports"
    )
    print(
        "claim boundary: held-out local source/output time evidence only; no stitch, "
        "view, stabilization, playback, seam-parity or fix semantic"
    )


def parse() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Reproduce the Studio frame-6369 adjacent-frame decoy receipt.",
        allow_abbrev=False,
    )
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--picture-dir", type=Path)
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--source-dir", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse()
    try:
        contract = load_canonical_contract(args.contract)
        regenerate_args = (args.artifact_dir, args.source_dir)
        if args.picture_dir is not None:
            if any(value is not None for value in regenerate_args):
                raise Refusal("--picture-dir cannot be combined with regeneration arguments")
            authenticate(args.picture_dir, contract)
        else:
            if any(value is None for value in regenerate_args):
                raise Refusal(
                    "supply --picture-dir, or both --artifact-dir and --source-dir"
                )
            scratch = ROOT / "scratch"
            try:
                scratch.mkdir(exist_ok=True)
                if scratch.is_symlink() or not scratch.is_dir():
                    raise Refusal(f"scratch root is not a regular nonsymlink directory: {scratch}")
            except OSError as error:
                raise Refusal(f"cannot prepare scratch root {scratch}: {error}") from error
            with tempfile.TemporaryDirectory(
                prefix="studio-picture-verify-", dir=scratch
            ) as temporary:
                work = Path(temporary)
                directory = work / "pictures"
                directory.mkdir()
                reframe_bin = build_reframe(work, contract)
                regenerate(
                    directory,
                    contract,
                    args.artifact_dir,
                    args.source_dir,
                    reframe_bin,
                )
                authenticate(directory, contract)
        return 0
    except (KeyError, TypeError, ValueError, Refusal) as error:
        print(f"REFUSED: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
