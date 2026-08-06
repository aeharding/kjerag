# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""Is there a line at the seam? The registry's own form: seam against a decoy in the same frame.

SOURCES: as corridor.py (read that file's header for the full stamp of both arms
and of how the seam is placed in Studio's picture).

WHAT IT MEASURES
  Local Weber contrast straddling a line, at 1 and 2 pixel lags perpendicular to
  it, sampled along the line and averaged. Texture reads a few percent everywhere,
  so a raw number cannot tell a line from a scene; what is reported is the EXCESS
  over the same statistic on DECOY lines a few degrees away, parallel, in the same
  frame and the same content. That is the standing bar's own definition
  (docs/research/reference-views.md, "Standing bars").

  Both pictures are brought to the same 51.20 px/deg first, so a pixel lag is the
  same angle in both and the two arms' numbers are the same statistic.
"""

import sys
import numpy as np
import cv2

sys.path.insert(0, "/tmp/claude-1000/-home-aeharding-wingover/09ccdb9e-e174-4e20-af54-412298c7ce58/scratchpad/oracle")
import corridor

OUT = corridor.OUT
PPD = 51.20
DECOYS = (-5.0, -4.0, -3.0, 3.0, 4.0, 5.0)
LAGS = (1, 2)
SAMPLES = 600


def sample(picture, points):
    """Bilinear luma at float points, NaN outside."""
    xs, ys = points[:, 0], points[:, 1]
    h, w = picture.shape
    ok = (xs >= 1) & (xs < w - 2) & (ys >= 1) & (ys < h - 2)
    out = np.full(len(points), np.nan)
    if not ok.any():
        return out
    out[ok] = cv2.remap(
        picture.astype(np.float32), xs[ok].astype(np.float32).reshape(-1, 1),
        ys[ok].astype(np.float32).reshape(-1, 1), cv2.INTER_LINEAR
    ).ravel()
    return out


def weber(picture, point, direction, offset_deg, lag):
    """Mean Weber contrast across a line `offset_deg` from `point`, at `lag` px."""
    d = direction / np.hypot(*direction)
    n = np.array([-d[1], d[0]])
    hub = np.array(point, dtype=np.float64) + n * offset_deg * PPD
    span = np.linspace(-SAMPLES / 2, SAMPLES / 2, SAMPLES)
    line = hub[None, :] + span[:, None] * d[None, :]
    a = sample(picture, line + n * (lag / 2.0))
    b = sample(picture, line - n * (lag / 2.0))
    ok = ~np.isnan(a) & ~np.isnan(b) & ((a + b) > 8)
    if ok.sum() < SAMPLES // 4:
        return None, int(ok.sum())
    return float((np.abs(a[ok] - b[ok]) / ((a[ok] + b[ok]) / 2)).mean()), int(ok.sum())


def main():
    got = corridor.frames()
    usable = [f for f in got if f[4] is not None]
    shrink = PPD / corridor.ST_PPD
    rows = []
    for t, a, b, (x, y, tilt), ends, inliers in usable:
        small = cv2.resize(b, None, fx=shrink, fy=shrink, interpolation=cv2.INTER_AREA)
        arms = {
            "kjerag": (a, np.array([x, y]),
                       np.array([np.cos(np.radians(tilt)), np.sin(np.radians(tilt))])),
            "studio": (small, ends[0] * shrink, (ends[1] - ends[0])),
        }
        for arm, (picture, hub, direction) in arms.items():
            for lag in LAGS:
                at_seam, count = weber(picture, hub, direction, 0.0, lag)
                if at_seam is None:
                    continue
                decoys = [weber(picture, hub, direction, o, lag)[0] for o in DECOYS]
                decoys = [v for v in decoys if v is not None]
                if len(decoys) < 3:
                    continue
                floor = float(np.mean(decoys))
                rows.append((t, arm, lag, at_seam, floor, len(decoys), count, inliers))
    with open(f"{OUT}/seam-line.csv", "w") as fh:
        fh.write(
            "# mean Weber contrast straddling the seam, and straddling decoy lines 3 to 5 deg\n"
            "# away in the same frame. excess_pct = 100*(seam/decoy - 1): a line the content\n"
            "# does not read everywhere shows as a positive excess.\n"
            "# both pictures resampled to 51.20 px/deg so a pixel lag is one angle in both.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00"
            " lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4"
            " FOV 20 Distortion 0 pan -53.7 tilt 3.5 roll 0\n"
            "time_s,arm,lag_px,seam_weber,decoy_weber,decoys,samples,inliers\n"
        )
        for t, arm, lag, at_seam, floor, decoys, count, inliers in rows:
            fh.write(f"{t:.4f},{arm},{lag},{at_seam:.6f},{floor:.6f},{decoys},{count},{inliers}\n")
    print("=== a line at the seam? Weber contrast across it, over the decoys' floor ===")
    print("  arm      lag   frames   seam Weber   decoy Weber    excess")
    for arm in ("kjerag", "studio"):
        for lag in LAGS:
            here = [(s, f) for _, a, l, s, f, *_ in rows if a == arm and l == lag]
            if not here:
                continue
            seam = np.array([s for s, _ in here])
            floor = np.array([f for _, f in here])
            excess = 100.0 * (seam / floor - 1.0)
            print(f"  {arm:<8} {lag:3d} {len(here):8d} {seam.mean():13.5f} {floor.mean():13.5f}"
                  f"   {excess.mean():+7.2f} % +- {excess.std():.2f}")
    print(f"  wrote {OUT}/seam-line.csv")


if __name__ == "__main__":
    main()
