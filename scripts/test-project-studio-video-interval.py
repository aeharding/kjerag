#!/usr/bin/env python3

import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest import mock


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "project_studio_interval", HERE / "project-studio-video-interval.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def contract():
    return {
        "schema_version": 1,
        "producer": {"application": "Insta360 Studio"},
        "media_contract": {"video": {"time_base": "1/30000"}},
        "packet_grid": {
            "first_index": 0,
            "last_index": 9,
            "step": 1001,
            "duration": 1001,
        },
    }


class Tests(unittest.TestCase):
    def test_vertical_fov_preserves_square_and_derives_wide_view(self):
        self.assertAlmostEqual(MODULE.vertical_fov(57.95, 1024, 1024), 57.95)
        expected = math.degrees(
            2 * math.atan(math.tan(math.radians(57.95) / 2) / (16 / 9))
        )
        self.assertAlmostEqual(MODULE.vertical_fov(57.95, 3840, 2160), expected)

    def test_exact_frame_grid_and_names(self):
        frames = MODULE.expected_frames(contract(), 3, 3)
        self.assertEqual([frame["index"] for frame in frames], [3, 4, 5])
        self.assertEqual([frame["pts"] for frame in frames], [3003, 4004, 5005])
        self.assertEqual(frames[0]["png"], "frame-00000003.png")
        self.assertEqual(frames[0]["seconds"], "0.100100000")

    def test_zero_count_and_out_of_range_refuse(self):
        with self.assertRaisesRegex(MODULE.Refusal, "greater than zero"):
            MODULE.expected_frames(contract(), 3, 0)
        with self.assertRaisesRegex(MODULE.Refusal, "outside"):
            MODULE.expected_frames(contract(), 9, 2)

    def test_showinfo_requires_exact_consecutive_pts(self):
        frames = MODULE.expected_frames(contract(), 3, 2)
        exact = b"""[Parsed_showinfo_2 @ 0x1] config in time_base: 1/30000, frame_rate: 30000/1001
[Parsed_showinfo_2 @ 0x1] n:   0 pts:   3003 pts_time:0.1001
[Parsed_showinfo_2 @ 0x1] n:   1 pts:   4004 pts_time:0.133467
"""
        MODULE.parse_showinfo(exact, frames, "1/30000")
        wrong = exact.replace(b"pts:   4004", b"pts:   5005")
        with self.assertRaisesRegex(MODULE.Refusal, "sequence changed"):
            MODULE.parse_showinfo(wrong, frames, "1/30000")

    def test_atomic_receipt_and_png_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = root / "receipt.json"
            with mock.patch.object(MODULE.os, "fsync", wraps=os.fsync) as sync:
                MODULE.atomic_json(receipt, {"schema": "test", "value": 1})
                self.assertEqual(sync.call_count, 2)
            self.assertEqual(json.loads(receipt.read_text()), {"schema": "test", "value": 1})

            png = root / "frame.png"
            png.write_bytes(
                MODULE.PNG_SIGNATURE
                + struct.pack(">I", 13)
                + b"IHDR"
                + struct.pack(">II", 3840, 2160)
            )
            self.assertEqual(MODULE.png_dimensions(png), (3840, 2160))

    def test_destination_appeared_race_refuses_without_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            staging = root / "staging"
            staging.mkdir()
            (staging / "new").write_text("new")
            destination = root / "destination"
            destination.mkdir()
            (destination / "owner").write_text("owner")
            with self.assertRaisesRegex(MODULE.Refusal, "already exists"):
                MODULE.rename_noreplace(staging, destination)
            self.assertTrue(staging.is_dir())
            self.assertEqual((destination / "owner").read_text(), "owner")
            self.assertFalse((destination / "new").exists())

    def test_noreplace_publication_and_sync_helpers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            staging = root / "staging"
            staging.mkdir()
            leaf = staging / "leaf"
            leaf.write_bytes(b"durable")
            with mock.patch.object(MODULE.os, "fsync", wraps=os.fsync) as sync:
                MODULE.fsync_regular_file(leaf)
                MODULE.fsync_directory(staging)
                self.assertEqual(sync.call_count, 2)
            destination = root / "destination"
            MODULE.rename_noreplace(staging, destination)
            MODULE.fsync_directory(root)
            self.assertEqual((destination / "leaf").read_bytes(), b"durable")

    def test_pinned_path_replacement_refuses(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "input"
            path.write_bytes(b"pinned")
            pinned = MODULE.pin_regular_file(path, "test input")
            try:
                replacement = root / "replacement"
                replacement.write_bytes(b"replacement")
                replacement.replace(path)
                self.assertEqual(pinned.raw, b"pinned")
                with self.assertRaisesRegex(MODULE.Refusal, "changed during projection"):
                    MODULE.require_pinned_unchanged(pinned, "test input")
            finally:
                pinned.close()

    def test_contract_and_verifier_require_canonical_bytes(self):
        contract_pin = MODULE.pin_regular_file(
            MODULE.DEFAULT_CONTRACT, "oracle contract"
        )
        verifier_pin = MODULE.pin_regular_file(MODULE.VERIFIER, "oracle verifier")
        try:
            MODULE.require_canonical(
                contract_pin,
                MODULE.CANONICAL_CONTRACT_SHA256,
                "oracle contract",
            )
            MODULE.require_canonical(
                verifier_pin,
                MODULE.CANONICAL_VERIFIER_SHA256,
                "oracle verifier",
            )
        finally:
            contract_pin.close()
            verifier_pin.close()

        with tempfile.TemporaryDirectory() as directory:
            altered = Path(directory) / "contract.json"
            altered.write_bytes(MODULE.DEFAULT_CONTRACT.read_bytes() + b"\n")
            altered_pin = MODULE.pin_regular_file(altered, "oracle contract")
            try:
                with self.assertRaisesRegex(MODULE.Refusal, "not the canonical"):
                    MODULE.require_canonical(
                        altered_pin,
                        MODULE.CANONICAL_CONTRACT_SHA256,
                        "oracle contract",
                    )
            finally:
                altered_pin.close()

    def test_authentication_executes_private_pinned_bytes(self):
        verifier = b"""#!/usr/bin/env python3
import argparse
import hashlib
from pathlib import Path
parser = argparse.ArgumentParser()
parser.add_argument('--contract')
parser.add_argument('--artifact-dir')
args = parser.parse_args()
print(hashlib.sha256(Path(args.contract).read_bytes()).hexdigest())
"""
        contract_raw = b'{"pinned":true}\n'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            work = root / "private"
            work.mkdir()
            output = MODULE.authenticate(verifier, contract_raw, work, root)
            self.assertEqual(
                output.decode().strip(),
                MODULE.hashlib.sha256(contract_raw).hexdigest(),
            )
            self.assertEqual(list(work.iterdir()), [])

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg is not installed")
    def test_real_ffmpeg_selects_exact_frame_numbers_and_pts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            video = root / "input.mp4"
            generated = subprocess.run(
                [
                    "ffmpeg",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=64x32:rate=30000/1001:duration=0.2",
                    "-c:v",
                    "mpeg4",
                    "-video_track_timescale",
                    "30000",
                    str(video),
                ],
                capture_output=True,
                check=False,
            )
            self.assertEqual(generated.returncode, 0, generated.stderr.decode())
            output = root / "output"
            output.mkdir()
            frames = MODULE.expected_frames(contract(), 1, 2)
            descriptor = os.open(video, os.O_RDONLY)
            try:
                arguments = MODULE.project(
                    descriptor,
                    output,
                    frames,
                    0.0,
                    0.0,
                    60.0,
                    MODULE.vertical_fov(60.0, 64, 36),
                    64,
                    36,
                    "1/30000",
                )
            finally:
                os.close(descriptor)
            self.assertEqual(
                sorted(path.name for path in output.iterdir()),
                ["frame-00000001.png", "frame-00000002.png"],
            )
            normalized = MODULE.receipt_arguments(arguments, "oracle.mp4")
            self.assertEqual(normalized[normalized.index("-i") + 1], "oracle.mp4")
            self.assertEqual(normalized[-1], "frame-%08d.png")


if __name__ == "__main__":
    unittest.main()
