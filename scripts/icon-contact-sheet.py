#!/usr/bin/env python3
"""Render resources/icon-drafts/*.svg as a contact sheet for design review.

Each candidate is shown at 128, 64 and 32 px (true 1:1 rasters, so small-size
legibility is judged honestly), plus the 32 px raster magnified 4x with
nearest-neighbour so its pixels can be inspected without squinting. Every row
is repeated on the COSMIC light and dark desktop greys.

Backgrounds are cosmic-theme's `gray_1`: #D7D7D7 light, #1B1B1B dark
(pop-os/libcosmic, cosmic-theme/src/model/{light,dark}.ron).

Needs `resvg` on PATH (cargo install resvg). Writes to scratch/, which is
gitignored: rendered PNGs are review artifacts, not source.
"""

import glob
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DRAFTS = os.path.join(ROOT, "resources", "icon-drafts")
OUT = os.path.join(ROOT, "scratch", "icon")

SIZES = (128, 64, 32)
MAG = 4  # magnification of the 32 px raster
BACKGROUNDS = (("light", "#D7D7D7", "#1B1B1B"), ("dark", "#1B1B1B", "#EDEDED"))

LABEL_W, PAD, GAP, ROW_GAP, HEADER = 104, 22, 18, 22, 54
FONT = "Fira Sans"


def panel_width():
    return PAD * 2 + sum(SIZES) + GAP * len(SIZES) + 32 * MAG


def main():
    if not shutil.which("resvg"):
        sys.exit("resvg not found on PATH; try: cargo install resvg")

    candidates = sorted(glob.glob(os.path.join(DRAFTS, "*.svg")))
    if not candidates:
        sys.exit(f"no candidate SVGs in {DRAFTS}")
    os.makedirs(OUT, exist_ok=True)

    # True 32 px rasters, for the magnified column.
    for svg in candidates:
        stem = os.path.splitext(os.path.basename(svg))[0]
        subprocess.run(
            ["resvg", "--width", "32", "--height", "32", svg,
             os.path.join(OUT, f"{stem}-32.png")],
            check=True,
        )

    pw = panel_width()
    row_h = max(SIZES) + ROW_GAP
    width = LABEL_W + len(BACKGROUNDS) * (pw + GAP)
    height = HEADER + len(candidates) * row_h + PAD

    svg = [
        f'<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}" '
        'xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">',
        f'<rect width="{width}" height="{height}" fill="#8A8A8E"/>',
    ]

    for col, (name, bg, fg) in enumerate(BACKGROUNDS):
        x = LABEL_W + col * (pw + GAP)
        svg.append(f'<rect x="{x}" y="{HEADER - 30}" width="{pw}" '
                   f'height="{height - HEADER + 8}" fill="{bg}"/>')
        svg.append(f'<text x="{x + PAD}" y="{HEADER - 42}" font-family="{FONT}" '
                   f'font-size="17" fill="#FFFFFF">COSMIC {name}</text>')
        cx = x + PAD
        for s in SIZES:
            svg.append(f'<text x="{cx}" y="{HEADER - 12}" font-family="{FONT}" '
                       f'font-size="13" fill="{fg}" opacity="0.75">{s}</text>')
            cx += s + GAP
        svg.append(f'<text x="{cx}" y="{HEADER - 12}" font-family="{FONT}" '
                   f'font-size="13" fill="{fg}" opacity="0.75">32 x{MAG}</text>')

    for row, path in enumerate(candidates):
        stem = os.path.splitext(os.path.basename(path))[0]
        letter = stem[0].upper()
        y = HEADER + row * row_h
        mid = y + max(SIZES) / 2
        svg.append(f'<text x="{PAD}" y="{mid - 4}" font-family="{FONT}" '
                   f'font-size="30" font-weight="bold" fill="#FFFFFF">{letter}</text>')
        svg.append(f'<text x="{PAD}" y="{mid + 18}" font-family="{FONT}" '
                   f'font-size="11" fill="#F0F0F0">{stem[2:]}</text>')

        for col, _ in enumerate(BACKGROUNDS):
            x = LABEL_W + col * (pw + GAP) + PAD
            for s in SIZES:
                svg.append(f'<image xlink:href="{path}" x="{x}" '
                           f'y="{mid - s / 2}" width="{s}" height="{s}"/>')
                x += s + GAP
            png = os.path.join(OUT, f"{stem}-32.png")
            svg.append(f'<image xlink:href="{png}" x="{x}" y="{mid - 32 * MAG / 2}" '
                       f'width="{32 * MAG}" height="{32 * MAG}" '
                       'image-rendering="optimizeSpeed"/>')

    svg.append("</svg>")
    sheet = os.path.join(OUT, "contact-sheet.svg")
    with open(sheet, "w") as fh:
        fh.write("\n".join(svg))

    out = os.path.join(OUT, "contact-sheet.png")
    subprocess.run(["resvg", "--resources-dir", OUT, sheet, out], check=True)
    print(out)


if __name__ == "__main__":
    main()
