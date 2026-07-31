#!/usr/bin/env python3
"""Draw the app icon: a small diving figure and a big world.

The figure is built from a joint skeleton rather than traced by hand. Each
bone is the tangent trapezoid between two joint circles, plus a circle at
each joint; the union is a tapered limb with rounded joints. Three
hand-authored attempts at the same silhouette failed before this - guessing
bezier control points for a human profile gives a tadpole, and the way to
fix a tadpole is to move a joint, which is a number here and a re-trace
there.

Anatomy follows the 7.5-head canon at 200 units tall. The proportion that
actually decides whether it reads as a person is chest depth against head
depth: roughly 2:1. At 1:1 it is a tadpole no matter what the limbs do.

Writes the workshop drafts under resources/icon-drafts/round-{4,5,6}/ and
the shipped drawings under resources/icons/. Every shipped SVG comes out of
here, so an edit to the art is an edit to a number in this file followed by
a re-run; scripts/icon-export.py then renders and checks the rasters.
"""

import math
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DRAFTS = os.path.join(ROOT, "resources", "icon-drafts")
ICONS = os.path.join(ROOT, "resources", "icons")
APPID = "dev.harding.Kjerag"

# Joint positions and radii, in a local frame where +y is the dive
# direction: the head leads at +y, the feet trail at -y.
SKELETON = {
    "neck":   (2, 64, 12),   "chest":  (-2, 48, 23),
    "waist":  (-8, 12, 16),  "hip":    (-11, 0, 18),
    "knee":   (-13, -46, 13), "ankle":  (-26, -86, 8), "toe": (-32, -98, 3.5),
    "sh":     (-6, 54, 11),  "elbow":  (-36, 32, 9),
    "wrist":  (-52, 2, 7),   "hand":   (-58, -5, 6.5),
    "sh2":    (-2, 51, 10),  "elbow2": (-28, 34, 8.5),
    "wrist2": (-45, 8, 6.5), "hand2":  (-51, 1, 6),
    "hip2":   (-6, -2, 16),  "knee2":  (3, -48, 12),
    "ankle2": (2, -90, 7),   "toe2":   (1, -102, 3.5),
}
HEAD = (5, 82, 15, 18, -14)  # cx, cy, rx, ry, rotation

CHAINS = [
    ([("sh2", "elbow2"), ("elbow2", "wrist2"), ("wrist2", "hand2")],
     ["sh2", "elbow2", "wrist2", "hand2"]),
    ([("hip2", "knee2"), ("knee2", "ankle2"), ("ankle2", "toe2")],
     ["hip2", "knee2", "ankle2"]),
    ([("neck", "chest"), ("chest", "waist"), ("waist", "hip"),
      ("hip", "knee"), ("knee", "ankle"), ("ankle", "toe")],
     ["neck", "chest", "waist", "hip", "knee", "ankle"]),
]
ARM = ([("sh", "elbow"), ("elbow", "wrist"), ("wrist", "hand")],
       ["sh", "elbow", "wrist", "hand"])

# The world, unchanged from the round-2 draft the owner picked.
LAND = (
    '      <path d="M89 154C99 142 110 150 123 145C136 140 146 128 159 133'
    'C167 136 168 147 168 147L168 190L89 190Z" fill="#73C48F"/>\n'
    '      <path d="M89 154C99 142 110 150 123 145C136 140 146 128 159 133'
    'C167 136 168 147 168 147L168 156C168 156 166 144 158 142C147 139 136 149'
    ' 123 154C110 159 99 151 89 164Z" fill="#A6E0B4"/>'
)

def entry(cx, cy, radius, angle, scale, depth):
    """Place the figure so the world's rim crosses it at local y=`depth`.

    The dive runs along local +y, so `depth` names the landmark that sits on
    the rim: SKELETON["chest"][1] is chest-deep, ["waist"][1] waist-deep.
    Anything above the head's y leaves the figure clear of the world.
    """
    theta = math.radians(angle)
    dx, dy = -math.sin(theta), math.cos(theta)
    reach = radius + scale * depth
    return (f"translate({cx - reach * dx:.1f} {cy - reach * dy:.1f}) "
            f"rotate({angle}) scale({scale})")


ANKLE, KNEE = SKELETON["ankle"][1], SKELETON["knee"][1]

# name, world radius, world centre, figure transform, 32 px mark transform
ROUND4 = [
    ("h1-exit-line", 78, 162, 166,
     "translate(73 72) rotate(-40) scale(0.62)", "translate(90 84) rotate(-40)"),
    ("h2-piercing", 82, 128, 158,
     "translate(176 66) rotate(35) scale(0.58)", "translate(168 78) rotate(35)"),
    ("h3-long-way", 70, 128, 176,
     "translate(116 53) rotate(-8) scale(0.48)", "translate(120 62) rotate(-8)"),
]

# Round 5 converges on H2: diver moved to the left and mostly inside the
# world, with only a jut past the rim. The rim crosses the figure near the
# feet, so the depths below sit just either side of the ankle. Same world
# every time; only the entry changes.
# World nudged down; the spread is re-centred on the deeper jut the owner
# liked, so only how far the legs clear the rim varies. One angle for all
# three keeps the pick apples to apples.
_W = (134, 162, 84)
_S = 0.55
_A = -62
ROUND5 = [
    ("k1-shin-out", *_W[2:], _W[0], _W[1],
     entry(*_W, _A, _S, ANKLE + 20), entry(*_W, _A, 1.0, -22)),
    ("k2-calf-out", *_W[2:], _W[0], _W[1],
     entry(*_W, _A, _S, KNEE - 8), entry(*_W, _A, 1.0, -22)),
    ("k3-knee-out", *_W[2:], _W[0], _W[1],
     entry(*_W, _A, _S, KNEE + 4), entry(*_W, _A, 1.0, -22)),
]

# The figure's gradient is per round, because which end of it must be dark
# depends on what sits behind the head. In round 4 the head is against
# transparent sky and the light end advances; in round 5 it is against the
# world's pale land and the light end vanishes. Same figure, opposite ramp.
# Land shift is in land-local units, positive = down. Round 5 drops the
# landmass so open water sits under the diver's head; moving the world or
# the diver cannot do it, because entry() places the figure relative to the
# world and the head/land relationship travels with them.
# Round 6 pins the world to the platform's full-bleed circular convention:
# 240 diameter, centred, spanning 8..248 on the 256 grid. Measured, not
# guessed - Pop's circular app icons (accessories-clock, alarm-clock,
# avatar-default, web-browser at icon-theme 1a575a8) all draw a 238-240 wide
# circle centred in the 256 baseplate, and that 8-unit margin is the same
# live area the COSMIC app icons use for wide art.
#
# The world can therefore never move or shrink to make room for the diver.
# The jut instead goes into the corner headroom: entry runs along the
# top-left diagonal, so the feet reach into the square's corner, outside the
# inscribed circle but inside the canvas.
_W6 = (128, 128, 120)
_A6 = -45
ROUND6 = [
    ("n1-shin-out", *_W6[2:], _W6[0], _W6[1],
     entry(*_W6, _A6, _S, ANKLE + 20), entry(*_W6, _A6, 1.0, -22)),
    ("n2-calf-out", *_W6[2:], _W6[0], _W6[1],
     entry(*_W6, _A6, _S, KNEE - 8), entry(*_W6, _A6, 1.0, -22)),
    ("n3-knee-out", *_W6[2:], _W6[0], _W6[1],
     entry(*_W6, _A6, _S, KNEE + 4), entry(*_W6, _A6, 1.0, -22)),
]

# Round 7 grows the figure while the legs keep clearing the rim by the same
# absolute amount, so scale and depth have to move together: everything past
# the crossing point is outside the world, so a bigger figure must be
# crossed lower down to jut the same distance. The growth goes inward and
# the head reaches further along the entry line, deeper into open water.
#
# Both depths hold the outermost point at N2's 27.1 units past the rim.
# They are solved against the drawn geometry rather than named landmarks the
# way KNEE - 8 is, so they read as arbitrary; icon-export.py measures how far
# past the rim each rendered raster reaches, which keeps them honest.
ROUND7 = [
    ("q1-grown-9", *_W6[2:], _W6[0], _W6[1],
     entry(*_W6, _A6, 0.600, -58.11), entry(*_W6, _A6, 1.0, -22)),
    ("q2-grown-18", *_W6[2:], _W6[0], _W6[1],
     entry(*_W6, _A6, 0.649, -61.53), entry(*_W6, _A6, 1.0, -22)),
]

# Land shift is in land-local units, positive = down, and exists so open
# water sits under the diver's head. Round 6 needs far less of it than
# round 5: at the full radius the head sits further up the entry line, so 6
# is for balance and margin rather than to fix an overlap. Round 7 sinks the
# head back toward the coast and still needs no more of it.
ROUNDS = {
    "round-4": (ROUND4, ("#D9481F", "#F5A056"), 0),
    "round-5": (ROUND5, ("#EE9048", "#B8371A"), 14),
    "round-6": (ROUND6, ("#EE9048", "#B8371A"), 6),
    "round-7": (ROUND7, ("#EE9048", "#B8371A"), 6),
}


def bone(a, b):
    (ax, ay, ra), (bx, by, rb) = a, b
    dx, dy = bx - ax, by - ay
    length = math.hypot(dx, dy)
    if length < 1e-6 or abs(ra - rb) >= length:
        return None  # one joint swallows the other; its circle covers it
    ux, uy = dx / length, dy / length
    px, py = -uy, ux
    sin_a = (ra - rb) / length
    cos_a = math.sqrt(max(0.0, 1 - sin_a * sin_a))

    def tangent(jx, jy, r, side):
        return (jx + r * (sin_a * ux + side * cos_a * px),
                jy + r * (sin_a * uy + side * cos_a * py))

    pts = [tangent(ax, ay, ra, 1), tangent(bx, by, rb, 1),
           tangent(bx, by, rb, -1), tangent(ax, ay, ra, -1)]
    return "M" + "L".join(f"{x:.1f} {y:.1f}" for x, y in pts) + "Z"


def chain(bones, joints, table):
    out = []
    for a, b in bones:
        d = bone(table[a], table[b])
        if d:
            out.append(f'    <path d="{d}"/>')
    for name in joints:
        x, y, r = table[name]
        out.append(f'    <circle cx="{x}" cy="{y}" r="{r}"/>')
    return out


def figure():
    parts = []
    for bones, joints in CHAINS:
        parts += chain(bones, joints, SKELETON)
    cx, cy, rx, ry, rot = HEAD
    parts.append(f'    <ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}" '
                 f'transform="rotate({rot} {cx} {cy})"/>')
    parts += chain(*ARM, SKELETON)
    return "\n".join(parts)


def mark():
    """The 32 px stand-in: the figure reduced to a head and a tapering body.

    A 200-unit person is about 12 px at 32 and cannot hold limbs, so the
    small size gets its own drawing instead of a downscale of this one.
    """
    joints = {"head": (0, 22, 14), "chest": (-2, 3, 11), "tail": (-9, -34, 4)}
    return "\n".join(chain([("head", "chest"), ("chest", "tail")],
                           ["head", "chest"], joints))


def draft(key, radius, cx, cy, transform, body, y0, y1, stops, land_shift=0):
    inner = radius - 3
    return f'''<svg width="256" height="256" viewBox="0 0 256 256" fill="none" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="{key}-ocean" x1="{cx - radius + 8}" y1="{cy - radius + 8}" x2="{cx + radius}" y2="{cy + radius}" gradientUnits="userSpaceOnUse">
      <stop stop-color="#5CD0E2"/>
      <stop offset="1" stop-color="#1B7B9C"/>
    </linearGradient>
    <linearGradient id="{key}-fig" x1="0" y1="{y0}" x2="30" y2="{y1}" gradientUnits="userSpaceOnUse">
      <stop stop-color="{stops[0]}"/>
      <stop offset="1" stop-color="{stops[1]}"/>
    </linearGradient>
    <clipPath id="{key}-globe"><circle cx="{cx + 2}" cy="{cy + 2}" r="{inner}"/></clipPath>
  </defs>
  <circle cx="{cx}" cy="{cy}" r="{radius}" fill="#FFE0B0"/>
  <circle cx="{cx + 2}" cy="{cy + 2}" r="{inner}" fill="url(#{key}-ocean)"/>
  <g clip-path="url(#{key}-globe)">
    <g transform="translate({cx + 2} {cy + 2}) scale({inner / 41:.4f}) translate(-130 {-146 + land_shift})">
{LAND}
    </g>
  </g>
  <g transform="{transform}" fill="url(#{key}-fig)">
{body}
  </g>
</svg>
'''


# --- What ships -------------------------------------------------------------
#
# Round 7's Q2 is the approved drawing, so the scalable icon and the 32 px
# redraw are that candidate under the app's own name. 16 and 24 are separate
# drawings rather than exports of it, because that is what both icon sets do:
# in pop-os/icon-theme 1a575a8 the median app icon has 6 elements at 16 and 24
# against 6.5 at 32 and up, and the redraw is a real one where the art is busy
# (accessories-clock 8 against 21, accessories-text-editor 6 against 19).
FINAL = ROUND7[1]
FINAL_STOPS, FINAL_LAND_SHIFT = ROUNDS["round-7"][1], ROUNDS["round-7"][2]

# The coastline in world radii from the world's centre: the master drawing's
# three cubics reduced to one, measured off that path after its transform so
# the small sizes carry the same coast rather than a second one drawn by eye.
CREST = ((-1.05, 0.34), (-0.40, 0.28), (0.25, -0.30), (1.05, 0.14))
SHELF = 0.15  # how far under the crest the pale band reaches, in world radii

# 16 and 24 are drawn on their own pixel grid. Both sets do this - every 16 px
# file in pop-os/icon-theme and in cosmic-{player,files,edit} carries
# viewBox="0 0 16 16" - and it is the only way to hold the 1 px margin they
# keep at those sizes: rendered at 16 and 24 the 256 art reaches the boundary
# pixel, while accessories-clock, drawn on the small grid, does not.
#
# `depth` is the local y the world's rim crosses, so it sets how far the dart
# juts into the corner; `joints` is the whole drawing apart from the world.
SMALL = {
    16: dict(radius=7, rim=0.2, band=False, depth=-1.45,
             joints={"head": (0, 1.6, 1.1), "chest": (-0.2, -0.4, 0.85),
                     "tail": (-0.7, -2.9, 0.28)}),
    24: dict(radius=11, rim=0.3, band=True, depth=-1.9,
             joints={"head": (0, 2.2, 1.5), "chest": (-0.3, -0.5, 1.15),
                     "tail": (-1.0, -4.1, 0.38)}),
}


def num(v):
    """Trim a coordinate to two places, and to none when it has none."""
    return f"{v:.2f}".rstrip("0").rstrip(".")


def coast(c, r, drop):
    """The crest closed into a land mass, in canvas units. `drop` sinks it."""
    def at(u, v):
        return f"{num(c + u * r)} {num(c + (v + drop) * r)}"
    a, b, d, e = CREST
    return (f"M{at(*a)}C{at(*b)} {at(*d)} {at(*e)}"
            f"L{at(1.05, 1.4)}L{at(-1.05, 1.4)}Z")


def small_icon(size, key):
    """The 16 or 24 px drawing: world at full size, diver reduced to a dart."""
    spec = SMALL[size]
    r, c = spec["radius"], size / 2
    bands = [("#A6E0B4", 0.0), ("#73C48F", SHELF)] if spec["band"] \
        else [("#73C48F", 0.0)]
    land = "\n".join(f'    <path d="{coast(c, r, drop)}" fill="{fill}"/>'
                     for fill, drop in bands)
    joints = spec["joints"]
    dart = "\n".join(chain([("head", "chest"), ("chest", "tail")],
                           ["head", "chest"], joints))
    head, tail = joints["head"], joints["tail"]
    ocean = r - spec["rim"]
    return f'''<svg width="{size}" height="{size}" viewBox="0 0 {size} {size}" fill="none" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="{key}-ocean" x1="{num(c - r * 0.93)}" y1="{num(c - r * 0.93)}" x2="{num(c + r)}" y2="{num(c + r)}" gradientUnits="userSpaceOnUse">
      <stop stop-color="#5CD0E2"/>
      <stop offset="1" stop-color="#1B7B9C"/>
    </linearGradient>
    <linearGradient id="{key}-fig" x1="0" y1="{num(tail[1] - tail[2])}" x2="{num(r * 0.25)}" y2="{num(head[1] + head[2])}" gradientUnits="userSpaceOnUse">
      <stop stop-color="{FINAL_STOPS[0]}"/>
      <stop offset="1" stop-color="{FINAL_STOPS[1]}"/>
    </linearGradient>
    <clipPath id="{key}-globe"><circle cx="{num(c)}" cy="{num(c)}" r="{num(ocean)}"/></clipPath>
  </defs>
  <circle cx="{num(c)}" cy="{num(c)}" r="{num(r)}" fill="#FFE0B0"/>
  <circle cx="{num(c)}" cy="{num(c)}" r="{num(ocean)}" fill="url(#{key}-ocean)"/>
  <g clip-path="url(#{key}-globe)">
{land}
  </g>
  <g transform="{entry(c, c, r, -45, 1, spec['depth'])}" fill="url(#{key}-fig)">
{dart}
  </g>
</svg>
'''


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(text)
    print(os.path.relpath(path, ROOT))


def shipped(key="kjerag"):
    """The four drawings under resources/icons/hicolor/."""
    _, radius, cx, cy, transform, transform32 = FINAL
    args = (key, radius, cx, cy)
    tail = (FINAL_STOPS, FINAL_LAND_SHIFT)

    def apps(size):
        return os.path.join(ICONS, "hicolor", size, "apps", f"{APPID}.svg")

    write(apps("scalable"),
          draft(*args, transform, figure(), -104, 98, *tail))
    write(apps("32x32"),
          draft(*args, transform32, mark(), -38, 36, *tail))
    for size in (24, 16):
        write(apps(f"{size}x{size}"), small_icon(size, key))


def main():
    body, small = figure(), mark()
    for round_name, (candidates, stops, land_shift) in ROUNDS.items():
        out = os.path.join(DRAFTS, round_name)
        os.makedirs(out, exist_ok=True)
        for name, radius, cx, cy, transform, transform32 in candidates:
            key = name.split("-")[0]
            with open(os.path.join(out, f"{name}.svg"), "w") as fh:
                fh.write(draft(key, radius, cx, cy, transform, body, -104, 98, stops, land_shift))
            with open(os.path.join(out, f"{name}-32.svg"), "w") as fh:
                fh.write(draft(key, radius, cx, cy, transform32, small, -38, 36, stops, land_shift))
            print(f"{round_name}/{name}")
    shipped()


if __name__ == "__main__":
    main()
