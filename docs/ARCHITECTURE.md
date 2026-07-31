# Architecture

## Layers

```
app      shell + input (drag = yaw/pitch, scroll = FOV zoom, timeline)
render   wgpu: dmabuf import, one WGSL pass (NV12 -> RGB + Mei reprojection
         + seam blend), offscreen render for screenshots
media    ffmpeg demux, dual VA-API HEVC decoders, frame clock, keyframe
         index, seek. No UI dependencies.
meta     .insv trailer via telemetry-parser: per-lens Mei calibration,
         gyro track, per-frame exposure. No UI dependencies.
```

`media` and `meta` know nothing about the shell. The shell decision
(libcosmic vs winit + wgpu 30) is open until milestone M0 resolves it with
data; everything below the shell must not care.

## The frame path (zero-copy)

```
VA-API decode (two 3840x3840 HEVC streams, one demuxer)
  -> av_hwframe_map(dst = AV_PIX_FMT_DRM_PRIME, MAP_READ | MAP_DIRECT)
  -> AVDRMFrameDescriptor { fd, offset, pitch, format_modifier } per layer
  -> two single-plane wgpu textures per frame:
       R8Unorm  from layer 0 (luma,   DRM_FORMAT_R8)
       Rg8Unorm from layer 1 (chroma, DRM_FORMAT_GR88 - note GR, not RG)
  -> queue-family EXTERNAL acquire (no-op on RADV)
  -> single fragment pass: sample both lenses, Mei inverse-project,
     blend, YUV->RGB, to swapchain at display resolution
```

Fallback if the import fights back: `vaDeriveImage` mapping (measured
0.53 ms/frame at 4K; use `hwmap=mode=read+direct`, never `hwdownload`).

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
- The synchronous map stalls the GPU: keep 2-3 frames in flight.
- Reference import code: `ez-ffmpeg` 0.17 `wgpu_filter/hw_interop.rs`,
  `iroh-live` `rusty-codecs/src/render/dmabuf_import.rs`, `bevy-dmabuf`.
- GStreamer was evaluated and rejected: no wgpu or dmabuf-to-Vulkan sink.
- System ffmpeg is 6.1 (Pop!_OS): pin the ffmpeg-next major that matches,
  or vendor a newer ffmpeg; do not assume the 8.x APIs from the research
  notes are present.

## Projection

Insta360 stores a full Mei/UCM camera model per lens in the trailer
(`offset_v3`): xi, fx/fy, cx/cy, k1-k3, p1/p2, per-lens extrinsics
(~33 mm baseline). The X4 Air fixture: xi = 2.31494. The forward map is
~20 lines of WGSL; Gyroflow's `distortion_models/insta360.wgsl` (GPL-3.0,
AGPL-compatible) is the reference implementation. Static calibrated warp
with a smooth blend; no optical flow (measured to not help). A reframed
view centered near a lens axis contains no seam at all.

Reframing, stabilization, and rolling-shutter correction fuse into ONE
backward mapping per output pixel. No intermediate equirect, ever.

## Clock domains (the correctness minefield)

Video PTS, `first_frame_timestamp` (us), gyro timestamps (us; divide by
1000 twice when `is_raw_gyro`), `gyro_timestamp` offset when
`is_has_gyro_timestamp`, per-lens exposure timestamps, and rolling-shutter
row time (15.9 ms on the X4 Air). Failure mode is a swimming horizon, not
a crash: build the diff-vs-Studio-export harness before trusting any of it.

## Open questions

- Shell: libcosmic pins wgpu 28 (hand-rolled ash import, ~120 unsafe
  lines) vs winit + wgpu 30 (`texture_from_dmabuf_fd` exists). M0 decides.
- Vignetting coefficients are not in the metadata; the seam band may show
  rolloff. Needs flat-field calibration if it bites.
- The `roll ~= 90` field reading (vs `half_fov`) is unverified against a
  rendered frame; verify during the first shader bring-up.
