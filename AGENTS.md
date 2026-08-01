# Kjerag — agent session guide

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

The repo is a cargo workspace, one crate per layer: `crates/meta`,
`crates/media`, `crates/render`, `crates/app` (the `kjerag` binary) and
`crates/spike`. `cargo run` still starts the app, because `default-members`
is the app; everything else takes `-p`
(`cargo run --release -p kjerag-spike -- <file.insv>`). `crates/spike` holds
a second binary, `reframe`, which runs the app's own projection pass over
one frame and writes a PNG, so a reframed view can be looked at with no
compositor:
`cargo run --release -p kjerag-spike --bin reframe -- <file.insv> yaw=30 fov=60`

The `[patch.crates-io]` wgpu entry lives in the root manifest, which is the
only place cargo reads one. It pins the fork by `rev`, not by branch, so that
a force-push on the fork cannot break a recorded build (issue #68).

**A change to `Cargo.lock` is a change to `flatpak/cargo-sources.json`.** That
file is the Flatpak build's whole supply of crates, one source per crate,
generated from the lock and committed so the build needs no network
(issue #72). Regenerate and commit it in the same change:

```sh
scripts/cargo-sources.sh
```

A stale one is not a build that fetches what it is missing; it is a build that
fails.

That rule is per commit and the failure is per merge, which is not the same
thing: on 2026-07-31 one branch bumped the ffmpeg pin and another regenerated
the sources, both merged clean because they touch different files, and `main`
then carried a lock file wanting ffmpeg-next 7.1 and a source list offering
6.1.1. So CI checks it, and so can you, with no network and no generator:

```sh
scripts/cargo-sources.sh --check
```

**Two checkouts must not share a `CARGO_TARGET_DIR`.** Pointing a worktree's
build at the main checkout's `target/` to reuse its dependency cache works
until the moment something is built from the other tree: the uplifted
binaries in `target/release/` are one name each, so `target/release/reframe`
silently became the main checkout's while `target/release/zoom` stayed the
worktree's, and an instrument then measured the wrong code and rendered the
wrong picture (issue #47, 2026-07-31). If a comparison against another commit
is what is wanted, build each side into its own target directory, or check
the binary carries the change before believing a number it prints.

`kjerag-meta` depends on no C library and must stay that way:
`cargo test -p kjerag-meta` is expected to pass on a box with no libav
headers, and CI has a job with nothing installed that proves it.

`ffmpeg-sys-next` binds the system ffmpeg headers through bindgen, so a
bare box cannot build the other layers. The root manifest pins ffmpeg 7.1
(issue #65: every freedesktop runtime ships 7 and the Flatpak has no way to
be built against anything else), and Ubuntu 24.04 ships 6.1, so the ffmpeg
half comes from a PPA. On Pop!_OS / Ubuntu 24.04:

```sh
sudo add-apt-repository -y ppa:ubuntuhandbook1/ffmpeg7 && sudo apt install \
  libavcodec-dev libavdevice-dev libavfilter-dev libavformat-dev \
  libavutil-dev libpostproc-dev libswresample-dev libswscale-dev \
  libclang-dev libdrm-dev libwayland-dev libxkbcommon-dev libasound2-dev
```

`libclang-dev` is bindgen's. The three after it are libcosmic's. The last is
cpal's, which the sound goes out through (issue #13): its Linux target links
`alsa` whatever host it ends up using, and PipeWire is what actually plays
what it writes, through `pipewire-alsa`. libcosmic also needs a newer rustc
than Ubuntu ships (`rust-version = "1.93"`); `rustup update stable`.

Nothing on the box loses its ffmpeg to that line. The 6.1 runtime is
libavcodec60/libavutil58 and 7.1's is libavcodec61/libavutil59, separate
packages that sit side by side, and only the `-dev` packages are replaced,
which are one name per library and hold headers.

Without sudo, or to leave the system on 6.1, `scripts/ffmpeg7-local.sh`
unpacks the same PPA .debs under `~/.local` and prints the three variables
that point a build at them; it installs nothing. The third is `RUSTFLAGS`
and it is not optional: alsa-sys puts `-L /usr/lib/x86_64-linux-gnu` on the
linker's command line, the system's 6.1 `libavcodec.so` lives there, and
6.1 and 7.1 export the same names, so a build that picks up the wrong one
links without a word and is wrong only once it runs. What settles which a
binary got is `readelf -d <binary> | grep NEEDED`: ffmpeg 7.1 is
`libavcodec.so.61`, 6.1 is `libavcodec.so.60`.

## Gates (run before pushing)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/name-check.sh
```

The `--workspace` and `--all` are load-bearing: without them cargo only
looks at `default-members`, which is the app crate alone.

The last one is the rename lock (issue #75). The project had another name
until 2026-08-01 and the owner's terms for the sweep were that it exist
nowhere in the tree, in a file or in a path; the script is a grep over what
git tracks and CI runs it as its own job. Git history and the archived
issues and pull requests keep their copies, and nothing is rewritten there.

## UI verification

A change to the window, the keys or the frame path gets one run of the
headless harness before it ships:

```sh
scripts/uitest.sh ~/Videos/<file>.insv   # or set KJERAG_TEST_MEDIA
scripts/uitest.sh                        # no footage: the checks that
                                         # need none, and it says so
KJERAG_FLATPAK=dev.harding.Kjerag \
  scripts/uitest.sh ~/Videos/<file>.insv # the same checks, answered by
                                         # the INSTALLED bundle
```

The third is the release check (docs/RELEASING.md). A binary that plays on
this box says nothing about a bundle that plays inside the sandbox, where
the Mesa, the ffmpeg and the libva are the runtime's and the file arrives
the way flatpak hands one over. 0.1.1 shipped having been installed,
started, and seen to draw a window, and nothing had ever played a frame
in it.

It runs the release binary inside `cage` on a headless wlroots backend,
presses keys with `wtype`, captures the output with `grim`, and reads the
app's own report lines. The session is isolated: its own Wayland socket,
its own home and its own XDG directories, so it neither sees the desktop
you are looking at nor writes anything into it. **That holds in Flatpak
mode too, with the shipped permission set untouched**: flatpak resolves a
by-name grant against the caller's environment, so the session's `HOME`
and `XDG_CONFIG_HOME` decide what `xdg-config/cosmic` and
`~/.local/state/cosmic` mean, and the developer's own `~/.config/cosmic`
is never bound into the sandbox (measured; docs/DISTRIBUTION.md 3.9).
Captures land in gitignored
`scratch/uitest/`, because a frame of real footage is personal video.
Needs `cage wtype grim ffmpeg` installed, plus `wl-clipboard` for the one
check that reads the session's clipboard, which skips without it.

CI does not run it and cannot: decode is VA-API against
`/dev/dri/renderD128` (`crates/media/src/decode.rs`), and with no such
device every file is refused with `av_hwdevice_ctx_create: Input/output
error` (measured), so a GPU-less runner would be checking nothing.

## Releasing

`cargo release patch --execute` on `main`, and that is the whole of it
(issue #106): cargo-release bumps the version, stamps a dated entry into the
metainfo changelog, tags the plain version with no `v`, and pushes. The tag is
what makes `.github/workflows/release.yml` build the Flatpak and publish it as
a GitHub Release. Its config is `release.toml`; its dry run, which is the
default, runs `scripts/uitest.sh`, so the harness above is not skippable on
the way to a tag. docs/RELEASING.md is one page and says the rest.

## Sound etiquette

Any instrument or app run that emits audio (sync, playback, the app
itself outside the harness) goes through `scripts/quiet.sh`, which routes
the stream to a null sink: the owner's speakers are not a test fixture.
Timing and underrun accounting are unaffected (verified; the #49 pop
analysis already measured through a null sink). The one exception: a
measurement whose purpose is real-device latency uses the real sink with
the STREAM volume zeroed, never audible playback, and says so.

## Hard rules

- Branch + PR for all work after the bootstrap commits. Never force-push.
- ALL work stays inside the owner's repositories. Never open, file, or
  comment on issues or pull requests of any outside project, ever. This
  includes "goodwill" bug reports and backport offers to dependencies.
  Forking a dependency into the owner's account for our own use is fine;
  interacting with the upstream project is not. Third-party findings are
  documented in our own docs and issues only. ONE scoped exception
  (owner-granted 2026-07-31): Flathub publishing for this app, done
  together with the owner and with him told before every outward action;
  everything is prepared and previewed in this repo first. Self-hosted
  flatpak distribution is declined by the owner; Flathub is the channel.
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
- Owner-reported defects (owner directive, 2026-07-31): reproduce the
  exact reported symptom through the real pipeline BEFORE writing the
  fix, and confirm through the coordinator that the reproduction matches
  what the owner saw (a rendered artifact or precise steps he can
  compare). A fix merges only with a regression test exercising the path
  the owner actually used, AND only after the owner has tested the fix
  himself - the branch build is what he tests; main gets it after his
  confirmation, never before. No exception for good-looking numbers.
  Born of PR #51's same-day revert (a filter-level unit test validated
  the agent's model of the bug, not the bug) and re-learned on PR #59
  (merged on its measurements before the owner's retest, against his
  explicit instruction).
- Accepted tradeoffs are owner decisions (owner root-cause, 2026-07-31):
  when an agent measures a user-visible compromise and decides to accept
  it (placement, overlap, quality, feel), that acceptance goes in a
  clearly-labeled "Accepted tradeoffs" list at the TOP of the PR body,
  and the coordinator relays each item to the owner as an explicit
  question before the owner is asked to test. Documenting a tradeoff in
  prose is not surfacing it: the toast-over-scrubber call was measured,
  disclosed mid-report, relayed by nobody, and found by the owner.
- Zero-config playback (owner ruling, 2026-07-31): pressing play on any
  file must yield the best available result with no user action, ever. No
  calibration buttons, no setup rituals, nothing the Insta360 app would
  not ask. Automatic background measurement that improves things silently
  is the pattern; a menu item that gates quality is a design failure.
- UI copy: plain words, no em dashes.
- UI design defers to COSMIC system apps best practice (owner doctrine,
  2026-07-31): use libcosmic's stock widgets and the patterns of
  cosmic-files / cosmic-player / cosmic-edit (header bar, standard
  controls, system theming) rather than custom chrome. When in doubt, do
  what a System76 first-party app would do. Spending real time reading
  the cosmic-player / cosmic-files / cosmic-edit sources and the COSMIC
  HIG before building UI is encouraged (owner: "don't be afraid" of that
  time); getting the idiom right beats shipping fast.

## Test media

Real footage lives on Alex's box at `~/Videos/*.insv` (36 GB, Insta360
X4 Air). Its parsed calibration (PII stripped: no serial, GPS, or capture
times) is checked in at docs/research/x4air-calibration.json — use it as
the calibration fixture. Never commit raw trailer dumps: they carry the
camera serial and GPS data.
Headless verification renders to PNG (no window needed); ask Alex for
human-eye testing of anything interactive.
