# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""The line check's positive control: a line put in, on each arm's own content.

A step of `codes` is added to one side of the seam in the very frames line.py
scored, and the same statistic is re-read. A statistic that cannot see a planted
line cannot be quoted for the absence of one.
"""

import sys
import numpy as np
import cv2

sys.path.insert(0, "/tmp/claude-1000/-home-aeharding-wingover/09ccdb9e-e174-4e20-af54-412298c7ce58/scratchpad/oracle")
import corridor
import line as L


def planted(picture, hub, direction, codes):
    d = direction / np.hypot(*direction)
    n = np.array([-d[1], d[0]])
    ys, xs = np.mgrid[0:picture.shape[0], 0:picture.shape[1]]
    side = (xs - hub[0]) * n[0] + (ys - hub[1]) * n[1] > 0
    out = picture.astype(np.float64).copy()
    out[side] += codes
    return np.clip(out, 0, 255).astype(np.uint8)


def main():
    got = [f for f in corridor.frames() if f[4] is not None][:20]
    shrink = L.PPD / corridor.ST_PPD
    print("=== planted line, read by the same statistic (20 frames, lag 1 px) ===")
    print("  arm        planted codes   seam Weber   decoy Weber   excess")
    for arm in ("kjerag", "studio"):
        for codes in (0.0, 0.5, 1.0, 2.0, 4.0):
            excesses = []
            for t, a, b, (x, y, tilt), ends, inliers in got:
                if arm == "kjerag":
                    picture, hub = a, np.array([x, y])
                    direction = np.array([np.cos(np.radians(tilt)), np.sin(np.radians(tilt))])
                else:
                    picture = cv2.resize(b, None, fx=shrink, fy=shrink, interpolation=cv2.INTER_AREA)
                    hub, direction = ends[0] * shrink, (ends[1] - ends[0])
                marked = planted(picture, hub, direction, codes) if codes else picture
                seam, _ = L.weber(marked, hub, direction, 0.0, 1)
                decoys = [L.weber(marked, hub, direction, o, 1)[0] for o in L.DECOYS]
                decoys = [v for v in decoys if v is not None]
                if seam is None or len(decoys) < 3:
                    continue
                excesses.append(100.0 * (seam / float(np.mean(decoys)) - 1.0))
            if not excesses:
                continue
            e = np.array(excesses)
            print(f"  {arm:<8} {codes:14.1f} {'':12} {'':13} {e.mean():+7.2f} % "
                  f"(sem {e.std()/np.sqrt(len(e)):.2f}, {len(e)} frames)")


if __name__ == "__main__":
    main()
