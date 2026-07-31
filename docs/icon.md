# App icon: style research and round-1 candidates

The app has no icon yet. This is the workshop round for issue-free
exploration: six drafts in `resources/icon-drafts/`, **none of which
ships**. The owner picks a direction and the next round narrows.

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

Not this round, recorded so it is not re-researched. The official template
(`cosmic-app-template`) ships one
`resources/icons/hicolor/scalable/apps/icon.svg`; all four first-party apps
instead ship **seven fixed-size dirs containing SVGs**,
`res/icons/hicolor/{16x16,...,256x256}/apps/<APPID>.svg`, installed by the
justfile. The `.desktop` `Icon=` key carries the app ID with no path and no
extension, and `Categories` starts with `COSMIC`. Shipping SVG inside
`NNxNN/` dirs is unusual - those are `Type=Fixed` in the spec - but it is
what lets each size be hand-tuned, which is the Pop habit worth keeping.

## The candidates

All six read as a cliff form against a wrapped sky. Letters match the
contact sheet.

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

## Regenerating the contact sheet

```sh
python3 scripts/icon-contact-sheet.py     # needs: cargo install resvg
```

Writes `scratch/icon/contact-sheet.png` (gitignored - rendered PNGs are
review artifacts, not source). Each candidate appears at 128, 64 and 32 px
as true 1:1 rasters, plus the 32 px raster magnified 4x with
nearest-neighbour, on both COSMIC desktop greys: `gray_1` is `#D7D7D7`
light and `#1B1B1B` dark (`cosmic-theme/src/model/{light,dark}.ron`).
