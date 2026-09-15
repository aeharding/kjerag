#!/usr/bin/env python3
"""CPU-only regressions for uitest.sh's visible-playback decision.

An optional first argument selects an archived uitest.sh.  In particular, the
pre-fix script fails these tests because backdrop-to-picture and chrome-only
changes satisfy its motion probe.
"""

from __future__ import annotations

import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "scripts" / "uitest.sh"
if len(sys.argv) > 1 and sys.argv[1].endswith(".sh"):
    SOURCE = Path(sys.argv.pop(1)).resolve()

WIDTH = 4
HEIGHT = 4
HEADER_BAND = 1
CONTROL_BAND = 1
FLAT = (27, 27, 27)
BLACK = (0, 0, 0)
BRIGHT = (220, 220, 220)


def extract_function(source: str, name: str) -> str | None:
    """Return one top-level Bash function, whose closing brace is unindented."""
    anchor = f"{name}() {{"
    lines = source.splitlines(keepends=True)
    starts = [index for index, line in enumerate(lines) if line.rstrip("\n") == anchor]
    if not starts:
        return None
    if len(starts) != 1:
        raise AssertionError(f"expected one {anchor!r}, found {len(starts)}")
    start = starts[0]
    for end in range(start + 1, len(lines)):
        if lines[end].rstrip("\n") == "}":
            return "".join(lines[start : end + 1])
    raise AssertionError(f"unterminated function {name} in {SOURCE}")


def ppm(path: Path, rows: list[list[tuple[int, int, int]]]) -> None:
    assert len(rows) == HEIGHT and all(len(row) == WIDTH for row in rows)
    pixels = bytes(channel for row in rows for pixel in row for channel in pixel)
    path.write_bytes(f"P6\n{WIDTH} {HEIGHT}\n255\n".encode() + pixels)


def image(body: list[list[tuple[int, int, int]]], edge: tuple[int, int, int] = FLAT):
    return [[edge] * WIDTH, *body, [edge] * WIDTH]


class VisiblePlaybackTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not SOURCE.is_file():
            raise AssertionError(f"source script does not exist: {SOURCE}")
        scratch = ROOT / "scratch"
        scratch.mkdir(exist_ok=True)
        cls.artifact = Path(tempfile.mkdtemp(prefix="uitest-playback.", dir=scratch))

        source = SOURCE.read_text()
        names = (
            "said",
            "picture",
            "same_picture",
            "moving_picture",
            "visible_picture",
            "await_visible_playback",
            "with_media",
        )
        functions = [function for name in names if (function := extract_function(source, name))]
        cls.extracted = cls.artifact / "extracted.sh"
        cls.extracted.write_text("\n".join(functions))
        cls.source_text = source

        flat_body = [[FLAT] * WIDTH for _ in range(HEIGHT - 2)]
        dark_body = [
            [(1, 1, 1), (2, 1, 1), (1, 2, 1), (1, 1, 2)],
            [(2, 2, 1), (2, 1, 2), (1, 2, 2), (3, 2, 1)],
        ]
        other_body = [
            [(1, 1, 1), (8, 1, 1), (1, 8, 1), (1, 1, 8)],
            [(8, 8, 1), (8, 1, 8), (1, 8, 8), (9, 2, 1)],
        ]

        cls.flat = cls.artifact / "flat.ppm"
        cls.black = cls.artifact / "black.ppm"
        cls.bright = cls.artifact / "bright.ppm"
        cls.coloured = cls.artifact / "coloured.ppm"
        cls.video = cls.artifact / "dark-textured.ppm"
        cls.video_copy = cls.artifact / "dark-textured-copy.ppm"
        cls.other = cls.artifact / "other-picture.ppm"
        cls.controls = cls.artifact / "controls-only-change.ppm"
        ppm(cls.flat, image(flat_body))
        ppm(cls.black, [[BLACK] * WIDTH for _ in range(HEIGHT)])
        ppm(cls.bright, [[BRIGHT] * WIDTH for _ in range(HEIGHT)])
        ppm(cls.coloured, [[(7, 23, 91)] * WIDTH for _ in range(HEIGHT)])
        ppm(cls.video, image(dark_body))
        ppm(cls.video_copy, image(dark_body))
        ppm(cls.other, image(other_body))
        ppm(cls.controls, image(dark_body, edge=BRIGHT))

        cls.bad_magic = cls.artifact / "bad-magic.ppm"
        cls.bad_magic.write_bytes(cls.video.read_bytes().replace(b"P6\n", b"P3\n", 1))
        cls.bad_depth = cls.artifact / "bad-depth.ppm"
        cls.bad_depth.write_bytes(cls.video.read_bytes().replace(b"255\n", b"254\n", 1))
        cls.noncanonical = cls.artifact / "noncanonical-header.ppm"
        cls.noncanonical.write_bytes(cls.video.read_bytes().replace(b"4 4\n", b"4  4\n", 1))
        cls.truncated = cls.artifact / "truncated.ppm"
        cls.truncated.write_bytes(cls.video.read_bytes()[:-1])
        cls.trailing = cls.artifact / "trailing.ppm"
        cls.trailing.write_bytes(cls.video.read_bytes() + b"x")

    def bash(self, body: str, timeout: float = 5) -> subprocess.CompletedProcess[str]:
        prelude = f"""
set -uo pipefail
root={shlex.quote(str(ROOT))}
HEADER_BAND={HEADER_BAND}
CONTROL_BAND={CONTROL_BAND}
log={shlex.quote(str(self.artifact / 'probe.log'))}
: >"$log"
source {shlex.quote(str(self.extracted))}
"""
        return subprocess.run(
            ["bash", "-c", prelude + body],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            env={**os.environ, "LC_ALL": "C"},
            check=False,
        )

    def assert_shell(self, body: str, expected: int = 0) -> subprocess.CompletedProcess[str]:
        result = self.bash(body)
        self.assertEqual(
            result.returncode,
            expected,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}\nartifacts: {self.artifact}",
        )
        return result

    def test_visible_picture_rejects_every_flat_colour(self) -> None:
        for path in (self.flat, self.black, self.bright, self.coloured):
            with self.subTest(path=path.name):
                self.assert_shell(f"! visible_picture {shlex.quote(str(path))}\n")

    def test_visible_picture_accepts_dark_texture(self) -> None:
        self.assert_shell(f"visible_picture {shlex.quote(str(self.video))}\n")

    def test_visible_picture_fails_closed_on_invalid_input(self) -> None:
        absent = self.artifact / "absent.ppm"
        for path in (
            absent,
            self.bad_magic,
            self.bad_depth,
            self.noncanonical,
            self.truncated,
            self.trailing,
        ):
            with self.subTest(path=path.name):
                self.assert_shell(f"! visible_picture {shlex.quote(str(path))}\n")

    def moving(self, first: Path | None, second: Path | None, prefix: str = "probe",
               media_line: str = ""):
        first_value = "FAIL" if first is None else str(first)
        second_value = "FAIL" if second is None else str(second)
        return self.bash(
            f"""
printf '%s\\n' {shlex.quote(media_line)} >"$log"
sleep() {{ :; }}
grab() {{
    case "$1" in
    {prefix}-a) [ {shlex.quote(first_value)} != FAIL ] || return 1
        printf '%s\\n' {shlex.quote(first_value)} ;;
    {prefix}-b) [ {shlex.quote(second_value)} != FAIL ] || return 1
        printf '%s\\n' {shlex.quote(second_value)} ;;
    *) return 1 ;;
    esac
}}
if moving_picture {shlex.quote(prefix)}; then
    printf 'pass:%s\\n' "$motion_problem"
else
    printf 'fail:%s\\n' "$motion_problem"
fi
"""
        )

    def assert_moving(self, first: Path | None, second: Path | None, expected: str,
                      problem: str = "", prefix: str = "probe") -> None:
        result = self.moving(first, second, prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith(expected + ":"), result.stdout)
        if problem:
            self.assertIn(problem, result.stdout)

    def test_motion_rejects_backdrop_transition(self) -> None:
        self.assert_moving(self.flat, self.video, "fail", "flat video area")

    def test_motion_rejects_identical_visible_pictures(self) -> None:
        self.assert_moving(self.video, self.video_copy, "fail", "identical")

    def test_motion_ignores_header_and_control_changes(self) -> None:
        self.assert_moving(self.video, self.controls, "fail", "identical")

    def test_motion_accepts_differing_picture_bodies(self) -> None:
        self.assert_moving(self.video, self.other, "pass")

    def test_motion_reports_each_capture_failure(self) -> None:
        self.assert_moving(None, self.video, "fail", "first capture")
        self.assert_moving(self.video, None, "fail", "second capture")

    def test_motion_refuses_one_lens_open_advice(self) -> None:
        result = self.moving(self.video, self.other, media_line="media:  1 lens stream, 4x4")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("fail:"), result.stdout)
        self.assertIn("opening advice", result.stdout)

    def test_transient_import_recovery_uses_the_same_visible_motion_contract(self) -> None:
        self.assertIn("moving_picture hiccup", self.source_text)
        self.assert_moving(self.flat, self.video, "fail", "flat video area", prefix="hiccup")
        self.assert_moving(self.video, self.other, "pass", prefix="hiccup")

    def readiness(
        self,
        initial_log: str,
        capture: str,
        append_after_sleep: str = "",
        dead_at: int | None = None,
    ) -> subprocess.CompletedProcess[str]:
        log = self.artifact / "readiness.log"
        log.write_text(initial_log)
        clock = self.artifact / "readiness.clock"
        clock.write_text("0\n")
        dead = 999 if dead_at is None else dead_at
        flat = shlex.quote(str(self.flat))
        video = shlex.quote(str(self.video))
        append = shlex.quote(append_after_sleep)
        return self.bash(
            f"""
log={shlex.quote(str(log))}
clock={shlex.quote(str(clock))}
READY=2
# Remove Bash's automatic ticking so only the fixture's sleep advances time.
unset SECONDS
SECONDS=0
alive() {{ [ "$(<"$clock")" -lt {dead} ]; }}
grab() {{
    case {shlex.quote(capture)} in
    flat) printf '%s\\n' {flat} ;;
    video) printf '%s\\n' {video} ;;
    flat-then-video)
        if [ "$(<"$clock")" -eq 0 ]; then
            printf '%s\\n' {flat}
        else
            printf '%s\\n' {video}
        fi ;;
    fail) return 1 ;;
    esac
}}
sleep() {{
    local now
    now=$(<"$clock")
    now=$((now + 1))
    printf '%s\\n' "$now" >"$clock"
    SECONDS=$((SECONDS + 1))
    if [ "$now" -eq 1 ] && [ -n {append} ]; then
        printf '%s\\n' {append} >>"$log"
    fi
}}
if await_visible_playback; then echo ready; else echo not-ready; fi
"""
        )

    def assert_ready(self, expected: bool, **kwargs) -> None:
        result = self.readiness(**kwargs)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "ready" if expected else "not-ready")

    def test_readiness_accepts_report_then_picture(self) -> None:
        self.assert_ready(
            True,
            initial_log="play:       0.61 s, 1.81 fps presented in 2.0 redraws/s\n",
            capture="flat-then-video",
        )

    def test_readiness_accepts_picture_then_report(self) -> None:
        self.assert_ready(
            True,
            initial_log="",
            capture="video",
            append_after_sleep="play:       1.00 s, 0.25 fps presented in 2.0 redraws/s",
        )

    def test_readiness_latches_an_earlier_positive_report(self) -> None:
        self.assert_ready(
            True,
            initial_log="play:       0.61 s, 1.81 fps presented in 2.0 redraws/s\n",
            capture="flat-then-video",
            append_after_sleep="play:       1.00 s, 0.00 fps presented in 2.0 redraws/s",
        )

    def test_readiness_rejects_missing_zero_and_malformed_reports(self) -> None:
        reports = (
            "",
            "play:       0.61 s, 0.00 fps presented in 2.0 redraws/s\n",
            "play: malformed\n",
        )
        for report in reports:
            with self.subTest(report=report):
                self.assert_ready(False, initial_log=report, capture="video")

    def test_readiness_rejects_flat_capture_after_positive_report(self) -> None:
        self.assert_ready(
            False,
            initial_log="play:       0.61 s, 1.81 fps presented in 2.0 redraws/s\n",
            capture="flat",
        )

    def test_readiness_refuses_one_lens_open_advice(self) -> None:
        self.assert_ready(
            False,
            initial_log="media:  1 lens stream, 4x4\n"
                        "play:       0.61 s, 1.81 fps presented in 2.0 redraws/s\n",
            capture="video",
        )

    def test_readiness_rejects_capture_failure_and_dead_app(self) -> None:
        positive = "play:       0.61 s, 1.81 fps presented in 2.0 redraws/s\n"
        self.assert_ready(False, initial_log=positive, capture="fail")
        self.assert_ready(False, initial_log=positive, capture="video", dead_at=0)

    def with_media(self, capture: Path) -> subprocess.CompletedProcess[str]:
        marker = self.artifact / "with-media-ready"
        marker.unlink(missing_ok=True)
        return self.bash(
            f"""
log={shlex.quote(str(self.artifact / 'with-media.log'))}
session={shlex.quote(str(self.artifact))}
media=/fixture/paired.insv
play_args=()
READY=2
REPORT=2
unset SECONDS
SECONDS=0
printf '%s\\n' 'media: paired fixture' \
    'play:       0.61 s, 1.81 fps presented in 2.0 redraws/s' \
    'dmabuf import: all extensions enabled' >"$log"
boot() {{ :; }}
await() {{ return 0; }}
await_paint() {{ return 0; }}
alive() {{ return 0; }}
teardown() {{ :; }}
lost() {{ exit 94; }}
sleep() {{ SECONDS=$((SECONDS + 1)); }}
grab() {{ printf '%s\\n' {shlex.quote(str(capture))}; }}
moving_picture() {{ return 0; }}
pass() {{
    case "$1" in
    'playback reaches a visible picture before the motion check')
        : >{shlex.quote(str(marker))} ;;
    'the picture moves while playing')
        [ -e {shlex.quote(str(marker))} ] && exit 30
        exit 98 ;;
    esac
}}
fail() {{
    case "$1" in
    'playback reaches a visible picture before the motion check') exit 31 ;;
    esac
}}
with_media
exit 97
"""
        )

    def test_with_media_requires_visible_readiness_before_motion(self) -> None:
        ready = self.with_media(self.video)
        self.assertEqual(ready.returncode, 30, ready.stderr)
        backdrop = self.with_media(self.flat)
        self.assertEqual(
            backdrop.returncode,
            31,
            f"stdout:\n{backdrop.stdout}\nstderr:\n{backdrop.stderr}",
        )


if __name__ == "__main__":
    unittest.main()
