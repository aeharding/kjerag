# Roadmap

> **History:** the complete roadmap and experiment log through 2026-09-12 is
> preserved verbatim in
> [ROADMAP-HISTORY-20260912.md](ROADMAP-HISTORY-20260912.md). Date-based links
> from older research and source comments refer to that record.

This is the current product and delivery map. GitHub issues remain the work
queue. Update this page in any PR that changes product status, qualification or
release state.

## Product direction

Kjerag is a native COSMIC/Rust player for 360 camera files. The product priority
is smooth, zero-config playback with a correct horizon and Studio-like visible
stitching. Exact numerical reproduction of Insta360 Studio is useful evidence,
not the shipping requirement.

The selected design is one shared GPU stitcher with camera-specific calibration
and geometry. Qualified ONE X2 and X4 Air routes use hardware decode, GPU-resident
source preparation, stitching at source cadence, and reuse of the completed
stitch result across view redraws. The detailed ownership and frame path are in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Current delivery

PR [#183](https://github.com/aeharding/kjerag/pull/183) is the cumulative GPU
stitching delivery. At preparation checkpoint `fc0e6775`, it is ready for review,
non-draft, and all six fresh CI checks pass. No merge, version tag or public
release has been performed yet.

The owner tested the installed Flatpak built from source
`48e1d741f1bf7e7d8882624dff87754227bb8c7d` and accepted the actual branch
player: "looks good. not perfect but pretty damn good." That acceptance covers
the reviewed installed result, including source-owned world-coordinate temporal
correction. It does not establish exact Studio parity, all-camera coverage or
hitch-free playback.

The delivery includes:

- automatic GPU stitching and lens color matching for the qualified ONE X2 and
  X4 Air routes;
- source-owned alignment, temporal correction and photometric controls, paired
  exactly with the source frame that produced them;
- horizontally periodic temporal processing and rotation-stabilized world
  coordinates independent of mouse direction, window size and horizon display;
- first-picture preparation and responsive seeks, with completed GPU work and
  decoder resources retired by completion rather than by submission;
- readable reference implementations, real-path regressions and retained
  research evidence.

The exact package identities, accepted tradeoffs, test qualifications, evidence
paths and release review are recorded in
[MERGE_READINESS.md](MERGE_READINESS.md). That page is the authority for claims
about this delivery; private captures and receipts remain in ignored `scratch/`
paths and are not release assets.

## Qualification summary

The owner-tested installed package passed 40 X4 Air and 44 ONE X2 UI checks,
including the reported views, pause, backward seek and actual scrubber paths.
At 2256x1504, 40-second installed-player cohorts measured:

| Camera | Completed redraws/s | Source advances/s | Completion p99 / max |
| --- | ---: | ---: | ---: |
| X4 Air | 280.499 | 29.975 | 13.813 / 23.463 ms |
| ONE X2 | 318.024 | 29.975 | 9.729 / 23.788 ms |

Both cohorts advanced 1,199 consecutive sources with no reported drops,
starvation or audio underruns. These are rendering-capacity measurements during
playback, not 240 distinct decoded frames or 240 Hz physical presentation.

The standard local gates pass on Rust 1.97.1: formatting, workspace/all-target
Clippy, 1,496 workspace tests with 52 explicitly ignored, source consistency,
rename, harness-startup, controls-log, AppStream and metadata-without-libav
checks. The required-Radeon rerun with hardware access passes; the retained first
run selected llvmpipe and failed 38 GPU checks. See MERGE_READINESS for the
qualified command environment and logs.
The final code/documentation cleanup passes the same complete local gate set;
its logs are `scratch/merge-readiness-20260912/cleanup-final-01/`. Its new head
still requires fresh CI before merge.

## Accepted tradeoffs and boundaries

The owner-approved tradeoffs are listed at the top of
[MERGE_READINESS.md](MERGE_READINESS.md#accepted-tradeoffs). In brief, the
accepted installed result can differ from readable CPU/global references through
reduced-resolution temporal correction, source-rate prefiltering, independent
patch-row alignment, GPU sampling and rounding, and history restart after seek.
First-picture preparation adds roughly 0.1 seconds to avoid a later activation
pause.

Frame-time spikes and sustained source lag are limitations, not accepted
performance tradeoffs. No seam-refresh cadence reduction or invented gradual
color-update policy is selected.

## Remaining work

- **Native/SDK performance gap, issue
  [#186](https://github.com/aeharding/kjerag/issues/186):** the installed X4 Air cohort exceeds the 240
  redraw/s capacity target, while the native result remains below it at 217.949
  redraws/s and 28.575 source advances/s. The runtime gap is unexplained.
- **Frame-time spikes:** installed throughput clears the average capacity target,
  but p99 and maximum completion spacing remain well above the 4.17 ms capacity
  budget. Do not call playback hitch-free.
- **Exact photometric parity, issue
  [#185](https://github.com/aeharding/kjerag/issues/185):** automatic lens color
  matching is implemented and visually reviewed. An isolated proof of exact
  Studio Image Fusion behavior is not complete.
- **Coverage, issue
  [#88](https://github.com/aeharding/kjerag/issues/88):** qualification covers
  the named ONE X2 and X4 Air footage and tested AMD Radeon 760M Flatpak setup,
  not every camera, recording mode, bit depth, architecture or GPU. aarch64 is
  compiled and unit-tested in CI, not playback-tested on hardware.

## Delivery next steps

The next feature release is **0.3.0**. Keep the accepted runtime intact through
merge. After PR #183 is merged, follow [RELEASING.md](RELEASING.md): run the
documented cargo-release dry run on clean `main` with real footage, execute the
minor release, then qualify the resulting x86_64 release bundle inside its
sandbox on both cameras. The branch-package qualification does not replace the
post-tag release-bundle check.

Do not claim the release complete until the tag-driven GitHub Release and signed
channel publication have succeeded and the resulting bundle has passed that
qualification.
