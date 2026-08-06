# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""Where Studio's export points, inside a wide kjerag render of the same instants.

SOURCES
  kjerag wide  /home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-wide/
      `band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=70.00 lock=1
       size=2048 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`
      over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv, worktree
      research/oracle-probe at origin/main 67a4bcf. Frame k is source frame 719+k
      at (719+k)/29.97003 s.
  studio  /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4, gray PNGs
      from `ffmpeg -ss 30 -frames:v 900 -vf format=gray`. Frame j is at 30+j/30 s.

WHY WIDE
  At 20 degrees each the two views stop overlapping whenever the two stitchers'
  pointing differs by more than about ten degrees, and over this window it often
  does, so a 20-against-20 registration can only be read where the answer is
  small. kjerag drawn at 70 degrees contains Studio's 20 whatever the offset is,
  up to 35 degrees, so the reading is available at every instant and never comes
  off a chain.

METHOD
  Studio's frame is matched into kjerag's wide frame by ORB + RANSAC under a
  homography (exact for two rectilinear views of one far field), and what is read
  is where STUDIO's picture centre lands in kjerag's picture, as an angle through
  kjerag's own focal length. Sign is then flipped so the column reads the same way
  round as the narrow run: right>0 means kjerag looks RIGHT of Studio.
"""

import sys
import numpy as np
import cv2

KJ = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-wide"
ST = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/studio-creek"
STEM = "VID_20260714_193252_00_006"
FPS = 30000.0 / 1001.0
FIRST = 719
KJ_SIZE, KJ_FOV = 2048, 70.0
ST_W, ST_H, ST_FOV = 1920, 1080, 20.0
F_KJ = (KJ_SIZE / 2.0) / np.tan(np.radians(KJ_FOV / 2.0))
F_ST = (ST_W / 2.0) / np.tan(np.radians(ST_FOV / 2.0))
# Studio is shrunk to kjerag's own scale before matching, so ORB sees one scale.
SHRINK = (F_KJ / F_ST)

ORB = cv2.ORB_create(nfeatures=8000, scaleFactor=1.12, nlevels=14, fastThreshold=5)
MATCHER = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
CLAHE = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))
KEEP = 30


def kjerag(index):
    return cv2.imread(f"{KJ}/{STEM}-{index:04d}.png", cv2.IMREAD_GRAYSCALE)


def studio(index):
    picture = cv2.imread(f"{ST}/creek-{index:04d}.png", cv2.IMREAD_GRAYSCALE)
    if picture is None:
        return None
    return cv2.resize(picture, None, fx=SHRINK, fy=SHRINK, interpolation=cv2.INTER_AREA)


def pair(t):
    return int(round(t * FPS)) - FIRST, int(round((t - 30.0) * 30.0))


def homography(a, b):
    """H taking a's pixels to b's."""
    ka, da = ORB.detectAndCompute(CLAHE.apply(a), None)
    kb, db = ORB.detectAndCompute(CLAHE.apply(b), None)
    if da is None or db is None or len(ka) < 12 or len(kb) < 12:
        return None
    knn = MATCHER.knnMatch(da, db, k=2)
    good = [m for m, n in (p for p in knn if len(p) == 2) if m.distance < 0.78 * n.distance]
    if len(good) < 15:
        return None
    src = np.float32([ka[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
    dst = np.float32([kb[m.trainIdx].pt for m in good]).reshape(-1, 1, 2)
    H, mask = cv2.findHomography(src, dst, cv2.RANSAC, 2.0, maxIters=8000, confidence=0.9995)
    if H is None or mask is None or mask.sum() < 8:
        return None
    keep = mask.ravel().astype(bool)
    moved = cv2.perspectiveTransform(src[keep], H).reshape(-1, 2)
    residual = float(np.sqrt(((moved - dst[keep].reshape(-1, 2)) ** 2).sum(axis=1)).mean())
    return H, int(keep.sum()), len(good), residual


def read(t):
    k, j = pair(t)
    a, b = studio(j), kjerag(k)
    if a is None or b is None:
        return None
    got = homography(a, b)
    if got is None:
        return dict(t=t, k=k, j=j, inliers=0)
    H, inliers, matches, residual = got
    centre = H @ np.array([a.shape[1] / 2.0, a.shape[0] / 2.0, 1.0])
    u, v = centre[0] / centre[2], centre[1] / centre[2]
    # Studio's axis inside kjerag's picture; negate for "kjerag relative to Studio".
    right = -np.degrees(np.arctan((u - KJ_SIZE / 2.0) / F_KJ))
    up = np.degrees(np.arctan((v - KJ_SIZE / 2.0) / F_KJ))
    return dict(t=t, k=k, j=j, right=right, up=up, u=u, v=v,
                inliers=inliers, matches=matches, residual=residual)


def controls():
    k, j = pair(36.303)
    a, b = studio(j), kjerag(k)
    print("=== null and plant, on the wide registration ===")
    got = homography(a, a)
    centre = got[0] @ np.array([a.shape[1] / 2, a.shape[0] / 2, 1.0])
    print(f"  null (Studio's frame against itself): centre moves "
          f"{centre[0]/centre[2] - a.shape[1]/2:+.4f}, {centre[1]/centre[2] - a.shape[0]/2:+.4f} px")
    base = read(36.303)
    for asked in (0.5, 2.0, 6.0, -10.0):
        rot = np.radians(asked)
        R = np.array([[np.cos(rot), 0, np.sin(rot)], [0, 1, 0], [-np.sin(rot), 0, np.cos(rot)]])
        K = np.array([[F_KJ, 0, KJ_SIZE / 2], [0, F_KJ, KJ_SIZE / 2], [0, 0, 1.0]])
        turned = cv2.warpPerspective(b, K @ R @ np.linalg.inv(K), (KJ_SIZE, KJ_SIZE),
                                     flags=cv2.INTER_CUBIC)
        got = homography(a, turned)
        if got is None:
            print(f"  plant {asked:+6.2f} deg: no fit")
            continue
        centre = got[0] @ np.array([a.shape[1] / 2.0, a.shape[0] / 2.0, 1.0])
        u = centre[0] / centre[2]
        right = -np.degrees(np.arctan((u - KJ_SIZE / 2.0) / F_KJ))
        print(f"  plant kjerag turned {asked:+6.2f} deg -> reading moves "
              f"{right - base['right']:+7.4f} deg ({got[1]} inliers)")


def main():
    if sys.argv[1] == "controls":
        controls()
        return
    first, last, step, out = float(sys.argv[1]), float(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
    rows = [read(float(t)) for t in np.arange(first, last + 1e-9, step)]
    rows = [r for r in rows if r]
    with open(out, "w") as fh:
        fh.write(
            "# how far kjerag's delivered view points from Studio's export's, degrees.\n"
            "# right>0: kjerag looks RIGHT of Studio.  up>0: kjerag looks ABOVE Studio.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=70.00"
            " lock=1 size=2048 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4"
            " FOV 20 Distortion 0 pan -53.7 tilt 3.5 roll 0, shrunk to kjerag's scale\n"
            "# fit: ORB+RANSAC homography, Studio into kjerag; the mapped centre is the reading\n"
            f"# keep: inliers >= {KEEP}\n"
            "time_s,kjerag_frame,studio_frame,right_deg,up_deg,centre_u_px,centre_v_px,"
            "inliers,matches,residual_px\n"
        )
        for r in rows:
            if r["inliers"] == 0:
                fh.write(f"{r['t']:.4f},{r['k']},{r['j']},,,,,0,0,\n")
                continue
            fh.write(
                f"{r['t']:.4f},{r['k']},{r['j']},{r['right']:.5f},{r['up']:.5f},"
                f"{r['u']:.2f},{r['v']:.2f},{r['inliers']},{r['matches']},{r['residual']:.3f}\n"
            )
    kept = [r for r in rows if r["inliers"] >= KEEP]
    print(f"wrote {out}: {len(rows)} instants, {len(kept)} kept ({100*len(kept)/max(1,len(rows)):.0f}%)")


if __name__ == "__main__":
    main()
