# Roadmap (living doc)

Update this file in any PR that changes project status. Work queue is
GitHub issues; this doc is the map, issues are the tasks.

**Status 2026-07-31:** feasibility study complete (docs/research/), repo
bootstrapped, M0 done. `cargo run --release -p kyerag-spike -- <file.insv>`
decodes one 3840x3840 lens on VA-API, imports the dmabuf planes into wgpu
with no copy, and renders to PNG at 103 fps (3.4x realtime). `cargo run
--release -- <file.insv>` shows that frame in a libcosmic window, imported
zero-copy onto the device iced created and sampled inside iced's own render
pass: the shell, the shader widget and the wgpu-28 import all confirmed on
screen. M1 has started: `crates/meta/` reads the trailer's calibration
(issue #2), and the source tree is now a workspace with one crate per layer
(issue #19). Next: dual decode and the Mei reprojection (issue #3), which
consumes the `CalibrationSet` as it stands.

## Milestones

- **M0 Pipeline proof** — decode one lens via VA-API, import into wgpu
  zero-copy, render headless to PNG with timings. Done (`crates/spike/`,
  issue #6). Shell bring-up followed in issue #1: libcosmic window, shader
  widget, and the wgpu-28 port of the import.
- **M1 Reframing player** — dual decode, calibrated Mei reprojection,
  drag to reframe, scroll to zoom, play/pause/seek, screenshots. The MVP.
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

## Ideas parked (complexity needs an observed failure first)

- Decoded-GOP cache in GPU memory for instant reverse scrubbing
  (~44 MB/frame; a 30-frame window is ~1.3 GB).
- Vulkan Video decode (drops VA-API plumbing; blocked on wgpu exposure
  and Rust HEVC support anyway).
- Batch screenshot/export queue across multiple files.
