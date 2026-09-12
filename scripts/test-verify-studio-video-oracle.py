#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "verify_studio_video_oracle", HERE / "verify-studio-video-oracle.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Tests(unittest.TestCase):
    def test_canonical_contract_is_bound_to_committed_bytes(self):
        contract, pinned = MODULE.load_canonical_contract(MODULE.DEFAULT_CONTRACT)
        try:
            self.assertEqual(contract["schema_version"], 1)
            self.assertEqual(pinned.sha256, MODULE.CANONICAL_CONTRACT_SHA256)
        finally:
            pinned.close()

        with tempfile.TemporaryDirectory() as directory:
            altered = Path(directory) / "contract.json"
            altered.write_bytes(MODULE.DEFAULT_CONTRACT.read_bytes() + b"\n")
            with self.assertRaisesRegex(MODULE.Refusal, "SHA-256 changed"):
                MODULE.load_canonical_contract(altered)

    def test_pinned_artifact_path_replacement_is_detected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "artifact.bin"
            path.write_bytes(b"authenticated")
            pinned = MODULE.pin_regular_file(path, "artifact")
            try:
                replacement = root / "replacement.bin"
                replacement.write_bytes(b"replacement")
                replacement.replace(path)
                with self.assertRaisesRegex(MODULE.Refusal, "changed during verification"):
                    MODULE.reverify_file(pinned, pinned.sha256)
            finally:
                pinned.close()

    def test_target_picture_helper_matches_canonical_hash(self):
        pinned = MODULE.pin_regular_file(
            MODULE.TARGET_PICTURE_HELPER,
            "target-picture verifier",
            expected_sha256=MODULE.CANONICAL_TARGET_PICTURE_HELPER_SHA256,
        )
        try:
            self.assertEqual(
                pinned.sha256,
                MODULE.CANONICAL_TARGET_PICTURE_HELPER_SHA256,
            )
            MODULE.reverify_file(
                pinned, MODULE.CANONICAL_TARGET_PICTURE_HELPER_SHA256
            )
        finally:
            pinned.close()


if __name__ == "__main__":
    unittest.main()
