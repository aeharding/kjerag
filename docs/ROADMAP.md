# Roadmap (living doc)

Update this file in any PR that changes project status. Work queue is
GitHub issues; this doc is the map, issues are the tasks.

**Status 2026-07-31:** feasibility study complete (docs/research/), repo
bootstrapped, M0 done, and the picture moves.
`cargo run --release -p kyerag-spike -- <file.insv>` decodes one 3840x3840
lens on VA-API, imports the dmabuf planes into wgpu with no copy, and
renders to PNG at 103 fps (3.4x realtime). `cargo run --release --
<file.insv>` plays the file in a libcosmic window, every frame imported
zero-copy onto the device iced created and reprojected inside iced's own
render pass: the shell, the shader widget and the wgpu-28 import all
confirmed on screen. M1 is under way: `crates/meta/` reads the trailer's calibration
(issue #2), the source tree is a workspace with one crate per layer
(issue #19), the picture is reprojected through the lenses' own Mei/UCM
models with drag to look around and scroll to zoom (issues #3, #26), and the
playback core plays it (issue #4): one demuxer, both lenses decoded in
lockstep and delivered as pairs, a presentation clock that paces 29.97 fps
content by due time, space to pause, and frames pullable by index or
timestamp. Measured over 60 s of real footage: 29.94 fps presented, zero
dropped, zero starved. **The sphere is closed** (issue #27): the shader
projects every ray into both lenses and samples the one whose optical axis
it is nearer, so turning around shows the back hemisphere, upright and
unmirrored, with a hard seam and an exposure step where the two meet. Next:
the seam blend and exposure match (issue #7), which is what makes that seam
disappear, and seek (issue #5) on the same pull API.

## Milestones

- **M0 Pipeline proof** — decode one lens via VA-API, import into wgpu
  zero-copy, render headless to PNG with timings. Done (`crates/spike/`,
  issue #6). Shell bring-up followed in issue #1: libcosmic window, shader
  widget, and the wgpu-28 port of the import.
- **M1 Reframing player** — dual decode, calibrated Mei reprojection,
  drag to reframe, scroll to zoom, play/pause/seek, screenshots. The MVP.
  Reprojection and the mouse are done (issue #3), and so is playback:
  dual-stream decode, the presentation clock and play/pause (issue #4).
  Full 360-degree look-around lands here too (issue #27): both lenses
  sampled, hard seam, which #7 blends in M2.
- **M2 Quality** — seam blend + per-frame exposure match, gyro horizon
  lock (+ Studio-diff test harness), rolling-shutter correction,
  hemisphere-aware decode gating, high-quality zoom sampling.
- **M3 Export & sound** — clip export (reframed VCN encode, and lossless
  time-range remux), audio playback.

## Decisions log

- 2026-07-30 License AGPL-3.0 (Alex; matches wingover). Unlocks Gyroflow
  GPL-3.0 shader reference with attribution.
- 2026-07-30 No LRV proxy dependence (Alex): full-res decode must stand
  alone; generate proxies only if ever proven necessary.
- 2026-07-30 Frame delivery via DRM_PRIME dmabuf, not hwcontext_vulkan
  (device-lost reproduced on target hardware) and not GStreamer (no
  wgpu/dmabuf sink).
- 2026-07-30 No optical-flow stitching: static calibrated warp measured
  equal or better for a player.
- 2026-07-30 Insta360 MediaSDK rejected: NDA, non-redistributable, no
  seek API, bundled cloud-calling codec.
- 2026-07-30 Primary target is AMD/Intel Mesa (VA-API). NVIDIA would need
  an NVDEC backend variant; out of scope until someone needs it.
- 2026-07-30 ffmpeg-next/ffmpeg-sys-next pinned to 6.1, matching the system
  ffmpeg. The 8.x APIs in the research notes are not present.
- 2026-07-30 Zero-copy import is not a hand-rolled ash routine: wgpu 30's
  `Device::texture_from_dmabuf_fd` (wgpu-hal Vulkan) imports the VA-API
  planes as they come, 0.12 ms/frame for both. On libcosmic's wgpu 28 the
  same import is written by hand (see next entry).
- 2026-07-30 Shell: libcosmic from day one (Alex). Native COSMIC chrome is
  part of the product identity. We accept the hand-rolled ash dmabuf
  import against libcosmic's wgpu 28, and delete it the day libcosmic
  reaches wgpu 30.
- 2026-07-30 PR policy (Alex): the coordinator self-merges once CI is
  green and the diff is reviewed. Alex steers via issues, the roadmap,
  and check-ins.
- 2026-07-30 The trailer is read directly (`crates/meta/src/trailer.rs`, ~45
  lines) instead of through `telemetry-parser`. The published 0.2.6
  aborts on our X4 Air footage: it serializes the metadata protobuf's
  enum fields with `unsafe { transmute }` of the raw i32, and the file
  carries a value no enum in that schema has. The fix exists only on an
  unpublished master that pulls two further git forks. Kyerag was already
  bypassing that crate's lens profile (wrong on the Air) and would have
  had to bypass its merged exposure records (M2), so what remained was
  the record walk. `prost` decodes the eleven fields we read.
- 2026-07-30 libcosmic still pins wgpu 28 (checked, not assumed:
  `pop-os/libcosmic@dc1cf9f` vendors `pop-os/iced@7346cff`, whose
  workspace `Cargo.toml` says `wgpu = "28.0"`; the lockfile resolves
  wgpu 28.0.0 / wgpu-hal 28.0.1). The hand-rolled import stands.
- 2026-07-30 The whole crate is on wgpu 28, spike included. Two wgpu
  majors in one graph would mean two sets of incompatible types for the
  same textures, and the spike's job is to measure the code the app runs.
- 2026-07-31 Carry a wgpu fork (Alex). Nothing exposes
  `VK_EXT_image_drm_format_modifier` on the device `iced_wgpu` creates, so
  the shader widget could not import a frame on stock dependencies. We
  carry the extension-enable hunk of gfx-rs/wgpu#9366 (shipped in wgpu 30)
  applied verbatim to the v28.0.1 tag, on
  `aeharding/wgpu@v28-drm-modifier-backport`, through `[patch.crates-io]`.
  The alternative was a second wgpu device and an external-memory handoff:
  roughly 200 more unsafe lines and a CPU stall per frame.
  **Deletion condition: the day libcosmic reaches wgpu 30, delete the patch
  entry, the fork, and `crates/render/src/dmabuf.rs`, and call
  `vulkan::Device::texture_from_dmabuf_fd` instead.**
- 2026-07-31 The patch entry names `wgpu`, not `wgpu-hal`. Patching
  wgpu-hal alone leaves the rest of the tree on the crates.io wgpu-types,
  and two wgpu-types in one graph is two incompatible `TextureFormat`s.
  `wgpu` is the only crate in that workspace anything outside it depends on.
- 2026-07-31 The app turns libcosmic's content container off
  (`core.window.content_container = false`). It insets the view by
  `border_padding` on the right and, because `nav_bar.active` defaults to
  true even with no nav model, by nothing on the left. Video wants both
  edges (issue #22).
- 2026-07-31 One crate per layer, in a workspace (issue #19): `kyerag-meta`,
  `kyerag-media`, `kyerag-render`, `kyerag` (the app) and `kyerag-spike`.
  The layer diagram is now a build constraint, and `kyerag-meta` builds and
  tests with no libav headers anywhere on the box, which a CI job that
  installs nothing checks on every push. `[patch.crates-io]` moved to the
  workspace root, the only manifest cargo reads one from.
- 2026-07-31 `kyerag-render` depends on libcosmic, for one file. The three
  `iced::widget::shader` impls are a foreign trait on types `render` owns,
  and coherence forbids writing them in `kyerag`. The alternative, a set of
  forwarding newtypes in the app crate, is more code for the same wiring;
  they live in `crates/render/src/widget.rs` and nothing else in the crate
  mentions iced.

- 2026-07-31 The lens pose composes as `Rz(roll - 90) * Ry(yaw) * Rx(pitch)`
  in the delivered frame's own axes (x right, y down, z along the optical
  axis). The quarter-turn datum is measured, not assumed: applying `roll` as
  `offset_v3` writes it renders an X4 Air on its side, and dropping `roll`
  renders a ONE X2 on its side. Four candidate rotations were rendered
  against plumb references on both cameras
  (docs/research/insv-format.md 4.8). This closes the last open question
  from the format study's section 10.
- 2026-07-31 The Mei forward map is written from the model description
  (Mei/Rives, OpenCV `omnidir`) rather than transcribed from Gyroflow's
  `insta360.wgsl`, so `crates/render/src/projection.rs` is plain AGPL-3.0
  with no SPDX header. The Gyroflow route stays open and stays licensed:
  any file that takes it carries its own header.
- 2026-07-31 The forward map exists twice, in WGSL and in Rust, sharing one
  `Reframe` uniform block. The Rust copy is what `cargo test` can check
  against known angles on a box with no GPU and no footage, and
  `min_binding_size` is what makes wgpu reject a pipeline whose two
  definitions have drifted.
- 2026-07-31 The camera lives in the shader widget's own iced state
  (`Viewpoint`), not in the app's model. Panning is a widget concern, the
  shell has no opinion about it, and no message round trip happens per
  mouse move.
- 2026-07-31 The drag anchors and solves (issue #29). A press stores the
  world direction under the cursor, and every move solves for the view that
  puts that direction back under the cursor: height above the horizon fixes
  the pitch, bearing then fixes the yaw. Stepping yaw and pitch by the
  cursor's own movement is only grab-the-world near the middle of the view,
  because near the pole a yaw turns about an axis nearly along the view
  ray. The horizon stays level, so where an exact answer would need roll
  the pitch clamps and the drag reads as a wall; and where a view pitched
  near the vertical sees past the pole, the solve takes the tilt nearest
  the one it is already at rather than the mirrored view that fits equally
  well.
- 2026-07-31 A ray is in the picture only where the Mei map stays one to
  one, `cos(theta) > -1/xi`, as well as inside the image circle (issue
  #30). Past that turning point the map folds rays from behind the camera
  back inside the circle, which showed as a raw circular fisheye hanging
  behind the reframed view. The bound comes out of the calibration's own
  xi, so no maximum field of view has to be guessed at. It is per lens: a
  fold in one lens is now a ghost printed over a picture the other lens is
  drawing correctly.
- 2026-07-31 The pick between lenses is nearest axis, and it is a branch
  rather than a blend (issue #27). One lens is sampled per output pixel,
  which halves the texture fetches against sampling both and selecting; the
  cost is a hard seam and an exposure step, and both are #7's. A lens that
  has the ray beats one that does not, so the overlap covers a lens running
  out of coverage before the halfway line, and nothing is left grey: the
  two 97.5-degree caps overlap by about 15 degrees. The branch samples with
  an explicit mip level, because a `textureSample` needs uniform control
  flow to compute one and every imported texture has a single level anyway.
- 2026-07-31 Lens 1's nominal arrangement is a half turn about the body's
  vertical, multiplied on the right of the block's own angles (issue #27,
  docs/research/insv-format.md 4.9). The file does not contain the flip at
  all, and the two orders differ by twice lens 1's roll residual, so this
  was measured rather than picked: each lens rendered alone across the
  seam, and the far-field content correlated between the two pictures.
  0.4 degrees of along-seam residual for this order against 1.5 for the
  other, and the half turn about x, which is a rear sensor upside down,
  correlates with nothing at all.
- 2026-07-31 The shell's design is written down before it is built, in
  docs/UI.md, from the cosmic-player / cosmic-files / cosmic-edit sources
  (issue #16). Every decision in it cites the first-party file it copies,
  and the places where no COSMIC precedent exists are listed as open
  questions rather than answered by us. The written guidelines turned out
  to cover almost none of this: `system76/hig` is one README about dialogs
  and copy that defers to the elementary HIG, which has no keyboard,
  header-bar or media page either. The source is the guideline.

- 2026-07-31 Frames are delivered in pairs, not one stream at a time
  (issue #4). `Frames` carries every video stream at one PTS, so the two
  lenses cannot drift apart: there is no code path that delivers lens 0
  without lens 1. Both are imported into wgpu, and since issue #27 both are
  sampled.
- 2026-07-31 Playback is paced by due time, not by counting refreshes. Each
  frame's due time comes off a monotonic clock anchored to the first frame
  presented, and the shell sleeps until it (`RedrawRequest::At`). The
  shell-side alternative, a `window::frames()` subscription pumping the
  clock on each redraw message, was built first and measured: 33-46
  redraws/s on a 60 Hz display and 1-18 dropped frames per 5 s, because the
  event has to leave iced and come back. The clock is pumped inside the
  redraw pass instead, in `kyerag_render`'s shader widget, which costs a
  `RefCell` in `Scene` and buys 30.0 redraws/s with nothing dropped.
- 2026-07-31 The engine holds container PTS as the frame clock for now. The
  trailer's `pts_type = 2` (`VideoPtsEexposureFile`) suggests the per-frame
  exposure records are the camera's authoritative clock; #4 is pacing, not
  gyro alignment, and #8's Studio-diff harness is what can tell. Only
  `Frames::timestamp` changes if it turns out otherwise.
- 2026-07-31 `Reader::lookahead` is 2 (issue #4). Mapping the oldest queued
  surface rather than the newest hides the `vaSyncSurface` inside
  `av_hwframe_map`: 2.19x realtime at depth 0, 2.46x at depth 2, 2.47x at
  depth 4, so depth 2 takes the whole win. docs/ARCHITECTURE.md's "2-3
  frames in flight" now has a number behind it.
- 2026-07-31 `media::first_frame` is gone. Everything reads through
  `Reader`, which takes a `Cue` (frame index or timestamp), seeks to the
  keyframe at or before it and walks forward without mapping what it
  passes: 0.22 s cold to any frame in a 3 GB file, position-independent.
  This is the entry point #5's seek and #8's harness build on, and the
  `reframe` instrument now takes `frame=` and `time=`.

## Measured on the target box (AMD Phoenix, RADV, 3840x3840 HEVC)

Per frame, 300 frames, one lens, one frame in flight (`crates/spike/`):

| path      | demux | decode | deliver | import | render | fps   |
| --------- | ----: | -----: | ------: | -----: | -----: | ----: |
| zero-copy | 0.14  | 0.85   | 7.64    | 0.12   | 0.91   | 103.0 |
| copy      | 0.10  | 0.57   | 45.3    | 2.25   | 3.05   |  18.4 |

`deliver` is `av_hwframe_map` (zero-copy) or `av_hwframe_transfer_data`
(copy); `import` is the dmabuf import or `write_texture`. The map stage is
dominated by the `vaSyncSurface` inside it, so it is really decode wait: a
player that keeps 2-3 frames in flight gets that time back. The copy path
does not reach realtime for even one of the two lenses.

Playback, both lenses, 60 s of a 3840x3840 29.97 fps file, rendering
2560x1440 (`cargo run --release -p kyerag-spike --bin playback`):

| lookahead | decode        |
| --------- | ------------- |
| 0         | 2.19x realtime |
| 2         | 2.46x realtime |
| 4         | 2.47x realtime |

| what                        | measured |
| --------------------------- | -------- |
| presented                   | 29.94 fps |
| redraws                     | 30.0 /s   |
| dropped                     | 0         |
| starved                     | 0         |
| worst late                  | 6.6 ms    |
| reprojection pass           | 1.31 ms/redraw |
| CPU (decode + import + pass) | 9.1% of one core |

Sampling both lenses (issue #27) costs the pass about a quarter more, and
nothing else: measured back to back against the same binary before the
change, three runs each at 2560x1440, 1.26 to 1.48 ms/redraw with one lens
against 1.59 to 1.90 with two, still 0 dropped and 0 starved either way.
Every output pixel now runs the Mei map twice, once per lens, and samples
once. Sampling both and selecting afterwards would have cost the fetches
too; skipping the second projection where the first lands well inside its
own hemisphere is the obvious saving and is not taken, because 1.6 ms of a
33 ms frame is not a problem yet (see also issue #10).

The windowed app over the same 60 s: zero dropped and zero starved in
every 5 s report, 30.0-30.2 redraws/s, 13.4% of one core and 295 MiB RSS
for the whole libcosmic process.

## Ideas parked (complexity needs an observed failure first)

- Decoded-GOP cache in GPU memory for instant reverse scrubbing
  (~44 MB/frame; a 30-frame window is ~1.3 GB).
- Vulkan Video decode (drops VA-API plumbing; blocked on wgpu exposure
  and Rust HEVC support anyway).
- Batch screenshot/export queue across multiple files.
