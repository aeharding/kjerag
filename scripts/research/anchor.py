# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""How far the scene has slid inside each arm's own 20 degree view, against one anchor frame.

SOURCES (identical to drift.py's header; see that file for the full stamp)
  kjerag  scratch/oracle/kjerag-window/  `band mode=sequence from=24.0 count=1080
          yaw=3.78 pitch=5.44 fov=20.00 lock=1 size=1024
          seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`
          over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv
  studio  /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4, FOV 20,
          Distortion 0, pan -53.7, tilt 3.5, roll 0

  Each arm is registered to ITS OWN frame at the anchor instant, so no chain
  accumulates and the two arms are never compared to each other here. What moves
  in the reading is the scene: the aircraft's own translation, which is the same
  world motion for both arms, plus whatever that arm's view did. A refusal is
  printed as a refusal; at 20 degrees a view that has swung far enough stops
  sharing content with its own anchor, and that is the answer rather than a gap.
"""

import sys
import numpy as np
import cv2

KJ = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window"
ST = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/studio-creek"
STEM = "VID_20260714_193252_00_006"
FPS = 30000.0 / 1001.0
FIRST = 719
ANCHOR = 36.303
ARMS = {
    "kjerag": dict(load=lambda i: cv2.imread(f"{KJ}/{STEM}-{i:04d}.png", cv2.IMREAD_GRAYSCALE),
                   index=lambda t: int(round(t * FPS)) - FIRST,
                   f=(1024 / 2) / np.tan(np.radians(10)), size=(1024, 1024)),
    "studio": dict(load=lambda i: cv2.imread(f"{ST}/creek-{i:04d}.png", cv2.IMREAD_GRAYSCALE),
                   index=lambda t: int(round((t - 30.0) * 30.0)),
                   f=(1920 / 2) / np.tan(np.radians(10)), size=(1920, 1080)),
}
ORB = cv2.ORB_create(nfeatures=6000, scaleFactor=1.12, nlevels=14, fastThreshold=5)
MATCHER = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
CLAHE = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))


def fit(a, b):
    ka, da = ORB.detectAndCompute(CLAHE.apply(a), None)
    kb, db = ORB.detectAndCompute(CLAHE.apply(b), None)
    if da is None or db is None:
        return None
    knn = MATCHER.knnMatch(da, db, k=2)
    good = [m for m, n in (p for p in knn if len(p) == 2) if m.distance < 0.78 * n.distance]
    if len(good) < 15:
        return None
    src = np.float32([ka[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
    dst = np.float32([kb[m.trainIdx].pt for m in good]).reshape(-1, 1, 2)
    H, mask = cv2.findHomography(src, dst, cv2.RANSAC, 2.0, maxIters=8000, confidence=0.9995)
    if H is None or mask is None or mask.sum() < 20:
        return None
    return H, int(mask.sum())


def main():
    out = sys.argv[1]
    times = np.arange(30.0, 48.01, 0.5)
    lines = []
    for arm, spec in ARMS.items():
        base = spec["load"](spec["index"](ANCHOR))
        w, h = spec["size"]
        centre = np.array([w / 2.0, h / 2.0, 1.0])
        for t in times:
            picture = spec["load"](spec["index"](float(t)))
            if picture is None:
                continue
            got = fit(base, picture)
            if got is None:
                lines.append((float(t), arm, None, None, 0))
                continue
            H, inliers = got
            p = H @ centre
            u, v = p[0] / p[2], p[1] / p[2]
            lines.append((
                float(t), arm,
                np.degrees(np.arctan((u - w / 2.0) / spec["f"])),
                -np.degrees(np.arctan((v - h / 2.0) / spec["f"])),
                inliers,
            ))
    with open(out, "w") as fh:
        fh.write(
            "# where the content of the anchor frame (t=36.303 s) has moved to in each arm's own\n"
            "# 20 degree view, degrees. x>0 the content has moved RIGHT, y>0 it has moved UP.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00"
            " lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4"
            " FOV 20 Distortion 0 pan -53.7 tilt 3.5 roll 0\n"
            "# fit: ORB+RANSAC homography against that arm's own anchor frame; blank = refused\n"
            "time_s,arm,x_deg,y_deg,inliers\n"
        )
        for t, arm, x, y, inliers in lines:
            if x is None:
                fh.write(f"{t:.3f},{arm},,,0\n")
                continue
            fh.write(f"{t:.3f},{arm},{x:.4f},{y:.4f},{inliers}\n")
    print(f"wrote {out}")
    by = {}
    for t, arm, x, y, inliers in lines:
        by.setdefault(t, {})[arm] = (x, y, inliers)
    print("  t        kjerag x     y   inl |   studio x     y   inl")
    for t in sorted(by):
        k = by[t].get("kjerag", (None, None, 0))
        s = by[t].get("studio", (None, None, 0))
        fmt = lambda v: "   refused" if v[0] is None else f"{v[0]:+8.2f} {v[1]:+6.2f} {v[2]:5d}"
        print(f"  {t:6.2f}  {fmt(k)} | {fmt(s)}")


if __name__ == "__main__":
    main()
