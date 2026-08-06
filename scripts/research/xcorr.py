# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""Audio cross-correlation between the source .insv and Studio's exports.

Source: ~/Videos/Insta/VID_20260714_193252_00_006.insv (stream 2, aac)
        ~/Videos/Insta/studio_exports/{20-fov-creek,tiny-planet}.mp4
Segments: ffmpeg -ss 20 -t 80, mono 8 kHz. A positive lag means the export's
audio arrives LATER than the source's, i.e. the export starts EARLIER in the clip.
"""
import sys, wave
import numpy as np

def read(path):
    with wave.open(path, "rb") as w:
        n = w.getnframes()
        raw = np.frombuffer(w.readframes(n), dtype=np.int16).astype(np.float64)
    return raw

base = "/tmp/claude-1000/-home-aeharding-wingover/09ccdb9e-e174-4e20-af54-412298c7ce58/scratchpad/oracle/audio"
src = read(f"{base}/src.wav")
RATE = 8000
for tag in ("creek", "planet"):
    other = read(f"{base}/{tag}.wav")
    n = min(len(src), len(other))
    a = src[:n] - src[:n].mean()
    b = other[:n] - other[:n].mean()
    size = 1 << int(np.ceil(np.log2(2 * n)))
    fa = np.fft.rfft(a, size)
    fb = np.fft.rfft(b, size)
    corr = np.fft.irfft(fa * np.conj(fb), size)
    corr = np.concatenate([corr[-RATE * 5:], corr[: RATE * 5 + 1]])
    lags = np.arange(-RATE * 5, RATE * 5 + 1)
    k = int(np.argmax(corr))
    peak = corr[k]
    # parabolic subsample
    if 0 < k < len(corr) - 1:
        d = corr[k - 1] - 2 * corr[k] + corr[k + 1]
        sub = 0.0 if d == 0 else -0.5 * (corr[k + 1] - corr[k - 1]) / d
    else:
        sub = 0.0
    lag = (lags[k] + sub) / RATE
    ratio = peak / np.sort(corr)[-len(corr)//20]
    norm = peak / np.sqrt((a**2).sum() * (b**2).sum())
    print(f"{tag:8s} lag {lag:+.4f} s   normalized peak {norm:.4f}   peak/95th {ratio:.2f}")
