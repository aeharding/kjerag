#!/usr/bin/env python3
"""Turn a `--bin seam mode=solve` log's answer back into arguments.

Branch-only research instrument (research/parity-proof). The cross-validation
in docs/research/parity-protocol.md section 11.2 B2 fits the camera at one
aim's geometries and then HOLDS it while another aim is predicted, so the
second run has to be handed the first run's answer exactly rather than a
rounded copy of it typed out by hand.

    scripts/research/knobs.py <solve.txt>

prints one line, `start=<five> startradial=<five>:<five>`, ready to paste into
the next `seam mode=solve`.
"""

import re
import sys

NAMES = {
    "lens1 roll": ("fit", "roll"),
    "lens1 yaw": ("fit", "yaw"),
    "lens1 pitch": ("fit", "pitch"),
    "lens1 cx": ("fit", "cx"),
    "lens1 cy": ("fit", "cy"),
}


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    fit = {}
    radial = {0: [0.0] * 5, 1: [0.0] * 5}
    seen = set()
    with open(sys.argv[1], encoding="utf-8", errors="replace") as handle:
        for line in handle:
            # The answer table only. The correlation matrix further down opens
            # its rows with the same names, and reading those instead would
            # hand the next run a column of correlation coefficients wearing a
            # calibration's units.
            if line.startswith("=== arm"):
                break
            # `lens1 cx               -25.68296  ...`
            match = re.match(r"^(lens1 (?:roll|yaw|pitch|cx|cy))\s+(-?[0-9.eE+-]+)\s", line)
            if match and match.group(1) not in seen:
                seen.add(match.group(1))
                fit[NAMES[match.group(1)][1]] = float(match.group(2))
                continue
            match = re.match(r"^l([01]) rad([1-5])\s+(-?[0-9.eE+-]+)\s", line)
            if match and match.group(0)[:7] not in seen:
                seen.add(match.group(0)[:7])
                radial[int(match.group(1))][int(match.group(2)) - 1] = float(match.group(3))
    if len(fit) != 5:
        print(f"{sys.argv[1]}: found {len(fit)} of lens 1's five knobs", file=sys.stderr)
        return 1
    start = "roll:{roll:.6f},yaw:{yaw:.6f},pitch:{pitch:.6f},cx:{cx:.6f},cy:{cy:.6f}".format(**fit)
    both = ":".join(
        ",".join(f"{value:.9e}" for value in radial[lens]) for lens in (0, 1)
    )
    print(f"start={start} startradial={both}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
