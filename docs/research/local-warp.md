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

## Reframe reference audit

The locally available Insta360 Reframe 5.8.4 installer was unpacked
*statically* in a disposable directory; neither its installer nor its Windows
plug-ins were executed.  This is an interoperability reference, not source to
copy.  Its small Adobe `.aex`/`.prm` hosts load a 79.5 MiB `Insta360CoreMedia`
renderer.  That renderer retains diagnostics and type names for
`FisheyeModel`, `CameraOffsetCalib`, `DynamicStitcher`, `StitchFusion`, and
`StitchAlphaMapFrom`.  It reads camera-offset metadata including per-camera
and refined dual-offset forms, constructs a fisheye model from it, and has a
custom-template blend angle.  Its fixed calibrated projection is therefore
the baseline, rather than a universal image-space warp.

The same payload has explicitly optional dynamic modes: `OPTFLOW1/2`,
`DISFLOW`, `BLOCKFLOW`, `AIFLOW`, and `DYNAMICSTITCH`; it reports that dynamic
stitch is ignored in POV mode.  The shipped opaque `.ins` model assets contain
flow-network graph labels and are used through MNN; nearby plaintext XML
assets are OpenCV SVM models selected in camera/configuration code.  Static
evidence cannot identify their exact run-time gates or outputs, but it does
establish that they are learned content-adaptive fusion aids, not a
camera-agnostic calibration table.

The newer locally available Studio 5.9.2 installer lists the same five
hashed `.ins` assets byte-for-byte among its own model set.  This confirms the
flow/fusion layer is shared Insta360 renderer infrastructure, not a
Premiere-bridge workaround; it does not add any inference about when a given
mode runs.

The engineering consequence is deliberately limited: Kjerag already consumes
the per-unit `offset_v3` calibration, which is the appropriate first layer.
Any later adaptive seam method must be independently measured, optional, and
gated on a fixed far-field correspondence/hold-out protocol.  Reframe's
existence does not license an always-on local warp, and its flow machinery is
especially not evidence that an infinity residual is parallax.

That question has an observability gate. Four horizon crossings are four
**scalar edge-normal** constraints, not four two-axis observations. They
cannot reject a five-knob map: their Jacobian has rank at most four and no
residual degrees of freedom. `band::Along` also has five free harmonic terms;
it is a smooth residual field, not a three-knob pose that can be assumed
instead. Stage 9 needs at least four true two-axis correspondences: eight
scalar rows leave three residual degrees of freedom after the five-knob fit.
Equivalently it needs at least eight independent, nonparallel scalar features
whose five-knob Jacobian is well-conditioned. A horizon is acceptance evidence, never a
substitute for its missing tangent measurement.

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
raw lens planes. Every actual 50/50 root receives the same globally declared
3-by-3 overlap-strip lattice in the baseline-derived `[perp, epi]` frame.
Each site must have a complete angular reference patch; no rectangle is
zero-filled. Target coverage is then recorded independently at every declared
shift, including unavailable shifts. This bounded probe reports roots, sites,
site offsets, shifts, and coverage only: it does not pick a view, patch, or
warp based on texture or score. A later two-axis registration must continue to
refuse a rank-deficient edge for the aperture problem.

The probe sweeps one declared, global angular support ladder (1.20/1.00,
2.00/1.60, 2.80/2.40, and 3.68/3.00 degrees of span/search at 0.08-degree
sampling). `span=` and `search=` may replace that ladder only for the whole
invocation; comma-separated lists are paired, with a single value broadcast
over the other list. Each rung reports roots, fixed sites, reference-complete
patches, attempted target shifts, and each coverage outcome. Thus a growing
support refusal is evidence of lens geometry, while complete patches that
increasingly refuse for aperture are evidence of inadequate two-axis texture.
Neither is silently promoted to a displacement, and the sweep does not choose
a rung or a view. Target coverage is evaluated at each site and candidate
shift, not over an enclosing search rectangle: unavailable offsets are retained
as coverage diagnostics and do not erase legal neighbouring shifts. When
registration is reintroduced, a maximum on the declared search rail, a tied
maximum, or no complete target patch refuses.

Raw coverage is itself a result. Shrinking an angular patch does not create
overlap, and a lens-separated *rendered* image would be contaminated by the
pass's bend, blend, tone, colour conversion and sampling. The evidence source
remains synchronized raw planes projected through the warmed `Reframe` map.
The next diagnostic emits CPU-only per-lens valid masks and a per-candidate
coverage census. A valid sample is exactly `Landing.inside &&
Plane::at(...).is_some()`; renderer cap pretests and blend weights are not
part of it. The raw pair's PTS must equal the warmed frame for a geometric
reading; a loose warning cannot promote it to evidence.

The locator is also part of the measurement. `body.z = 0` is only a nominal
great circle and can be outside one calibrated lens while the rendered seam
is plainly visible. Stage 9 therefore traces the **actual** calibrated 50/50
handover contour: `Blend.weights[0] == Blend.weights[1]`, with both weights
positive and both raw projections valid. Its camera-frame node is built from
that root ray, not from an azimuth placed back on the nominal circle. This is
location evidence only; the raw planes remain the source of registration
pixels. Each root must prove weight equality and two-lens projection before a
patch is considered.

This does **not** yet make a shared pose Jacobian, establish an empirically
calibrated condition threshold, or infer/apply a warp. Its uncertainty is the
local linearized luma residual only; repeatability and raw-versus-scene PTS
agreement remain explicit follow-up controls.

## Fixed-site response control

`responses=1` is available only with an explicit stored `seam=` fit.  It does
not mutate an already-warmed scene.  Instead it independently rebuilds and
warms the baseline plus the negative and positive central perturbation for
each of `roll`, `yaw`, `pitch`, `cx`, and `cy`, using the same cue, camera,
horizon, sampling, and rendered traversal.  The tool refuses a perturbation
whose final PTS or warm-frame count differs from baseline.  Roots and 3-by-3
sites are declared from the baseline map once; they are not re-traced on any
perturbed map.  The report currently counts only whether each fixed site has
a finite central response, is projected out, or is locally singular.  It does
not fit a pose or apply a warp.

On the May BAD view at 50.150100000 s with the explicit May fit and the
1.20/1.00-degree support rung, all 153 frozen sites completed all five
central-response controls: none projected out and none were locally singular.
Every independently warmed map ended on the same PTS after the same 61-frame
traversal.  This is only a Jacobian-availability result; it does not compare
crossings, calculate the shared-pose residual, or license a warp.

## First valid coverage result

The first lattice run was invalid: it handed a body-frame node directly to
`Reframe::project`, whose input is a view-frame ray.  The resulting universal
`ProjectedOut` result was a coordinate error, not evidence about raw overlap.
The instrument now explicitly transforms body back to view before projection,
and a regression test proves that every traced root round-trips to its view ray
and reproduces both `Blend` landings.

On the May 1 same-instant pair, warmed to 50.150100000 s with exact matching
raw PTS and the same file-local fit, the corrected 1.20-degree span / 1.00-
degree search rung has complete support everywhere it declares:

| view | roots | fixed sites | complete reference sites | complete target shifts |
| --- | ---: | ---: | ---: | ---: |
| GOOD (`yaw=-74.43`) | 15 | 135 | 135 | 84,375 / 84,375 |
| BAD (`yaw=106.98`) | 17 | 153 | 153 | 95,625 / 95,625 |

Those counts are a location-and-coverage result only.  They do not select a
texture, calculate a correspondence, identify an error as calibration rather
than parallax, or authorize a warp.  They merely clear the former raw-overlap
blocker for a predeclared 2-D registration experiment.

The next opt-in raw observation check used the explicit May fit
`roll:0.789,yaw:-2.171,pitch:-1.299,cx:-1.71,cy:-14.08` (rather than
`seam=file`) and the same 1.20/1.00-degree rung.  The May BAD view retained
149 two-axis readings of 153 declared sites, with 4 `NoPeak`, no aperture
refusals, and no missing support.  The May GOOD view retained 117 of 135, with
18 `NoPeak`, again no aperture or support refusals.  A no-peak site remains a
reported ambiguity, not a reason to pick a different site.  These same-
capture checks establish that two-dimensional raw observations are available;
they neither pair a physical feature across views nor fit a pose.

Reciprocal raw registration is now a separate control: it holds the same
body-frame axes for lens 0-to-1 and 1-to-0, records either directional refusal,
and reports their closure and summed covariance.  Under the stored May fit on
the same rung, BAD yielded 147 reciprocal sites, closure mean
`[epi -0.0090, perp +0.0033]` degrees and site RMS `0.2505` degrees; GOOD
yielded 104, mean `[-0.0276, +0.0294]` and RMS `0.8062` degrees.  The mean is
not a substitute for the site scatter: these RMS values show that the current
per-site raw signal still includes large scene-dependent effects.  They close
the door on interpreting the existing single-view pose fits as an applied
camera-frame warp.  The next required control is the same physical feature
tracked through time and depth/disparity strata.

The first PTS-locked forward temporal run confirms that this next control is
practical, without yet drawing a depth conclusion.  At May BAD's 50.150100000
second anchor, with the same stored fit and four consecutive transitions,
all 153 declared sites remained active: 612 successful lens-0 tracking steps,
and zero missing-patch, peak, aperture, or 5-degree-excursion refusals.
Sites were declared only at the anchor; no later frame retraced or replaced
them.  Forward/backward closure is explicitly unavailable in this first
forward-only traversal.  Stereo depth/disparity accounting is the next layer,
not an inference from this trackability count.

The original temporal report classified a positive epipolar disparity using
`point - sigma`. That produces an **upper** distance bound, so its old
near/mid/`far >=10 m` summaries are not evidence about infinity and are now
retired. The instrument instead has four predeclared categories:
`ProvenFar300`, `FiniteButNotFar`, `UncertainOrUnplaceable`, and `Invalid`.
`ProvenFar300` is strict: the physical baseline and one-sided maximum
plausible positive disparity `point + 3 sigma` must still give
`baseline / tan(point + 3 sigma) >= 300 m`. This is a lower distance bound.
A zero or sign-uncertain disparity is compatible with infinity but does not
prove it, so it stays `UncertainOrUnplaceable`.

This narrows Stage 9 to the actual problem: the horizon and terrain at
hundreds of metres. It makes no claim from the existing all-depth corpus and
does not authorize a warp.

## Stage 9 decision on the available all-depth corpus

**No local warp is authorized from the current four owner references.** The
previous all-depth temporal categories cannot establish an infinity-only
result and have been retired. The May one-step temporal replay closure did
pass: BAD closed all 153 fixed sites at mean `[epi -0.0007, perp +0.0081]`
degrees; GOOD closed 131, with two unavailable forward tracks and two reverse
refusals, at `[+0.0051, -0.0386]`. This establishes that the tracker itself
is not merely drifting, but it is not a far-field geometric claim.

The owner views face roughly opposite seam azimuths, so they are not
same-feature cross-view pairs. Stage 9 has produced a reusable instrument and
an all-depth refusal, not an applied geometry change. The far-field seam
problem remains open.

The far-field follow-up needs a stricter classification, not a calibration
scene: `ProvenFar300` only when a predeclared one-sided maximum disparity
still implies a distance of at least 300 m; separate `FiniteButNotFar`,
`Uncertain`, and invalid populations; and an explicit stable sky/earth
horizon class.  Horizon measurements are one-dimensional edge-normal
acceptance evidence, never invented 2-D pose rows.  A candidate can use only
fixed, textured `ProvenFar300` sites with PTS lock, reciprocal and
forward/reverse closure, covariance, and held-outs.  Until then, `feat/warp`
must not add a local warp or merge one as a seam fix.

## Required controlled-capture protocol

The next experiment is a *paired-observation* capture, not another selection
of two attractive seam screenshots.  Before looking at a registration score,
record the following manifest alongside the capture: the two view poses, exact
PTS anchor and replay frame IDs, one explicit stored or pooled `seam=` fit,
the support/search rung, and the feature and hold-out assignments.  The same
fit is supplied to every view and every replay; `seam=file` is prohibited.

1. Capture a static scene from two overlapping views so that each declared
   physical feature is visible in both raw lens observations at the same PTS.
   Declare at least four spatially distinct, textured, non-aperture features
   before fitting.  Each is one two-axis correspondence with its full 2-by-2
   covariance; multiple samples of an edge, or of one object patch, are not
   independent features.
2. Classify every registration before analysis. Only `ProvenFar300` sites may
   support an infinity claim: `point + 3 sigma` must triangulate to a lower
   distance bound of at least 300 m. Retain `FiniteButNotFar`,
   `UncertainOrUnplaceable`, invalid, and registration-refusal populations;
   they may not be pooled into the far result.
3. Track those same declared features over the predeclared contiguous replay
   frames.  Do not re-trace, replace, or texture-select a feature after the
   anchor.  Run both forward and reverse tracking, report per-feature
   forward/backward closure with summed covariance, and refuse a feature that
   cannot close.  This control distinguishes tracker drift from a repeatable
   camera-frame residual.
4. Split by physical feature *and* by capture/replay before fitting.  Fit the
   shared five-knob pose, and only a subsequently proposed bounded warp, on
   the development partition.  Freeze support, taper, axes, fit parameters,
   and condition rule before opening the hold-out partition.  No held-out
   feature may be used to choose a site, tune a threshold, or refit either
   model.
5. Accept neither model on an average alone.  On the held-out features and
   held-out replay/capture, report the covariance-normalized residual by depth
   stratum, pose prediction, local-model prediction, and closure.  A local
   warp is eligible only if the global-pose model is rejected by the calibrated
   controls, its residual is repeatable across time and depth rather than
   parallax-linked, and it predicts every predeclared held-out group without
   moving the defect elsewhere.  Otherwise the result is a refusal.

This protocol supplies the missing physical correspondence and independence;
the current May/April opposite-yaw views do not.  It does not authorize an
implementation change by itself.

For a shared-pose comparison across captures, `seam=file` is not a valid
baseline: it fits each flight from its own scene and can absorb parallax.  The
future multi-capture invocation must receive one explicit stored/pooled seam
fit for every reference.  It must obtain at least four independent,
non-aperture two-axis correspondences (eight scalar rows, three residual
degrees of freedom after the five-knob fit), use their full positive-definite
2-by-2 covariance (including its off-diagonal term), and calibrate its
condition gate with planted-pose and self/repeat controls. The reported
normalized RMS is square-root chi-squared per those residual degrees of
freedom, not per raw axis; condition is reported but has no invented
empirical cut-off.

## Controls

- A self-pair runs the whole tracer and is statistically zero.
- `fit=1 plant=roll:...,yaw:...,pitch:...,cx:...,cy:...` is the current
  planted global-pose sanity control.  It first builds the five central
  responses through independently warmed maps, then uses their prediction as
  synthetic displacement at the exact fixed sites that passed raw
  registration, retaining each measured full covariance.  The normal
  assembly and shared-pose solver must recover the declared five native-unit
  knobs.  This checks map-response/assembly/unit/covariance/fit plumbing and
  reports the same condition number as the real fit.
- It is deliberately **not** described as a raw-pixel pose recovery.  The
  delivered lens planes are one physical capture, so changing a projection
  map does not create a second capture with a perturbed lens pose.  Comparing
  a perturbed map's resampling with baseline raw pixels would test a different
  registration problem and could hide interpolation or content effects.
  Thus `plant=` cannot validate raw registration, finite-difference
  linearity, a condition threshold, or an applied warp; independently
  captured/calibrated controls remain required for those claims.
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
