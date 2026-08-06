# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""The seam corridor in Studio's own picture, and what happens in it.

SOURCES
  kjerag  /home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window/
      `band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00 lock=1
       size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`
      over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,
      worktree research/oracle-probe at origin/main 67a4bcf. 51.20 px/deg.
  seam    scratch/oracle/shear-window/VID_20260714_193252_00_006-probe-frames.csv,
      `shear time=34.0 yaw=3.78 pitch=5.44 fov=20.00 lock=1 frames=200 warm=6.0
       seam=roll:0.577,...` - the seam point nearest the picture centre and the
      seam's own direction, per frame, in that same 1024 px view.
  studio  /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4 (FOV 20,
      Distortion 0, pan -53.7, tilt 3.5, roll 0), gray PNGs from
      `ffmpeg -ss 30 -frames:v 900 -vf format=gray`. 96.00 px/deg.

HOW THE SEAM IS PUT INTO STUDIO'S PICTURE
  Studio's stitch leaves no line to find, so the corridor is placed by geometry
  and not by looking: kjerag's own seam point and seam direction are carried into
  Studio's picture by the homography that registers the two views at that instant
  (ORB + RANSAC, median residual about 1 px = 0.01 deg). What that inherits is
  kjerag's calibration of where the lens boundary is; the two makers' boundaries
  need not agree, and nothing here measures the difference. Treat the placement
  as good to the registration (0.01 deg) plus an unmeasured calibration term that
  the registry's cross-camera work puts at a few tenths of a degree.

  The seam runs nearly along the rows here (tilt 1 to 14 deg) and is treated as a
  straight line over the 20 degrees of view, which a great circle is to well
  under a pixel at this scale.
"""

import sys
import numpy as np
import cv2

KJ = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window"
ST = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/studio-creek"
SEAM_CSV = ("/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/shear-window/"
            "VID_20260714_193252_00_006-probe-frames.csv")
STEM = "VID_20260714_193252_00_006"
OUT = "/tmp/claude-1000/-home-aeharding-wingover/09ccdb9e-e174-4e20-af54-412298c7ce58/scratchpad/oracle"
FPS = 30000.0 / 1001.0
FIRST = 719
KJ_PPD, ST_PPD = 1024.0 / 20.0, 1920.0 / 20.0
WINDOW = (35.60, 37.60)
NEAR_DEG = 2.0          # the corridor: within this many degrees of the seam
FAR_DEG = (3.0, 5.0)    # the control band in the same picture
ORB = cv2.ORB_create(nfeatures=6000, scaleFactor=1.12, nlevels=14, fastThreshold=5)
MATCHER = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
CLAHE = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))


def seam_track():
    out = {}
    for line in open(SEAM_CSV):
        if line.startswith("#") or line.startswith("offset"):
            continue
        p = line.split(",")
        out[float(p[2])] = (float(p[3]), float(p[4]), float(p[5]))
    return out


def homography(a, b):
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
    if H is None or mask is None or mask.sum() < 40:
        return None
    return H, int(mask.sum())


def frames():
    """(time, kjerag picture, studio picture, kjerag seam line, studio seam line)."""
    track = seam_track()
    stamps = sorted(track)
    out = []
    for k in range(int(round(WINDOW[0] * FPS)), int(round(WINDOW[1] * FPS)) + 1):
        t = k / FPS
        near = min(stamps, key=lambda s: abs(s - t))
        if abs(near - t) > 0.02:
            continue
        x, y, tilt = track[near]
        a = cv2.imread(f"{KJ}/{STEM}-{k - FIRST:04d}.png", cv2.IMREAD_GRAYSCALE)
        b = cv2.imread(f"{ST}/creek-{int(round((t - 30.0) * 30.0)):04d}.png", cv2.IMREAD_GRAYSCALE)
        if a is None or b is None:
            continue
        got = homography(a, b)
        if got is None:
            out.append((t, a, b, (x, y, tilt), None, 0))
            continue
        H, inliers = got
        step = np.array([np.cos(np.radians(tilt)), np.sin(np.radians(tilt))]) * 100.0
        ends = cv2.perspectiveTransform(
            np.float32([[[x, y]], [[x + step[0], y + step[1]]]]), H
        ).reshape(2, 2)
        out.append((t, a, b, (x, y, tilt), ends, inliers))
    return out


def distance_map(shape, point, direction, ppd):
    """Signed distance from the seam line, in degrees, for every pixel."""
    h, w = shape
    n = np.array([-direction[1], direction[0]], dtype=np.float64)
    n = n / np.hypot(*n)
    ys, xs = np.mgrid[0:h, 0:w]
    return ((xs - point[0]) * n[0] + (ys - point[1]) * n[1]) / ppd


def sharpness(picture, distance, near, far):
    """Mean squared horizontal luma gradient in the band and either side of it."""
    grad = np.diff(picture.astype(np.float64), axis=1) ** 2
    d = np.abs(distance[:, :-1])
    inside, outside = d <= near, (d >= far[0]) & (d <= far[1])
    if inside.sum() < 2000 or outside.sum() < 2000:
        return None
    return float(grad[inside].mean()), float(grad[outside].mean()), int(inside.sum()), int(outside.sum())


def main():
    got = frames()
    usable = [f for f in got if f[4] is not None]
    print(f"=== the corridor in both pictures, t {WINDOW[0]}-{WINDOW[1]} s ===")
    print(f"  {len(got)} frames, {len(usable)} with a registration")
    rows = []
    for t, a, b, (x, y, tilt), ends, inliers in usable:
        direction_kj = np.array([np.cos(np.radians(tilt)), np.sin(np.radians(tilt))])
        d_kj = distance_map(a.shape, (x, y), direction_kj, KJ_PPD)
        direction_st = ends[1] - ends[0]
        d_st = distance_map(b.shape, ends[0], direction_st, ST_PPD)
        got_kj = sharpness(a, d_kj, NEAR_DEG, FAR_DEG)
        got_st = sharpness(b, d_st, NEAR_DEG, FAR_DEG)
        rows.append((t, inliers, ends[0], got_kj, got_st))
    with open(f"{OUT}/corridor-share.csv", "w") as fh:
        fh.write(
            "# mean squared horizontal luma gradient inside the seam corridor and either side,\n"
            f"# corridor = within {NEAR_DEG} deg of the seam, either side = {FAR_DEG[0]} to"
            f" {FAR_DEG[1]} deg, in each picture's own pixels.\n"
            "# a doubled edge lowers the share and a single one does not, so each stitch is its\n"
            "# own control and the two makers' tone curves divide out.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00"
            " lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4\n"
            "# seam placed in Studio's picture through the ORB homography, see corridor.py\n"
            "time_s,inliers,seam_u_px,seam_v_px,kj_in,kj_out,kj_share,st_in,st_out,st_share\n"
        )
        for t, inliers, hub, kj, st in rows:
            if kj is None or st is None:
                continue
            fh.write(
                f"{t:.4f},{inliers},{hub[0]:.1f},{hub[1]:.1f},{kj[0]:.2f},{kj[1]:.2f},"
                f"{kj[0]/kj[1]:.4f},{st[0]:.2f},{st[1]:.2f},{st[0]/st[1]:.4f}\n"
            )
    good = [(kj, st) for _, _, _, kj, st in rows if kj and st]
    for name, index in (("kjerag", 0), ("Studio", 1)):
        shares = np.array([g[index][0] / g[index][1] for g in good])
        print(f"  {name:<8} share {shares.mean():.4f} +- {shares.std():.4f} over {len(shares)} frames")
    print(f"  wrote {OUT}/corridor-share.csv")
    return rows


if __name__ == "__main__":
    main()
