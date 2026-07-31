#!/usr/bin/env python3
"""Generate the round-4 icon drafts: a small diving figure and a big world.

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

Writes resources/icon-drafts/round-{4,5}/. Re-run after editing SKELETON.
"""

import math
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DRAFTS = os.path.join(ROOT, "resources", "icon-drafts")

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
_W = (134, 152, 86)
_S = 0.55
ROUND5 = [
    ("k1-foot-jut", *_W[2:], _W[0], _W[1],
     entry(*_W, -50, _S, ANKLE - 8), entry(*_W, -50, 1.0, -30)),
    ("k2-ankle-jut", *_W[2:], _W[0], _W[1],
     entry(*_W, -50, _S, ANKLE + 8), entry(*_W, -50, 1.0, -30)),
    ("k3-calf-jut", *_W[2:], _W[0], _W[1],
     entry(*_W, -62, _S, KNEE - 8), entry(*_W, -62, 1.0, -26)),
]

# The figure's gradient is per round, because which end of it must be dark
# depends on what sits behind the head. In round 4 the head is against
# transparent sky and the light end advances; in round 5 it is against the
# world's pale land and the light end vanishes. Same figure, opposite ramp.
ROUNDS = {
    "round-4": (ROUND4, ("#D9481F", "#F5A056")),
    "round-5": (ROUND5, ("#EE9048", "#B8371A")),
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


def draft(key, radius, cx, cy, transform, body, y0, y1, stops):
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
    <g transform="translate({cx + 2} {cy + 2}) scale({inner / 41:.4f}) translate(-130 -146)">
{LAND}
    </g>
  </g>
  <g transform="{transform}" fill="url(#{key}-fig)">
{body}
  </g>
</svg>
'''


def main():
    body, small = figure(), mark()
    for round_name, (candidates, stops) in ROUNDS.items():
        out = os.path.join(DRAFTS, round_name)
        os.makedirs(out, exist_ok=True)
        for name, radius, cx, cy, transform, transform32 in candidates:
            key = name.split("-")[0]
            with open(os.path.join(out, f"{name}.svg"), "w") as fh:
                fh.write(draft(key, radius, cx, cy, transform, body, -104, 98, stops))
            with open(os.path.join(out, f"{name}-32.svg"), "w") as fh:
                fh.write(draft(key, radius, cx, cy, transform32, small, -38, 36, stops))
            print(f"{round_name}/{name}")


if __name__ == "__main__":
    main()
