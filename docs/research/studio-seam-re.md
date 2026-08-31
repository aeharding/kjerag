# Studio-derived ONE X2 stitching: shipping evidence summary

**Status, 2026-08-31:** this is the compact evidence boundary carried by the
shipping branch. It is not the complete reverse-engineering ledger. The full
forensic record, instruments, recovered call paths and negative results remain
at commit `672f9f4f17e1633c29945e49c73279ff19e8990d`.

This summary records what the selected player implements, what differs from
Studio, which video boundary the owner has judged, and what remains open. It
must be read with `MANDATES.md`. In particular, computed traces and hashes do
not replace the owner's eye, and a verdict on one build or interval does not
transfer to another build automatically.

## Recovered semantic boundary

An ordinary open of a supported ONE X2 automatically selects the causal
Studio-derived route. It does not ask the pilot to calibrate anything and it
does not depend on the legacy Optical Flow setting.

The production transaction is:

1. Decode the two physical lens files as one aligned pair.
2. Bind that delivery to an opaque `FrameStamp`.
3. Read the two luma planes while retaining the decoded render sources.
4. Construct the recovered camera masks, parent maps and source belts.
5. Start with the recovered cold transition at frame zero, then consume every
   adjacent pair through the recovered warm state.
6. Materialize the native 200 by 100 packed type-2 map and copied-pole alpha.
7. Draw the exact decoded pair and exact map through the direct type-2
   consumer only when their full `FrameStamp` values match.

The player uses `PresentationPolicy::EveryFrame` for this route. A
discontinuous seek creates a fresh owner and causally replays from frame zero
to the requested target. Reusing only an index and timestamp after a seek or
reopen is insufficient because the opaque delivery identity changes.

The selected route does not use the removed per-file seam fit, saved seam
pool, or supplied-map app inspector. `seam=factory` is the parity base. The
permanent `playback` instrument records consecutive production pictures,
packed maps and alpha from the real causal path. `range-trace` consumes those
authenticated leaves without rerunning `FrameOwner` or rerendering the base
PNG, decodes the exact retained source pair, binds the saved map to that fresh
delivery identity, and paints the computed selected-alpha crossing.

## Owner-approved orientation substitution

The selected ONE X2 basis, calibration packing, 51-pose schedule and Metal
parent-map law are READ from Studio. Kjerag uses its own orientation track as
the pose provider in place of Studio's unrecovered `PrecomputeStabilization`
pose-cache producer; the owner approved that internal substitution. This is
an implementation difference, not recovered Studio provider semantics.

The masks, cold and warm estimator, map materialization, alpha and type-2
consumer implement the recovered semantics around that boundary. The
substitution is disclosed because it can move output coordinates. It is not a
Studio-provider parity claim, and a whole-view stabilization difference
remains visible in ordinary Studio versus Kjerag review material.

## Ordinary Studio oracle and exact target alignment

The exact source pair is:

| Physical lane | File | SHA-256 |
| --- | --- | --- |
| `_00_` | `VID_20251018_191318_00_002.insv` | `d690e7889345d8d8441fbc4fdb9f4ad3cf170003c01811b91d297ae6fe6545ef` |
| `_10_` | `VID_20251018_191318_10_002.insv` | `fd9bb5865576a76a5c8e52a7536e7acdcefd90b2224f5065488f5d0001f09ca6` |

`docs/research/studio-video-oracle-602.json` seals those inputs, both ordinary
Studio 6.0.2 Flow On/Off exports, their project files, media/sample grids and
the bounded target-picture authentication.

The ordinary Studio Flow On export is a complete stitched-sphere video at the
source's exact `30000/1001` frame rate. Adjacent-frame decoys authenticate the
reported target as:

```text
Studio output frame 6369 at 212.512300 seconds
    -> source frame 6369 at 212.512300 seconds
```

Frame 6369 uniquely wins the held-out adjacent-source tests described in the
full ledger. This directly authenticates that one target picture. Matching
rational grids support same-index pairing elsewhere, but do not directly
authenticate source/output identity for every Studio sample. The oracle does
not expose Studio's internal maps, parent-provider state, seek reset behavior
or a per-frame stabilization transform.

The reported Kjerag view is `yaw=71.13`, `pitch=-13.99`, `fov=57.95`, locked
horizon, file readout, Sharp sampling and factory calibration at 3840 by 2160.

## Causal production and computed trace

The accepted review interval is frames 6339 through 6399, 61 consecutive
frames around the target. Its causal producer began at frame zero, sought
nowhere, consumed every pair through frame 6399 and retained each interval
picture, packed map and alpha. The trace consumer authenticated both source
files, every leaf, the exact view, frame index, timestamp, map binding,
runtime checkout and executable before publishing its no-replace output.

The last archived optimized evidence build is
`5b325d885d716073fdbc4c71e93b3f7e41ac0b06`. Its evidence identifiers are:

| Artifact | SHA-256 |
| --- | --- |
| Causal frames 0 through 6399 receipt | `8d0276cc9ffd55b0c3e697abea42dc7dabefeac97109efa0e755dd7bee98c6ce` |
| Frames 6339 through 6399 trace receipt | `4889eaa8adc89f18fd8c03bffe6f7dd77d1338a2beeef7543cfa2f055d616f55` |
| Native-rate three-panel review | `25f0e6bf385bc64a33977603c4a71140aa4a6e328d0d448f546ced911636b6e5` |
| Quarter-speed three-panel review | `07413901c22a674975d2c65204c991041d34084c2eeb9f1c2d0c1d33a2cb8d9d` |

That causal run presented all 6,400 frames with zero dropped and zero starved
in 354.653647778 wall-clock seconds. Every retained picture, packed map,
alpha and computed trace over the 61-frame interval was byte-identical to the
preceding accepted boundary. Those hashes are archived provenance evidence,
not outputs of the clean shipping branch.

Shipping tooling commit `829c78c828dfc474136092b41793ba6e6ac6e21f`
(tree `0cfe7cd7d29d24681d93781ecc29b1167a0240a2`) ports the exact
`crates/spike/src/bin/playback.rs` blob
`e3bc354fae6e75ecca9e0d21eb326f5d83ec79db`, `crates/spike/build.rs` blob
`ffad4f2f0bf579a56a14ac3a864e7317ca3729ee` and shared
`crates/spike/src/seam_trace.rs` blob
`95ed7c70bf7daa6d1d88f3610f0d21e04f3e7bf7` from the archived provenance.

`range-trace` is deliberately adapted rather than byte-identical. After the
supplied-map app inspector was removed, it instead decodes and authenticates
the exact source pair, binds the authenticated map leaves to that fresh
delivery's opaque stamp, and rejects a preparation that is not the zero-shift
ONE X2 type-2 projection. A fresh exact-branch causal range and trace are
still required before merge.

## Bounded owner-eye verdict

The final review placed independently cropped Kjerag, Studio Flow On and
computed-trace Kjerag panels side by side. It applied no clip-specific
registration transform. A footer stated that the panel coordinates do not
align and directed the viewer to compare riser continuity inside each panel.

On 2026-08-31 the owner reviewed the native and slow review sequence and
reported "Looks good." That verdict covers the reported floating-riser and
loop continuity defect over frames 6339 through 6399 at the stated view and
settings. The owner reported the same bounded result again after the first
internal direction-concurrency change.

Later optimized artifacts were byte-identical, but the exact
`5b325d885d716073fdbc4c71e93b3f7e41ac0b06` build did not receive a fresh
owner verdict before the provenance branch was frozen. The clean shipping
branch likewise has no exact-build verdict yet. Neither coordinator inspection
nor byte identity inherits an earlier eye gate.

## Throughput and sound

The last controlled prefix comparison used the exact owner clip and view,
frames zero through 200, factory seam, Sharp sampling and a quiet null sink.
The optimized candidate mean was 17.992537313 fps against a 29.97-fps source
clock, or 60.035160% of real time. It was one machine, one clip, one view and
four balanced pairs with a material order effect. It is not statistically
significant, portable, whole-file or end-to-end throughput evidence.

Correctness currently outranks speed, but real-time playback remains the
product target. Continuous sound is unresolved and no sound compromise has
been accepted.

## Claims not made

This record does not establish:

- Studio's unread stabilization or pose-cache internals;
- Studio internal-map identity;
- source/output identity for every Studio frame;
- whole-video, other-view or other-capture parity;
- general seek and reset parity;
- real-time playback or continuous sound;
- portable or statistically significant performance;
- an accepted tradeoff;
- an owner verdict on the exact shipping build; or
- merge readiness.

Before merge, the exact shipping build still needs the complete workspace and
GPU gates, ordinary player and installed Flatpak playback, a new authenticated
causal range and computed trace, and the owner's verdict on the rendered
sequence from that build.
