# Stage 9: local warp versus pose

**Status:** instruments first. No local warp is applied by this stage.
**Issue:** #103. **Authorized:** owner, 2026-08-01: "stage 9".

## Question

Stages 5 and 6 apply the smallest field that represents a camera pose: a
constant plus one and two cycles around the seam. The owner accepted that
correction, but the remaining visible line is local and geometric. At the
gear crossing its one-pixel Weber excess is +5.87 percent, and the rejected
photometric branch left it 5.94 to 5.94 percent. Photometry is not its cause.

The controls say a further global pose is not safe to assume. At one instant
in the May reference, one seam crossing is matched while the other is not.
Two April views likewise differ two seconds apart. A pose that improves only
one crossing is not a solution; it has traded the defect around the ring.

Stage 9 asks a narrower question before it changes the shader: can one shared
five-knob pose explain all of those crossings within the tracing error? If it
cannot, is the residue a smooth, camera-frame-local geometric displacement
with finite support rather than a view-specific adjustment?

## First deliverable

`kjerag-spike --bin local-warp` is an instruments-only deliverable. It takes
the paired owner references from `reference-views.md`, renders each through
the shipped path, and reports a close-window edge displacement at the visible
seam crossing. It then perturbs the five existing calibration knobs to build
one shared pose Jacobian across the observations and reports:

- the observed displacement and each trace's fit error;
- the correction a single global pose predicts at every crossing;
- residuals after the shared fit and leave-one-pair-out prediction;
- the live-band versus held-off control; and
- the conditioning and refusals behind every conclusion.

The verdict is not a large absolute `step` number. Terrain makes the old wide
window non-absolute. The close window and the *difference* between the same
view under a controlled perturbation are the acceptance reading.

## Controls

- A self-pair is exactly zero.
- A planted global five-knob change to lens 1 is recovered at all references.
- A planted seam-local two-axis displacement is rejected by the global pose
  fit, including outside its support where it is exactly zero.
- The May same-instant GOOD/BAD pair prevents time and content drift from
  standing in for a local result. The April pair is reported independently,
  never averaged into it.
- The instrument carries the state to `warm` exactly as `step` does. Direct
  seek is a separate cold picture, not an approximation of warm playback.

## Rules for a later applied candidate

A candidate may be a deterministic camera-frame displacement in the seam's
`epi` and `perp` axes, with a declared smooth taper to exactly zero outside
its support. It is fitted from measurements, not supplied per view or per
clip. It is applied before projection from the unwarped body ray; blend
weights remain functions of that unwarped ray.

It may not reuse the old depth prototype's arbitrary per-direction table or
nearest-neighbour fill. That is a field with holes, the same mechanism that
made stage 5 scallop and stage 8 stripe. It may not widen the blend or apply
photometry to conceal a registration error.

Any applied follow-up must improve both May crossings without trading one for
the other; report both April views separately; preserve one-lens paths;
observe the no-fold/cap invariants; and pass `step`, `seam`, one/two-pixel
same-content Weber excess, and `colour`'s interior-coherence metric across
the whole support. A field is accepted on the area it changes, not at the
seam boundary alone. Flicker and a credible 16.6 ms frame-budget story remain
release gates.
