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
stitching delivery. It merged at `53ecc929` after all six fresh CI checks passed
on final cleanup head `2afef25c`, followed by 50 native X4 UI checks. Tag
`0.3.0` names release commit `c21afcd0`; its dry run and execution each passed
47 real-footage UI checks. All jobs in release workflow `34678413630` passed,
and the GitHub bundles and signed channel are public. Post-publication package
qualification remains open in [issue #187](https://github.com/aeharding/kjerag/issues/187);
publication alone is not a completed qualification verdict.

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

The GitHub x86_64 bundle, whose executable hash begins `9a9241...`, installs and
passes 40 X4 Air and 44 ONE X2 UI checks. Both reported-view PPMs are byte-for-byte
identical to the owner-accepted `48e1d741` package. This verifies the tested
paths and pictures, not the performance target.

At 2256x1504 over 40 seconds with the 300 Hz moving-view input, the published
bundle measured:

| Camera/run | Completed redraws/s | Source advances/s |
| --- | ---: | ---: |
| X4 Air, first | 255.174 | 27.175 |
| X4 Air, repeat | 250.199 | 26.675 |
| ONE X2 | 312.399 | 29.950 |

The X4 runs exceed 240 redraws/s but do not sustain full source cadence. A
restored run of the exact owner-accepted `48e1d741` package likewise measured
254.899 redraws/s and 27.225 source advances/s; a 250 Hz control measured
232.049 and 27.150. The current slowdown is therefore not proven to be a 0.3.0
code regression, but neither the published X4 package nor the restored accepted
package currently qualifies the combined 240-redraw/full-source target.

Earlier `48e1d741` cohorts of 280.499 redraws/s and 29.975 source advances/s on
X4, and 318.024/29.975 on ONE X2, remain valid historical observations. They are
not a current unconditional performance guarantee. The same SDK executable's
228.174 host versus 272.850 Flatpak result remains evidence that environment
contributes to the gap, without identifying a unique cause.

The signed-channel x86_64 binary is a separate rebuild with a different hash; it
separately passes 40 X4 and 44 ONE X2 UI checks, with both reported-view PPMs
byte-identical to the accepted package. Separate capacity is 253.949 redraws/s
and 27.600 source/s on X4, versus 314.724 and 29.975 on ONE X2. Both architectures' app and AppStream
refs are authenticated and present in the signed-channel summaries. Actual
aarch64 installation and playback are untested because this host exposes only
x86_64/i386 clients.
The signed 0.3.0 package omits the license text; an explicit shared-manifest
install is prepared for the next patch package, tracked with #187.

The exact local gate counts, qualified Radeon environment, retained llvmpipe
failure and logs are in MERGE_READINESS. Final cleanup CI passed all six jobs in
run `34677590261`; tag CI passed all jobs in release workflow `34678413630`.

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
  [#186](https://github.com/aeharding/kjerag/issues/186):** current X4 runs show
  shared source-cadence slowdowns across the published and restored accepted
  packages. Host/Flatpak measurements show an environment contribution but do
  not isolate the cause.
- **Frame-time spikes, issue #186:** throughput averages do not retire the 4.17
  ms capacity budget or hitch risk. Do not call playback hitch-free.
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

Feature release **0.3.0** is tagged and published through GitHub and the signed
channel. Complete issue #187's qualification of the separately built signed
x86_64 package and preserve the unresolved performance evidence in issue #186.
The installed app now follows the signed public `kjerag` remote. Its separate
UI qualification passes; the accepted `48e1d741` bundle remains available
for rollback.

The owner has been asked whether to keep the release goal open for performance
work or hand off 0.3.0 with the limitation documented. No answer is recorded, so
do not infer acceptance of the slowdown or closure of #187. The release flow is
recorded in [RELEASING.md](RELEASING.md).
