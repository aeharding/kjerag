# Branch-only research instrument (research/oracle-probe, 2026-08-06): the Studio
# oracle probe against ~/Videos/Insta/studio_exports. Not on any shipped path and
# not wired into a gate. Every number it writes carries its own source stamp.
"""Does anything wobble in the seam corridor that does not wobble beside it?

SOURCES: as corridor.py; read that file's header for both arms' full stamp and
for how the seam is placed in Studio's picture.

METHOD (the shimmer campaign's, adapted to an arm with no band-off control)
  There is no second arm for Studio's export, so the differential the shimmer
  scripts used is not available. What is available inside one arm is the same
  statistic in two places: the corridor, and rows of the SAME picture a few
  degrees off it.

  Both arms are brought to 51.20 px/deg. Global motion is taken out frame to
  frame by phase correlation on a band 3 to 7 degrees off the seam, which is
  content the corridor cannot reach, and the same numbers are applied to the
  whole picture. A slit of 8 columns at the picture's centre is stacked over the
  window into a strip, and roughness is the mean absolute SECOND difference of
  luma along the time axis: content that has been compensated correctly is
  smooth along time, a bend that changes under it is not.

  What is read is the corridor's roughness OVER the away rows' roughness in the
  same arm and the same strip. That floor is not zero for either arm: the
  compensation is one whole-picture translation rounded to a pixel and parallax,
  zoom and rounding all leave something behind, and for Studio the export's own
  HEVC quantization leaves more. `plant=` puts a per-frame displacement of known
  size into the corridor so the floor can be priced before a flat reading is
  called flat.
"""

import sys
import numpy as np
import cv2

sys.path.insert(0, "/tmp/claude-1000/-home-aeharding-wingover/09ccdb9e-e174-4e20-af54-412298c7ce58/scratchpad/oracle")
import corridor

OUT = corridor.OUT
PPD = 51.20
SLIT = 8
NEAR_DEG = 1.5
FAR = (3.0, 5.0)
TRACK = (3.0, 7.0)   # the band the global motion is read on


def compensate(a, b):
    """Translation putting b onto a, by phase correlation."""
    window = cv2.createHanningWindow((a.shape[1], a.shape[0]), cv2.CV_32F)
    (dx, dy), _ = cv2.phaseCorrelate(a.astype(np.float32), b.astype(np.float32), window)
    return dy, dx


def strip_of(pictures, seams, plant=0.0):
    """(strip, seam row per frame in the compensated picture)."""
    height, width = pictures[0].shape
    band = []
    for picture, (hub, direction) in zip(pictures, seams):
        d = direction / np.hypot(*direction)
        n = np.array([-d[1], d[0]])
        ys, xs = np.mgrid[0:height, 0:width]
        distance = ((xs - hub[0]) * n[0] + (ys - hub[1]) * n[1]) / PPD
        band.append(distance)
    if plant:
        pushed = []
        for index, (picture, distance) in enumerate(zip(pictures, band)):
            shift = plant * (1.0 if index % 2 == 0 else -1.0)
            rolled = np.roll(picture, int(np.sign(shift)), axis=1) if abs(shift) >= 1 else picture
            fine = cv2.warpAffine(
                picture.astype(np.float32), np.float32([[1, 0, shift], [0, 1, 0]]),
                (width, height), flags=cv2.INTER_CUBIC, borderMode=cv2.BORDER_REFLECT,
            )
            inside = np.abs(distance) <= NEAR_DEG
            out = picture.astype(np.float32).copy()
            out[inside] = fine[inside]
            pushed.append(out)
        pictures = pushed
        del rolled
    offsets = [(0.0, 0.0)]
    for k in range(len(pictures) - 1):
        mask = (np.abs(band[k]) >= TRACK[0]) & (np.abs(band[k]) <= TRACK[1])
        rows = np.where(mask.any(axis=1))[0]
        if len(rows) < 40:
            offsets.append(offsets[-1])
            continue
        top, bottom = rows[0], rows[-1]
        a = np.asarray(pictures[k])[top:bottom, :]
        b = np.asarray(pictures[k + 1])[top:bottom, :]
        dy, dx = compensate(a, b)
        offsets.append((offsets[-1][0] + dy, offsets[-1][1] + dx))
    at = width // 2
    slices, seam_rows = [], []
    for k, picture in enumerate(pictures):
        rolled = np.roll(np.roll(np.asarray(picture, dtype=np.float64),
                                 -int(round(offsets[k][0])), axis=0),
                         -int(round(offsets[k][1])), axis=1)
        slices.append(rolled[:, at - SLIT // 2: at + SLIT // 2].mean(axis=1))
        column = band[k][:, at]
        seam_rows.append(float(np.argmin(np.abs(column))) - offsets[k][0])
    return np.stack(slices, axis=1), np.array(seam_rows)


def roughness(strip, seam_rows):
    rows = np.arange(strip.shape[0])[:, None]
    distance = np.abs(rows - seam_rows[None, :]) / PPD
    bend = np.abs(np.diff(strip, 2, axis=1))
    inside = (distance <= NEAR_DEG)[:, 1:-1]
    away = ((distance >= FAR[0]) & (distance <= FAR[1]))[:, 1:-1]
    if inside.sum() < 500 or away.sum() < 500:
        return None
    return float(bend[inside].mean()), float(bend[away].mean()), int(inside.sum()), int(away.sum())


def main():
    got = [f for f in corridor.frames() if f[4] is not None]
    shrink = PPD / corridor.ST_PPD
    arms = {"kjerag": [], "studio": []}
    for t, a, b, (x, y, tilt), ends, inliers in got:
        arms["kjerag"].append((a, (np.array([x, y]),
                                   np.array([np.cos(np.radians(tilt)), np.sin(np.radians(tilt))]))))
        small = cv2.resize(b, None, fx=shrink, fy=shrink, interpolation=cv2.INTER_AREA)
        arms["studio"].append((small, (ends[0] * shrink, ends[1] - ends[0])))
    print(f"=== corridor roughness along time, {len(got)} frames, "
          f"t {got[0][0]:.3f}-{got[-1][0]:.3f} s, 51.20 px/deg ===")
    print("  arm      plant px   corridor   away   corridor over away   excess")
    lines = []
    for arm, items in arms.items():
        pictures = [p for p, _ in items]
        seams = [s for _, s in items]
        for plant in (0.0, 0.1, 0.25, 0.5, 1.0):
            strip, rows = strip_of(pictures, seams, plant)
            got_it = roughness(strip, rows)
            if got_it is None:
                print(f"  {arm:<8} {plant:8.2f}   too few rows")
                continue
            near, away, n_in, n_out = got_it
            print(f"  {arm:<8} {plant:8.2f} {near:10.4f} {away:7.4f} {near/away:20.4f}"
                  f" {near-away:+9.4f}")
            lines.append((arm, plant, near, away, n_in, n_out))
    with open(f"{OUT}/corridor-motion.csv", "w") as fh:
        fh.write(
            "# mean |second difference| of luma along time in a motion compensated slit strip,\n"
            f"# 8 columns at the picture centre, corridor = within {NEAR_DEG} deg of the seam,\n"
            f"# away = {FAR[0]} to {FAR[1]} deg, both arms at 51.20 px/deg, {len(got)} frames.\n"
            "# plant_px is a per-frame alternating displacement put into the corridor only,\n"
            "# so the floor can be priced before a flat reading is called flat.\n"
            "# kjerag: band mode=sequence from=24.0 count=1080 yaw=3.78 pitch=5.44 fov=20.00"
            " lock=1 size=1024 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91\n"
            "#   over /home/aeharding/Videos/Insta/VID_20260714_193252_00_006.insv,"
            " worktree research/oracle-probe at origin/main 67a4bcf\n"
            "# studio: /home/aeharding/Videos/Insta/studio_exports/20-fov-creek.mp4"
            " FOV 20 Distortion 0 pan -53.7 tilt 3.5 roll 0\n"
            "arm,plant_px,corridor_codes,away_codes,corridor_pixels,away_pixels\n"
        )
        for arm, plant, near, away, n_in, n_out in lines:
            fh.write(f"{arm},{plant},{near:.5f},{away:.5f},{n_in},{n_out}\n")
    print(f"  wrote {OUT}/corridor-motion.csv")


if __name__ == "__main__":
    main()
