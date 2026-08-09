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

USAGE
  python3 scripts/research/pair.py <dir-of-8kHz-mono-wavs>
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
# Windows: disjoint, spread over the whole 1799.8 s, avoiding the first and
# last few seconds where an encoder's priming can leave the tracks ragged.
WINDOW_S = 40.0
STARTS = [30.0, 260.0, 500.0, 740.0, 980.0, 1220.0, 1460.0, 1700.0]
# How far the search looks. Studio can only have taken the whole clip, so a
# real lag is small; the window is wide enough that a wrong answer has room to
# be wrong in rather than being clamped into looking right.
SEARCH_S = 5.0


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


def measure(src, other, start, shift_samples=0):
    """One window's lag, with the export optionally shifted by a known amount."""
    first = int(start * RATE)
    last = first + int(WINDOW_S * RATE)
    a = src[first:last]
    b = other[first + shift_samples : last + shift_samples]
    if len(a) < 1000 or len(b) < 1000 or len(a) != len(b):
        return None
    lags, corr = correlate(a, b)
    if lags is None:
        return None
    lag, top, sides = peak(lags, corr)
    return dict(start=start, lag=lag, top=top, sides=sides)


def report(tag, src, other):
    frame_s = 1.0 / SRC_FPS
    print(f"\n=== {tag} ===")

    rows = [r for r in (measure(src, other, s) for s in STARTS) if r]
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
    drift_over_clip = slope * 1799.8
    print(f"\n  lag, mean over {len(rows)} windows: {lags.mean():+.5f} s "
          f"({lags.mean()/frame_s:+.3f} source frames), sd {spread:.5f} s "
          f"({spread/frame_s:.3f} frames)")
    print(f"  CONTROL drift: slope {slope:+.3e} s/s, which is "
          f"{drift_over_clip:+.4f} s ({drift_over_clip/frame_s:+.3f} frames) "
          f"across the whole 1799.8 s clip")

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
        got = [measure(src, other, s, shift) for s in STARTS[:4]]
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
    print(f"\n  VERDICT {tag}: resolve-a-frame {'PASS' if resolves else 'FAIL'}, "
          f"plant {'PASS' if planted_ok else 'FAIL'}, "
          f"steady {'PASS' if steady else 'FAIL'}, "
          f"no-drift {'PASS' if no_drift else 'FAIL'}")
    return resolves and planted_ok and steady and no_drift


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "/tmp/parity/audio"
    src = read(os.path.join(base, "full-src.wav"))
    print(f"source: {os.path.join(base, 'full-src.wav')}, "
          f"{len(src)/RATE:.3f} s at {RATE} Hz mono")
    every = True
    for tag in ("creek", "planet"):
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
