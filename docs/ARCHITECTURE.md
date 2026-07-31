# Architecture

## Layers

One crate per layer, in a cargo workspace (issue #19). A layer cannot use
a dependency it does not declare, so the diagram below is enforced by
`cargo build` rather than by good intentions.

```
crates/app      kyerag         libcosmic shell + window. The view is an
                               `iced::widget::shader` around a Scene, and
                               the mouse reaches it through that widget.
crates/render   kyerag-render  wgpu: dmabuf import, one WGSL pass (NV12 ->
                               RGB + Mei reprojection + seam blend),
                               camera state (drag = yaw/pitch, scroll = FOV),
                               offscreen render for screenshots
crates/media    kyerag-media   ffmpeg demux, dual VA-API HEVC decoders,
                               frame clock, keyframe index, seek. No UI.
crates/meta     kyerag-meta    .insv trailer, read directly: per-lens Mei
                               calibration, gyro track, per-frame exposure.
                               No UI, no ffmpeg, no wgpu.
crates/spike    kyerag-spike   the headless instruments, kept out of the
                               app's dependency graph: `spike` (M0 frame-path
                               timings) and `reframe` (the projection pass to
                               a PNG, no compositor needed)
```

`app` -> `render` -> `media`, `render` -> `meta` for the calibration the
shader runs on, and `meta` depends on nothing but `prost`. That last one is
the point of the split: `cargo test -p kyerag-meta` passes on a box with no
libav headers, and a CI job that installs nothing proves it on every push.

`media` and `meta` know nothing about the shell. `render` names libcosmic
for exactly one file, `crates/render/src/widget.rs`: those three
`iced::widget::shader` impls put a foreign trait on types `render` owns, and
Rust's coherence rules will not let the shell crate write them. The
alternative was a set of forwarding newtypes in `app`, which is more code
for the same wiring. Nothing else in `render` mentions iced.

The shell is libcosmic, which pins wgpu 28, so `render` is written against
28 and owns the one module that wgpu 30 would delete
(`crates/render/src/dmabuf.rs`).

`Size` and `Fallible` live in `media`: they are frame types, and `render`
depends on `media` rather than the other way round. `render` re-exports both
and adds the `Extent` trait, which is the `wgpu::Extent3d` half of `Size`
that cannot live in a crate with no wgpu.

## The frame path (zero-copy)

```
VA-API decode (two 3840x3840 HEVC streams, one demuxer)
  -> av_hwframe_map(dst = AV_PIX_FMT_DRM_PRIME, MAP_READ | MAP_DIRECT)
  -> AVDRMFrameDescriptor { fd, offset, pitch, format_modifier } per layer
  -> two single-plane wgpu textures per frame:
       R8Unorm  from layer 0 (luma,   DRM_FORMAT_R8)
       Rg8Unorm from layer 1 (chroma, DRM_FORMAT_GR88 - note GR, not RG)
  -> single fragment pass: sample both lenses, Mei inverse-project,
     blend, YUV->RGB, to swapchain at display resolution
```

No queue-family EXTERNAL acquire step is needed, on either wgpu version:
`create_texture_from_hal` offers no hook for it, `TextureUses::
UNINITIALIZED` works, and the spike's output is byte-identical to the
copy path's. wgpu 28's `create_texture_from_hal` has no `initial_state`
argument at all (that is wgpu#9496, new in 30), so the layout-discard
hazard is unavoidable there; it stays benign for the same two RADV
reasons, and the byte-identical PNGs are the check.

There is no viable fallback. The copy path measured 45.3 ms/frame of
delivery (18.4 fps) in the M0 spike: it cannot sustain realtime for even
one lens. Zero-copy import is a requirement, not an optimization. (An
earlier research note put `vaDeriveImage` at 0.53 ms/frame; that was the
map call alone, with nothing reading the pixels through it.)

## Trap list (each verified in the 2026-07 study)

- Use descriptor `pitch[]`/`offset[]` verbatim. Chroma pitch is
  `align(width, 512)`: at 3840-wide that is 4096 != 3840, and computed
  strides shear chroma on real footage while passing on 1920/2560 tests.
  The M0 spike saw exactly this on X4 Air footage, plus the other half of
  the trap: luma pitch is 3840, NOT padded. Padding is per-plane, so no
  single computed rule is right for both. (Chroma offset 14745600,
  modifier `0x200000010401b04`.)
- radeonsi exports ONE fd; later planes reference object 0. `dup()` per
  plane use; caller closes every fd. Spike saw `nb_objects` 1 with both
  layers on `object_index` 0.
- Pre-flight the format modifier via
  `vkGetPhysicalDeviceImageFormatProperties2` before image creation;
  an unsupported modifier is UB, not a clean error.
- Do NOT use ffmpeg's `hwcontext_vulkan` (AVVkFrame) route: handing an
  imported frame to a second Vulkan consumer produced
  `VK_ERROR_DEVICE_LOST` on the target box, twice. DRM_PRIME only.
- `av_hwframe_map` with `MAP_READ` calls `vaSyncSurface` first (ffmpeg
  6.1 `hwcontext_vaapi.c:1337`): the map waits for the decode to finish.
  The spike's 7.64 ms "deliver" is mostly this wait with one frame in
  flight; keep 2-3 frames in flight to hide it.
- Reference import code: `ez-ffmpeg` 0.17 `wgpu_filter/hw_interop.rs`,
  `iroh-live` `rusty-codecs/src/render/dmabuf_import.rs`, `bevy-dmabuf`.
- GStreamer was evaluated and rejected: no wgpu or dmabuf-to-Vulkan sink.
- System ffmpeg is 6.1 (Pop!_OS): pin the ffmpeg-next major that matches,
  or vendor a newer ffmpeg; do not assume the 8.x APIs from the research
  notes are present.
- wgpu-hal 28 enables `VK_KHR_external_memory_fd` and
  `VK_EXT_external_memory_dma_buf` whenever the adapter has them, but never
  `VK_EXT_image_drm_format_modifier`, and `iced_wgpu` builds its device from
  a fixed `DeviceDescriptor` with no hook. The `[patch.crates-io]` entry in
  the workspace root manifest is what turns the third one on, for iced's
  device and ours alike; the spike additionally forces it through wgpu-hal's
  `open_with_callback` because it builds its own device. `dmabuf::import`
  still checks `enabled_device_extensions()` and refuses: creating an image
  with a disabled extension's structures is UB, not an error, so the day
  someone drops the patch entry the failure must be loud.
- libcosmic's content container insets the app's view by `border_padding`
  on the right and, because `nav_bar.active` defaults to true even with no
  nav model, by nothing on the left (`app/mod.rs`, `main_content_padding`).
  Measured at scale 1.25: 1 physical px of border left, 10 right. The app
  sets `core.window.content_container = false` (issue #22).
- `iced_renderer` silently drops shader primitives when the tiny-skia
  fallback is chosen (`fallback.rs`: a `log::warn!` and nothing drawn), so
  a blank widget can mean "wrong renderer", not "wrong shader". libcosmic's
  `wgpu` feature is not on by default.
- iced's surface is sRGB when it gamma-corrects, while the spike's offscreen
  target is `Rgba8Unorm`. The same WGSL writes different numbers to the two:
  gamma-encoded video has to be linearised before an sRGB target re-encodes
  it. `TextureFormat::is_srgb()` decides at runtime.

## Projection

Insta360 stores a full Mei/UCM camera model per lens in the trailer
(`offset_v3`): xi, fx/fy, cx/cy, k1-k3, p1/p2, per-lens extrinsics
(~33 mm baseline). The X4 Air fixture: xi = 2.31494. The forward map is
~20 lines of WGSL, written from the Mei/OpenCV-omnidir description in
docs/research/insv-format.md 5.1; Gyroflow's
`distortion_models/insta360.wgsl` (GPL-3.0, AGPL-compatible) remains an
available reference for anything the description does not cover, and a file
that takes it carries an SPDX header. Nothing does today. Static calibrated
warp with a smooth blend; no optical flow (measured to not help). A reframed
view centered near a lens axis contains no seam at all.

`kyerag-meta` turns that string into a `CalibrationSet` whose pixel numbers
are already in delivered-frame coordinates (3840x3840 per lens), not the
15360x7680 side-by-side calibration canvas the file writes them on. The
shader consumes them as they come; nothing downstream rescales.

The rotation is `Rz(roll - 90 deg) * Ry(yaw) * Rx(pitch)`, in a frame whose
axes are the delivered frame's own (x right, y down, z along the optical
axis). That quarter-turn datum was measured against rendered frames from two
cameras, not assumed: applying roll as the file writes it puts the world on
its side. docs/research/insv-format.md 4.8 has the frames and the table.

The forward map exists twice, in `crates/render/src/projection.rs`: once in
WGSL for the GPU and once in Rust so `cargo test` can check known angles
with no GPU and no footage. Both read one `Reframe` uniform block, and the
bind group's `min_binding_size` makes wgpu reject a pipeline whose two
definitions have drifted apart.

Reframing, stabilization, and rolling-shutter correction fuse into ONE
backward mapping per output pixel. No intermediate equirect, ever.

## Clock domains (the correctness minefield)

Video PTS, `first_frame_timestamp` (us), gyro timestamps (us; divide by
1000 twice when `is_raw_gyro`), `gyro_timestamp` offset when
`is_has_gyro_timestamp`, per-lens exposure timestamps, and rolling-shutter
row time (15.9 ms on the X4 Air). Failure mode is a swimming horizon, not
a crash: build the diff-vs-Studio-export harness before trusting any of it.

## Open questions

- Vignetting coefficients are not in the metadata; the seam band may show
  rolloff. Needs flat-field calibration if it bites.
- The **order** of `yaw`, `pitch` and `roll` within the lens pose. Their
  composition is settled (above); the order is not, and no known camera can
  distinguish it, because every one of them records sub-degree yaw and
  pitch (docs/research/insv-format.md 4.8).
