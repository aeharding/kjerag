#!/usr/bin/env python3
"""CPU-only negative controls for the real hover-layout log check."""

import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "hover_layout", Path(__file__).with_name("check-hover-layout.py")
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def events(shown_y=0, shown_height=720, changed_camera=False):
    result = []
    for x in [620, 640, 660]:
        for event, y, height in [
            (f"CursorMoved {{ position: Point {{ x: {x}.0, y: 350.0 }} }}", 0, 720),
            ("CursorLeft", shown_y, shown_height),
        ]:
            yaw = 1 if changed_camera and event == "CursorLeft" else 0
            result.append(
                f"native-input: Mouse({event}), dragging=false, "
                f"camera=Camera {{ yaw: {yaw}.0, pitch: 0.0, fov: 1.5707964 }}, "
                f"bounds=Rectangle {{ x: 0.0, y: {y}.0, width: 1280.0, height: {height}.0 }}, "
                "cursor=Unavailable"
            )
    return "\n".join(result)


class HoverLayout(unittest.TestCase):
    def test_same_full_window_projection_passes(self):
        self.assertTrue(checker.check(events())["passed"])

    def test_released_header_geometry_fails_even_with_unchanged_camera(self):
        result = checker.check(events(shown_y=48, shown_height=672))
        self.assertFalse(result["passed"])
        self.assertEqual(len(result["failures"]), 3)
        self.assertTrue(all("projection changed" in why for why in result["failures"]))

    def test_missing_video_input_does_not_pass_vacuously(self):
        self.assertFalse(checker.check("media: opened\nplay: playing\n")["passed"])

    def test_an_unchanged_rectangle_does_not_hide_a_camera_jump(self):
        self.assertFalse(checker.check(events(changed_camera=True))["passed"])


if __name__ == "__main__":
    unittest.main()
