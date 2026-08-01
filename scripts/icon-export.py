#!/usr/bin/env python3
"""Render the shipped icon rasters, check them, and draw the review sheet.

The drawings come from scripts/icon-diver.py. This script only exports them:
one PNG per hicolor size, each rendered from the drawing that size ships
(16, 24 and 32 have their own; everything larger comes off the scalable one).

Two things get checked rather than assumed, because both have been wrong
before in this repo's icon work:

- **Nothing is clipped.** Every raster must keep at least one clear pixel of
  margin on all four sides. A downscale of the 256 art reaches the boundary
  pixel at 16 and 24, which is why those sizes are redrawn on their own grid.
- **The alpha is one piece.** A hairline of background between two abutting
  shapes is invisible on the desktop greys and obvious as a hole in the
  silhouette, so the fill is flood-checked from the outside in.

Each line also reports how far past the world's rim the drawing reaches.
That is the quantity round 7 holds fixed while the figure grows, so a change
to the art that moves it shows up here (docs/icon.md).

Writes the PNGs next to their SVG under resources/icons/hicolor/, and the
review sheet to scratch/icon/final-sheet.png, which is gitignored: rendered
sheets are review artifacts, not source.

Needs `resvg` on PATH (cargo install resvg).
"""

import math
import os
import shutil
import struct
import subprocess
import sys
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "resources", "icons", "hicolor")
OUT = os.path.join(ROOT, "scratch", "icon")
APPID = "dev.harding.Kjerag"

SIZES = (256, 128, 64, 48, 32, 24, 16)
REDRAWN = (32, 24, 16)  # sizes with a drawing of their own
BLOWUP = (48, 32, 24, 16)
MAG = 6
MIN_MARGIN = 1  # clear pixels required on every side, at every size

# The world's radius in each raster. Everything off the 256 grid carries the
# full-bleed circle, r=120 of 128; 24 and 16 are drawn on their own grid.
SMALL_RIM = {24: 11, 16: 7}

# cosmic-theme's gray_1, light and dark (pop-os/libcosmic,
# cosmic-theme/src/model/{light,dark}.ron).
BACKGROUNDS = (("light", "#D7D7D7", "#1B1B1B"), ("dark", "#1B1B1B", "#EDEDED"))
FONT = "Fira Sans"
LABEL_W, PAD, GAP, HEADER = 96, 24, 18, 56


def source(size):
    d = f"{size}x{size}" if size in REDRAWN else "scalable"
    return os.path.join(ICONS, d, "apps", f"{APPID}.svg")


def raster(size):
    return os.path.join(ICONS, f"{size}x{size}", "apps", f"{APPID}.png")


def read_png(path):
    """Decode a non-interlaced 8-bit PNG to (width, height, rows of RGBA)."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path}: not a PNG")
    pos, idat = 8, b""
    width = height = channels = None
    while pos < len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        kind, chunk = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, colour, _, _, interlace = \
                struct.unpack(">IIBBBBB", chunk)
            if (depth, interlace, colour) != (8, 0, 6):
                sys.exit(f"{path}: expected 8-bit RGBA, got {depth}/{colour}")
            channels = 4
        elif kind == b"IDAT":
            idat += chunk
        elif kind == b"IEND":
            break
        pos += 12 + length
    raw, stride = zlib.decompress(idat), width * channels
    rows, prev, p = [], bytearray(stride), 0
    for _ in range(height):
        filt, line, p = raw[p], bytearray(raw[p + 1:p + 1 + stride]), p + 1 + stride
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up = prev[i]
            upleft = prev[i - channels] if i >= channels else 0
            if filt == 1:
                line[i] = (line[i] + left) & 255
            elif filt == 2:
                line[i] = (line[i] + up) & 255
            elif filt == 3:
                line[i] = (line[i] + ((left + up) >> 1)) & 255
            elif filt == 4:
                guess = left + up - upleft
                dl, du, dul = (abs(guess - left), abs(guess - up),
                               abs(guess - upleft))
                near = left if dl <= du and dl <= dul else (up if du <= dul else upleft)
                line[i] = (line[i] + near) & 255
        rows.append([tuple(line[x * 4:x * 4 + 4]) for x in range(width)])
        prev = line
    return width, height, rows


def margins(rows):
    """Clear pixels on each side, counting any non-transparent pixel as ink."""
    h, w = len(rows), len(rows[0])
    xs = [x for y in range(h) for x in range(w) if rows[y][x][3]]
    ys = [y for y in range(h) for x in range(w) if rows[y][x][3]]
    if not xs:
        return None
    return min(xs), min(ys), w - 1 - max(xs), h - 1 - max(ys)


def past_rim(rows):
    """How far the drawing reaches beyond the world's rim, in this raster's px.

    The diver's feet are the only thing outside the circle, so this measures
    the jut the drawing was placed for, off the rendered alpha rather than
    off the skeleton that asked for it.
    """
    size = len(rows)
    centre = size / 2
    ink = max(math.hypot(x + 0.5 - centre, y + 0.5 - centre)
              for y in range(size) for x in range(size) if rows[y][x][3])
    return ink - SMALL_RIM.get(size, size * 120 / 256)


def holes(rows):
    """Transparent pixels enclosed by ink: a seam between abutting shapes."""
    h, w = len(rows), len(rows[0])
    seen = [[False] * w for _ in range(h)]
    stack = [(x, y) for x in range(w) for y in (0, h - 1)]
    stack += [(x, y) for y in range(h) for x in (0, w - 1)]
    while stack:
        x, y = stack.pop()
        if not (0 <= x < w and 0 <= y < h) or seen[y][x] or rows[y][x][3]:
            continue
        seen[y][x] = True
        stack += [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
    return sum(1 for y in range(h) for x in range(w)
               if not rows[y][x][3] and not seen[y][x])


def export():
    failures = []
    for size in SIZES:
        src, dst = source(size), raster(size)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        subprocess.run(["resvg", "--width", str(size), "--height", str(size),
                        src, dst], check=True)
        _, _, rows = read_png(dst)
        edge, hole = margins(rows), holes(rows)
        note = "own drawing" if size in REDRAWN else "from scalable"
        if edge is None:
            failures.append(f"{size}: empty raster")
            continue
        if min(edge) < MIN_MARGIN:
            failures.append(f"{size}: margin {edge}, want >= {MIN_MARGIN} px")
        if hole:
            failures.append(f"{size}: {hole} enclosed transparent pixels")
        print(f"{size:>4}  margins l/t/r/b {edge}  holes {hole}  "
              f"past rim {past_rim(rows):+.1f}px  ({note})")
    return failures


def sheet():
    """One picture of every shipped size on both desktop greys."""
    strip = sum(SIZES) + GAP * len(SIZES)
    zoom = sum(s * MAG for s in BLOWUP) + GAP * len(BLOWUP)
    panel = PAD * 2 + max(strip, zoom)
    width = LABEL_W + len(BACKGROUNDS) * (panel + GAP)
    top, bottom = HEADER + max(SIZES), HEADER + max(SIZES) + 40
    height = bottom + max(BLOWUP) * MAG + PAD * 2

    svg = [f'<svg width="{width}" height="{height}" viewBox="0 0 {width} '
           f'{height}" xmlns="http://www.w3.org/2000/svg" '
           'xmlns:xlink="http://www.w3.org/1999/xlink">',
           f'<rect width="{width}" height="{height}" fill="#8A8A8E"/>']

    for col, (name, bg, fg) in enumerate(BACKGROUNDS):
        left = LABEL_W + col * (panel + GAP)
        svg.append(f'<rect x="{left}" y="{HEADER - 34}" width="{panel}" '
                   f'height="{height - HEADER + 22}" fill="{bg}"/>')
        svg.append(f'<text x="{left + PAD}" y="{HEADER - 44}" '
                   f'font-family="{FONT}" font-size="17" fill="#FFFFFF">'
                   f'COSMIC {name}</text>')

        x = left + PAD
        for size in SIZES:
            svg.append(f'<text x="{x}" y="{HEADER - 14}" font-family="{FONT}" '
                       f'font-size="12" fill="{fg}" opacity="0.75">{size}</text>')
            svg.append(f'<rect x="{x}" y="{top - size}" width="{size}" '
                       f'height="{size}" fill="none" stroke="{fg}" '
                       'stroke-opacity="0.22" stroke-width="1"/>')
            svg.append(f'<image xlink:href="{raster(size)}" x="{x}" '
                       f'y="{top - size}" width="{size}" height="{size}"/>')
            x += size + GAP

        x = left + PAD
        for size in BLOWUP:
            tag = "own drawing" if size in REDRAWN else "from scalable"
            svg.append(f'<text x="{x}" y="{bottom - 8}" font-family="{FONT}" '
                       f'font-size="12" fill="{fg}" opacity="0.75">'
                       f'{size} x{MAG}, {tag}</text>')
            svg.append(f'<image xlink:href="{raster(size)}" x="{x}" '
                       f'y="{bottom}" width="{size * MAG}" '
                       f'height="{size * MAG}" image-rendering="optimizeSpeed"/>')
            x += size * MAG + GAP

    svg.append(f'<text x="{PAD}" y="{top - 10}" font-family="{FONT}" '
               'font-size="20" font-weight="bold" fill="#FFFFFF">1:1</text>')
    svg.append(f'<text x="{PAD}" y="{bottom + 26}" font-family="{FONT}" '
               f'font-size="20" font-weight="bold" fill="#FFFFFF">x{MAG}</text>')
    svg.append("</svg>")

    os.makedirs(OUT, exist_ok=True)
    src = os.path.join(OUT, "final-sheet.svg")
    with open(src, "w") as fh:
        fh.write("\n".join(svg))
    out = os.path.join(OUT, "final-sheet.png")
    subprocess.run(["resvg", "--resources-dir", OUT, src, out], check=True)
    return out


def main():
    if not shutil.which("resvg"):
        sys.exit("resvg not found on PATH; try: cargo install resvg")
    failures = export()
    print(sheet())
    if failures:
        sys.exit("\n".join(["icon export failed:"] + failures))


if __name__ == "__main__":
    main()
