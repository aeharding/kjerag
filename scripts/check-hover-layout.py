#!/usr/bin/env python3
"""Check the video program's coordinates across three real hover-bar wakes.

The private compositor in uitest-controls-wake.sh has a 1280x720 window.
Mouse input and drawing must both use that full-window projection, not the
shorter content rectangle allocated beneath the visible COSMIC header.
The widget's CPU tests separately check that draw and input receive the same
rectangle. This check observes the actual Scene receiving real pointer events;
it is not a cadence measurement or a claim of identical moving source frames.
"""

import json
from pathlib import Path
import re
import sys


INPUT = re.compile(
    r"^native-input: Mouse\((CursorMoved\b.*|CursorLeft)\), "
    r"dragging=(true|false), camera=(.*), bounds=Rectangle "
    r"\{ x: ([\d.]+), y: ([\d.]+), width: ([\d.]+), height: ([\d.]+) \},"
)


def check(text):
    events = []
    failures = []
    for number, line in enumerate(text.splitlines(), 1):
        match = INPUT.match(line)
        if not match:
            continue
        event, dragging, camera, *coordinates = match.groups()
        bounds = [float(value) for value in coordinates]
        events.append({"line": number, "event": event, "bounds": bounds})
        if dragging != "false":
            failures.append(f"line {number}: hover unexpectedly dragged the camera")
        if bounds != [0.0, 0.0, 1280.0, 720.0]:
            failures.append(f"line {number}: projection changed to {bounds}")
        if len(events) == 1:
            first_camera = camera
        elif camera != first_camera:
            failures.append(f"line {number}: hover changed the camera")
    wakes = sum(event["event"].startswith("CursorMoved") for event in events)
    departures = sum(event["event"] == "CursorLeft" for event in events)
    if wakes != 3 or departures != 3:
        failures.append(f"expected three wakes and departures, found {wakes} and {departures}")
    return {"passed": not failures, "events": events, "failures": failures}


if __name__ == "__main__":
    result = check(Path(sys.argv[1]).read_text())
    print(json.dumps(result, indent=2))
    sys.exit(0 if result["passed"] else 1)
