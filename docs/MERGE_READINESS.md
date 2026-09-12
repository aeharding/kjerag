# GPU stitching delivery: merge and release review

Current checkpoint: 2026-09-12. PR #183 targets `main`; no merge or release has
been performed. This page summarizes the cumulative delivery without requiring
reviewers to reconstruct the chronological experiment log in ROADMAP.

The active owner goal includes cleanup, merge and release. The living roadmap
is now separated from its verbatim [historical record](ROADMAP-HISTORY-20260912.md),
with a [research navigation index](research/README.md). Known native/Flatpak
performance differences are tracked in [issue #186](https://github.com/aeharding/kjerag/issues/186).

## Accepted tradeoffs

- The owner accepts the installed branch player: "looks good. not perfect but
  pretty damn good" (2026-09-12). This is Studio-like output on reviewed footage,
  not exact Studio parity or universal camera/mode coverage.
- Reduced-resolution temporal correction and source-rate prefiltering can
  change noise, softness and moving-edge detail. The owner accepted the moving
  comparisons and subsequently the installed player. Full-resolution source
  detail remains the base picture; readable full-resolution references remain.
- Opening/seeking prepares the first picture before starting playback. The
  owner explicitly accepted roughly 0.1 seconds of additional preparation to
  avoid the measured filter-activation pause.
- Seeking restarts stitching history. Initial pictures can differ slightly
  from uninterrupted playback; drag previews are keyframes, release is exact.
- Independent 16-patch-row alignment regions, hardware lens sampling and GPU
  interpolation/threshold rounding can differ from the readable global/CPU
  references. These were disclosed before branch testing; acceptance of the
  installed result does not establish identical internal arithmetic.

Frame-time spikes and sustained source lag are limitations, not accepted
performance tradeoffs. No reduced seam-refresh cadence or invented gradual
color-update policy is selected.

## What is being merged

- One automatic GPU-resident stitcher for the qualified ONE X2 and X4 Air
  calibration routes, with source-owned alignment maps and photometric ratios.
- Bounded source preparation, exact source/map/color pairing, asynchronous
  readiness notification and completion-based decoder/GPU resource retirement.
  A busy draw retains the prior complete picture and controls together; that
  fail-safe is not permission for arbitrary control lag or playback stalls.
- GPU temporal correction at source cadence, horizontally periodic sampling,
  and rotation-stabilized world coordinates independent of mouse direction,
  window size and the display's horizon setting. Redraws reuse completed work.
- Responsive direct seeks and first-picture preparation; the local iced
  named-child reconciliation correction preserves Scene across controls
  appearing/disappearing. The known separate COSMIC issue #149 is not claimed
  fixed by this work.
- Local, pinned iced device-limit/presentation patches with their original MIT
  licenses and provenance. Readable reference implementations, regression tests
  and research records are retained; personal video evidence stays untracked.

The branch is based on main `67c7fea25ec57c0eb6e8d6d63940e9d63303dd67`, verified
against the remote on 2026-09-12. There are no main-only commits to integrate.
Keep the tested runtime intact rather than reorder its dependent implementation
commits. Preparation changes documentation, store description and ignores for
local build/Python caches, not player code, shaders or dependency versions.

The subsequent cleanup removes unreachable resident vertex-cache bridges and
restricts that cache's module, pipeline state and shader builders to tests.
Its full-field reference and comparison consumers remain. No selected shader,
filter weight, source cadence, correction chart or scheduling rule changes;
this reduces unused production surface, not a claimed speedup. Obsolete module
comments are corrected to describe the selected GPU/temporal path.

## Exact owner-tested package

- Runtime source: `48e1d741f1bf7e7d8882624dff87754227bb8c7d`.
- Installed OSTree: `e91cb1142e034257e56d72714b34af79fe0e0f484a572d6ff31848dd08fdca04`.
- Executable SHA256: `6e5f5c2e410b2a604f543fae50ca0247ce2a1a62a9259c0c5ce498d65451b44d`.
- Bundle SHA256: `655a0aa11e8485af3268616bdcb6eb9fb131907b59b19b14a5eb9717c0524956`.

Both-camera installed UI qualification passes: 40 X4 and 44 ONE X2 checks,
including exact reported views, pause, backward seek and the actual scrubber.
Package identities are unchanged after qualification. Isolated sandbox sound
device/volume and preload/import-fault cases remain skipped; the shader check
uses the native Rust twin. Separate capacity runs have active null-sink audio.
Permissions, runtime and origin are unchanged. The prior `dad5d709` verified
bundle is retained for rollback.

At 2256x1504, 40-second installed-player moving-view cohorts measure:

| Camera | Completed redraws/s | Source advances/s | Completion spacing p99 / max |
| --- | ---: | ---: | ---: |
| X4 Air | 280.499 | 29.975 | 13.813 / 23.463 ms |
| ONE X2 | 318.024 | 29.975 | 9.729 / 23.788 ms |

Both have 1,199 consecutive source advances and no reported drops, starvation
or audio underruns. These are completed rendering-capacity measurements, not
240 unique decoded frames or physical scanout on the 60 Hz panel. Native X4
remains below target (217.949 redraws/s, 28.575 source advances/s); the native/SDK
gap remains unexplained. Do not substitute the native result for the tested
Flatpak, or use the Flatpak result to promise every native build's performance.

Local evidence: `scratch/flatpak-delivery-48e1d741/`,
`scratch/installed-capacity/world1147-installed-candidate-01`,
`scratch/installed-capacity/world-x2-installed-candidate-01`, and
`scratch/chromatic1147/`. Captures and receipts are not release assets.

## Merge gates

Prior exact-runtime qualification passes 1,496 release-workspace tests with 52
explicitly ignored, release workspace/all-target Clippy, formatting, source-lock
and rename checks. Source-policy, actual-shader and real-Scene tests cover
source pose ownership, camera/aspect/horizon changes, history wrap and seeking.
All 31 temporal-off controls and 62 full/half reference pictures remain exact.

The standard non-release local gates pass on Rust 1.97.1: formatting,
workspace/all-target Clippy, **1,496 workspace tests with 52 ignored**, source
consistency, rename, five harness-startup checks, 14 controls-log tests,
AppStream validation and the metadata tests with libav discovery disabled.
Logs are in `scratch/merge-readiness-20260912/radeon-01/`.

The final cleanup tree passes the same complete gate set: 1,496 workspace tests,
52 explicitly ignored, and every ancillary check above. Its separate logs are
`scratch/merge-readiness-20260912/cleanup-final-01/`. This includes the test-only
vertex-cache restriction and removal of its unreachable production bridges.

The first restricted-sandbox test run selected llvmpipe software rendering and
failed 38 GPU checks. Its log is retained, not erased or called a pass. The
successful rerun requires Radeon Vulkan explicitly (`KJERAG_REQUIRE_GPU=1`,
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/radeon_icd.json`, `WGPU_BACKEND=vulkan`)
with serial tests. No runtime code or assertions were changed. Existing
vendored dependency warnings remain; workspace Clippy exits successfully.
Both-architecture GitHub CI passes on preparation head `fc0e6775` in run
`34676341245`, all six jobs green. Later cleanup heads require their own CI.
Release uses the existing hardware-independent CI policy; software-GPU
arithmetic compatibility is not established by the Radeon qualification.

## Remaining scope and release plan

- Camera modes, higher-bit-depth snapshots and GPUs beyond the qualified
  footage/hardware are not newly certified. X4 model-6 projection still uses
  the older calibration's image-circle/alpha support at the coverage boundary.
- Existing DJI Osmo 360 playback is unchanged and not requalified here. Its
  `.osv` command-line/drop path is distinct from the `.insv` chooser and shared
  Insta360 engine; the store/README now acknowledge that existing support.
- Automatic lens color matching is implemented and visually reviewed. Exact
  isolated Studio photometric parity is not claimed; issue #185 remains open
  rather than treating all of its stronger criteria as closed by this delivery.
- aarch64 is compiled/unit-tested in CI, not playback-tested on hardware.
- Current tests bound the reported defects; they do not prove absence of every
  hitch, seam artifact, noisy patch or moving-detail difference.

The active owner goal now explicitly includes cleanup, merge and release.
The next feature release is **0.3.0**. Do not bump the version or create a tag
in the preparation PR. After final cleanup gates,
merge through PR, run the documented cargo-release dry run on clean main with
real footage, then execute the minor release. The tag starts public bundle and
signed-channel publication. Qualify the resulting x86_64 release bundle inside
its sandbox on both cameras; a branch-package pass does not replace that step.
See [RELEASING.md](RELEASING.md) for the existing release mechanism.

Suggested release notes:

- Automatic GPU stitching and lens color matching for ONE X2 and X4 Air.
- Reduced flickering seam artifacts, including rotating-camera and panoramic
  wrap-boundary cases.
- Reuse completed stitch results while reframing; prepare the first picture
  before playback and seeking, and retain video state when controls appear.
- Verified full-rate video and over 240 redraws/sec of rendering capacity on
  the tested AMD Radeon 760M Flatpak setup. Performance varies by hardware,
  runtime and footage; stitching remains Studio-like rather than identical.
