#!/usr/bin/env python3

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("owner_three_panel", HERE / "build-owner-three-panel-review.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fake_png(path):
    path.write_bytes(MODULE.PNG_SIGNATURE + struct.pack(">I", 13) + b"IHDR" + struct.pack(">II", MODULE.WIDTH, MODULE.HEIGHT))


def leaf(path, key="file"):
    return {key: path.name, "bytes": path.stat().st_size, "sha256": sha(path)}


def fixtures(root):
    kdir, sdir, tdir = root / "kjerag", root / "studio", root / "trace"
    for path in (kdir, sdir, tdir): path.mkdir()
    kframes, sframes, tframes = [], [], []
    source = [
        {"path": "/source/00.insv", "bytes": 10, "sha256": "1" * 64,
         "stable_identity": {"device": 1, "inode": 2}, "decoder_lane": 0, "picked": True},
        {"path": "/source/10.insv", "bytes": 11, "sha256": "2" * 64,
         "stable_identity": {"device": 1, "inode": 3}, "decoder_lane": 1, "picked": False},
    ]
    request = {"start": MODULE.START, "count": MODULE.COUNT, "end_inclusive": MODULE.END,
               "no_seek": True, "cold_start_at_range_boundary": False,
               "every_source_frame_consumed": True, "captured_map_substitution": False}
    view = {"yaw_radians": 1.0, "pitch_radians": -0.2, "fov_radians": 1.1,
            "yaw_degrees": 71.12999725341797, "pitch_degrees": -13.989999771118164,
            "fov_degrees": 57.95000457763672, "horizon_locked": True, "readout": "file",
            "sampling": "Sharp", "seam_band": True, "exposure_tone": True,
            "capture_width": MODULE.WIDTH, "capture_height": MODULE.HEIGHT}
    for index in range(MODULE.START, MODULE.END + 1):
        kp = kdir / f"frame-{index:010d}.png"; sp = sdir / f"frame-{index:08d}.png"; tp = tdir / f"frame-{index:010d}-trace.png"
        fake_png(kp); fake_png(sp); fake_png(tp)
        packed = kdir / f"frame-{index:010d}.packed-f32le.bin"
        alpha = kdir / f"frame-{index:010d}.alpha-f32le.bin"
        packed.write_bytes(bytes(320_000)); alpha.write_bytes(bytes(80_000))
        total_ns = index * MODULE.PTS_STEP * MODULE.NANOS // 30_000
        kframes.append({"index": index, "timestamp_seconds": total_ns // MODULE.NANOS,
                        "timestamp_nanoseconds": total_ns % MODULE.NANOS,
                        "image": {**leaf(kp), "width": MODULE.WIDTH, "height": MODULE.HEIGHT},
                        "production_map": {"packed": leaf(packed), "alpha": leaf(alpha)}})
        sframes.append({"index": index, "pts": index * MODULE.PTS_STEP, "time_base": "1/30000",
                        "seconds": f"{index * MODULE.PTS_STEP / 30000:.9f}", **leaf(sp, "png")})
        tframes.append({"index": index, "timestamp_seconds": total_ns // MODULE.NANOS,
                        "timestamp_nanoseconds": total_ns % MODULE.NANOS, "base_png_sha256": sha(kp),
                        "packed_sha256": sha(packed), "alpha_sha256": sha(alpha),
                        "trace": leaf(tp)})
    build = {"dirty_at_build": False, "runtime_tracked_tree_clean": True, "runtime_git_commit": "a" * 40}
    run = {"presented": MODULE.END + 1, "dropped": 0, "starved": 0, "scene_redraws": MODULE.END + 1}
    kr = {"schema": MODULE.KJERAG_SCHEMA, "claim": MODULE.KJERAG_CLAIM, "request": request,
          "view": view, "source": source, "build": build, "frames": kframes, "run": run}
    kreceipt = kdir / "range-receipt.json"; kreceipt.write_text(json.dumps(kr))
    sr = {"schema": MODULE.STUDIO_SCHEMA, "authentication": MODULE.CANONICAL_AUTH,
          "source": MODULE.CANONICAL_STUDIO_SOURCE,
          "projection": {"input": "equirect", "output": "flat", "width": MODULE.WIDTH, "height": MODULE.HEIGHT,
                         "yaw": view["yaw_degrees"], "pitch": view["pitch_degrees"],
                         "horizontal_fov": view["fov_degrees"]},
          "interval": {"start_frame": MODULE.START, "count": MODULE.COUNT, "end_frame": MODULE.END, "frames": sframes},
          "extraction": {"temporal_interpolation": False}}
    sreceipt = sdir / "projection-receipt.json"; sreceipt.write_text(json.dumps(sr))
    trace_source = [{k: v for k, v in item.items() if k != "picked"} for item in source]
    trace_view = {**view, "seam": "factory", "width": MODULE.WIDTH, "height": MODULE.HEIGHT}
    tr = {"schema": MODULE.TRACE_SCHEMA, "claim": MODULE.TRACE_CLAIM,
          "input_receipt": {"sha256": sha(kreceipt), "schema": MODULE.KJERAG_SCHEMA, "claim": MODULE.KJERAG_CLAIM},
          "request": request, "view": trace_view, "source": trace_source, "build": {"trace": True},
          "input_run": run, "selected_frames": list(range(MODULE.START, MODULE.END + 1)), "frames": tframes}
    treceipt = tdir / "range-trace-receipt.json"; treceipt.write_text(json.dumps(tr))
    return kreceipt, sreceipt, treceipt


def pinned(path, label):
    return MODULE.PinnedReceipt(path, sha(path), label)


class Tests(unittest.TestCase):
    def test_exact_receipts_leaves_and_bindings_authenticate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); kr, sr, tr = fixtures(root)
            inputs = root / "inputs"; inputs.mkdir()
            for name in ("k", "s", "t"): (inputs / name).mkdir()
            kp, sp, tp = pinned(kr, "Kjerag"), pinned(sr, "Studio"), pinned(tr, "trace")
            try:
                k = MODULE.validate_kjerag(kp, inputs / "k")
                s = MODULE.validate_studio(sp, inputs / "s", k)
                t = MODULE.validate_trace(tp, inputs / "t", kp, k)
                self.assertEqual(len(k["frames"]), 61); self.assertEqual(len(s["frames"]), 61); self.assertEqual(len(t["frames"]), 61)
            finally:
                kp.close(); sp.close(); tp.close()

    def test_tampered_leaf_refuses(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); kr, _sr, _tr = fixtures(root)
            (kr.parent / f"frame-{MODULE.START:010d}.png").write_bytes(b"tampered")
            receipt = pinned(kr, "Kjerag")
            try:
                with self.assertRaisesRegex(MODULE.Refusal, "size changed|SHA-256 changed"):
                    MODULE.validate_kjerag(receipt, root / "copy")
            finally: receipt.close()

    def test_canonical_oracle_tamper_refuses(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); kr, sr, _tr = fixtures(root)
            value = json.loads(sr.read_text()); value["authentication"]["producer"]["version"] = "6.0.3"; sr.write_text(json.dumps(value))
            kp, sp = pinned(kr, "Kjerag"), pinned(sr, "Studio")
            (root / "k").mkdir(); (root / "s").mkdir()
            try:
                k = MODULE.validate_kjerag(kp, root / "k")
                with self.assertRaisesRegex(MODULE.Refusal, "canonical 6.0.2"):
                    MODULE.validate_studio(sp, root / "s", k)
            finally: kp.close(); sp.close()

    def test_fabricated_receipt_and_orange_png_refuse_private_derivation(self):
        supplied = {"projection": {"yaw": 71.13}, "frames": []}
        derived = {"projection": {"yaw": 71.13}, "frames": []}
        for index in range(MODULE.START, MODULE.END + 1):
            base = {"index": index, "pts": index * MODULE.PTS_STEP, "time_base": "1/30000", "bytes": 24}
            supplied["frames"].append({**base, "sha256": hashlib.sha256(b"orange").hexdigest()})
            derived["frames"].append({**base, "sha256": hashlib.sha256(b"canonical Studio pixels").hexdigest()})
        with self.assertRaisesRegex(MODULE.Refusal, "differs from canonical private derivation"):
            MODULE.compare_studio_derivation(supplied, derived)

    def test_trace_source_binding_tamper_refuses(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); kr, _sr, tr = fixtures(root)
            value = json.loads(tr.read_text()); value["source"][0]["sha256"] = "f" * 64; tr.write_text(json.dumps(value))
            kp, tp = pinned(kr, "Kjerag"), pinned(tr, "trace")
            (root / "k").mkdir(); (root / "t").mkdir()
            try:
                k = MODULE.validate_kjerag(kp, root / "k")
                with self.assertRaisesRegex(MODULE.Refusal, "source binding"):
                    MODULE.validate_trace(tp, root / "t", kp, k)
            finally: kp.close(); tp.close()

    def test_destination_race_refuses_without_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / "stage"; destination = root / "final"
            source.mkdir(); (source / "new").write_text("new")
            destination.mkdir(); (destination / "owner").write_text("owner")
            with self.assertRaisesRegex(MODULE.Refusal, "already exists"):
                MODULE.rename_noreplace(source, destination)
            self.assertEqual((destination / "owner").read_text(), "owner")
            self.assertFalse((destination / "new").exists())

    def test_filter_has_permanent_footer_and_no_registration(self):
        graph = MODULE.filter_graph(Path("/font.ttf"))
        self.assertIn(MODULE.FOOTER, graph)
        self.assertIn("crop=2000:2160:900:0", graph)
        self.assertIn("crop=2000:2160:1840:0", graph)
        self.assertNotIn("rotate=", graph)
        self.assertNotIn("perspective=", graph)


if __name__ == "__main__": unittest.main()
