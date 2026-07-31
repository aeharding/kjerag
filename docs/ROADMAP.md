# Roadmap (living doc)

Update this file in any PR that changes project status. Work queue is
GitHub issues; this doc is the map, issues are the tasks.

**Status 2026-07-30:** feasibility study complete (docs/research/), repo
bootstrapped, M0 frame path proven end to end on real footage.
`cargo run --release --bin spike -- <file.insv>` decodes one 3840x3840 lens
on VA-API, imports the dmabuf planes into wgpu with no copy, and renders to
PNG at 103 fps (3.4x realtime). Next: the shell decision, then dual decode
and the Mei reprojection.

## Milestones

- **M0 Pipeline proof** — decode one lens via VA-API, import into wgpu
  zero-copy, render headless to PNG with timings. Done (`src/bin/spike.rs`,
  issue #6). The shell question (libcosmic/wgpu28 vs winit/wgpu30) is now a
  decision rather than an unknown: see the decisions log.
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
  planes as they come, 0.12 ms/frame for both. Open for Alex: this is a
  point for winit + wgpu 30 over libcosmic's wgpu 28, where the same import
  has to be written by hand.

## Measured on the target box (AMD Phoenix, RADV, 3840x3840 HEVC)

Per frame, 300 frames, one lens, one frame in flight (`src/bin/spike.rs`):

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
