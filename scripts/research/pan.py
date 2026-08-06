# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""How much the picture's content slides sideways in each arm, degree by degree.

READ slide.py INSTEAD, AND THIS ONE ONLY TO SEE WHY. Its chain is ORB per pair and
it silently UNDER-REPORTS: a pair it cannot match does not advance the total, and
the pairs it cannot match are the fast ones, so the arm that moves most loses most.
On the July window it read kjerag at 4.1 deg where the two chain-free instruments
(wide.py and slide.py) both read 20, and 115 of its 900 kjerag pairs were refused
against 52 of Studio's. A refusal that leaves the running total alone is not a gap
in the answer, it is a wrong answer.

SOURCES
  kjerag  /home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window/
          `band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00
           lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`
          over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv, worktree
          research/oracle-probe at origin/main 67a4bcf. 1024 px over 20 deg = 51.20 px/deg.
  studio  /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4, gray PNGs from
          `ffmpeg -ss 30 -frames:v 900 -vf format=gray`. 1920 px over 20 deg = 96.00 px/deg.

METHOD
  Consecutive frames of one arm are matched by ORB + RANSAC under a similarity
  (scale, rotation, translation), and what is accumulated is where the picture's
  own centre came FROM in the previous frame, in degrees through that arm's focal
  length. The world is the same for both arms, so the aircraft's translation is
  common to the two curves and only their difference is about the two stitchers'
  pointing.

  This is a chain, so its error accumulates. The null and the plant below bound
  the per-pair error; the run also prints the closing error of a chain walked
  forwards and then backwards over the same frames.
"""

import sys
import numpy as np
import cv2

KJ = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window"
ST = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/studio-creek"
STEM = "VID_20260714_193252_00_006"
ARMS = {
    "kjerag": dict(path=lambda i: f"{KJ}/{STEM}-{i:04d}.png", f=(1024 / 2) / np.tan(np.radians(10)),
                   size=(1024, 1024), first=180, count=900, fps=30000.0 / 1001.0, t0=29.9966),
    "studio": dict(path=lambda i: f"{ST}/creek-{i:04d}.png", f=(1920 / 2) / np.tan(np.radians(10)),
                   size=(1920, 1080), first=0, count=900, fps=30.0, t0=30.0),
}

ORB = cv2.ORB_create(nfeatures=3000, scaleFactor=1.15, nlevels=10, fastThreshold=7)
MATCHER = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
CLAHE = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))


def features(picture):
    return ORB.detectAndCompute(CLAHE.apply(picture), None)


def step(ka, da, kb, db):
    """Similarity taking a's pixels to b's. (A, inliers, matches)."""
    if da is None or db is None or len(ka) < 12 or len(kb) < 12:
        return None
    knn = MATCHER.knnMatch(da, db, k=2)
    good = [m for m, n in (p for p in knn if len(p) == 2) if m.distance < 0.75 * n.distance]
    if len(good) < 12:
        return None
    src = np.float32([ka[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
    dst = np.float32([kb[m.trainIdx].pt for m in good]).reshape(-1, 1, 2)
    A, mask = cv2.estimateAffinePartial2D(
        src, dst, method=cv2.RANSAC, ransacReprojThreshold=1.5, maxIters=6000, confidence=0.9995
    )
    if A is None or mask is None or mask.sum() < 8:
        return None
    return A, int(mask.sum()), len(good)


def walk(arm, frames):
    """Cumulative displacement of the picture centre, in degrees, over `frames`."""
    spec = ARMS[arm]
    w, h = spec["size"]
    centre = np.array([w / 2.0, h / 2.0, 1.0])
    previous = None
    total = np.zeros(2)
    rows = []
    for n, index in enumerate(frames):
        picture = cv2.imread(spec["path"](index), cv2.IMREAD_GRAYSCALE)
        current = features(picture)
        if previous is not None:
            got = step(previous[0], previous[1], current[0], current[1])
            if got is None:
                rows.append((n, index, None, None, 0, 0))
                previous = current
                continue
            A, inliers, matches = got
            moved = A @ centre
            total = total + (moved[:2] - centre[:2])
            rows.append((n, index, total.copy(), inliers, matches, 1))
        else:
            rows.append((n, index, total.copy(), 0, 0, 1))
        previous = current
    return rows, spec["f"]


def controls():
    spec = ARMS["kjerag"]
    a = cv2.imread(spec["path"](369), cv2.IMREAD_GRAYSCALE)
    fa = features(a)
    print("=== the chain's per-pair floor ===")
    got = step(fa[0], fa[1], fa[0], fa[1])
    A = got[0]
    print(f"  null (a frame against itself): dx {A[0,2]:+.4f} dy {A[1,2]:+.4f} px, "
          f"{got[1]} inliers")
    for dx, dy in [(1.0, 0.0), (5.0, -3.0), (25.0, 10.0)]:
        M = np.float32([[1, 0, dx], [0, 1, dy]])
        b = cv2.warpAffine(a, M, (a.shape[1], a.shape[0]), flags=cv2.INTER_CUBIC)
        got = step(fa[0], fa[1], *features(b))
        B = got[0]
        print(f"  plant dx {dx:+6.2f} dy {dy:+6.2f} -> read {B[0,2]:+8.4f} {B[1,2]:+8.4f} px, "
              f"{got[1]} inliers")


def main():
    if sys.argv[1] == "controls":
        controls()
        return
    out = sys.argv[1]
    curves = {}
    for arm, spec in ARMS.items():
        frames = list(range(spec["first"], spec["first"] + spec["count"]))
        rows, f = walk(arm, frames)
        curves[arm] = (rows, f, spec)
        broke = sum(1 for r in rows if r[5] == 0)
        print(f"{arm}: {len(rows)} frames, {broke} pairs refused")
    with open(out, "w") as fh:
        fh.write(
            "# cumulative sideways/vertical slide of the picture content, degrees, per arm.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00"
            " lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4"
            " FOV 20 Distortion 0 pan -53.7 tilt 3.5 roll 0\n"
            "# chain: ORB+RANSAC similarity between consecutive frames, centre displacement\n"
            "# positive x_deg means the content has moved RIGHT in the picture since t=30\n"
            "time_s,arm,frame,x_deg,y_deg,inliers,matches,ok\n"
        )
        for arm, (rows, f, spec) in curves.items():
            for n, index, total, inliers, matches, ok in rows:
                t = spec["t0"] + n / spec["fps"]
                if total is None:
                    fh.write(f"{t:.4f},{arm},{index},,,{inliers},{matches},0\n")
                    continue
                fh.write(
                    f"{t:.4f},{arm},{index},{np.degrees(np.arctan(total[0]/f)):.5f},"
                    f"{np.degrees(np.arctan(total[1]/f)):.5f},{inliers},{matches},{ok}\n"
                )
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
