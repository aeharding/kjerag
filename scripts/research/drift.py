# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""Where kjerag's delivered view points, against where Studio's export points.

SOURCES
  kjerag  /home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window/
          `band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00
           lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`
          over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,
          worktree research/oracle-probe at origin/main 67a4bcf.
          Frame k is source frame 719+k at (719+k)/29.97003 s (the run's own report:
          "frame: 719 at 23.991 s").
  studio  /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4 (Insta360
          Studio optical-flow stitch, Chromatic Calibration on, FOV 20,
          Distortion 0, pan -53.7, tilt 3.5, roll 0), gray PNGs from
          `ffmpeg -ss 30 -frames:v 900 -vf format=gray`. Frame j is at 30+j/30 s.

METHOD
  Both views are rectilinear, so one maps into the other by a homography, fitted
  by ORB + RANSAC on CLAHE-equalized luma. What is READ off the fit is only where
  kjerag's picture centre lands in Studio's picture, turned into an angle through
  Studio's own focal length (960/tan(10 deg) = 5444.3 px). The homography is NOT
  decomposed into a rotation: at 20 degrees of view a rotation and a translation
  are nearly the same picture, so the projective terms are noise and a decomposed
  angle reads several degrees off a planted one. The mapped centre does not:
  planted rotations of 0.5 to 8 degrees read back to 0.02 deg, and a planted zero
  reads 0.005.

  There is NO world reference here and none is claimed. This reads the DIFFERENCE
  between the two views' pointing. The aircraft's own turn is in both and cancels.
"""

import sys
import numpy as np
import cv2

KJ = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window"
ST = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/studio-creek"
OUT = "/tmp/claude-1000/-home-aeharding-wingover/09ccdb9e-e174-4e20-af54-412298c7ce58/scratchpad/oracle"
STEM = "VID_20260714_193252_00_006"
FPS = 30000.0 / 1001.0
FIRST = 719
KJ_SIZE, KJ_FOV = 1024, 20.0
ST_W, ST_H, ST_FOV = 1920, 1080, 20.0

F_KJ = (KJ_SIZE / 2.0) / np.tan(np.radians(KJ_FOV / 2.0))
F_ST = (ST_W / 2.0) / np.tan(np.radians(ST_FOV / 2.0))
K_KJ = np.array([[F_KJ, 0, KJ_SIZE / 2.0], [0, F_KJ, KJ_SIZE / 2.0], [0, 0, 1.0]])
K_ST = np.array([[F_ST, 0, ST_W / 2.0], [0, F_ST, ST_H / 2.0], [0, 0, 1.0]])

ORB = cv2.ORB_create(nfeatures=6000, scaleFactor=1.12, nlevels=14, fastThreshold=5)
MATCHER = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
CLAHE = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))
KEEP_INLIERS = 40


def kjerag(index):
    return cv2.imread(f"{KJ}/{STEM}-{index:04d}.png", cv2.IMREAD_GRAYSCALE)


def studio(index):
    return cv2.imread(f"{ST}/creek-{index:04d}.png", cv2.IMREAD_GRAYSCALE)


def pair(t):
    return int(round(t * FPS)) - FIRST, int(round((t - 30.0) * 30.0))


def homography(a, b):
    ea, eb = CLAHE.apply(a), CLAHE.apply(b)
    ka, da = ORB.detectAndCompute(ea, None)
    kb, db = ORB.detectAndCompute(eb, None)
    if da is None or db is None or len(ka) < 12 or len(kb) < 12:
        return None
    knn = MATCHER.knnMatch(da, db, k=2)
    good = [m for m, n in (p for p in knn if len(p) == 2) if m.distance < 0.78 * n.distance]
    if len(good) < 15:
        return None
    src = np.float32([ka[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
    dst = np.float32([kb[m.trainIdx].pt for m in good]).reshape(-1, 1, 2)
    H, mask = cv2.findHomography(src, dst, cv2.RANSAC, 2.0, maxIters=8000, confidence=0.9995)
    if H is None or mask is None:
        return None
    keep = mask.ravel().astype(bool)
    if keep.sum() < 8:
        return None
    moved = cv2.perspectiveTransform(src[keep], H).reshape(-1, 2)
    residual = float(np.sqrt(((moved - dst[keep].reshape(-1, 2)) ** 2).sum(axis=1)).mean())
    return H, int(keep.sum()), len(good), residual, dst[keep].reshape(-1, 2).mean(axis=0)


def angles(H):
    p = H @ np.array([KJ_SIZE / 2.0, KJ_SIZE / 2.0, 1.0])
    u, v = p[0] / p[2], p[1] / p[2]
    return (
        np.degrees(np.arctan((u - ST_W / 2.0) / F_ST)),
        -np.degrees(np.arctan((v - ST_H / 2.0) / F_ST)),
        u,
        v,
    )


def fit(t):
    k, j = pair(t)
    a, b = kjerag(k), studio(j)
    if a is None or b is None:
        return None
    got = homography(a, b)
    if got is None:
        return dict(t=t, k=k, j=j, inliers=0)
    H, inliers, matches, residual, hub = got
    right, up, u, v = angles(H)
    return dict(
        t=t, k=k, j=j, right=right, up=up, u=u, v=v, inliers=inliers,
        matches=matches, residual=residual, reach=float(np.hypot(u - hub[0], v - hub[1])),
    )


def main():
    first, last, step = (float(v) for v in sys.argv[1:4])
    out = sys.argv[4]
    rows = [fit(float(t)) for t in np.arange(first, last + 1e-9, step)]
    rows = [r for r in rows if r]
    with open(out, "w") as fh:
        fh.write(
            "# kjerag's picture centre, as a direction in Studio's picture, degrees.\n"
            "# right>0: kjerag looks right of Studio.  up>0: kjerag looks above Studio.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00"
            " lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4"
            " FOV 20 Distortion 0 pan -53.7 tilt 3.5 roll 0\n"
            "# fit: ORB+RANSAC homography on CLAHE luma; the mapped centre is the reading.\n"
            "# reach_px: how far the mapped centre sits from the inlier centroid, i.e. how far"
            " the fit is being extrapolated.\n"
            f"# keep: inliers >= {KEEP_INLIERS}\n"
            "time_s,kjerag_frame,studio_frame,right_deg,up_deg,centre_u_px,centre_v_px,"
            "inliers,matches,residual_px,reach_px\n"
        )
        for r in rows:
            if r["inliers"] == 0:
                fh.write(f"{r['t']:.4f},{r['k']},{r['j']},,,,,0,0,,\n")
                continue
            fh.write(
                f"{r['t']:.4f},{r['k']},{r['j']},{r['right']:.5f},{r['up']:.5f},"
                f"{r['u']:.2f},{r['v']:.2f},{r['inliers']},{r['matches']},"
                f"{r['residual']:.3f},{r['reach']:.1f}\n"
            )
    kept = [r for r in rows if r["inliers"] >= KEEP_INLIERS]
    print(f"wrote {out}: {len(rows)} instants, {len(kept)} kept")
    for r in kept[:: max(1, len(kept) // 30)]:
        print(
            f"  t={r['t']:7.3f}  right {r['right']:+8.3f}  up {r['up']:+8.3f}"
            f"  inliers {r['inliers']:5d}  residual {r['residual']:5.2f} px  reach {r['reach']:6.0f} px"
        )


if __name__ == "__main__":
    main()
