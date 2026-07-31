# UI plan: window, controls, and the keyboard

The design the app shell is built from (issue #16). The doctrine it serves
is AGENTS.md's: **UI design defers to COSMIC system apps best practice**.
Where a first-party app has already answered a question, this document
copies its answer and cites the file. Where none has, it says so instead of
inventing a house style.

Kyerag's one shape that no COSMIC app shares: the middle of the window is
not a picture, it is a view direction. Dragging in it means something, and
that single fact is what most of the tensions below come from.

## Sources read

Cloned and read in full, at these revisions:

| source                                          | revision  | dated      |
| ----------------------------------------------- | --------- | ---------- |
| `pop-os/cosmic-player`                          | `23d5944` | 2026-07-28 |
| `pop-os/cosmic-files`                           | `24e34ea` | 2026-07-28 |
| `pop-os/cosmic-edit`                            | `4ac0da3` | 2026-07-28 |
| `system76/hig`                                  | `9c9ef64` | 2017-12-01 |
| `pop-os/libcosmic` (the rev this repo pins)     | `dc1cf9f` | -          |

File and line citations below are against those revisions. libcosmic paths
resolve locally to `~/.cargo/git/checkouts/libcosmic-*/dc1cf9f/`.

## What COSMIC's written guidelines actually cover

Almost nothing that this issue needs. `system76/hig` is a single README
whose UI half is one section, "Dialogs & Actions" (primary text, secondary
text, actions aligned right to left at the bottom), and whose remaining
two thirds are copy rules for a hardware store: product names, payment
methods, units, resolutions, third-party brands. It has no page on header
bars, keyboard shortcuts, media playback, or windows, and it closes by
saying "For anything not covered in this document, refer to the elementary
HIG". The elementary HIG's own sitemap (docs.elementary.io/hig/sitemap.md)
has no keyboard, header-bar, or media page either; its one directly usable
page is the welcome screen (cited under "Nothing open" below).

So the operative guideline for this app is the source of the first-party
apps, which is what the owner's doctrine already says. Two rules from the
written HIG do apply and are followed: dialog actions sit at the bottom
right, and units get a space before them ("10 s", not "10s").

Anything below marked **open question** is a place where the first-party
apps disagree or are silent. Those are for the owner, not for the
implementing agent to settle alone.

## The window

One window, no tabs, one file at a time.

- `core.window.content_container = false`, already set and already
  explained in `crates/app/src/app.rs`: video wants both window edges.
  cosmic-player reaches the same place from the other direction, with
  `core.window.border_padding = Some(0)` (`src/main.rs:895`).
- Size limits stay as they are (360 x 240). cosmic-player uses 360 x 180
  (`src/main.rs:156`).
- `Settings::default().theme(config.app_theme.theme())` at startup, so the
  window opens in the configured theme rather than flashing the default
  one (cosmic-player `src/main.rs:154-155`).

### Header bar

`header_start` holds the menu bar and nothing else. That is unanimous:
cosmic-player `src/main.rs:1646-1655`, cosmic-files `src/app.rs:6411-6420`,
cosmic-edit `src/main.rs:3029-3037`.

`header_end` holds app-level buttons that are not part of the content;
cosmic-files puts its search there (`src/app.rs:6422-6455`). Kyerag puts
nothing there. Every action we have is either a transport control, which
belongs in the overlay row, or a menu item.

The header bar carries **no title text**. libcosmic renders the title only
when `header_center` is empty and the title is non-empty
(`src/widget/header_bar.rs:398-413`), and cosmic-player never sets a header
title at all: its header bar is the menu bar plus the window buttons. A
player's title bar competing with the picture is exactly what we do not
want.

### No nav bar

`fn nav_model()` returns `None`. cosmic-player returns `Some` because it
has a playlist and a folder tree (`src/main.rs:962-964`); we have neither.
This is not just an omission: libcosmic adds the nav-bar toggle button to
the header only when `nav_model()` is `Some` (`src/app/mod.rs:786`), so
returning `None` is what keeps a dead toggle out of our header bar. It also
retires the left-edge padding asymmetry recorded in ARCHITECTURE.md, which
exists because `nav_bar.active` defaults to true even with no model.

If a playlist is ever wanted, the nav bar is where it goes, and
cosmic-player's `ProjectNode` tree (`src/project.rs`) is the pattern.

### Window title

`{file name} - Kyerag`, and plain `Kyerag` with nothing open.

cosmic-files writes `format!("{tab_title} — {}", fl!("cosmic-files"))`
(`src/app.rs:1888-1898`), with an em dash. AGENTS.md forbids em dashes in
UI copy, and a window title is UI copy, so we use a hyphen. This is the one
place where a COSMIC precedent is knowingly not copied character for
character.

cosmic-player sets a static `"COSMIC Media Player"` with a `//TODO:
filename?` next to it (`src/main.rs:838-842`); its own author considers
that unfinished, so it is not a precedent for leaving the file name out.

## Nothing open: the welcome view

Centered column, in this order (cosmic-player `src/main.rs:1676-1695`):

1. flexible space
2. an icon at 64 px, and a line of body text under it
3. a suggested button that opens the file dialog
4. flexible space

cosmic-player uses `folder-symbolic` and "No video or audio file open" /
"Open file". Kyerag uses `video-x-generic-symbolic` (present in
libcosmic's bundled icons under `freedesktop/scalable/mimetypes/`), "No
video open", and "Open video". The elementary HIG's welcome-screen page,
which the System76 HIG defers to, describes exactly this shape: explain the
situation, then offer the action that fixes it.

**Failure to open lands here too.** The welcome view returns with a second
line of body text under the first, saying what went wrong in plain words
("That file could not be opened."), plus a `log::warn!` with the detail.
cosmic-player only logs (`src/video.rs:63`), which
leaves the pilot staring at an unchanged window; a player with exactly one
job should say when it cannot do it. No dialog: the HIG's dialog section is
about asking the user something, and there is nothing to ask.

## Opening a file

Four ways in, all landing on one `Message::FileLoad(PathBuf)`.

**The dialog.** The XDG portal chooser, through libcosmic:

```rust
cosmic::dialog::file_chooser::open::Dialog::new()
    .title("Open video")
    .open_file()
```

cosmic-player `src/main.rs:1066-1085`. This needs libcosmic's `xdg-portal`
feature (`Cargo.toml:108`), which is not in libcosmic's default set, so the
app crate's dependency gains `features = ["xdg-portal"]` alongside `wgpu`.
A filter for `.insv` is set on the dialog; the exact API is
`file_chooser::open::Dialog`'s filter builder, to be read at implementation
time.

**Drag and drop.** cosmic-player does not implement it at all: nothing in
its source touches drag and drop. cosmic-files does, so that is the
precedent. Wrap the shader widget in
`cosmic::widget::dnd_destination::dnd_destination_for_data` with a small
type whose `AllowedMimeTypes::allowed()` is `["text/uri-list"]`, modelled
on cosmic-files' `ClipboardPaste` (`src/clipboard.rs:108-160`); the widget
is `src/widget/dnd_destination.rs:16-27` in libcosmic and the wiring
example is cosmic-files `src/app.rs:6491-6496`. First file wins, others are
ignored. Also handle `FILE_TRANSFER_MIME`
(`src/widget/dnd_destination.rs:33`) if it comes for free; it is how the
portal hands over files from sandboxed sources.

**The command line.** `kyerag <file.insv>` already works. Keep it a path,
not a URL: cosmic-player parses freestanding arguments as URLs
(`src/argparse.rs:70-79`) because GStreamer streams from the network, and
we decode local files only. Add `--help` and `--version` (cosmic-player
`src/argparse.rs:139-169`), which cost a dozen lines and are what a
terminal user tries first.

**Recent files.** A `File > Open recent` folder item, as in cosmic-player
`src/menu.rs:113-120`, backed by cosmic-config *state*
(not config): `ConfigState { recent_files: VecDeque<PathBuf> }`, most
recent first, deduplicated, truncated to 10 (`src/main.rs:397-401`), with a
trailing divider and "Clear recent list" when the list is not empty
(`src/menu.rs:49-55`). `PathBuf` rather than cosmic-player's `Url` for the
same reason as the command line.

## The controls

### Where they live

An overlay at the bottom of the video, not a docked bar. cosmic-player
builds it as `widget::popover(video).position(Position::Bottom)` with the
control row as the popup (`src/main.rs:1775`, `2088-2090`). The row is a
`widget::container` with `padding([space_xxs, space_xs])` and
`theme::Container::WindowBackground`, wrapped in a `mouse_area` whose
`on_press` re-arms the auto-hide timer (`src/main.rs:2052-2060`).

Copy that exactly. Spacing between controls is `space_xxs`, and the row is
`align_y(Alignment::Center)` (`src/main.rs:1930-1932`).

### What is in the row

Left to right, following cosmic-player's order and dropping what we do not
have (no subtitles, no playback speed, no repeat):

```
[back 10 s] [play/pause] [forward 10 s]   00:12:34  =====O---------  00:17:26   [frame] [fullscreen] [volume]
```

- Jump buttons: `SeekRelative(-10.0)` and `SeekRelative(10.0)`
  (`src/main.rs:1933-1977`). Their icons are not in the icon theme;
  cosmic-player ships them in its own `res/` and loads them with
  `icon::from_svg_bytes(JUMP_BACKWARD_ICON).symbolic(true)`
  (`src/main.rs:51-58`). Copying those two SVGs is allowed here:
  cosmic-player is GPL-3.0, which is one-way compatible with our AGPL-3.0,
  and AGENTS.md requires such a file to carry an SPDX header and
  attribution.
- Play/pause: one button whose icon is
  `media-playback-start-symbolic` or `media-playback-pause-symbolic`
  (`src/main.rs:1950-1959`).
- Elapsed time, scrubber, remaining time (see below).
- Frame capture: `camera-photo-symbolic`, which is in libcosmic's bundled
  icons (`cosmic-icons/freedesktop/scalable/devices/`). No precedent; see
  "Screenshots" below.
- Fullscreen: `view-fullscreen-symbolic` (`src/main.rs:2026-2031`).
- Volume, **after** fullscreen and not before it, which is cosmic-player's own
  order (`src/main.rs:2013-2051`). One speaker button, whose four icons say
  what the sound is doing: `audio-volume-muted-symbolic` when muted, and
  `-high-`, `-medium-` or `-low-` above two thirds, above one third and below
  it (`src/main.rs:2033-2051`). Pressing it opens the slider, below.

A button with no `on_press` renders disabled, which is how the row can
exist before the capability behind a button does. The volume button is drawn
that way for a file with no sound in it and for a box with no working output
device.

### The volume slider

Not in the row: in a popup **above** it, right aligned under the button that
opened it, exactly as cosmic-player's audio dropdown
(`src/main.rs:1777-1807` for the contents and `1899-1926` for the frame). The
contents are a second copy of the speaker button, which is the mute toggle,
and `Slider::new(0.0..=1.0, volume, ...)` with `.step(0.01)`. Moving the
slider unmutes, because the pilot is asking to hear something
(`src/main.rs:1229-1235`).

One deliberate improvement on the source: cosmic-player styles that popup with
a hand-rolled container closure carrying a `//TODO: move style to libcosmic`
beside it (`src/main.rs:1905-1922`). libcosmic has since moved it.
`theme::Container::Dropdown` is the same component base, divider border and
small radius (`src/theme/style/iced.rs:608-619`), so the stock one is what we
use.

**When it closes.** cosmic-player holds its controls up for as long as a
dropdown is open (`src/main.rs:1627`) and closes the dropdown on a click in
the video, on fullscreen, and on every transport action
(`src/main.rs:1192`, `1254`, `1327`, `1349`, `1501`, `1508-1513`). We cannot
copy the first two of those. Holding the controls up while a dropdown is open
would hold them up forever, because in this app dragging the picture is
pointer input; and a click in the video is the look-around grab (conflict 1,
below), which fires before a `mouse_area` around it could see it. So ours
closes with the control row, on the same 2 s of stillness, and on fullscreen.

### Auto-hide

Two seconds of no pointer input hides the control row **and the header
bar** together (`static CONTROLS_TIMEOUT: Duration = Duration::new(2, 0)`,
`src/main.rs:45`; `update_controls`, `src/main.rs:628-643`). Any cursor
movement anywhere in the window brings both back, via a global
`event::listen_with` subscription mapping `CursorMoved` to a `ShowControls`
message (`src/main.rs:2119`).

The part that is easy to get wrong: **the controls do not hide while
paused.** cosmic-player never states this, it falls out of the wiring. The
hide check only runs on `Message::NewFrame` (`src/main.rs:1613-1628`), and
for video that message comes from the player's per-frame callback
(`src/main.rs:1705`), which stops arriving when playback stops. A naive
timer would hide the controls out from under someone who paused to look
around, which for a reframing player is the normal way to use it. So: the
timeout is checked only while playing.

Kyerag's equivalent hook is the redraw the scene already drives; the check
belongs where "a frame was presented" is known.

### The cursor

Hidden along with the controls, over the video, while playing.
cosmic-player asks its video widget for this with
`.mouse_hidden(!self.controls)` (`src/main.rs:1701`).

We have no such widget, but the mechanism is in iced:
`mouse::Interaction::Hidden` (`iced/core/src/mouse/interaction.rs:7`) maps
to no cursor icon in the winit conversion, and the window then calls
`set_cursor_visible(false)` (`iced/winit/src/window.rs:291-305`). So
`Scene`'s existing `mouse_interaction` returns `Interaction::Hidden`
instead of `Grab` when the controls are hidden, which means the `Scene`
needs to know that one bit. That is the only new coupling this design adds
to `crates/render`.

### Narrow windows

When `core.is_condensed()`, the row splits in two: buttons on the first
row, and elapsed time / scrubber / remaining on a second row below it
(cosmic-player `src/main.rs:1999-2012` and `2062-2086`). Copy that.

## The scrubber

`widget::Slider`, full width of the space left between the time labels:

```rust
Slider::new(0.0..=self.duration, self.position, Message::Seek)
    .step(0.1)
    .on_release(Message::SeekRelease)
```

cosmic-player `src/main.rs:2005-2008`. Times are monospace
(`font::mono()`), elapsed on the left, **remaining** on the right, both
`HH:MM:SS` from a six-line formatter (`src/main.rs:1668-1674`, `2003`,
`2010`). For a 30-minute file that reads `00:12:34` and `00:17:26`. The
leading zeros are noise, and we copy them anyway: a player that formats
time its own way for no reason is a player that looks foreign.

**Where we deviate, and why.** cosmic-player's `Message::Seek` fires on
every slider tick and does an accurate seek each time, after pausing
(`src/main.rs:1325-1338`); the release then seeks once more and restores
the previous paused state (`src/main.rs:1347-1357`). An accurate seek per
tick on a 30-minute dual-stream 3840x3840 HEVC file is a decode of every
frame from the last keyframe, twice, per pixel of drag. Issue #5 exists
precisely because that has to be nearest-keyframe-then-decode-forward.

So: while the slider is being dragged, seek to the nearest keyframe only,
and let the picture update from there. On release, do the accurate seek to
the exact position. The message shape stays cosmic-player's
(`Seek` / `SeekRelease`); only what `Seek` asks the media layer for
changes. This is the one behavioral deviation in the whole document, and it
is forced by the file, not by taste.

## Fullscreen

- `window::set_mode(id, Mode::Fullscreen)` and back, with
  `core.window.show_headerbar = !fullscreen` set at the same time
  (`src/main.rs:1190-1206`).
- `fn on_escape()` leaves fullscreen and does nothing otherwise
  (`src/main.rs:966-972`). Note that libcosmic gives Escape to the app
  through this hook; do not bind it in the key map.
- Double click on the video toggles it (`src/main.rs:1773`).
- The keys are `f` and `Alt+Enter` (`src/key_bind.rs:26-27`).

The header bar comes back on exit, subject to the auto-hide rule above.
In fullscreen the control row still appears on pointer movement, over the
video, exactly as in a window.

## The keyboard

The map is cosmic-player's, extended with the standard app keys the other
two first-party apps agree on. Three bare letters are invented, because no
COSMIC app does what they do: `s`, `h` and `m`.

| key             | action                    | precedent                                            |
| --------------- | ------------------------- | ---------------------------------------------------- |
| `Space`         | play / pause              | cosmic-player `src/key_bind.rs:28`                   |
| `Left`          | back 10 s                 | cosmic-player `src/key_bind.rs:29`                   |
| `Right`         | forward 10 s              | cosmic-player `src/key_bind.rs:30`                   |
| `,`             | previous frame            | cosmic-player `src/key_bind.rs:32`                   |
| `.`             | next frame                | cosmic-player `src/key_bind.rs:31`                   |
| `f`             | fullscreen                | cosmic-player `src/key_bind.rs:26`                   |
| `Alt+Enter`     | fullscreen                | cosmic-player `src/key_bind.rs:27`                   |
| `Escape`        | leave fullscreen          | cosmic-player `src/main.rs:966` (`on_escape`)        |
| `Ctrl+O`        | open video                | cosmic-edit `src/key_bind.rs:34`                     |
| `Ctrl+W`        | close video               | cosmic-edit `src/key_bind.rs:23`                     |
| `Ctrl+Q`        | quit                      | cosmic-edit `src/key_bind.rs:36`                     |
| `Ctrl+,`        | settings                  | cosmic-edit `src/key_bind.rs:68`, cosmic-files `:62` |
| `Ctrl+=`, `Ctrl++` | zoom in                | cosmic-edit `:44-45`, cosmic-files `:50-51`          |
| `Ctrl+-`        | zoom out                  | cosmic-edit `:43`, cosmic-files `:53`                |
| `Ctrl+0`        | default view              | cosmic-edit `:42`, cosmic-files `:52`                |
| `s`             | save frame                | none; see below                                      |
| `Ctrl+C`        | copy frame                | cosmic-files `src/key_bind.rs:73`                    |
| `h`             | lock the horizon          | none; see below                                      |
| `m`             | mute                      | none; mpv's key, see below                           |

Notes:

- **The arrows seek, they do not look around.** That is a real decision:
  arrow keys are the obvious way to turn a 360 view, and cosmic-player has
  already spent them on seeking. Transport wins, because a player that
  seeks differently from every other COSMIC app is worse than one whose
  view is turned only by the mouse. Keyboard look, if ever wanted, has
  `Shift+arrows` free.
- **Zoom means field of view.** Ctrl+= narrows it, Ctrl+- widens it, Ctrl+0
  restores the default view (yaw, pitch and field of view together, one
  action, one menu item). cosmic-files and cosmic-edit both bind that trio
  the same way, and cosmic-edit's source notes why those three characters
  in particular: they are not special to terminals, so they are free
  (`src/key_bind.rs:41`).
- **The zoom range runs from 20 degrees to the whole sphere** (issue #47).
  Scrolling out does not stop at a wide flat view any more: past 110 degrees
  the picture bends, through the tiny planet at 220, and ends with the whole
  sphere as a ball sitting in the middle of the window with room around it.
  The far end is where the ball fills 0.8 of the window's shorter side, so it
  is further out on a wide window than on a square one; the room around the
  ball is the same grey the player paints anywhere it has no picture. It is
  one continuous scroll out and back, the ball can be grabbed and turned like
  any other view, and Ctrl+0 comes back to the default in one press. Nothing
  about the ordinary range changed. Dragging the view down on the way out is
  what puts the nadir in the middle, which is the tiny planet as Insta360
  frames it; dragging it up gives the same picture inside out, with the sky in
  the middle and the ground wrapped round the rim.
- **`s` for save frame** has no first-party precedent, because no COSMIC
  app captures its own view. Bare unmodified letters are idiomatic in this
  app class though: cosmic-player binds `f` and `a` with no modifier. The
  owner asked for `s` in issue #16, so `s` it is.
- **`h` for the horizon lock and `m` for mute** are the same kind of
  invention. No COSMIC app locks a horizon, and cosmic-player binds no mute
  key at all, so `m` follows mpv rather than a first-party app. The owner
  asked for `h` in issue #8 and approved `m` on 2026-07-31. `m` sends the
  speaker button's own message, so it is remembered the same way the button
  is, and a file with no sound in it ignores it exactly as the disabled
  button does.
- Implement this as libcosmic's `HashMap<KeyBind, Action>` plus a
  `Message::Key(modifiers, physical_key, key)` from a global subscription,
  matched with `KeyBind::matches` (cosmic-player `src/main.rs:1207-1213`,
  `2113-2118`). That is worth doing rather than hand-matching keys, for two
  reasons: `KeyBind::matches` already falls back to the physical key
  position on non-Latin layouts
  (libcosmic `src/widget/menu/key_bind.rs:47-63`), which is the concern
  that the current `space_bar` handler solves by hand, and the same map is
  what draws the accelerators next to the menu items.
- Keep the subscription's `event::Status::Ignored` guard that the current
  handler has: it is what stops a key firing twice when a widget already
  took it.

## The pointer, and the two conflicts

| input               | Kyerag                        | note                              |
| ------------------- | ----------------------------- | --------------------------------- |
| left drag on video  | look around                   | already built, issues #3 / #29    |
| wheel over video    | zoom, anchored at the cursor  | already built                     |
| double click        | fullscreen                    | cosmic-player `src/main.rs:1773`  |
| single click        | nothing                       | conflict, below                   |
| move                | show the controls             | cosmic-player `src/main.rs:2119`  |

**Conflict 1: the primary button.** cosmic-player makes a single click on
the video toggle play/pause (`src/main.rs:1771-1772`, `1507-1513`). Kyerag
cannot: the same press starts a drag to look around
(`crates/render/src/widget.rs`, `ButtonPressed` grabs immediately), and a
control that fires on press cannot coexist with a grab that starts on
press. Telling a click from a drag after the fact means a threshold and a
deferred action, which is complexity with no observed failure behind it.

Resolution: the video area does not toggle playback on click. Space does,
and the button in the control row does. Double click still toggles
fullscreen, which is safe: the two no-op grabs it also produces move
nothing.

**Conflict 2: the wheel.** Three first-party answers exist, and they
disagree with each other:

- cosmic-player: a bare wheel anywhere in the window changes the volume,
  suppressed while the nav bar is open (`src/main.rs:1277-1324`, `2120`).
- cosmic-files: the wheel zooms **only with Ctrl held**, and does nothing
  otherwise (`src/tab.rs:7326-7346`, wired at `src/tab.rs:6480`, with unit
  tests for both halves).
- libcosmic scrollables: a bare wheel scrolls, everywhere else.

Kyerag has no scrollable content in the video area, and the wheel is already
bound to zoom. Keep it: a bare wheel over the video zooms the view. It matches
cosmic-files' meaning (the wheel resizes what you are looking at) without
cosmic-files' modifier, which exists there only because a file list also
scrolls, and ours does not.

Audio has now landed (issue #13) and **the wheel stayed on zoom**. Volume went
where cosmic-player's own volume slider is, in the control row
(`src/main.rs:1780-1807`). Three things weighed against copying
cosmic-player's bare-wheel volume, and the owner's earlier ruling that the
wheel is zoom stands on all three:

1. The wheel is the *only* way to change the field of view with a pointer. A
   drag looks around; there is no second gesture free. Volume has a button, a
   slider and (later) MPRIS and the media keys.
2. cosmic-player's own wheel handler suppresses itself while the nav bar is
   open (`src/main.rs:1318`), which is an admission that a bare wheel over
   content is contested. Its content is a still picture; ours is a view
   direction.
3. cosmic-files is the closer precedent for what a wheel means over content
   that has a size: it resizes (`src/tab.rs:7326-7346`). We do the same thing
   without its Ctrl, because nothing under our pointer scrolls.

What the wheel does not do is settled, then, and `Ctrl+wheel` stays free.
Left open for the owner after real use: whether the **speaker button should
also take a wheel**, which is cosmic-player's own `//TODO`
(`src/main.rs:2032`) and would give volume a wheel without taking one from
zoom.

**Mute is `m`.** cosmic-player binds no mute key: its `key_bind.rs` is `f`,
`Alt+Enter`, `Space`, the two arrows, `.`, `,` and `a`, and mute is reachable
only through the dropdown's button (and through MPRIS, which we do not have
yet). Kyerag binds it anyway, which is mpv's key and is in keeping with the
two bare letters this app had already invented where no COSMIC app had a
precedent (`s` for save frame, `h` for the horizon lock). The owner approved
it on 2026-07-31. The key sends the speaker button's own message, so mute
persists the same way from either one, and a file with no sound in it takes
`m` and does nothing, which is the state the button is drawn disabled in.

## Screenshots, and where export will go

No COSMIC app captures its own view, so there is no precedent to defer to.
There is a shape to copy, though: in cosmic-player's control row, the right
hand group holds the actions that are about the view rather than the
transport (subtitles, speed, fullscreen, volume: `src/main.rs:2013-2051`).
A frame capture is exactly that kind of action.

- **Format:** a JPEG at quality 93 with full size chroma, because a still is
  a file to share and 12 MB is not (issue #15): 0.7 to 1.8 MB a still against
  5.3 to 13.3 MB as a lossless PNG, 52 to 54 dB PSNR over five real captures,
  and it opens on a stock desktop with nothing installed. The clipboard stays
  PNG.
- **Button:** `camera-photo-symbolic` in the control row, immediately left
  of fullscreen.
- **Menu:** `File > Save frame`, showing the `s` accelerator. No ellipsis,
  because it does not open a dialog: it writes to the configured folder,
  which is what issue #15 asks for.
- **Clipboard:** `File > Copy frame`, on `Ctrl+C` (cosmic-files
  `src/key_bind.rs:73`). Issue #15's paste-friendly copy.
- **Feedback:** a toast. Decided below.

### The capture toast

Shipped, and the open question is closed. cosmic-files is the only
first-party app with toasts at all: nothing in cosmic-player or cosmic-edit
mentions `toaster`. So its use of them is the whole precedent, and this
copies it rather than reasoning from the widget.

**The placement is a deviation from cosmic-files, and it is the owner's
call.** cosmic-files puts its toasts at the bottom of the window, and it can:
the bottom of a file manager is empty space. The bottom of this window is the
transport. The owner asked for the message at the top after seeing it land
over the progress bar, and that is what ships: the toast hangs under the
header bar, centred, one `space_m` below it, and the control row and the
scrubber are never covered.

**Which means the stock toaster widget cannot be used, and this is worth
recording, because reading it wrong costs a build.** `widget::toaster` does
not place its stack relative to whatever it is mounted over. Its overlay is
laid out against the bounds iced hands every overlay, which are the whole
window's (`ToasterOverlay::layout`, libcosmic
`src/widget/toaster/widget.rs:199-215`, against `overlay.layout(renderer,
self.bounds)` in `iced/runtime/src/user_interface.rs:228`), and it puts the
stack 15 px above the bottom of those bounds with no anchor, offset or
position argument. Mounting it over a fixed-height band at the top of the
window was built and captured under the harness: the toast did not move, and
sat across the scrubber exactly as before.

**So the stack is drawn rather than delegated**, out of the same pieces
libcosmic's own `toaster()` builds a toast from and with its own spacings:
`container(row![text, button::icon("window-close-symbolic")])
.padding([space_xxs, space_s, space_xxs, space_m])
.class(theme::Container::Tooltip)`, one per line, newest first
(`src/widget/toaster/mod.rs:33-63`). It is a `Stack` layer over the picture,
top aligned and centred. Three things fall out of that choice and all three
are load-bearing:

- **The control row keeps working.** A `Toaster` with a toast up returns its
  own overlay *instead of* its content's (`toaster/widget.rs:137-162`, whose
  author left a `//TODO` beside it) and our control row *is* an overlay, the
  popover's. `Stack` hands back `overlay::from_children` instead, so the row
  is still there while a toast is up. That was the reason cosmic-files'
  mount-over-an-empty-element trick was copied in the first place, and it is
  no longer needed.
- **The picture still takes a drag.** `Stack::update` only takes the cursor
  away from the layer beneath where the layer above reports an interaction
  for that position (`iced/widget/src/stack.rs`), and a container of text
  reports none, so looking around still starts anywhere except on a toast's
  own close button.
- **The layer is mounted whether or not it holds a toast.** Building the
  stack only when one arrives changes the shape of the tree around the
  shader widget, and that is not free: measured under the harness, the toast
  reached the screen on the first capture after it landed with a fixed tree
  and on the sixth, two seconds later, with a tree that grew a layer.

Measured on a 1280x720 headless session, dark and light: the toast occupies
rows 72 to 119 of the window, the header bar is rows 0 to 47 and the control
row is rows 672 to 719. Five stacked toasts reach row 327, still 297 rows
clear of the control row. `scripts/uitest.sh` asserts it rather than trusting
it: "a toast is drawn clear of the controls" captures the paused window with
and without a toast up and requires the top 64 rows and the bottom 96 to be
byte for byte identical between the two, and requires something to have
changed somewhere so that the check cannot pass by showing nothing. Against
the bottom placement this PR replaced, that check fails.

**The copy**, in the app's own vocabulary. The menu says `Save frame` and
`Copy frame`, so the toast says frame:

| event                | toast                                       |
| -------------------- | ------------------------------------------- |
| `Save frame` worked  | `Frame saved to "Screenshots"`              |
| `Copy frame` worked  | `Frame copied to the clipboard`             |
| `Save frame` failed  | `Frame not saved: {reason}`                 |
| `Copy frame` failed  | `Frame not copied: {reason}`                |
| `Calibrate seam` worked | `Seam calibrated for this camera`        |
| `Calibrate seam` failed | `Seam not calibrated: {reason}`          |

The calibration toast says **camera** rather than video, because that is the
whole difference between the action and what the app does on its own: the
answer is kept, and every later video off the same camera opens with it.

The destination is the folder's own name in quotes and never a path, which
is how cosmic-files names one in its toasts: `copied = Copied {$items} items
from "{$from}" to "{$to}"` (`i18n/en/cosmic_files.ftl:231-234`) built from
`file_name(to)`, the last component only (`src/operation/mod.rs:563-568`,
`309-312`). The name follows whatever the capture resolved to, so a session
with `XDG_SCREENSHOTS_DIR` set elsewhere says that folder's name instead
(the headless harness reads `Frame saved to "shots"`). The whole path still
goes to the terminal, where it does not have to be read at a glance.

**Duration and stacking are stock**, and they are stock in the strong sense:
the numbers and the mechanism are both libcosmic's, only the anchor moved.
Five seconds is `Duration::Short` (`toaster/mod.rs:79-85`), which is what
cosmic-files leaves every toast on (`src/app.rs:1344-1358`). Five lines are
kept and the oldest is dropped past that (`toaster/mod.rs:162-181`); toasts
stack rather than replace. A line is taken away by a five second sleep on the
async runtime, which is exactly how `Toasts::push` does it
(`toaster/mod.rs:183-196`), and not by a timer the shell keeps: a poll was
written first and measured against the stock build, and it cost 3 to 6 extra
redraws a second and dropped frames in 2 of 18 report windows, against 0 of
18 with the sleep. Reading the queue back to front puts the newest line
nearest the anchored edge, which is libcosmic's order (`toaster/mod.rs:56-63`)
read against the top, so five quick captures stack downward from the header
and no line already on screen moves when the next one lands.

**No action button, for now.** libcosmic's toast carries one if it is asked
to (`Toast::action`, `toaster/mod.rs:132-144`), and cosmic-files asks in
exactly one place: `Undo` on the toast for a delete
(`src/app.rs:1344-1352`). That is the shape of it there, a way back from
something destructive. Nothing in cosmic-files, cosmic-player or cosmic-edit
opens a location from a toast; cosmic-files' own "Open item location" is a
context menu item (`i18n/en/cosmic_files.ftl:91`, `src/menu.rs:244`,
`379`). So a "Show in Files" button has no first-party precedent to copy and
does not ship. If the owner wants it later, the portal call is
`org.freedesktop.FileManager1.ShowItems` and the toast already carries the
path it would need.

**Export (issue #12, M3)** goes in the `File` menu only, as
`Export clip...` with the ellipsis, since it opens a dialog and then runs
for a long time. Progress belongs in `fn footer()`: cosmic-files renders
its running file operations there as a progress bar with a title, shown
only while operations are pending (`src/app.rs:6297-6320`). Nothing about
export goes in the control row.

## Menus

Two roots, `File` and `Playback`, exactly as cosmic-player has
(`src/menu.rs:113-140`), plus the `View` root that cosmic-files and
cosmic-edit both have and that our zoom actions need. Build it with
`responsive_menu_bar()` (cosmic-edit `src/menu.rs:229-240`,
cosmic-files `src/menu.rs`), not the older `MenuBar::new` that cosmic-player
still uses: it collapses to a single button on narrow windows, which for a
video player is a normal size. `item_height(ItemHeight::Dynamic(40))`,
`item_width(ItemWidth::Uniform(320))`.

```
File                      Playback                 View
  Open video...             Play / Pause             Zoom in
  Open recent >             Back 10 seconds          Default view
  Close video               Forward 10 seconds       Zoom out
  ---                       ---                      ---
  Save frame                Previous frame           [x] Lock horizon
  Copy frame                Next frame               Calibrate seam from this video
  ---                                                ---
  Quit                                               Fullscreen
                                                     ---
                                                     Settings...
                                                     About Kyerag...
```

- Ellipsis on items that open a dialog, none on items that act
  (cosmic-player `Open media...` vs `Close file`, `src/menu.rs:119-121`).
- `Lock horizon` is a checkbox rather than a pair of items, because it is a
  state (issue #8); cosmic-files spells `Show hidden files` the same way.
- `Calibrate seam from this video` measures what this camera's two lenses
  disagree by and keeps the answer for every later video off that camera
  (issue #48). It sits beside the horizon lock because both are about how
  the picture is put together rather than about the file it came out of, it
  is disabled for a capture with no seam in it, and it says what it did in
  a toast: the run takes a second or two on a worker thread and the window
  keeps playing while it does. Nothing on screen says it is running, which is
  the same call the capture toast makes: an action that is over in two seconds
  and has a result line does not want a progress bar in a window whose job is
  to keep drawing.

  **A capture from a camera standing still is what it wants pointed at it**,
  and the reason is measured rather than a preference
  (docs/research/insv-format.md 6.8): a fit taken through a flight's seam
  absorbs that flight's own parallax into the answer, and a still capture of
  far content has none to absorb. The app says so where it matters rather
  than in a dialog nobody reads: a camera with no calibration prints the
  advice in its report line as it falls back to fitting off the open file.
- `Settings...` then a divider then `About <app>...` at the end of `View`
  is the shared convention: cosmic-files `src/menu.rs:762-764`, cosmic-edit
  `src/menu.rs:346-350`.
- Accelerators are drawn from the `key_binds` map automatically; the menu
  item just names the action.
- `Playback` items are disabled (no `on_press`) with no file open, which is
  cosmic-player's own pattern for the frame-step items: it builds them into
  the menu only when the loaded file has video (`src/menu.rs:89-97`).
- Two items here have no cosmic-player precedent: it has no `Play / Pause`
  and no `Fullscreen` menu item, only the key and the button for each. They
  are added because the menu is where a keyboard shortcut is advertised,
  and a shortcut nobody can find is a shortcut nobody uses.

## Settings

A context drawer page, not a separate window. cosmic-edit
`src/main.rs:2995-3026` and cosmic-files both do it this way:
`fn context_drawer()` returns
`context_drawer::context_drawer(self.settings(), Message::ToggleContextPage(ContextPage::Settings)).title("Settings")`,
and the page body is `widget::settings::view_column(vec![...])` of
`widget::settings::section().title(...)` with
`widget::settings::item::builder(label).control(...)` rows (cosmic-edit
`src/main.rs:1285-1375`).

`fn on_escape()` closes the drawer before it does anything else
(cosmic-edit `src/main.rs:1583-1592`); ours closes the drawer first and
leaves fullscreen second.

What we have to put in it, which is not much:

```
Appearance
  Theme            [ Match desktop | Dark | Light ]
Screenshots
  Save to          [ folder button ]
  Resolution       [ Window size | 2x window | Source ]
```

The theme row is verbatim cosmic-player's config shape:
`AppTheme { Dark, Light, System }` with `AppTheme::theme()` returning
`theme::Theme::dark()`, `light()`, or `system_preference()`
(`src/config.rs:13-27`), applied through `set_theme(...)` on change
(`src/main.rs:645-647`). The screenshot rows are issue #15's; the exact
resolution choices are #15's to settle, not this document's.

**Persistence.** Two cosmic-config entries, which is the split every
first-party app uses:

- `Config` (`CONFIG_VERSION: u64 = 1`): things the pilot chose.
  `app_theme`, `screenshot_dir`, `screenshot_scale`.
- `ConfigState`: things the app remembers. `recent_files`, and
  `seam_calibration`, one entry per camera under a serial-free fingerprint
  of that camera's own factory calibration (issue #48).

`seam_calibration` is state and not config for the same reason
`recent_files` is: it is something the app measured rather than something
the pilot expressed, it has no row on the Settings page, and resetting the
settings must not throw it away. It is not a cache either. Deleting it does
not cost a recompute, it costs the pilot the capture he pointed the app at.

Both derive `CosmicConfigEntry` and both get a `cosmic_config` subscription
so an external change applies live (cosmic-player `src/config.rs`,
`src/main.rs:121-152` and `2123-2155`). Also subscribe to
`cosmic_theme::THEME_MODE_ID` so "Match desktop" follows the desktop
(`src/main.rs:2145-2155`).

## About

`context_drawer::about(&self.about, |url| Message::LaunchUrl(url.into()),
Message::ToggleContextPage(ContextPage::About))` (cosmic-edit
`src/main.rs:3001-3005`), with the `About` value built once in `init`
(cosmic-edit `src/main.rs:1454-1468`):

```rust
About::default()
    .name("Kyerag")
    .icon(icon::from_name(Self::APP_ID))
    .version(env!("CARGO_PKG_VERSION"))
    .author("Alexander Harding")
    .comments("360 video player for the COSMIC desktop")
    .license("AGPL-3.0-only")
    .links([
        ("Repository", "https://github.com/aeharding/kyerag"),
        ("Support", "https://github.com/aeharding/kyerag/issues"),
    ])
```

- `LaunchUrl` is `open::that_detached(&url)` with a warn on failure, in
  both cosmic-edit (`src/main.rs:2137-2142`) and cosmic-files
  (`src/app.rs:3419-3424`).
- The `about` widget is behind libcosmic's `about` feature
  (`Cargo.toml:24`), which is not in the default set. The app crate's
  feature list becomes `["wgpu", "xdg-portal", "about"]`.
- No `developers([...])` list: that setter takes name and email pairs and
  turns them into `mailto:` links (`src/widget/about.rs:45-50`), and this
  repo does not publish personal addresses. Name in `.author()`, contact
  through the repository link.
- `.icon(icon::from_name(APP_ID))` resolves nothing until an app icon is
  installed. Icon and desktop-file packaging are out of scope for issue
  #16; see the build order.

## Copy

Plain words, no em dashes (AGENTS.md). Sentence case for labels, which is
what the first-party apps use ("Open recent media", "Clear recent list").
A space before a unit ("10 seconds", System76 HIG).

No i18n in the first landing. All three first-party apps use
`i18n-embed` + fluent with an `fl!` macro, and that is real machinery for a
pre-alpha with one user. Keep every user-facing string in one module so the
later move is mechanical, and treat this as a deliberate deviation to
revisit before any public release rather than as a decision that the
strings should live inline forever.

## Deviations from cosmic-player, collected

Everything above that does not copy cosmic-player, with the reason, so a
reviewer can find them in one place:

1. **Scrubbing seeks to keyframes until release** (accurate seek on
   release). A 30-minute dual 3840x3840 HEVC file cannot serve an accurate
   seek per slider tick. Issue #5.
2. **No click-to-play/pause on the video.** The press is the look-around
   grab.
3. **No nav bar.** No playlist, and returning `None` keeps a dead toggle
   out of the header bar.
4. **Local paths, not URLs**, on the command line and in recents. We decode
   files, not streams.
5. **A window title with the file name in it**, using a hyphen rather than
   cosmic-files' em dash, per AGENTS.md.
6. **`responsive_menu_bar` rather than `MenuBar::new`.** cosmic-player is
   the only one of the three still on the old API.
7. **No i18n yet.**
8. **Volume and mute are remembered.** cosmic-player keeps neither: both are
   GStreamer playbin properties and start at 1 and false every run. The owner
   asked for them to persist, and they go in `Config` beside the theme.
9. **The volume popup closes with the control row**, rather than holding the
   row open the way cosmic-player's dropdowns do. See "The volume slider".
10. **The capture toast is at the top of the window**, where cosmic-files
    puts its own at the bottom, and it is drawn rather than delegated to
    `widget::toaster`. Owner's call, and the reason is the window: the
    bottom of a file manager is empty space and the bottom of this one is
    the transport. See "The capture toast".

Two things cosmic-player does that we should copy later, neither of them
part of issue #16:

- **Idle inhibit while playing**, through the XDG inhibit portal
  (`src/xdg_portals.rs`, called from `load` and from play/pause at
  `src/main.rs:1264-1268`). A 30-minute video with no input events will
  blank the screen without it. Cheap, and worth its own issue.
- **MPRIS**, which is what makes the media keys and the sound-applet
  controls work (`src/mpris.rs`, 461 lines). Worth an issue once audio
  exists.

## Open questions

1. Answered by issue #13: the wheel stayed on zoom and volume went into the
   control row. The mute key was answered on 2026-07-31, and `m` is bound.
   What is left of it is smaller: should the speaker button take a wheel of
   its own? Above, under "Conflict 2".
2. Answered: the capture toasts shipped, with the wording and the reasoning
   under "The capture toast" above. They carry no action button, because no
   first-party app opens a location from a toast.
3. Whether the frame-capture button belongs in the control row at all, or
   whether the menu item plus `s` is enough. No precedent either way.
4. `Ctrl+0` as "default view" resets yaw, pitch and field of view together.
   The zoom trio it borrows from resets only zoom, because that is all
   those apps have. Splitting them later would need a second key.
5. An app icon and a MIME type. `.insv` has no registered MIME type
   anywhere; playing one from Files needs a shared-mime-info definition
   plus a desktop file, and thumbnails would need a `.thumbnailer` like
   cosmic-player's (`res/com.system76.CosmicPlayer.thumbnailer`). Out of
   scope here, wants its own issue.

## Build order

The shell splits cleanly around the two media issues it depends on. Nothing
in the first group needs a decoded frame, a clock, or a seek.

**Lands now, before seek (#5) and independent of the playback core (#4):**

- Window, header bar, menu bar, no nav model, no header title.
- The welcome view, and the failure-to-open line under it.
- Open: portal dialog, drag and drop, command line, recent files.
- cosmic-config `Config` and `ConfigState`, with both subscriptions and the
  theme-mode subscription.
- Settings context page (the theme row; screenshot rows land with #15).
- About context page, and the `View` menu that opens both.
- The key-bind map and `Message::Key`, replacing the hand-rolled space
  handler. Every binding whose action already exists is live; the rest sit
  in the map doing nothing, which is how the menu draws their accelerators
  before the actions behind them work.
- Fullscreen: key, menu item, button, double click, Escape, header hiding.
- The control row as a container, with the auto-hide timer, the cursor
  hiding, and the condensed two-row split. Play/pause goes live with issue
  #4, which is in flight and already has `Scene::toggle_play`; the seek
  buttons and the scrubber render disabled until then.
- Elapsed and remaining time. The duration needs no new media work once
  issue #4 lands: its `Timing` carries the frame count and the rational
  frame rate, and the two divide.
- View menu zoom in / zoom out / default view. Pure camera state, no media.

**Waits for the playback core (#4), which is landing in parallel:**

- Play/pause enabled state and the icon that follows it.
- The auto-hide timer's "only while playing" rule, which needs the
  presented-frame signal.

**Waits for seek (#5):**

- The scrubber becoming interactive: `Seek`, `SeekRelease`, and the
  keyframe-while-dragging behavior.
- `Left` / `Right` 10-second seeks and the two jump buttons.
- `,` / `.` frame stepping.

**Waits for screenshots (#15):**

- The frame-capture button and its two menu items, the save folder and
  resolution settings rows, and the toast.

One implementation note that crosses layers: the cursor hiding needs
`Scene::mouse_interaction` to return `Interaction::Hidden` while the
controls are hidden, so `crates/render` gains one boolean of shell state.
That is the only change this design asks for outside `crates/app`.
