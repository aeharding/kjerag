# App icon: style research, the seven draft rounds, and what ships

The icon is round 7's Q2, and the assets built from it are in
`resources/icons/` (see "What ships" at the end, and that directory's
README). Everything above that section is the workshop that got there, kept
because it records what was measured and what each choice cost. Drafts live
in `resources/icon-drafts/round-N/` and none of them ships as it stands.
Round 1 explored six concepts; the owner picked F, a small round world
wedged in a rock notch, so round 2 varies that one. Round 3 explores a
separate owner concept, a freefall figure diving at the same world, and
stands alongside round 2 rather than after it; rounds 4 and 5 are that
concept reworked to the owner's notes, round 5 converging on a single
composition, round 6 pinning the world to the platform's circle, and round 7
growing the figure without letting its feet reach any further out.

The doctrine it serves is AGENTS.md's: **UI design defers to COSMIC system
apps best practice**. Same method as docs/UI.md - where a first-party app
has answered a question, copy its answer and cite the file; where none has,
say so instead of inventing a house style.

## Sources read

| source                       | revision  | what for                    |
| ---------------------------- | --------- | --------------------------- |
| `pop-os/icon-theme`          | `1a575a8` | the desktop icon theme      |
| `pop-os/cosmic-player`       | `23d5944` | first-party app icon        |
| `pop-os/cosmic-files`        | `24e34ea` | first-party app icon        |
| `pop-os/cosmic-edit`         | `4ac0da3` | first-party app icon        |
| `pop-os/cosmic-store`        | `f56cb48` | first-party app icon        |
| `pop-os/libcosmic`           | `dc1cf9f` | palette + desktop greys     |
| `pop-os/cosmic-app-template` | `ec69dff` | how a third party ships one |

Claims below are marked **documented** (a project says it), **observed**
(measured in the files at those revisions) or **inference** (mine, and
therefore arguable).

## The finding that shapes everything: there are two icon languages

Conflating them produces a wrong icon, so it goes first.

**`pop-os/icon-theme`** is the desktop-wide theme. Its README is the only
published style statement anywhere in COSMIC (documented):

> "It uses a semi-flat design with raised 3D motifs to help give depth to
> icons." ... "Pop_Icons take inspiration from the Adwaita GNOME Icons."
> ... "Pop_Icons use Sam Hewitt's Paper icons as an architectural base."

That README also says (documented): *"Pop does not supply icons for
third-party applications, only those which come with Pop!\_OS."* So it is
a style reference for us, not a place we ship to.

**COSMIC-era app icons live in each app's own repo**, not in the theme, and
they are drawn differently. Observed, across the four first-party 256 px
app icons read in full:

|                      | Pop icon theme (190 app icons) | COSMIC app icons (4)   |
| -------------------- | ------------------------------ | ---------------------- |
| gradients            | 5 files of 190                 | every one, 1-8 each    |
| radial gradients     | 0                              | 0                      |
| strokes              | rare, ~1 per file when present | **0**                  |
| filters / shadows    | 1 of 190                       | **0**                  |
| depth technique      | flat tonal facets, `fill-opacity` | two-stop tonal gradients |

So "subtle gradients" is right for the COSMIC-era app icons and wrong for
the Pop theme, whose depth is entirely flat facets and translucent
overlays. Both are saturated; neither uses outlines or drop shadows.

One correction worth recording, because it is easy to assume otherwise: the
Pop theme's README derives its icons *from* Adwaita and from a GNOME
designer's Paper set. Framing Pop as positioned against GNOME flat
minimalism is not supportable from the sources. Warmer in palette, yes;
opposed in lineage, no.

**We target the COSMIC-era app-icon language**, because that is what sits
beside us in the dock, and we borrow the Pop theme's flat tonal faceting
for the "raised motif" depth its README describes.

## The conventions the drafts follow

- **256 x 256 canvas, `viewBox="0 0 256 256"`, `fill="none"` on the root**
  (observed: all four first-party icons).
- **Freeform silhouette, not a tile.** Observed: of 190 Pop app icons only
  7 are full-bleed rounded shapes, and those are circles (clocks, avatars),
  not squircles. The COSMIC four are a folder, a document, a bag, a
  terminal window. There is no universal backplate. A **full-bleed circle
  is an established Pop shape**, which is what makes the round drafts
  legitimate rather than imported.
- **Corner radius 16 on 256, and it is a true circular arc, not a
  superellipse** (observed). cosmic-player's body starts
  `M8 56C8 47.1634 15.1634 40 24 40`: the control offset is 8.8366, which
  is exactly `16 x 0.5523`, the quarter-circle kappa. Draft E uses this
  radius verbatim so its tile does not read as foreign.
- **Two-stop tonal gradients** - same hue, lightness shift, diagonal or
  axis-aligned (observed). cosmic-files is `#49BAC8 -> #05919F`;
  cosmic-term is `#243C5F -> #102A4C`.
- **Live area** (inference, from measuring the four): square-ish art fills
  about 80% of the canvas, wide art about 94%. The round drafts use a
  120 px radius on a 256 canvas (93.75%), which is the wide-art figure.
- **No strokes, no filters, no drop shadows, no raster, no text**
  (observed: zero of each across the four).
- **Palette.** `libcosmic`'s `cosmic_palette.rs` documents its `ext_*`
  group as *"Colors used for themes, app icons, illustrations, and other
  brand purposes"* - the closest thing to an official instruction that
  exists. `ext_orange #FFAD00`, `ext_blue #48B9C7`, `ext_yellow #FEDB40`,
  `ext_purple #CF7DFF`. The Pop theme's most-used fills are the System76
  web brand tokens (`#48B9C7` 65 uses, `#faa41a` 39, `#73c48f` 29), so the
  warm/teal family the drafts sit in is the house family.
- **Per-size redraw.** The Pop theme genuinely simplifies small sizes
  (observed: the text editor has 20 elements at 256 and 6 at 16). The
  COSMIC four mostly re-export the same art at every size, which is a
  regression from that. If a draft is chosen, its 16 and 24 get hand-tuned.

There is **no published COSMIC HIG or app-icon spec** (verified negative:
`cosmic-epoch/docs/` holds only `DEBUGGING.md`; `cosmic-icons`' README is
licensing text). The libcosmic book asserts guidelines exist without
publishing them, so the honest word is *unpublished*, not *nonexistent*.

## Shipping, when something is chosen

Researched before anything was picked, so that the choosing round did not
have to stop and find it out. The official template
(`cosmic-app-template`) ships one
`resources/icons/hicolor/scalable/apps/icon.svg`; all four first-party apps
instead ship **seven fixed-size dirs containing SVGs**,
`res/icons/hicolor/{16x16,...,256x256}/apps/<APPID>.svg`, installed by the
justfile. The `.desktop` `Icon=` key carries the app ID with no path and no
extension, and `Categories` starts with `COSMIC`. Shipping SVG inside
`NNxNN/` dirs is unusual - those are `Type=Fixed` in the spec - but it is
what lets each size be hand-tuned, which is the Pop habit worth keeping.

## Round 1: six concepts

All six read as a cliff form against a wrapped sky, each enclosed in a
full-bleed disc. Letters match the contact sheet.

| # | file                    | the idea | what it sacrifices |
| - | ----------------------- | -------- | ------------------ |
| A | `a-planet-cliff.svg`    | A sheer cliff step cut into a curved planet limb, sky wrapped around the disc. The generic reading, with no boulder at all. | The notch motif entirely, and it is the weakest at 32 px, where the step flattens into a smudge. |
| B | `b-boulder-notch.svg`   | The literal one: a round boulder gripped between two flat-topped cliff faces, the limb curving away on both sides. | Simplicity. It has the most parts of any candidate and the most to lose when it shrinks. |
| C | `c-minimal-notch.svg`   | The same event reduced to one dark V and one pale circle. | Atmosphere and dimensionality - it is nearly flat, and the least "fun" of the six. It is also the only one that still reads at 24 px. |
| D | `d-golden-hour.svg`     | Sky-heavy sunset; the land is a backlit silhouette and the boulder is rim-lit rather than modelled. | The boulder at small sizes, where it collapses to a bump and the icon becomes, simply, "a sunset". |
| E | `e-orb-notch.svg`       | The notch cut into the planet's own limb, in a rounded-square tile (radius 16) instead of a circle. | The circle silhouette; the tile reads more like a photo or thumbnail app, and the orb can read as a bitten shape. |
| F | `f-boulder-world.svg`   | The wedged round mass is itself a small world - teal with land - gripped in dark rock. | Literalness, for whimsy. Its teal is the loudest departure in the set, and also the most identifiable thing at 32 px. |

Honest small-size verdict from the sheet: **C and F survive 32 px best**
(C by shape, F by colour); **A and D lose their subject** and become a
texture and a sunset respectively.

## Round 2: F, with the silhouette as the subject

The owner's correction after round 1: no perfect-circle full-bleed disc,
because Pop and COSMIC icons are transparent shapes with complex outlines.
The research above already said so - only 7 of 190 Pop app icons are
full-bleed - so round 2 drops the enclosing disc entirely. The sky is
transparent, and the rock masses plus the wedged world **are** the outline.

| #  | file                   | the idea | what it sacrifices |
| -- | ---------------------- | -------- | ------------------ |
| F1 | `f1-silhouette.svg`    | Rocks and world alone, sunset gradient living on the rock faces, sky fully transparent. The baseline reading. | Any sky colour at all, so it leans hardest on the teal to carry the icon. |
| F2 | `f2-notch-sky.svg`     | F1 plus a deliberate warm wedge of sky inside the notch, behind the world, keeping the sunset where the eye lands. | A second focal colour under the world, which at 32 px is a warm smudge rather than a shape. |
| F3 | `f3-limb-footing.svg`  | F1 with the base rounded into a shallow horizon arc: a hint of a planet limb without being a circle. | Crispness at the bottom edge; the arc is the first thing to mush when it shrinks. |
| F4 | `f4-cliff-shelf.svg`   | Asymmetric: one tall cliff, one low shelf, the world large and resting between them. | Symmetry, and it reads less as "wedged" and more as "resting against". |

Two failed readings, recorded because both were only visible once
rendered and both cost a rebuild:

- **Thin flared arms read as a funnel, not cliffs.** A first pass narrowed
  the masses to a chalice with a narrow base; it looked like a stemmed
  glass. Solid, chunky masses with battered outer edges read as rock.
- **A narrow V turns the world into a map pin.** When the notch hides
  most of the sphere, the visible teardrop plus the dark point below it is
  the location-marker glyph, unmistakably. The notch has to be shallow
  enough that the world still reads as a circle.

Also worth keeping: vertical outer edges plus a flat base plus radius-16
corners rebuilt the enclosing tile the owner had just rejected. What keeps
a silhouette freeform is angled edges and unequal masses, not the absence
of a background.

32 px verdict: **F4 survives best** - the asymmetric profile stays
identifiable where the three symmetric ones converge on the same shape -
with **F1** the cleanest of those three.

## Round 3: a freefall figure diving at the world

A separate concept from the owner, explored alongside round 2 rather than
replacing it: the same small round world, with a human figure in freefall
diving at it, the figure partly outside the world's outline so the
silhouette stays freeform. No canopy, by instruction - and one would not
survive the small sizes anyway.

| #  | file                  | the idea | what it sacrifices |
| -- | --------------------- | -------- | ------------------ |
| G1 | `g1-dive-in.svg`      | Figure entering from the upper corner, world in the lower-opposite one; the diagonal is the motion. | Figure legibility - rotated off upright it stops reading as a person well before 32 px. |
| G2 | `g2-arch-over.svg`    | Upright arch centred over the world, legs breaking into its top edge. | Dynamism, and it is the narrowest composition of the three; it reads as held above rather than diving at. |
| G3 | `g3-figure-leads.svg` | Figure large and leading, world small and low; the subject is the jump, not the planet. | The world, which at 32 px nearly disappears, plus the same rotation cost as G1. |

**Does a human figure survive 32 px? Only upright, and only when it fills
most of the canvas.** G2 still reads as a person at 32 px: head, arms up,
legs out. G1 and G3 do not - rotated, they are an orange limbed blob
beside a ball. The body plan is what carries the reading, and it is only
recognisable head-up. If this concept wins, the figure wants to be upright
and dominant.

Three construction results, each measured by rendering rather than assumed:

- **A detached head plus bent arms is what makes the figure read.** The
  first attempt used straight limbs and a head merged into the torso; at
  every size it was a starfish. Separating the head and bending the arms
  up at the elbow - the box position, whose silhouette is a W above the
  head - turned it into a person at 128 and 64 px.
- **A head-down dive is not a person, it is an arrow.** Built properly,
  legs together and arms out, it read as a downward dart at 128 px, never
  mind 32. Rejected on that evidence rather than on taste, which is why
  the centred variation is an upright arch instead.
- **A rotated figure needs a neck.** With the head merely close to the
  torso, rotating it so the head leads left the head reading as a separate
  floating dot beside the world. A short neck capsule bridges it, and
  costs nothing upright, where it is hidden.

The figure is a saturated warm orange on purpose. With the sky
transparent, a dark figure disappears against the COSMIC dark grey and a
pale one disappears against the light grey; a mid-saturation warm reads on
both, and is complementary to the world's teal.

## Round 4: a small diver in side profile, world dominant

Round 3's feedback: the diver needs to be smaller, in side profile, and
look like a person rather than a stick figure. Side profile is also what
makes a dive read as a dive - round 3 established that a frontal figure
stops reading as soon as it is rotated.

| #  | file                | the idea | what it sacrifices |
| -- | ------------------- | -------- | ------------------ |
| H1 | `h1-exit-line.svg`  | Diver upper-left tracking head-down at the world lower-right: the classic exit line, read along the diagonal. | The head lands against the world's warm rim, the one place the figure has least contrast. |
| H2 | `h2-piercing.svg`   | Diver arcing over the limb with the head already inside the world's edge - the moment of entry. | Reads as arrival rather than travel; there is no distance left to fall. |
| H3 | `h3-long-way.svg`   | Small diver high, big world low, a lot of empty sky between: the long way down. | The most empty canvas of the three, and the smallest world of the three. |

### What 32 px ships

**Not a person.** A 200-unit figure at this scale is about 12 px tall at
32, which cannot hold a head and four limbs. Each candidate therefore
ships a size-specific `*-32.svg`, the way the Pop theme redraws its own
small sizes: the same world, with the diver reduced to a head and a
tapering body - a dart, not an anatomy. The contact sheet shows the naive
downscale and the redraw side by side (`32` against `32 art`, and both
blowups) so the cost is visible rather than asserted. **This is the
accepted design, not a defect**: at 32 px the world carries the icon and
the mark says only that something is falling into it. Ball-only is the
fallback if even the mark reads as grit.

### Building the figure

`scripts/icon-diver.py` generates round 4 from a joint skeleton: each bone
is the tangent trapezoid between two joint circles, plus a circle at each
joint, unioned into a tapered limb. It exists because three hand-traced
attempts failed in a row, and because the fix for a bad silhouette is to
move a joint - a number in the skeleton, against a re-trace by hand.

Two results worth keeping:

- **Chest depth against head depth decides whether it reads as a person.**
  The canon is roughly 2:1. The first attempts drew them about equal, and
  every one was a tadpole: a round head on a narrow tapering body, no
  matter what the limbs did.
- **Split legs are the strongest human cue at small size.** With the legs
  together the figure stayed a dart; scissoring them apart is what made it
  a diving person. Swept-back arms help, but they are the first thing to
  merge into the torso as the figure shrinks.

The figure's gradient runs deep at the trailing end to light at the head,
not the other way round. Drawn the other way the head was the darkest part
of the figure and receded exactly where the eye needs to land.

## Round 5: converging on H2, diver inside the world

The owner picked H2 and asked for the diver on the left and further through
the rim, then refined twice: the figure should be **mostly inside the world
with only a subtle jut breaking the outline**, and the diver's head must
land on **open water, not the green landmass**. So the rim crosses the
figure near the legs rather than the chest, and the freeform-outline
principle survives as that one break.

These are spacing variants of a single composition, not new ideas. All
three share a world (r=84 at 134,162), a figure scale of 0.55 and an entry
angle of -62; only how far the legs clear the rim changes.

| #  | file                | jut past the rim | the idea |
| -- | ------------------- | ---------------- | -------- |
| K1 | `k1-shin-out.svg`   | 22 units (10.9 px at 128) | Shins clear the rim. The tightest of the three. |
| K2 | `k2-calf-out.svg`   | 28 units (14.2 px at 128) | Calves out - the depth the owner pointed at. |
| K3 | `k3-knee-out.svg`   | 35 units (17.5 px at 128) | Out to the knee, the most pronounced break. |

`scripts/icon-diver.py` places these by naming the landmark the rim should
cross - `entry(..., KNEE - 8)` is literally calf-deep - so the depths above
are declared rather than dialled in by eye.

### Getting water under the head

Worth recording, because the obvious mechanism does not work: **moving the
world down, or driving the diver deeper, cannot put water under the head.**
`entry()` places the figure relative to the world, so the whole
composition travels together and the head/land relationship is invariant;
diving deeper pushes the head further *into* the land. Measured, the land
crest straddled the world's centre (-26 to +2 units) and every head landed
within 11 units of that centre.

What works is lowering the landmass inside the disc. The land now sits **28
world units lower** (a per-round `land_shift`), which clears the crest under
the head by about 15 units even on the variant whose head reaches deepest.
The world also moved down 10 units and lost 2 of radius, which is the
framing the owner asked for but is not what fixed the overlap.

### Contrast, checked twice against two different backdrops

Round 4 put the figure against transparent sky and the light end of its
gradient on the head. Round 5 moved it onto the world, where the head first
landed on the pale green crest and faded out; four tones were rendered at
128 and 64 before picking a deep head against a mid tail, `#EE9048` to
`#B8371A`. When the land dropped and the head moved onto blue, that ramp
was re-checked against the water rather than assumed - it holds, because
the head sits on the lighter teal side of the ocean gradient and a deep
warm on light teal is the strongest pairing of the four tested. No change
was needed, and no stroke or keyline was added; the language has none.

**Round 4's note that the head must be the light end was right for its
context and wrong for this one.** The head takes whichever end contrasts
with what is behind it, and that changed twice. The gradient is therefore a
per-round setting in the generator, and round 4's files are untouched.

### What 32 px ships, revisited

Same size-specific `*-32.svg` as round 4. The jut is 5.5 to 8.7 px at 128
but only 2.7 to 4.4 px at 32, so the broken outline is faint but no longer
absent the way it was at the earlier, shallower depths.

## Round 6: the world at the platform's full circular size

Before picking, the owner asked whether the jutting feet would force the
world to be shrunk or offset in a real desktop icon. The requirement that
came out of it: **the world sits at the platform's standard full-bleed
circular size, centred, always** - the diver never gets room by making the
world smaller or moving it. The jut instead lives in the square's corner
headroom, outside the inscribed circle but inside the canvas.

### The convention, measured

Pop's full-bleed circular app icons draw a **238-240 wide circle centred in
the 256 baseplate, spanning 8..248 - a uniform 8-unit margin** (observed at
icon-theme `1a575a8`: `accessories-clock` and `alarm-clock` at 240,
`avatar-default` and the browser icons at 238-239; the other circular icons
in the set, weather and audio-player, are objects rather than full-bleed and
sit at 206-208). That 8..248 span is the same live area the COSMIC app icons
use for wide art, measured independently in round 1, so two sources agree.

Round 6 therefore pins the world to **r=120 at (128,128)**.

### Corner headroom, and what actually fits

The canvas corner is 181.0 units from centre and the circle's edge is 120,
so the top-left diagonal has **61 units of headroom** past the rim. Holding
the extreme point to the same 8-unit margin the circle itself uses caps the
jut at about 49.7 units. Entry runs at -45 degrees so the feet travel
straight at that corner.

Measured from the rendered alpha, not calculated from the skeleton:

| #  | file                | past the rim | corner-most ink | nearest canvas edge |
| -- | ------------------- | ------------ | --------------- | ------------------- |
| N1 | `n1-shin-out.svg`   | 20.7 units   | (29, 27)        | 27 units            |
| N2 | `n2-calf-out.svg`   | 27.1 units   | (25, 22)        | 22 units            |
| N3 | `n3-knee-out.svg`   | 34.2 units   | (20, 17)        | 17 units            |

Even N3, the deepest, keeps more margin than the circle itself does.

### Clipping across the export sizes

Rendered at 16, 24, 32, 48, 64, 128 and 256: **nothing is cut off.** At 16
and 24 the ink does reach the boundary pixel, because the circle's own
8-unit margin is 0.5 px at 16 and antialiasing spills into the edge pixel.
That is the convention behaving normally, not a fault in this art - the
control proves it: **Pop's own `accessories-clock` rendered the same way
reports the same zero margins at 16 and 24**, and tighter margins than ours
at 32 and 64. It is also exactly why the Pop theme hand-redraws its 16 and
24, which is the plan here too.

### Head over water at the full radius

The larger radius pushes the head further up the entry line, so the head
clears the crest on geometry alone. The landmass shift drops from 14 units
(round 5) to **6**, and now serves balance and margin rather than fixing an
overlap. The contact sheet draws a faint canvas edge behind every cell,
since a transparent icon otherwise gives no way to see how the art sits in
the square.

## Round 7: a bigger diver, the same jut

Looking at the finals the owner asked for the figure a bit bigger, with the
legs still off the world by the same absolute amount. Those two pull against
each other, and which way is not obvious: `entry()` places the figure by
naming the local y the rim crosses, and **everything past that point is
outside the circle**, so scaling the figure up without touching the depth
lengthens the jut in proportion. Holding the jut therefore means crossing a
bigger figure lower down its legs, which pushes the growth inward - the head
travels further along the entry line, deeper into open water, while the feet
stop exactly where they stopped before.

Both candidates hold the outermost point at N2's 27.1 units past the rim,
solved against the drawn geometry rather than dialled in by eye:

| #  | file                | scale        | rim crosses at | head from centre |
| -- | ------------------- | ------------ | -------------- | ---------------- |
| Q1 | `q1-grown-9.svg`    | 0.600 (+9%)  | y = -58.11     | 36.1 (0.30 R)    |
| Q2 | `q2-grown-18.svg`   | 0.649 (+18%) | y = -61.53     | 27.0 (0.23 R)    |

N2, for comparison, is scale 0.550 crossing at y = -54 with the head 45.3
from centre. The depths are not landmarks the way round 5's `KNEE - 8` is,
so they read as arbitrary numbers; `scripts/icon-export.py` prints how far
past the rim each rendered raster actually reaches, which is what keeps them
honest.

Nothing else moves. Measured on the rendered alpha, the corner margins are
identical to N2's at every size - 8 px at 256, 4 at 128, 2 at 64, 1 at 48 -
because at 147.1 units the extreme point is still well inside the 169.7 that
the world's own bounding box reaches along the diagonal. The world governs
the square; the diver never touches it.

The head stays over open water without moving the landmass, so the shift
stays at 6. The clear water between the head and the coast falls from 40.1
px at 256 (N2) to 30.5 (Q1) to **21.1** (Q2), which is a real reduction and
still a wide gap. That zero-overlap result is worth only as much as the
instrument behind it, so the check was shown able to fail: driving the head
past the world's centre and into the landmass makes the same detector report
461 to 682 land pixels under it.

### The small sizes do not change

32, 24 and 16 keep the drawings they had. That is consistency rather than an
omission - the 32 mark's head is already **0.117 of the world's radius**
against the master figure's 0.083, so growing the figure to 0.098 narrows
the gap between the big art and the small marks instead of opening one.

## What ships

The owner picked Q2, so `resources/icons/hicolor/` is that drawing laid out
as an icon theme tree: the scalable SVG plus one PNG per size from 16 to
256, every basename the application ID `dev.harding.Kjerag` (issue #66).
Sizes 48 and up are renders of that single drawing, and **16, 24 and 32 each
have a drawing of their own**, which is a convention rather than an apology
for this art: the median app icon in `pop-os/icon-theme` `1a575a8` carries 6
elements at 16 and 24 against 6.5 at 32 and above, and where the art is busy
the cut is a real redraw (`accessories-clock` 8 against 21,
`accessories-text-editor` 6 against 19). The two smallest also sit on their
own pixel grid, `viewBox="0 0 16 16"` and `0 0 24 24`, which is what every
16 px file in that theme and in cosmic-{player,files,edit} does, and it is
what buys back the margin round 6 measured away: rendered at 16 or 24 the
256 art reaches the boundary pixel, while `accessories-clock`, drawn on the
small grid, holds one clear pixel, and so now do these. On that grid the
world is a circle of radius 7 at 16 and 11 at 24, centred, one pixel inside
the canvas; the coastline is the master's three cubics reduced to one and
scaled to that radius, so the same coast survives instead of a second one
drawn by eye; the pale band under the crest is kept at 24 and dropped at 16;
and the figure is a head and a tapering tail on the same -45 degree entry
line, crossing the rim so that about two pixels of it sit outside the
circle. What the small sizes give up is the figure itself: from 32 down the
world carries the icon and the mark says only that something is entering it,
which is the accepted design round 4 flagged and every sheet since has shown.

| size             | drawing                    | margin        |
| ---------------- | -------------------------- | ------------- |
| 256, 128, 64, 48 | `hicolor/scalable/apps/`   | 8, 4, 2, 1 px |
| 32               | its own SVG, 256 grid      | 1 px          |
| 24, 16           | its own SVG, own pixel grid | 1 px         |

The fixed-size directories carry PNGs, which is what `Type=Fixed` means in
the spec and what issue #67 asks for by size; where a size has its own
drawing, that SVG sits beside its PNG. Margins are measured off the rendered
alpha by `scripts/icon-export.py`, which refuses to finish if any size has
less than one clear pixel on any side, or any transparent pixel enclosed by
ink.

## Regenerating the assets and the sheets

```sh
python3 scripts/icon-diver.py                             # every SVG
python3 scripts/icon-export.py                            # PNGs, checks, sheet
python3 scripts/icon-contact-sheet.py                     # newest draft round
python3 scripts/icon-contact-sheet.py resources/icon-drafts/round-1
```

`icon-export.py` writes `scratch/icon/final-sheet.png`: every shipped size
at 1:1 and the four smallest magnified 6x, on both desktop greys. That is
the picture to look at before merging a change to the art.

Writes `scratch/icon/contact-sheet-round-N.png` (gitignored - rendered
PNGs are review artifacts, not source). Each candidate appears at 128, 64
and 32 px as true 1:1 rasters, plus the 32 px raster magnified 4x with
nearest-neighbour, on both COSMIC desktop greys: `gray_1` is `#D7D7D7`
light and `#1B1B1B` dark (`cosmic-theme/src/model/{light,dark}.ron`). The
panels are the real desktop colours rather than a checkerboard, so a
transparent draft shows the grey through it.

It also writes `scratch/icon/seam-round-N.png`, the same drafts over
magenta. Abutting shapes can leave a hairline of background between them
that is invisible against the greys; anything magenta inside a silhouette
is a hole, not a gradient.
