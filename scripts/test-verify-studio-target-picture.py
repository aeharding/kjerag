#!/usr/bin/env python3

import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import numpy as np


sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "verify_studio_target_picture", HERE / "verify-studio-target-picture.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Tests(unittest.TestCase):
    def test_contract_requires_canonical_committed_bytes(self):
        contract = MODULE.load_canonical_contract(MODULE.DEFAULT_CONTRACT)
        self.assertEqual(contract["schema_version"], 1)
        with tempfile.TemporaryDirectory() as directory:
            altered = Path(directory) / "contract.json"
            altered.write_bytes(MODULE.DEFAULT_CONTRACT.read_bytes() + b"\n")
            with self.assertRaisesRegex(MODULE.Refusal, "SHA-256 changed"):
                MODULE.load_canonical_contract(altered)

    def test_opencv_decodes_authenticated_bytes_not_reopened_path(self):
        raw = b"authenticated encoded picture bytes"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "picture.png"
            path.write_bytes(raw)
            receipt = {
                "name": path.name,
                "bytes": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
            }
            decoded = np.array([[17]], dtype=np.uint8)

            def decode(encoded, mode):
                self.assertEqual(mode, MODULE.cv2.IMREAD_GRAYSCALE)
                self.assertEqual(encoded.tobytes(), raw)
                path.write_bytes(b"substituted after authentication")
                return decoded

            with mock.patch.object(MODULE.cv2, "imdecode", side_effect=decode):
                self.assertIs(MODULE.load_image(root, receipt), decoded)

    def test_materialized_source_survives_original_path_substitution(self):
        raw = b"authenticated source"
        digest = hashlib.sha256(raw).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = root / "source.insv"
            original.write_bytes(raw)
            original_pin = MODULE.pin_regular_file(
                original, "source", expected_sha256=digest
            )
            private = root / "private.insv"
            private_pin = MODULE.materialize_pinned(original_pin, private, digest)
            try:
                replacement = root / "replacement.insv"
                replacement.write_bytes(b"replacement")
                replacement.replace(original)
                with self.assertRaisesRegex(MODULE.Refusal, "changed during use"):
                    MODULE.reverify_file(original_pin, digest)
                MODULE.reverify_file(private_pin, digest)
                self.assertEqual(private.read_bytes(), raw)
            finally:
                private_pin.close()
                original_pin.close()


if __name__ == "__main__":
    unittest.main()
