# Kyerag — agent session guide

Native COSMIC/Rust player for Insta360 `.insv` (dual-fisheye 360) files.
The doctrine: **smooth playback and a correct horizon beat features**.
Performance is a feature; the target is full use of modern hardware
(hardware decode, zero-copy frames, one render pass).

## Read before deciding

- docs/ARCHITECTURE.md — layer ownership, the frame path, the trap list.
- docs/ROADMAP.md — living status doc: milestones, decisions log, next up.
  Update it in any PR that changes project status.
- docs/research/ — the 2026-07-30 feasibility study (format, pipeline,
  landscape). Quote it to settle disputes; it contains measured numbers.

## Building

`ffmpeg-sys-next` binds the system ffmpeg headers through bindgen, so a
bare box cannot build this. On Pop!_OS / Ubuntu 24.04 (ffmpeg 6.1, which
is the version Cargo.toml pins to):

```sh
sudo apt install libavcodec-dev libavdevice-dev libavfilter-dev \
  libavformat-dev libavutil-dev libpostproc-dev libswresample-dev \
  libswscale-dev libclang-dev \
  libdrm-dev libwayland-dev libxkbcommon-dev
```

The last line is libcosmic's. libcosmic also needs a newer rustc than
Ubuntu ships (`rust-version = "1.93"`); `rustup update stable`.

## Gates (run before pushing)

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Hard rules

- Branch + PR for all work after the bootstrap commits. Never force-push.
- ALL work stays inside the owner's repositories. Never open, file, or
  comment on issues or pull requests of any outside project, ever. This
  includes "goodwill" bug reports and backport offers to dependencies.
  Forking a dependency into the owner's account for our own use is fine;
  interacting with the upstream project is not. Third-party findings are
  documented in our own docs and issues only.
- Subagents may write and commit directly on working branches (explicitly
  authorized by the owner, 2026-07-30); main changes only land via PR.
- Work queue is GitHub issues. Claim an issue by commenting; close via PR.
- License is AGPL-3.0. GPL-3.0 code (e.g. Gyroflow's Insta360 WGSL) MAY be
  used with attribution and an SPDX header on the file. Code under
  licenses incompatible with AGPL may NOT.
- dmabuf plumbing: use `AVDRMFrameDescriptor` layer `pitch`/`offset`
  values verbatim; never compute them. Chroma pitch at 3840-wide video is
  4096, not 3840 — computed strides shear chroma only on real footage.
- The camera's LRV proxy may not exist. Full-res decode stands alone;
  proxies are generated, never assumed.
- Simplest design first: smallest truthful version, owner-readable.
  Complexity only from observed failures.
- Pushing back is welcome (owner's standing request): if the existing
  structure makes your task awkward, or a refactor would make the code
  easier to work with, say so in your PR or report instead of contorting
  around it. Proposals with reasons get taken seriously; the plan pivots.
- UI copy: plain words, no em dashes.

## Test media

Real footage lives on Alex's box at `~/Videos/*.insv` (36 GB, Insta360
X4 Air). Its parsed calibration (PII stripped: no serial, GPS, or capture
times) is checked in at docs/research/x4air-calibration.json — use it as
the calibration fixture. Never commit raw trailer dumps: they carry the
camera serial and GPS data.
Headless verification renders to PNG (no window needed); ask Alex for
human-eye testing of anything interactive.
