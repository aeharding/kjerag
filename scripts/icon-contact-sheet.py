#!/usr/bin/env python3
"""Render resources/icon-drafts/*.svg as a contact sheet for design review.

Each candidate is shown at 128, 64 and 32 px (true 1:1 rasters, so small-size
legibility is judged honestly), plus the 32 px raster magnified 4x with
nearest-neighbour so its pixels can be inspected without squinting. Every row
is repeated on the COSMIC light and dark desktop greys.

Backgrounds are cosmic-theme's `gray_1`: #D7D7D7 light, #1B1B1B dark
(pop-os/libcosmic, cosmic-theme/src/model/{light,dark}.ron).

Takes a draft directory; with no argument it picks the highest-numbered
`resources/icon-drafts/round-*`. Transparent drafts are meant to show the
grey through, so the panels are the real desktop colours, not a checkerboard.

Needs `resvg` on PATH (cargo install resvg). Writes to scratch/, which is
gitignored: rendered PNGs are review artifacts, not source.
"""

import glob
import os
import re
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


def newest_round():
    rounds = sorted(
        glob.glob(os.path.join(DRAFTS, "round-*")),
        key=lambda p: int(re.search(r"(\d+)$", p).group(1)),
    )
    if not rounds:
        sys.exit(f"no round-* directories in {DRAFTS}")
    return rounds[-1]


def main():
    if not shutil.which("resvg"):
        sys.exit("resvg not found on PATH; try: cargo install resvg")

    src = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else newest_round())
    # Absolute: the sheet is written to scratch/, so relative hrefs would
    # resolve against that directory instead of the draft directory.
    candidates = sorted(glob.glob(os.path.join(src, "*.svg")))
    if not candidates:
        sys.exit(f"no candidate SVGs in {src}")
    os.makedirs(OUT, exist_ok=True)
    tag = os.path.basename(os.path.normpath(src))

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
        letter, _, rest = stem.partition("-")
        letter = letter.upper()
        y = HEADER + row * row_h
        mid = y + max(SIZES) / 2
        svg.append(f'<text x="{PAD}" y="{mid - 4}" font-family="{FONT}" '
                   f'font-size="27" font-weight="bold" fill="#FFFFFF">{letter}</text>')
        svg.append(f'<text x="{PAD}" y="{mid + 18}" font-family="{FONT}" '
                   f'font-size="11" fill="#F0F0F0">{rest}</text>')

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
    sheet = os.path.join(OUT, f"contact-sheet-{tag}.svg")
    with open(sheet, "w") as fh:
        fh.write("\n".join(svg))

    out = os.path.join(OUT, f"contact-sheet-{tag}.png")
    subprocess.run(["resvg", "--resources-dir", OUT, sheet, out], check=True)
    print(out)
    print(seam_check(candidates, tag))


def seam_check(candidates, tag):
    """Composite every draft over magenta.

    Abutting shapes can leave a hairline of background between them, which
    is invisible against the greys and obvious against magenta. Anything
    magenta inside a silhouette is a hole, not a gradient.
    """
    s, pad = 256, 8
    width = pad + len(candidates) * (s + pad)
    svg = [
        f'<svg width="{width}" height="{s + pad * 2}" '
        f'viewBox="0 0 {width} {s + pad * 2}" '
        'xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">',
        f'<rect width="{width}" height="{s + pad * 2}" fill="#FF00FF"/>',
    ]
    for i, path in enumerate(candidates):
        svg.append(f'<image xlink:href="{path}" x="{pad + i * (s + pad)}" '
                   f'y="{pad}" width="{s}" height="{s}"/>')
    svg.append("</svg>")

    src = os.path.join(OUT, f"seam-{tag}.svg")
    with open(src, "w") as fh:
        fh.write("\n".join(svg))
    out = os.path.join(OUT, f"seam-{tag}.png")
    subprocess.run(["resvg", src, out], check=True)
    return out


if __name__ == "__main__":
    main()
