# Application icons

`hicolor/` is an icon theme tree, laid out the way the Icon Theme
Specification wants one, so an installer copies rather than converts:

```sh
install -Dm0644 resources/icons/hicolor/48x48/apps/dev.harding.Kjerag.png \
  "$prefix"/share/icons/hicolor/48x48/apps/dev.harding.Kjerag.png
```

Every basename is the application ID (issue #66). That is not decoration:
the desktop entry's `Icon=` key, `icon::from_name(App::APP_ID)` in the About
page, and Flatpak's export rule all resolve an icon by that exact name.

| file | what it is |
| ---- | ---------- |
| `hicolor/scalable/apps/dev.harding.Kjerag.svg` | the drawing everything larger than 32 comes from |
| `hicolor/{256x256,128x128,64x64,48x48}/apps/*.png` | rendered from it |
| `hicolor/{32x32,24x24,16x16}/apps/*.svg` | one simplified drawing per size |
| `hicolor/{32x32,24x24,16x16}/apps/*.png` | rendered from those |

Nothing here is edited by hand. `scripts/icon-diver.py` draws the SVGs from
a joint skeleton and `scripts/icon-export.py` renders the PNGs, checks every
raster for clipping and for holes in the silhouette, and writes the review
sheet to `scratch/icon/final-sheet.png`:

```sh
python3 scripts/icon-diver.py && python3 scripts/icon-export.py
```

Why three sizes get their own drawing, why the world is 240 units wide on a
256 grid, and what each choice cost is in docs/icon.md. The workshop rounds
that led here are kept in `../icon-drafts/`.

One note for whoever wires up installation: the desktop entry, the metainfo
and the MIME package prototypes live under `res/` on the `docs/distribution`
branch, and this tree is `resources/`. Two resource roots in one tree is a
mistake worth fixing in one direction or the other when that branch lands.
