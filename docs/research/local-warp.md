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
the shipped path, locates the visible seam crossing, and registers a local
two-dimensional patch there in the seam's camera-frame axes. It then perturbs
the five existing calibration knobs through that same rendered path to build
one shared pose Jacobian across the observations and reports:

- the observed two-axis displacement, covariance, peak ambiguity and each
  trace's fit error;
- the correction a single global pose predicts at every crossing;
- residuals after the shared fit and leave-one-pair-out prediction;
- the live-band versus held-off control; and
- the conditioning and refusals behind every conclusion.

The horizon `step` instrument remains a picture-space acceptance check, not
the input to this fit. A single edge only observes displacement normal to
it: treating it as both camera-frame axes would manufacture evidence from the
aperture problem. A rank-deficient patch is refused, not promoted to a
two-axis reading. Terrain also makes the old wide `step` window non-absolute;
the close window and the *difference* between the same view under a controlled
perturbation are the horizon acceptance reading.

## Current bounded probe

The first binary is deliberately narrower than the eventual paired-view fit:

```sh
cargo run --release -p kjerag-spike --bin local-warp -- <file.insv> \
  time=50.117 warm=2.0 yaw=106.98 pitch=0.75 fov=62.79 lock=1
```

It runs the same warmed `Scene`/render traversal as `step` to locate the
visible `body.z = 0` seam contour, then discards the rendered pixels. One
candidate per camera-frame seam azimuth bin is tested against the synchronized
raw lens planes. A candidate must have a complete angular patch in both lenses
at the candidate offset; no rectangle is zero-filled. The selected patch is
the strongest two-axis raw-lens registration in the baseline-derived
`[perp, epi]` frame, and reports its covariance and structure-tensor condition.
A rank-deficient edge is refused for the aperture problem.

The probe sweeps one declared, global angular support ladder (1.20/1.00,
2.00/1.60, 2.80/2.40, and 3.68/3.00 degrees of span/search at 0.08-degree
sampling). `span=` and `search=` may replace that ladder only for the whole
invocation; comma-separated lists are paired, with a single value broadcast
over the other list. Each rung reports candidate count, reference-complete
patches, attempted offsets, complete target patches, accepted readings, and
the support/aperture/peak refusal counts. Thus a growing support refusal is
evidence of lens geometry, while complete patches that increasingly refuse
for aperture are evidence of inadequate two-axis texture. Neither is silently
promoted to a displacement, and the sweep does not choose a rung or a view.
Target coverage is evaluated at each candidate shift, not over the enclosing
search rectangle: unavailable offsets are omitted, while a maximum on the
declared search rail, a tied maximum, or no complete target patch refuses.

This does **not** yet make a shared pose Jacobian, establish an empirically
calibrated condition threshold, or infer/apply a warp. Its uncertainty is the
local linearized luma residual only; repeatability and raw-versus-scene PTS
agreement remain explicit follow-up controls.

## Controls

- A self-pair runs the whole tracer and is statistically zero.
- A planted global five-knob change to lens 1 is recovered through the
  delivered warm path at all references.
- A planted seam-local two-axis displacement is rejected by the global pose
  fit through the same tracer, including outside its support where it is
  exactly zero.
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
