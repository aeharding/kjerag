#!/usr/bin/env python3
"""Check that showing controls does not pause the native redraw pump."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
from typing import Any, TextIO


WINDOW_NS = 1_000_000_000
MAX_GAP_NS = 100_000_000
MIN_PUMP_COUNT = 10
PUMP_PREFIX = "native-pump:"
CURSOR = re.compile(
    r"^native-input: Mouse\(CursorMoved\b.*\), dragging=(true|false),"
)


def failure(message: str, line: int | None = None) -> dict[str, Any]:
    result: dict[str, Any] = {"message": message}
    if line is not None:
        result["line"] = line
    return result


def pump_timestamp(line: str) -> int:
    payload = line[len(PUMP_PREFIX) :].strip()
    try:
        record = json.loads(payload)
    except json.JSONDecodeError as error:
        raise ValueError(f"malformed native-pump JSON: {error.msg}") from error
    if not isinstance(record, dict):
        raise ValueError("native-pump payload is not a JSON object")
    timestamp = record.get("entered_unix_ns")
    if isinstance(timestamp, bool) or not isinstance(timestamp, int):
        raise ValueError("native-pump entered_unix_ns is missing or is not an integer")
    if timestamp < 0:
        raise ValueError("native-pump entered_unix_ns is negative")
    return timestamp


def new_wake(line_number: int) -> dict[str, Any]:
    return {
        "event_line": line_number,
        "anchor_line": None,
        "anchor_unix_ns": None,
        "closure_line": None,
        "closure_unix_ns": None,
        "pumps": [],
    }


def analyze_stream(source: TextIO, name: str) -> dict[str, Any]:
    wakes: list[dict[str, Any]] = []
    pending: list[dict[str, Any]] = []
    active: list[dict[str, Any]] = []
    errors: list[dict[str, Any]] = []
    previous_pump: tuple[int, int] | None = None
    pump_count = 0

    for line_number, raw_line in enumerate(source, 1):
        line = raw_line.rstrip("\r\n")
        cursor = CURSOR.match(line)
        if cursor and cursor.group(1) == "false":
            wake = new_wake(line_number)
            wakes.append(wake)
            pending.append(wake)

        if not line.startswith(PUMP_PREFIX):
            continue
        pump_count += 1
        try:
            timestamp = pump_timestamp(line)
        except ValueError as error:
            errors.append(failure(str(error), line_number))
            continue

        if previous_pump is not None and timestamp <= previous_pump[1]:
            errors.append(
                failure(
                    "native-pump entered_unix_ns did not strictly increase "
                    f"after line {previous_pump[0]}",
                    line_number,
                )
            )
        previous_pump = (line_number, timestamp)

        still_active: list[dict[str, Any]] = []
        for wake in active:
            wake["pumps"].append((line_number, timestamp))
            if timestamp > wake["anchor_unix_ns"] + WINDOW_NS:
                wake["closure_line"] = line_number
                wake["closure_unix_ns"] = timestamp
            else:
                still_active.append(wake)
        active = still_active

        for wake in pending:
            wake["anchor_line"] = line_number
            wake["anchor_unix_ns"] = timestamp
            wake["pumps"].append((line_number, timestamp))
            active.append(wake)
        pending.clear()

    if not wakes:
        errors.append(failure("no non-drag CursorMoved wake was found"))

    wake_summaries = []
    for wake in wakes:
        wake_errors: list[str] = []
        pumps: list[tuple[int, int]] = wake.pop("pumps")
        if wake["anchor_line"] is None:
            wake_errors.append("no native-pump follows this wake")
        elif wake["closure_line"] is None:
            wake_errors.append("log ends before the full follow-up window closes")
        elif len(pumps) < MIN_PUMP_COUNT:
            wake_errors.append(
                f"only {len(pumps)} pumps cover the follow-up window; "
                f"at least {MIN_PUMP_COUNT} are required"
            )

        gaps = [
            (right[1] - left[1], left[0], right[0])
            for left, right in zip(pumps, pumps[1:])
        ]
        maximum = max(gaps, default=None)
        if maximum is not None and maximum[0] > MAX_GAP_NS:
            wake_errors.append(
                f"maximum pump gap {maximum[0] / 1_000_000:.3f} ms exceeds "
                f"{MAX_GAP_NS / 1_000_000:.3f} ms"
            )

        wake_summary = {
            **wake,
            "pump_count": len(pumps),
            "max_gap_ms": None if maximum is None else maximum[0] / 1_000_000,
            "max_gap_from_line": None if maximum is None else maximum[1],
            "max_gap_to_line": None if maximum is None else maximum[2],
            "status": "pass" if not wake_errors else "fail",
            "errors": wake_errors,
        }
        wake_summaries.append(wake_summary)

    passed = not errors and all(wake["status"] == "pass" for wake in wake_summaries)
    return {
        "path": name,
        "status": "pass" if passed else "fail",
        "pump_count": pump_count,
        "wake_count": len(wakes),
        "wakes": wake_summaries,
        "errors": errors,
    }


def analyze_path(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as source:
            return analyze_stream(source, str(path))
    except (OSError, UnicodeError) as error:
        return {
            "path": str(path),
            "status": "fail",
            "pump_count": 0,
            "wake_count": 0,
            "wakes": [],
            "errors": [failure(f"cannot read log: {error}")],
        }


def report(paths: list[Path]) -> dict[str, Any]:
    logs = [analyze_path(path) for path in paths]
    passed = all(log["status"] == "pass" for log in logs)
    return {
        "schema_version": 1,
        "check": "controls-wake-native-pump",
        "status": "pass" if passed else "fail",
        "window_ms": WINDOW_NS / 1_000_000,
        "maximum_gap_ms": MAX_GAP_NS / 1_000_000,
        "minimum_pump_count": MIN_PUMP_COUNT,
        "bound_basis": (
            "Three 30 fps source-frame periods; this is a controls-wake regression "
            "bound, not a 240 Hz rendering-capacity claim."
        ),
        "logs": logs,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path, nargs="+", help="native player log")
    args = parser.parse_args(argv)
    result = report(args.log)
    json.dump(result, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
