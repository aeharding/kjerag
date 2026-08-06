# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""How far the picture's content slides sideways in each arm, with no features in it.

The third instrument to read the July window's pointing difference and the one
with the fewest moving parts: dense frame to frame phase correlation over the
whole picture, accumulated, turned into degrees through that arm's own pixels per
degree. No feature detector, no homography, no model of the projection. It agrees
with wide.py's chain-free registration to about 15 percent, which is what makes
either of them believable; pan.py's ORB chain does not, and its own header says
why.

  python3 scripts/research/slide.py > scratch/oracle/slide-summary.csv
"""

import sys

import cv2
import numpy as np

KJ = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/kjerag-window"
ST = "/home/aeharding/kjerag/.worktrees/oracle-probe/scratch/oracle/studio-creek"
STEM = "VID_20260714_193252_00_006"
FPS = 30000.0 / 1001.0
FIRST = 719
SPANS = ((35.5, 37.5), (34.5, 37.5), (33.0, 39.0), (31.0, 41.0))


def kjerag(t):
    return cv2.imread(f"{KJ}/{STEM}-{int(round(t * FPS)) - FIRST:04d}.png", cv2.IMREAD_GRAYSCALE)


def studio(t):
    return cv2.imread(f"{ST}/creek-{int(round((t - 30.0) * 30.0)):04d}.png", cv2.IMREAD_GRAYSCALE)


def slide(load, per_degree, first, last, step):
    total = 0.0
    stamps = np.arange(first, last + 1e-9, step)
    for a, b in zip(stamps[:-1], stamps[1:]):
        one, two = load(float(a)).astype(np.float32), load(float(b)).astype(np.float32)
        window = cv2.createHanningWindow((one.shape[1], one.shape[0]), cv2.CV_32F)
        (dx, _), _ = cv2.phaseCorrelate(one, two, window)
        total += dx
    return total / per_degree


def main():
    print("# how far the picture's content slides sideways in each arm's own 20 degree view.")
    print("# instrument: dense frame-to-frame phase correlation over the whole picture,")
    print("#   accumulated, in degrees through that arm's own pixels per degree.")
    print("# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00")
    print("#   lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91")
    print("#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv, worktree")
    print("#   research/oracle-probe at origin/main 67a4bcf. 51.20 px/deg.")
    print("# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4, FOV 20,")
    print("#   Distortion 0, pan -53.7, tilt 3.5, roll 0. 96.00 px/deg.")
    print("# the aircraft's own translation is in both arms and does not cancel here; what")
    print("# makes the pair readable is that it is the same translation in both.")
    print("arm,from_s,to_s,slide_deg,rate_deg_per_s,rate_deg_per_min")
    for arm, load, per_degree, step in (
        ("kjerag", kjerag, 1024.0 / 20.0, 1.0 / FPS),
        ("studio", studio, 1920.0 / 20.0, 1.0 / 30.0),
    ):
        for first, last in SPANS:
            moved = slide(load, per_degree, first, last, step)
            print(f"{arm},{first},{last},{moved:.2f},{moved/(last-first):.3f},"
                  f"{60*moved/(last-first):.1f}")


if __name__ == "__main__":
    sys.exit(main())
