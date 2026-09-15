#!/usr/bin/env python3
"""Reject invalid or flat, control-free grim PPM captures in the UI harness.

This is a visibility check for moving test footage, not proof of the displayed
source's identity. A completely uniform video frame is deliberately inconclusive.
Dark textured pictures are valid; no brightness or colour threshold is used.
The caller must exclude in-picture overlays: a toast is also non-flat pixels.
"""

import os
from pathlib import Path
import sys


def visible_picture(path: Path, header_rows: int, control_rows: int) -> bool:
    # grim emits a binary P6 image with this three-line header. Reject other
    # layouts rather than having this check and uitest.sh's picture() disagree.
    with path.open("rb") as capture:
        if capture.readline(128) != b"P6\n":
            return False
        dimensions = capture.readline(128)
        width, height = map(int, dimensions.split())
        if dimensions != f"{width} {height}\n".encode("ascii"):
            return False
        if capture.readline(128) != b"255\n":
            return False
        if width <= 0 or min(header_rows, control_rows) < 0:
            return False
        if height <= header_rows + control_rows:
            return False
        payload_start = capture.tell()
        payload_size = width * height * 3
        if os.fstat(capture.fileno()).st_size - payload_start != payload_size:
            return False
        capture.seek(payload_start + header_rows * width * 3)
        picture_size = (height - header_rows - control_rows) * width * 3
        picture = capture.read(picture_size)
    return len(picture) == picture_size and picture != picture[:3] * (len(picture) // 3)


def main() -> int:
    if len(sys.argv) != 4:
        print("usage: check-ui-picture.py capture.ppm header_rows control_rows", file=sys.stderr)
        return 2
    try:
        visible = visible_picture(Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]))
    except (OSError, ValueError):
        return 1
    return 0 if visible else 1


if __name__ == "__main__":
    sys.exit(main())
