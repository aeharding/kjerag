<p align="center">
  <img src="resources/icons/hicolor/scalable/apps/dev.harding.Kjerag.svg" width="128" height="128" alt="">
</p>

<h1 align="center">Kjerag</h1>

<p align="center">Native 360° video player for the COSMIC desktop, written in Rust.</p>

Kjerag plays Insta360 `.insv` files directly: no stitching step, no proxy
files, no export round-trip. Drag to reframe, scroll to zoom, take
screenshots. The dual-fisheye footage is hardware-decoded and reprojected
on the GPU using the calibrated lens model embedded in every `.insv` file.

**Status: pre-alpha.** See [docs/ROADMAP.md](docs/ROADMAP.md) for where
things stand and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how it
works.

## Why

Insta360 Studio has no Linux build, and nothing on Linux plays raw `.insv`
with calibrated reframing: VLC only handles pre-stitched equirectangular,
and mpv shader hacks have no lens calibration, seam blending, or horizon
lock. The gap is real; this fills it.

## How

An X4-class `.insv` is an MP4 carrying two 3840×3840 HEVC streams (one per
lens) plus a metadata trailer with full per-lens calibration (Mei/UCM
model), raw gyro, and per-frame exposure. Kjerag decodes both streams via
VA-API, imports the frames into wgpu zero-copy (dmabuf), and renders the
reframed view in a single shader pass. Measured on an AMD Phoenix iGPU:
dual-stream decode runs 2.4× realtime at 17% CPU.

## License

AGPL-3.0. GPL-3.0 code from
[Gyroflow](https://github.com/gyroflow/gyroflow) may be used where it helps
(GPL-3.0 is one-way compatible with AGPL-3.0), and any file that takes it
carries its own SPDX header. None does today: the projection math is written
from the published Mei/OpenCV-omnidir description of the model.
