#!/usr/bin/env python3
"""CPU-only contract tests for uitest.sh's copied-view round trip.

The real retry helper, receipt helpers and returns_to_the_copied_view() are
extracted from the requested source. Compositor, keyboard, log production and
the shared image predicates are replaced; the image predicates themselves are
covered with synthetic PPMs by test-uitest-playback.py.
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


def extract_function(source: str, name: str) -> str:
    anchor = f"{name}() {{"
    lines = source.splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line.rstrip("\n") == anchor]
    if len(starts) != 1:
        raise AssertionError(f"expected one {anchor!r}, found {len(starts)}")
    for end in range(starts[0] + 1, len(lines)):
        if lines[end].rstrip("\n") == "}":
            return "".join(lines[starts[0] : end + 1])
    raise AssertionError(f"unterminated function {name}")


class CopiedViewContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        source = SOURCE.read_text()
        names = (
            "press_until",
            "more_goto_lines",
            "more_view_display_lines",
            "current_display",
            "returned_view_is_current",
            "returns_to_the_copied_view",
        )
        scratch = ROOT / "scratch"
        scratch.mkdir(exist_ok=True)
        cls.artifact = Path(tempfile.mkdtemp(prefix="uitest-view.", dir=scratch))
        cls.extracted = cls.artifact / "extracted.sh"
        cls.extracted.write_text("\n".join(extract_function(source, name) for name in names))
        cls.source = source

    def run_case(self, scenario: str) -> subprocess.CompletedProcess[str]:
        case = self.artifact / scenario
        case.mkdir()
        body = f"""
set -uo pipefail
scenario={shlex.quote(scenario)}
session={shlex.quote(str(case))}
log="$session/app.log"
: >"$log"
media=/fixture/X3.insv
PRESSES=2
READY=3
TOAST_GONE=0
view_lines=0
display_lines=0
goto_lines=0
paired_view=
paired_display=
source {shlex.quote(str(self.extracted))}

fixture_view='/fixture/X3.insv time=7.941 yaw=0.00 pitch=0.00 fov=90.00 lock=1'
wrong='/fixture/X3.insv time=7.941 yaw=1.00 pitch=0.00 fov=90.00 lock=1'
receipt_a='index=238 time_ns=7941266666 current=1'
receipt_b="$receipt_a"
case "$scenario" in
wrong-view) fixture_returned="$wrong" ;;
*) fixture_returned="$fixture_view" ;;
esac
case "$scenario" in
wrong-index) receipt_b='index=237 time_ns=7941266666 current=1' ;;
wrong-time) receipt_b='index=238 time_ns=7941266667 current=1' ;;
stale-return) receipt_b='index=238 time_ns=7941266666 current=0' ;;
malformed-return) receipt_b='index=238 time_ns=nope current=1' ;;
unavailable-return) receipt_b='unavailable' ;;
esac

report_number=0
append_report() {{
    report_number=$((report_number + 1))
    local view receipt
    if [ "$report_number" = 1 ]; then view=$fixture_view; receipt=$receipt_a
    else view=$fixture_returned; receipt=$receipt_b; fi
    if [ "$scenario" = recovery-after-stale ] && [ "$report_number" = 2 ]; then
        receipt='index=238 time_ns=7941266666 current=0'
    fi
    if [ "$scenario" = missing-copy-receipt ] ||
        [ "$scenario" = old-pair-before-boundary ] ||
        {{ [ "$scenario" = missing-return ] && [ "$report_number" -ge 2 ]; }}; then
        printf 'view:   %s\n' "$view" >>"$log"
    elif [ "$scenario" = stale-pair-copy ] ||
        {{ [ "$scenario" = stale-pair-return ] && [ "$report_number" -ge 2 ]; }}; then
        printf 'view:   %s\nnoise: interrupts pair\ndisplay: %s\n' "$view" "$receipt" >>"$log"
    else
        printf 'view:   %s\ndisplay: %s\n' "$view" "$receipt" >>"$log"
        [ "$scenario" != later-noise ] || printf 'worker: unrelated completion\n' >>"$log"
        if [ "$scenario" = newer-unpaired-return ] && [ "$report_number" -ge 2 ]; then
            printf 'view:   %s\n' "$wrong" >>"$log"
        fi
    fi
}}

wandered=no
at_return=no
key() {{
    case " $* " in
    *' -k i '*) append_report ;;
    *' -k Right '*) wandered=yes ;;
    *' -k v '*)
        printf 'goto:   %s\n' "$fixture_view" >>"$log"
        at_return=yes ;;
    esac
}}
sleep() {{ :; }}
alive() {{ return 0; }}
lost() {{ printf 'LOST|%s\n' "$*"; }}
fail() {{ printf 'FAIL|%s\n' "$*"; }}
pass() {{ printf 'PASS|%s\n' "$*"; }}

grab() {{
    local name=$1 value path
    path="$session/$name.ppm"
    case "$name" in
    goto-there|custom-there) value=A ;;
    goto-away|custom-away) if [ "$wandered" = yes ]; then value=AWAY; else value=A; fi ;;
    goto-back-a|custom-back-a) value=B ;;
    goto-back-b|custom-back-b) value=B ;;
    *) return 1 ;;
    esac
    case "$scenario:$name" in
    missing-copy-capture:goto-there|missing-away:goto-away|missing-back-a:goto-back-a|missing-back-b:goto-back-b)
        return 1 ;;
    flat-copy:goto-there|flat-away:goto-away|flat-return:goto-back-a|flat-return:goto-back-b)
        value=FLAT ;;
    unchanged-away:goto-away) value=A ;;
    stale-away:goto-back-a|stale-away:goto-back-b) value=AWAY ;;
    unstable-return:goto-back-b) value=B2 ;;
    esac
    printf '%s\n' "$value" >"$path"
    printf '%s\n' "$path"
}}
visible_picture() {{ [ "$(<"$1")" != FLAT ]; }}
same_picture() {{ cmp -s "$1" "$2"; }}

case "$scenario" in
malformed-copy) receipt_a='index=238 time_ns=nope current=1' ;;
unavailable-copy) receipt_a='unavailable' ;;
stale-copy) receipt_a='index=238 time_ns=7941266666 current=0' ;;
esac
if [ "$scenario" = old-pair-before-boundary ]; then
    printf 'view:   %s\ndisplay: %s\n' "$fixture_view" "$receipt_a" >>"$log"
fi
if [ "$scenario" = custom-prefix ]; then
    returns_to_the_copied_view copied custom
else
    returns_to_the_copied_view
fi
"""
        return subprocess.run(
            ["bash", "-c", body],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={**os.environ, "LC_ALL": "C"},
            timeout=5,
            check=False,
        )

    def assert_pass(self, scenario: str) -> None:
        result = self.run_case(scenario)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS|ctrl+v goes back", result.stdout)
        self.assertNotIn("FAIL|", result.stdout)

    def assert_fail(self, scenario: str, reason: str) -> None:
        result = self.run_case(scenario)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("FAIL|", result.stdout)
        self.assertIn(reason, result.stdout)
        self.assertNotIn("PASS|", result.stdout)

    def test_accepts_history_variance_with_exact_source_and_stable_return(self) -> None:
        self.assert_pass("valid-history-variance")
        self.assert_pass("later-noise")
        self.assert_pass("recovery-after-stale")

    def test_custom_artifact_prefix_keeps_fixture_evidence_distinct(self) -> None:
        result = self.run_case("custom-prefix")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS|ctrl+v goes back", result.stdout)
        for suffix in ("there", "away", "back-a", "back-b"):
            self.assertTrue(
                (self.artifact / "custom-prefix" / f"custom-{suffix}.ppm").is_file(),
                suffix,
            )

    def test_old_pair_before_boundary_cannot_rescue_missing_new_receipt(self) -> None:
        self.assert_fail("old-pair-before-boundary", "no fresh paired")

    def test_newer_unpaired_view_invalidates_older_valid_pair(self) -> None:
        self.assert_fail("newer-unpaired-return", "did not reach a fresh")

    def test_rejects_missing_interrupted_or_malformed_receipts(self) -> None:
        for scenario, reason in (
            ("missing-copy-receipt", "no fresh paired"),
            ("missing-return", "did not reach a fresh"),
            ("stale-pair-copy", "no fresh paired"),
            ("stale-pair-return", "did not reach a fresh"),
            ("malformed-copy", "receipt is unavailable"),
            ("malformed-return", "did not reach a fresh"),
            ("unavailable-copy", "receipt is unavailable"),
            ("unavailable-return", "did not reach a fresh"),
            ("stale-copy", "receipt is unavailable"),
            ("stale-return", "did not reach a fresh"),
        ):
            with self.subTest(scenario=scenario):
                self.assert_fail(scenario, reason)

    def test_rejects_wrong_view_or_exact_frame(self) -> None:
        self.assert_fail("wrong-view", "did not reach a fresh")
        self.assert_fail("wrong-index", "did not reach a fresh")
        self.assert_fail("wrong-time", "did not reach a fresh")

    def test_rejects_capture_failures_and_nonpictures(self) -> None:
        for scenario, reason in (
            ("missing-copy-capture", "copied-view capture failed"),
            ("missing-away", "away capture failed"),
            ("missing-back-a", "first returned capture failed"),
            ("missing-back-b", "second returned capture failed"),
            ("flat-copy", "copied view is not a meaningful"),
            ("flat-away", "away view is not a meaningful"),
            ("flat-return", "returned view is not a meaningful"),
        ):
            with self.subTest(scenario=scenario):
                self.assert_fail(scenario, reason)

    def test_rejects_unchanged_away_stale_away_and_unstable_return(self) -> None:
        self.assert_fail("unchanged-away", "seek and the zoom moved nothing")
        self.assert_fail("stale-away", "still the away picture")
        self.assert_fail("unstable-return", "not stable")

    def test_call_site_guards_the_round_trip_on_a_successful_pause(self) -> None:
        guarded = '''if [ "$paused" = yes ]; then
\t\treturns_to_the_copied_view
\telse
\t\tskip "ctrl+v goes back to the copied view (pause failed)"
\tfi'''
        self.assertIn(guarded, self.source)


if __name__ == "__main__":
    unittest.main()
