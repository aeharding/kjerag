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
Normal branch-player playback, owner-visible review, both established camera
regressions and final CI remain separate qualification gates. A mapping that
looks upright in stills must not be called stable during motion without that
check.

Private artifacts are under `scratch/x3-horizon-20260915/`: source/binary
manifests, separate bounded-run receipts, original PNGs and `RESULT.md`.
No footage, raw trailer, serial, GPS or capture metadata is checked in here.
The headless processes ran separately with desktop and sound access hidden;
post-exit GPU-memory counters matched preflight and no new kernel entries
appeared. This is not a broad driver-safety or performance qualification.
