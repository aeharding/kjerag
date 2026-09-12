#!/usr/bin/env python3

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "check_controls_wake", HERE / "check-controls-wake.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def pump(milliseconds, payload=None):
    if payload is not None:
        return f"native-pump: {payload}\n"
    return (
        'native-pump: {"entered_unix_ns":'
        f"{1_800_000_000_000_000_000 + milliseconds * 1_000_000}"
        '}\n'
    )


def cursor(dragging=False):
    return (
        "native-input: Mouse(CursorMoved { position: Point { x: 1.0, y: 2.0 } }), "
        f"dragging={'true' if dragging else 'false'}, camera=Camera {{}}\n"
    )


def regular_window(start=0, step=50):
    return "".join(pump(value) for value in range(start, start + 1001 + step, step))


class Tests(unittest.TestCase):
    def analyze(self, text):
        return MODULE.analyze_stream(io.StringIO(text), "synthetic.log")

    def test_normal_wake_passes_with_first_pump_after_window(self):
        result = self.analyze(cursor() + regular_window())
        self.assertEqual(result["status"], "pass")
        wake = result["wakes"][0]
        self.assertEqual(wake["pump_count"], 22)
        self.assertEqual(wake["max_gap_ms"], 50.0)
        self.assertEqual(
            wake["closure_unix_ns"] - wake["anchor_unix_ns"], 1_050_000_000
        )

    def test_exact_100_ms_gap_passes(self):
        result = self.analyze(cursor() + regular_window(step=100))
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["wakes"][0]["max_gap_ms"], 100.0)

    def test_250_ms_gap_fails(self):
        times = [0, 250] + list(range(300, 1051, 50))
        result = self.analyze(cursor() + "".join(pump(value) for value in times))
        self.assertEqual(result["status"], "fail")
        self.assertEqual(result["wakes"][0]["max_gap_ms"], 250.0)
        self.assertRegex(result["wakes"][0]["errors"][0], "exceeds")

    def test_truncated_window_fails_even_with_many_pumps(self):
        result = self.analyze(
            cursor() + "".join(pump(value) for value in range(0, 1001, 50))
        )
        self.assertEqual(result["status"], "fail")
        self.assertIn("before the full", result["wakes"][0]["errors"][0])

    def test_delayed_closing_pump_is_included_and_fails(self):
        times = list(range(0, 951, 50)) + [1_200]
        result = self.analyze(cursor() + "".join(pump(value) for value in times))
        self.assertEqual(result["status"], "fail")
        wake = result["wakes"][0]
        self.assertEqual(
            wake["closure_unix_ns"] - wake["anchor_unix_ns"], 1_200_000_000
        )
        self.assertEqual(wake["max_gap_ms"], 250.0)

    def test_wake_without_a_following_pump_fails(self):
        result = self.analyze(pump(0) + cursor() + "unrelated output\n")
        self.assertEqual(result["status"], "fail")
        self.assertTrue(
            any(
                "no native-pump follows" in error
                for error in result["wakes"][0]["errors"]
            )
        )

    def test_log_without_any_pump_or_wake_fails(self):
        result = self.analyze("ordinary output\n")
        self.assertEqual(result["status"], "fail")
        self.assertEqual(result["wake_count"], 0)
        self.assertIn(
            "no non-drag CursorMoved wake was found", result["errors"][0]["message"]
        )

    def test_malformed_and_missing_timestamps_fail(self):
        malformed = self.analyze(cursor() + pump(0, "{broken") + regular_window())
        self.assertEqual(malformed["status"], "fail")
        self.assertIn("malformed native-pump JSON", malformed["errors"][0]["message"])

        missing = self.analyze(
            cursor() + pump(0, '{"request":"at"}') + regular_window()
        )
        self.assertEqual(missing["status"], "fail")
        self.assertIn("is missing", missing["errors"][0]["message"])

    def test_negative_and_boolean_timestamps_fail(self):
        for payload, message in (
            ('{"entered_unix_ns":-1}', "negative"),
            ('{"entered_unix_ns":true}', "not an integer"),
        ):
            with self.subTest(payload=payload):
                result = self.analyze(cursor() + pump(0, payload) + regular_window())
                self.assertEqual(result["status"], "fail")
                self.assertIn(message, result["errors"][0]["message"])

    def test_regressing_pump_timestamps_fail(self):
        text = cursor() + pump(0) + pump(50) + pump(25) + regular_window(100)
        result = self.analyze(text)
        self.assertEqual(result["status"], "fail")
        self.assertIn("did not strictly increase", result["errors"][0]["message"])

    def test_multiple_wakes_and_dragged_cursor_exclusion(self):
        text = cursor(dragging=True) + cursor() + regular_window()
        text += cursor() + regular_window(2_000)
        result = self.analyze(text)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["wake_count"], 2)

    def test_overlapping_wake_windows_are_checked_independently(self):
        text = cursor() + pump(0) + pump(50) + cursor()
        text += "".join(pump(value) for value in range(100, 1_151, 50))
        result = self.analyze(text)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["wake_count"], 2)
        self.assertEqual([wake["pump_count"] for wake in result["wakes"]], [22, 22])

    def test_sparse_window_fails_reasonable_pump_count(self):
        result = self.analyze(
            cursor() + "".join(pump(value) for value in range(0, 1201, 200))
        )
        self.assertEqual(result["status"], "fail")
        self.assertTrue(
            any("at least 10" in error for error in result["wakes"][0]["errors"])
        )

    def test_cli_emits_json_and_returns_nonzero_on_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "wake.log"
            log.write_text(cursor() + pump(0) + pump(250) + regular_window(300))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                status = MODULE.main([str(log)])
        summary = json.loads(output.getvalue())
        self.assertEqual(status, 1)
        self.assertEqual(summary["status"], "fail")
        self.assertIn("not a 240 Hz rendering-capacity claim", summary["bound_basis"])


if __name__ == "__main__":
    unittest.main()
