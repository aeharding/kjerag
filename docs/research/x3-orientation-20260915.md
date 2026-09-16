# X3 orientation check, 2026-09-15

Issue [#88](https://github.com/aeharding/kjerag/issues/88) tracks this camera's
unqualified coverage. The proposed IMU convention is `xzy`, in Kjerag's
`body_from_imu` composition, not a convention copied from another project.
This changes neither the shared stitcher nor the generic lens projection.

## Reproduction and method

The local two-file X3 sample identifies firmware `v1.0.86_build1`, two
2880-square lenses, and 29.970 fps. Current automatic selection falls through
to X4's `xZY`. Both the installed player and a separate decoder-wakeup
candidate show a visibly inverted scene during playback. That observation
predates this calibration branch and is unrelated to the decoder-wakeup change.

The exact headless reproduction uses the ordinary Scene projection, factory
calibration, yaw 0, pitch 0, fov 90, and frame 1798 (PTS 59.993 seconds).
Current `xZY` puts the rider and ground above the sky. At frame 60 (PTS 2.002
seconds), current lock instead points toward the sky; do not describe that
earlier frame as the same inversion. The unlocked opening view shows an
upright helmet from below.

The existing `horizon` instrument's six-frame unlocked skyline sweeps at
2 and 60 seconds find no usable horizon, so they provide no axis ranking.
Instead, its existing explicit `axes=` variants render all 24 proper signed
permutations at the opening frame. Visual inspection of the original pixels
shortlists two mappings using poles, buildings, clothing and ground direction.
The same two mappings are then rendered at the later, different camera pose:

| Convention | Opening frame 60 | Later frame 1798 |
| --- | --- | --- |
| Current `xZY` | Zero-pitch view points toward the sky | Rider and ground inverted |
| `xzy` | Upright rider and structural verticals | Upright path and surroundings |
| `yZx` | Plausibly upright, different heading | Zero-pitch view points at the ground |

ONE X2's `Zxy` was also evaluated, not assumed transferable. It looks upright
at the later frame but leaves the opening picture steeply tilted. It is not
the selected X3 candidate. The other explicit candidates and all original
images remain in the private evidence, including rejected views.

The two posed comparisons support `xzy`; one attractive still alone would not.
The case spelling is significant: lowercase negates an axis. `xzy` remains a
proper rotation, not a reflection, and differs from `xZY` by a half turn in
the sensor-axis composition.

## Boundaries and remaining qualification

These are proposed calibration evidence, not a published camera-support claim.
They do not establish exact Studio stabilization, all X3 firmware/modes,
rolling-shutter direction, or admission to the selected resident stitcher.
X3 remains on generic factory projection with its existing unknown readout.
ONE X2, X4 and X5 selection is unchanged.

The existing gyro tool's quietest ten-second interval has a mean acceleration
magnitude of 0.9675 g. That does not meet the earlier X2 near-1g stationary
reference criterion and is not presented as such. Its exposure/container
comparison measures at most 2.884 ms disagreement and 0.38 degrees of solved
orientation difference. That particular clock difference does not account for
the large observed turn; this does not prove every timing assumption correct.

The model-selection regression fails on the old fallback and passes with the
new entry. Full device-hidden workspace gates report 1,520 passes, 52 ignored
and no failures; unavailable-GPU/media returns are included. Formatting,
workspace Clippy, vendor warnings, naming and Cargo-source checks also pass.
These are portable checks, not real-footage rendering tests.
All six CI jobs pass on candidate `5fc99fb2`. The normal native X3 player
shows upright, moving footage in the captured opening sequence. Its UI suite
finishes with 47 total checks: 46 pass and one fails. The failed clipboard
round trip restores the same view text but changes picture bytes slightly in
the lower part of the view. The source-frame index was not recorded, and the
cause was not established in that run. This was not a fully passing UI
qualification; the test was not weakened. Portal and cross-mount paired-file
checks also skip.
The native shader/Rust twin passes on the Radeon GPU.

Owner-visible review, both established-camera UI regressions and cumulative
Flatpak qualification remain separate gates. The opening captures do not
establish stable horizon behavior throughout the recording.

Private artifacts are under `scratch/x3-horizon-20260915/`: source/binary
manifests, separate bounded-run receipts, original PNGs and `RESULT.md`.
No footage, raw trailer, serial, GPS or capture metadata is checked in here.
The earlier headless processes ran separately with desktop and sound access hidden;
post-exit GPU-memory counters matched preflight and no new kernel entries
appeared. This is not a broad driver-safety or performance qualification.
The later native UI run used a private compositor and null audio, but a host
monitor/dock hotplug coincided with an AMD display timeout and warning in the
desktop compositor's display-commit path. Host GPU-memory counters changed.
That run does not establish unchanged host health or a player leak. Further
GPU qualification was paused pending health review. The owner subsequently
confirmed that the desktop was normal and resumed bounded playback checks.
The installed app still excludes this candidate.

## Copied-view mismatch isolated, September 16

A separate diagnostic drives the real generic Scene through consecutive source
frames, a seek away and a return to the original source. Each normal draw
authenticates offered/displayed opaque frame identity. Both target deliveries
have index 238 and exact PTS 7.941266666 seconds; their opaque identities differ
because they come from different decode epochs. Normal before/after images
reproduce **both** retained native UI captures byte-for-byte throughout the
original control-free test area. This does not retroactively recover the old
UI run's unrecorded delivery history.

Holding the generic chromatic field neutral removes the broad difference while
leaving a smaller seam-anchor restart component. The actual applied normal
fields differ, and the plain shader consumes that field but not the legacy
pooled tone/disparity measurements. Of 716,800 test-area pixels, the normal
round trip changes 108,295; the neutral-field control changes 6,442 near the
bottom right. There are 102,444 normal changes where the control is identical,
with a maximum difference of five RGB8 codes. These are causal diagnostics,
not a visual-quality threshold or owner acceptance.

The check passed in one bounded hardware process. No new kernel messages or
scoped memory-limit events appeared, and post-exit GPU heap counters matched
preflight. Host memory pressure varied; this is not driver containment or
performance qualification. The test-only source is retained in local commit
`256cdb16`, with private receipts and authenticated outputs in
`scratch/x3-seek-history-20260916/`. No player arithmetic was changed.

The navigation test must therefore distinguish exact source/view restoration
from history-dependent rendering. The branch harness correction keeps exact
source index/time and camera/horizon checks, authenticates the currently shown
delivery, rejects a retained away picture and requires a stable paused result.
Seek-history pixel differences remain recorded for review, not hidden by a
tolerance or mask. Non-seek pixel-equality checks remain unchanged. This work
addresses only the copied-view portion of issue #170; its separate toast and
sound observations remain open. The integrated device-hidden workspace reports
1,585 passes, 53 ignored and no failures, including unavailable GPU/media
returns rather than additional hardware coverage. Formatting, full Clippy,
vendor warnings, naming and dependency-source checks pass. Nine portable
copied-view contract tests cover positive and negative controls; the existing
playback, startup, controls-wake and isolated-runner tests also pass. Fresh
candidate UI qualification is recorded below; owner review remains pending.

## Integrated native qualification

Frozen source `e9ecc01a` passes separate native UI suites: 53 X3, 53 X4 Air
and 54 ONE X2 checks, with zero failures. Each suite runs alone with resource
bounds, a private compositor and null audio. App, helper and source identities
are verified before and after. All three copied/printed-view round trips pass
per camera, along with playback, pause/resume, real input routing and import
failure recovery. Portal, cross-mount paired-file and explicit-view fixture
checks retain their documented skips. The native Radeon shader/Rust twin also
passes; it does not authenticate a Flatpak shader build.

The initial X3 round trip names source 237 at exactly 7.9079 seconds on both
sides with current delivery identity; it reports different history pixels and
a stable returned picture. The corresponding X4 and ONE X2 checks report
identical pixels as well as exact source/view restoration. Both motion captures
from each camera were inspected, as were retained round-trip pictures. The X3
main-loop captures were overwritten by the later spaced-path fixture's reused
names; its live verdict and source receipts remain, but the final files show
the source-zero spaced-path check. Subsequent artifact-prefix cleanup prevents
this overwrite without changing navigation assertions. X4 and ONE X2 main-loop
captures were separately preserved before the later fixture ran.

No new kernel messages or scoped runtime memory-limit events appeared. GPU
heap and host-memory counters varied, so this is not a driver-safety or leak
verdict. These 1280x720 functional checks do not establish throughput, the
4.17 ms frame-time target, all-camera support or whole-recording horizon
stability. The native candidate is offered for owner review. Final-head CI and
cumulative Flatpak qualification remain separate; no X3 merge or installation
has occurred. Private receipts are in `scratch/x3-view-qualification-20260916/`.
