# The blank paused window (issue #102), explained

Owner-reported four times and hit in real use. A paused window shows the
header bar over an empty pane instead of the frame it is holding, until the
next key press. This note is the demonstration, end to end, of why.

Everything here was measured on `scripts/uitest.sh` with real footage under
18 busy loops on a 12-core box, against a build of the shipped forks with
trace prints in the redraw path (`scratch/iced-probe/`, never committed; the
patch section below is the whole of what it changed).

## The symptom, in pixels

Over the whole video area of the failing capture, **every one of 2,150,400
bytes reads 27**: the theme's own window background. The header bar is drawn.
The control row is not. `nonblack` cannot see it, because bare theme
background is not black and passes that check with 100 percent of its bytes
nonzero.

## The chain

**1. The pause key changes the shape of the window's widget tree.** The
controls, and with them the header bar, go away after two seconds of
stillness while playing (`CONTROLS_TIMEOUT`, docs/UI.md). Bringing them back
adds a child to the one column libcosmic builds the window out of
(`src/app/mod.rs`, `view_main`: `column::with_capacity(2).push_maybe(header).push(content)`).

**2. The rebuilt widget tree is one child short.** `UserInterface::build`
diffs the old tree against the new widget list before laying it out. Measured
at the flex resolver of that column, on the build the pause key causes:

```
flex: max=1280x720 available=720 items=2 trees=1 children=["Fill/Fixed(48.0)", "Fill/Fill"]
flex out: fill_main_sum=0 remaining=672 available=672 nodes=["1280x48", "0x0"]
```

`items=2 trees=1`. Every good build of the same window in the same run reads
`items=2 trees=2` and `nodes=["1280x48", "1280x672"]`. That one number is the
skip condition, and it is true on exactly the bad builds.

**3. `zip` turns the short tree into a silently unlaid child.**
`layout::flex::resolve` walks `items.iter_mut().zip(trees.iter_mut())`, which
stops at the shorter of the two. The content is never laid out, its slot keeps
the `Node::default()` it was pre-filled with, and that is `0x0`. The same
`zip` truncation is in `Column`'s `update`, `draw`, `operate` and `overlay`.

**4. A zero-area node is culled from the draw.** `Column::draw` filters its
children with `layout.bounds().intersects(viewport)`, and
`Rectangle::intersection` returns `None` unless both `width > 0` and
`height > 0` (`iced/core/src/rectangle.rs`). So nothing under the content is
drawn: not the video, not the control row, which is an overlay inside it. The
header bar is 1280x48 and draws.

Measured at the renderer, the same frame: `Renderer::draw layers=4
prims=0,0,0,0` - four layers of chrome and **no shader primitive at all**,
against `layers=2 prims=1,0` on the frames either side of it. Our
`ScenePipeline::prepare` is never called, which is why an earlier round of
probes on our own side of the boundary found the pipeline's per-present hook
firing with nothing else.

**5. Nothing redraws it, because a redraw would not help.** Two things keep
the empty frame on screen:

- The layout is cached in that `UserInterface`. A redraw re-draws the same
  0x0 layout: measured, a redraw arrived 13 s later and drew the empty frame
  again. Only a **rebuild** re-lays it out.
- `Scene::pump` answers `Next::Never` while paused, so the app asks for
  nothing at all, and no message arrives to force a rebuild. The window sits
  there until the pilot presses something.

**6. The next rebuild heals it.** After the bad diff the tree holds one child
carrying the header's id, so the following build matches the header by id and
appends the content: `trees=2`. That is why any key press fixes the window,
and why a poke that produces one extra rebuild fixes it too.

## Where the short tree comes from

`Tree::diff_children_custom` (`iced/core/src/widget/tree.rs`). Children with a
custom id are matched by id; the rest positionally; anything unmatched gets a
fresh tree, **deferred** to a list and written afterwards:

```rust
for (new_tree, i) in new_trees {
    if self.children.len() > i {
        self.children[i] = new_tree;
    } else {
        self.children.push(new_tree);
    }
}
```

libcosmic gives both children a custom id when `content_container` is on:
`COSMIC_header` and `COSMIC_content_container`. With the header bar hidden the
old tree has one child, the content, filed under its id. Bringing the header
back:

- the header is new at index 0, matches no id and no position, so it is
  deferred;
- the content matches its id and is diffed in place at index **0**, where it
  physically sits;
- the deferred header is then written to `self.children[0]`, because
  `len() > 0`, **over the content's tree**.

The vector never grows. It ends length 1 for a widget with two children.
Inserting a child in front of an id-matched child cannot widen the tree.

## Which fork, which revision

`https://github.com/pop-os/libcosmic`, rev
`dc1cf9f00cbe2902a52166492654bb9fee8a73d1`, which is the rev in our
`Cargo.lock` and which vendors iced as a subdirectory. The file is
`iced/core/src/widget/tree.rs`.

Nothing about this has been raised with the upstream project and nothing will
be (AGENTS.md, hard rules). It is recorded here.

## The minimal patch

Keep the existing assembly and make the length invariant hold, because every
later stage zips against it:

```diff
--- a/iced/core/src/widget/tree.rs
+++ b/iced/core/src/widget/tree.rs
@@ -351,6 +351,20 @@ impl Tree {
         for (new_tree, i) in new_trees {
             if self.children.len() > i {
                 self.children[i] = new_tree;
             } else {
                 self.children.push(new_tree);
             }
         }
+
+        // A child inserted in front of an id-matched child is written over
+        // that child rather than inserted, so this list can end shorter than
+        // the widget's own children. Every later stage walks the two with
+        // `zip`, which stops at the shorter one, so the tail is not laid out,
+        // not updated and not drawn: its layout node stays `Node::default()`,
+        // which is 0x0, and `Column::draw` culls a zero-area child.
+        while self.children.len() < new_children.len() {
+            let i = self.children.len();
+            let mut tree = new_state(&new_children[i]);
+            diff(&mut tree, &mut new_children[i]);
+            self.children.push(tree);
+        }
     }
 }
```

It restores the invariant and nothing else. It does **not** fix the matching:
the child that was overwritten gets a fresh state, which is state loss of the
kind issue #77 was about. Our own camera does not live in widget state (it is
on the `Scene`, since #77), so nothing of ours is lost by it.

## Carrying cost of a fork

Taking this would mean an `aeharding/libcosmic` fork pinned by rev through
`[patch."https://github.com/pop-os/libcosmic"]`, the way the root manifest
already pins wgpu (issue #68). It is not one entry: libcosmic vendors iced as
path dependencies inside the same git source, and cargo patches per package,
so the section needs **seventeen** entries - `libcosmic`, `cosmic-config`,
`cosmic-config-derive`, `cosmic-theme` and the thirteen `iced*` crates - or
two copies of `iced_core` end up in the tree and nothing that passes a widget
between them compiles.

The standing cost is a rebase per libcosmic bump, a `flatpak/cargo-sources.json`
regeneration with it, and a fork that has to be kept alive for as long as the
defect is unfixed upstream. Set against a one-line-per-rebase patch to a
function that has not moved.
