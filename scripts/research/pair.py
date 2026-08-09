# Branch-only research instrument (research/parity-proof, 2026-08-08): pairs a
# Studio export to its source .insv in TIME, and proves the pairing can be
# wrong and say so. Not on any shipped path and not wired into a gate.
"""Which instant of the source .insv an instant of a Studio export is.

WHY TIME AND NOT FRAME NUMBER
  The source is 29.97 fps and Studio's exports are 30, over the same 1799.8
  seconds. Frame k of the export is NOT frame k of the source and the drift
  reaches 1.8 seconds by the end of the clip. Everything downstream pairs by
  time, and this instrument is what says that pairing is sound.

WHAT IS MEASURED
  Zero-mean normalized cross-correlation of the two audio tracks over disjoint
  windows spread across the whole clip. The sign convention, stated as the
  equation downstream actually uses it:

      source_time = export_time + lag

  i.e. the export's frame at time t shows the source at time t + lag. Derived
  and not asserted: the transform peaks where b(u) = a(u + lag), a being the
  source window and b the export's, so advancing the export's window by a known
  amount has to move the reading by exactly +that amount. That is control 1.

WHAT AUDIO CANNOT SETTLE
  A residual of a few tens of milliseconds is the same size as an AAC
  encoder/decoder priming delay, and audio alone cannot tell a priming delay
  from a real timeline offset. So this instrument establishes the pairing to
  WITHIN ONE FRAME and says so; the sub-frame part is settled in the picture,
  by scripts/research/register.py sweeping the time offset and showing the
  registration residual has a minimum. Nothing here should be quoted as a
  sub-frame result.

THE THREE CONTROLS, all of which must fire before a pairing is believed
  1. INJECTED SHIFT (the plant). The export's audio is deliberately advanced by
     a known whole number of frames and the estimator is re-run. It has to
     report exactly that shift back, or it cannot detect a real one either.
  2. FRAME REJECTION. The correlation at the winning lag is compared with the
     correlation one frame either side of it. A peak that is not measurably
     above its own +/-1 frame neighbours cannot resolve a frame, and the margin
     is quoted so nobody has to take that on trust.
  3. DRIFT. The lag is measured in every window independently and a straight
     line is fitted through it. A rate mismatch between the two tracks shows up
     as a slope; a slope consistent with zero is what lets one lag stand for
     the whole clip.

A TRIMMED EXPORT
  The July-14 exports were the whole clip, so `lag` was a few tens of
  milliseconds and a +/-5 s search around zero found it. An export cut out of
  the middle of a flight is not that: its `lag` is wherever the owner put the
  in-point, which is hundreds of seconds, and a search around zero would report
  the edge of its own window. So the pairing runs in TWO STAGES and the coarse
  one is gated too:

    COARSE. The whole export is correlated against the whole source at every
    offset, and the winner has to stand above the best rival more than one
    export-length away -- that is the same prominence question the picture's
    own registration asks, and it is what says the trim was found rather than
    landed on. A repeating engine note can correlate somewhere else; a peak
    that does not beat that is not a pairing.

    FINE. Everything below, run in disjoint windows inside the export's own
    span with the search centred on the coarse answer. The gates are unchanged
    and their thresholds are the protocol's.

  The windows are placed from the export's own duration rather than written
  down for one clip length, and they are SHORTER on a short export, which makes
  every gate below harder rather than easier: less audio per window is less
  correlation, not more.

USAGE
  python3 scripts/research/pair.py <dir-of-8kHz-mono-wavs> [tag ...]
  expecting full-src.wav and one full-<tag>.wav per export.
"""

import os
import sys
import wave

import numpy as np

RATE = 8000
# The source's own frame rate. A "frame" in the controls below is a source
# frame, because that is the unit the downstream pairing has to be good to.
SRC_FPS = 30000.0 / 1001.0
# Windows: disjoint, spread over whatever the export's own span is, avoiding
# the first and last few seconds where an encoder's priming can leave the
# tracks ragged. WINDOW_S is the most one window may be; a short export gets
# shorter windows and a harder test.
WINDOW_S = 40.0
WINDOWS = 8
EDGE_S = 4.0
# How far the FINE search looks around the coarse answer. Wide enough that a
# wrong answer has room to be wrong in rather than being clamped into looking
# right.
SEARCH_S = 5.0
# How far away a coarse rival has to be before it counts as a rival, as a
# fraction of the export's own length, and how far the coarse peak has to stand
# above it. The coarse correlation is normalized by the two windows' energies,
# so this is a plain correlation margin like every other one here.
RIVAL_SHARE = 1.0
COARSE_MARGIN = 0.05


def read(path):
    with wave.open(path, "rb") as w:
        assert w.getframerate() == RATE, f"{path} is {w.getframerate()} Hz, wanted {RATE}"
        assert w.getnchannels() == 1, f"{path} is not mono"
        raw = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
    return raw.astype(np.float64)


def correlate(a, b):
    """Normalized cross-correlation of two equal-length windows, and its lags.

    Returned as (lags_in_samples, correlation), both centred on zero lag and
    cut to +/-SEARCH_S. Normalization is by the whole windows' energies, so the
    number is comparable between windows and between files.
    """
    n = min(len(a), len(b))
    a = a[:n] - a[:n].mean()
    b = b[:n] - b[:n].mean()
    energy = np.sqrt((a**2).sum() * (b**2).sum())
    if energy <= 0:
        return None, None
    size = 1 << int(np.ceil(np.log2(2 * n)))
    corr = np.fft.irfft(np.fft.rfft(a, size) * np.conj(np.fft.rfft(b, size)), size)
    reach = int(SEARCH_S * RATE)
    corr = np.concatenate([corr[-reach:], corr[: reach + 1]]) / energy
    return np.arange(-reach, reach + 1), corr


def peak(lags, corr):
    """The winning lag in seconds, its correlation, and the parabolic subsample.

    Also the two numbers control 2 lives on: the correlation exactly one source
    frame either side of the winner.
    """
    k = int(np.argmax(corr))
    top = corr[k]
    if 0 < k < len(corr) - 1:
        curve = corr[k - 1] - 2 * corr[k] + corr[k + 1]
        sub = 0.0 if curve == 0 else -0.5 * (corr[k + 1] - corr[k - 1]) / curve
    else:
        sub = 0.0
    lag = (lags[k] + sub) / RATE
    step = int(round(RATE / SRC_FPS))
    sides = []
    for offset in (-step, step):
        j = k + offset
        sides.append(corr[j] if 0 <= j < len(corr) else float("nan"))
    return lag, top, sides


def trim(src, other):
    """Where the whole export sits in the whole source, and what that beat.

    The coarse stage. Returns the offset in seconds under `source_time =
    export_time + lag`, its normalized correlation, and the best rival more
    than one export-length away -- the number that says the trim was FOUND. A
    flight's audio is an engine and a wind and both repeat, so an offset that
    does not stand above every distant rival has not been located, it has been
    landed on.
    """
    a = src - src.mean()
    b = other - other.mean()
    if len(b) >= len(a):
        return None
    size = 1 << int(np.ceil(np.log2(len(a) + len(b))))
    corr = np.fft.irfft(np.fft.rfft(a, size) * np.conj(np.fft.rfft(b, size)), size)
    # corr[m] = sum_n a[n] b[n-m], so the peak sits at the offset m with
    # b(u) = a(u + m), which is the sign convention this file's header states.
    reach = len(a) - len(b)
    corr = corr[: reach + 1]
    # Normalized per offset by the SOURCE energy under the export's own span,
    # so a loud stretch of the flight cannot outscore a quiet true one.
    squared = np.concatenate([[0.0], np.cumsum(a * a)])
    energy = squared[len(b) : len(b) + reach + 1] - squared[: reach + 1]
    scale = np.sqrt(np.maximum(energy, 1e-9) * (b * b).sum())
    corr = corr / scale
    k = int(np.argmax(corr))
    apart = int(RIVAL_SHARE * len(b))
    away = np.concatenate([corr[: max(0, k - apart)], corr[k + apart :]])
    rival = float(away.max()) if len(away) else -1.0
    return dict(lag=k / RATE, top=float(corr[k]), rival=rival, samples=k)


def windows(duration):
    """Disjoint windows spread over an export's own span, longest first.

    Written from the export's duration rather than for one clip length: the
    July-14 exports were the whole 1799.8 s flight and these are three minutes
    of it, and a window list that does not fit falls off the end of the file
    and correlates nothing.
    """
    span = duration - 2 * EDGE_S
    width = min(WINDOW_S, span / WINDOWS)
    if width < 4.0:
        width = min(span / 2.0, WINDOW_S)
    step = span / WINDOWS
    starts = [EDGE_S + k * step for k in range(WINDOWS)]
    starts = [s for s in starts if s + width <= duration - EDGE_S / 2]
    return width, starts


def measure(src, other, start, width, shift_samples=0, base=0.0):
    """One window's lag, with the export optionally shifted by a known amount.

    `base` is the coarse pairing in seconds. The source window is taken that
    much later than the export's, so what the correlation reads is the
    REMAINDER of the pairing and the lag reported is the two added back
    together. On a whole-clip export `base` is zero and this is what it always
    was.
    """
    held = int(round(base * RATE))
    first = int(start * RATE)
    count = int(width * RATE)
    a = src[first + held : first + held + count]
    b = other[first + shift_samples : first + count + shift_samples]
    if len(a) < 1000 or len(b) < 1000 or len(a) != len(b):
        return None
    lags, corr = correlate(a, b)
    if lags is None:
        return None
    lag, top, sides = peak(lags, corr)
    return dict(start=start, lag=held / RATE + lag, top=top, sides=sides)


def report(tag, src, other):
    frame_s = 1.0 / SRC_FPS
    print(f"\n=== {tag} ===")
    duration = len(other) / RATE
    width, starts = windows(duration)

    # --- stage 1: where in the source this export was cut from -----------
    found = trim(src, other)
    base = 0.0
    coarse_ok = True
    if found is None:
        print("  the export is not shorter than the source, so there is no trim "
              "to find: the coarse stage is skipped and the search runs about zero.")
    else:
        base = found["lag"]
        stands = found["top"] - found["rival"]
        coarse_ok = stands >= COARSE_MARGIN
        print(f"  COARSE trim: the export sits at source {base:+.4f} s "
              f"({base/frame_s:+.1f} source frames), correlation {found['top']:.4f}, "
              f"best rival more than one export-length away {found['rival']:.4f}, "
              f"prominence {stands:+.4f}, wanted {COARSE_MARGIN:.2f}  "
              f"{'PASS' if coarse_ok else 'FAIL'}")
    print(f"  windows: {len(starts)} disjoint, {width:.1f} s each, over the "
          f"export's own {duration:.3f} s; the fine search is +/-{SEARCH_S:.0f} s "
          f"about the coarse answer")

    rows = [r for r in (measure(src, other, s, width, 0, base) for s in starts) if r]
    if not rows:
        print("  no window correlated at all")
        return False
    lags = np.array([r["lag"] for r in rows])
    tops = np.array([r["top"] for r in rows])

    print(f"  {'window s':>9} {'lag s':>10} {'lag frames':>11} {'peak':>8}"
          f" {'peak at -1f':>12} {'peak at +1f':>12} {'margin':>8}")
    margins = []
    for r in rows:
        margin = r["top"] - max(r["sides"])
        margins.append(margin)
        print(f"  {r['start']:>9.0f} {r['lag']:>+10.5f} {r['lag']/frame_s:>+11.3f}"
              f" {r['top']:>8.4f} {r['sides'][0]:>12.4f} {r['sides'][1]:>12.4f}"
              f" {margin:>8.4f}")

    # --- control 3: drift across the clip -------------------------------
    slope, intercept = np.polyfit([r["start"] for r in rows], lags, 1)
    spread = float(lags.std())
    drift_over_clip = slope * duration
    drift_over_source = slope * (len(src) / RATE)
    print(f"\n  lag, mean over {len(rows)} windows: {lags.mean():+.5f} s "
          f"({lags.mean()/frame_s:+.3f} source frames), sd {spread:.5f} s "
          f"({spread/frame_s:.3f} frames)")
    print(f"  CONTROL drift: slope {slope:+.3e} s/s, which is "
          f"{drift_over_clip:+.4f} s ({drift_over_clip/frame_s:+.3f} frames) "
          f"across this export's own {duration:.1f} s, and "
          f"{drift_over_source:+.4f} s ({drift_over_source/frame_s:+.3f} frames) "
          f"if the same rate mismatch is carried over the whole "
          f"{len(src)/RATE:.1f} s source")

    # --- control 2: can a frame be resolved at all? ----------------------
    worst_margin = float(np.min(margins))
    print(f"  CONTROL frame rejection: the winning peak beats its own +/-1 "
          f"frame neighbours by at least {worst_margin:.4f} correlation "
          f"(mean peak {tops.mean():.4f}) on every window")

    # --- control 1: the plant -------------------------------------------
    print("  CONTROL injected shift (the plant): the export is advanced by a "
          "known whole number of source frames and the estimator re-run.")
    planted_ok = True
    for frames in (-4, -1, 1, 4):
        shift = int(round(frames * RATE / SRC_FPS))
        got = [measure(src, other, s, width, shift, base) for s in starts[:4]]
        got = [r for r in got if r]
        if not got:
            print(f"    plant {frames:+d} frames: no window correlated -- FAILED")
            planted_ok = False
            continue
        moved = np.mean([r["lag"] for r in got]) - lags[:4].mean()
        # Advancing the export's window by +frames must move the reading by
        # +frames: b(u) = a(u + lag), so a later slice of b is a larger lag.
        wanted = frames * frame_s
        error_frames = (moved - wanted) / frame_s
        ok = abs(error_frames) < 0.05
        planted_ok &= ok
        print(f"    plant {frames:+d} frames -> reading moves {moved:+.5f} s "
              f"({moved/frame_s:+.3f} frames), wanted {wanted/frame_s:+.3f}, "
              f"error {error_frames:+.4f} frames  {'PASS' if ok else 'FAIL'}")

    resolves = worst_margin > 0.02
    steady = spread < 0.5 * frame_s
    no_drift = abs(drift_over_clip) < frame_s
    print(f"\n  LAG {tag}: source_time = export_time {lags.mean():+.5f} s")
    print(f"  VERDICT {tag}: coarse-trim {'PASS' if coarse_ok else 'FAIL'}, "
          f"resolve-a-frame {'PASS' if resolves else 'FAIL'}, "
          f"plant {'PASS' if planted_ok else 'FAIL'}, "
          f"steady {'PASS' if steady else 'FAIL'}, "
          f"no-drift {'PASS' if no_drift else 'FAIL'}")
    return coarse_ok and resolves and planted_ok and steady and no_drift


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "/tmp/parity/audio"
    tags = sys.argv[2:] or ["creek", "planet"]
    src = read(os.path.join(base, "full-src.wav"))
    print(f"source: {os.path.join(base, 'full-src.wav')}, "
          f"{len(src)/RATE:.3f} s at {RATE} Hz mono")
    every = True
    for tag in tags:
        path = os.path.join(base, f"full-{tag}.wav")
        if not os.path.exists(path):
            print(f"\n=== {tag} === missing {path}")
            every = False
            continue
        other = read(path)
        print(f"\nexport: {path}, {len(other)/RATE:.3f} s")
        every &= report(tag, src, other)
    print(f"\nPAIRING: {'PROVEN' if every else 'NOT PROVEN'}")
    return 0 if every else 1


if __name__ == "__main__":
    sys.exit(main())
