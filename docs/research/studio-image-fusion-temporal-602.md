# Mac Studio 6.0.2 color-update boundary

Bounded static investigation after the owner confirmed that holding Kjerag's
color coefficients constant removes the reported April flicker, 2026-09-09.
This does not establish a fix or complete source/output temporal parity.

Binary: local `worker602.arm64.dylib`, SHA-256
`0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452`.
This matches the worker authenticated by the existing X4 runtime capture in
`studio-image-fusion-maps-602.json`. The similarly named
`libstudio_worker.arm64.dylib` is a different image and was not substituted.
Private annotated instruction receipts are under
`scratch/studio-seam-ab-20260908-01/mac-color-update-01/`.

## Existing evidence and its limit

The runtime capture contains one fusion-map upload pair at one selected source
timestamp. Later records are repeated consumer draws, not consecutive captured
map contents. The existing real Studio movies remain the visual oracle, but do
not reveal the internal coefficient-update sequence. No new export had been
made at this initial static-only stage; the later diagnostic is recorded below.

The older Windows 5.9.10 selected path has temporal state in its content gate,
integer-quantized retained metric, warm-start solution fields and left ratio
row 40. Kjerag already implements those mechanisms. They do not establish what
the Mac 6.0.2 outer producer does; see `studio-image-fusion-spatial.md` for the
version and arithmetic boundaries.

## Optional publication smoothing exists, but is not the X4 Air default

`FisheyeImageFusionBase::GetOutputColorMaps(Mat&,Mat&,bool)` is at
`0x325148c..0x3251748`. It starts with current maps at object `+0x768/+0x7c8`.
When byte `+0xed1` is set, it uses retained maps `+0x828/+0x888`:

- Publication count `+0xed4 == 0`: copy the current pair into the retained pair.
- Count nonzero and the call's boolean true: apply `cv::addWeighted` in place
  to each retained map, **0.64 retained + 0.36 current**, zero offset.
- Count nonzero and boolean false: keep the retained pair unchanged.
- Select that pair as output, optionally resize according to `+0xef0`, and
  increment publication count. There is no timestamp-based interpolation here.

Both weights are read directly from instruction-built binary64 values:
`0x3fe47ae147ae147b` and `0x3fd70a3d70a3d70a`, respectively, at
`0x325151c..0x3251540`, repeated at `0x3251584..0x32515a8`.
The calls to `cv::addWeighted` are `0x3251558` and `0x32515c0`.

The two-CVPixelBuffer CPU color getter calls content predicate
`CheckIfUpdateRatioMap` at `0x325a358`. Its admitted branch solves then invokes
`GetOutputColorMaps` with true at `0x325a4d0`; its skipped branch passes false
at `0x325a53c`. Therefore even the enabled policy advances on admitted color
updates, not every source frame or redraw.

**Selection check:** the base constructor clears `+0xed0/+0xed1` at
`0x3246a40` and zeros the publication count. `Init` sets `+0xed1` only when
the camera at `+0x2c` equals 26 (`0x3247624..0x3247638`). The native camera-name
map pairs `Insta360 X4 Air` with **23**, not 26, at `0x168b46c..0x168b494`;
the next entry pairs `Antigravity A1` with **26** at `0x168b49c..0x168b4c0`.
ONE X2 is 10. Ordinary `StitchFusionImpl::SureInit` obtains that camera type,
sets it at `0x16a2448`, then initializes at `0x16a2540`.

`Reset` clears the smooth flag at `0x32482e4`. The public override
`SetSmoothFlag(bool)` writes it at `0x3241bd8`, but a scan of direct B/BL
instructions in this worker finds no callers. The ordinary private
StitchFusion setup does not invoke that override. These facts rule out
assuming this optional smoother is the ordinary X4 Air/ONE X2 behavior.
They do not establish the behavior of an unrelated external API client that
explicitly calls the exported setter.

## Inner temporal law and export handoff

The Mac MGP wrapper at `0x32243c0` retains the same kinds of state as the
Windows reference: a bounded metric, adaptive iteration budget and previous
channel solutions. One concrete instruction-level difference is the metric's
fused multiply-add at `0x322504c`, after the current-value multiplication at
`0x3225034`. Windows and the current Kjerag table use separate products and
addition. Under round-to-nearest/even, an exhaustive scalar emulation over
retained/current integers 20 through 100 differs at seven pairs:

```text
previous,current : Windows/Kjerag, Mac
33,58 : 33,34       49,74 : 49,50       56,31 : 55,56
58,33 : 57,58       66,41 : 65,66       73,98 : 73,74
98,73 : 97,98
```

These are emulated arithmetic transitions, not observed states in the April
clip. Their occurrence and visible significance remain unmeasured; do not
promote this small difference to the flicker cause. Current-metric iteration
budgets remain 5..25, with 100 for the first solve. The private `RESULT.md`
and `MGP2-TEMPORAL.md` record addresses, literal bits and the emulation.

The inspected export handoff does not supply a hidden interpolation either.
`RenderExporter2Render::SetFusionResult` at `0x8f3d48..0x8f4158` appends the
incoming shared result pointer to a FIFO; `GetFusionResult` at
`0x8f52e8..0x8f55f8` removes its front pointer. The immediate producer calls
`StitchFusion::Process` at `0x8fa1c8`, and the consumer attaches the selected
pair as frame side-data usages 3 and 4 at `0x8ed25c..0x8ed3f0`. A cached-pointer
substitution exists, but these inspected paths perform no arithmetic between
different correction-map pairs. The actual source ordinals processed, skipped
or reused still need runtime evidence; FIFO structure does not prove cadence.

## Decision and next bounded observation

Do not enable either this optional camera-specific recurrence or the proposed
Kjerag-specific 100 ms time constant as a purported Studio X4 Air fix. The
unbuilt time-based draft was archived and removed from active code.

The remaining discriminator is one short live observation of the reported
April interval: retain the source timestamp, actual color-input bands, content
admission, current/retained metric, and resulting correction maps at the native
producer boundary. That separates different input evidence from different
calculation/history. Existing captured inputs should then be replayed through
the readable reference before selecting a change. This does not require
reopening encoded-output frame alignment or exporting another comparison movie.
## First live input/history prefix, 2026-09-09

The same hash-pinned worker has now supplied 15 consecutive complete color
observations, source timestamps 607540.2666666667 through 608007.4 ms (saved
trim sources 18208..18222). Private evidence and the recorder are under
`mac-color-update-01/native-run-01/` in the existing comparison directory.
This was a new **1080p diagnostic export**, not a replacement for the accepted
7680x3840 Studio movie. Its observed consumer was the type-2 static render
object; equivalence to the earlier type-11 panorama export is not established.
Do not attribute a difference to the accepted oracle solely from this prefix.

The requested capture cap was 63, but it was manually closed at 15 fully
published observations after long gaps in export progress. No missing tail is
represented as captured. The debugger removed all its hooks and detached.
An initial guard stopped at the first source because it mistook metadata
`+0xa0` for original time: runtime shows `+0x28` is the original milliseconds
and `+0xa0` starts at zero for this export. The guard was corrected while still
at that first source, before any source/color observation was consumed, and
the same export continued. Both the failure and recovery remain in the trace.

Concrete observations:

- Camera 23, smoothing flag zero, output size 200x100 on every observation.
- Actual bands are 800x16 BGR8. Owner validity is 200x4 bytes and extended
  inner validity is 212x4; every observed validity byte is zero.
- Native color admission occurs only at ordinals **0, 7, 13**. All other
  published pairs are exact holds. Current and published maps are identical
  on all three admitted updates, with no intermediate output smoothing.
- Native MGP starts with its first flag set, empty warm vector and retained
  metric 20. The observed raw metrics are 7, 6, 6; bounded/retained metric
  remains 20 and stored continuing budget remains 5. These observations do
  not exercise the seven fused-rounding edge cases above.
- Feeding these same bands and zero validity through Kjerag's readable CPU
  reference reproduces **all 15 admission decisions**, the raw metrics and
  retained metric. Its first solve uses 100 iterations as expected, then 5.
  Ratios are **not numerically identical**: maximum coefficient differences
  across these three updates are about 0.013..0.024 on the left and
  0.011..0.014 on the right. Those numbers are diagnostics, not visual verdicts.

This localizes the excess update frequency in the earlier Kjerag sequence
toward different measurement inputs, not a missing generic output fade. The
earlier Kjerag sequence also starts one source later; that baseline difference
must be controlled in the direct input comparison. A retained native 200x4
sampling-coordinate pair after source 18222 was saved for that comparison.
The next check is those coordinates and source bands against the real Kjerag
Scene, without changing production sampling or color policy yet.

The new CPU replay adapter is test-only. Its three validation tests and the
15-observation replay pass; ordinary production behavior and the installed
player remain unchanged. A replay pass means the data was processed and the
differences recorded, not that the native ratios matched.

## X4 camera-boundary mismatch localized

The real Scene diagnostic now retains sources 18208..18238, starting cold at
the same source as the native prefix. Its 31-source capture test passes. An
additional comparison against the older cold-18209 baseline fails on packed
geometry, so that run is not claimed to preserve the older baseline. The
preceding attempt stopped after one source because the diagnostic sampler had
populated the ordinary renderer's preparation state; it was corrected to use
the separate diagnostic pipeline without weakening the ordinary-state check.
Both attempts are retained as `april-fusion-inputs-01/02`.

With identical zero validity, the native bands admit at ordinals 0/7/13, while
Kjerag's bands admit on 14 of the first 15 sources (only ordinal 6 holds).
The CPU reference reproduces both sequences from their respective inputs.
This establishes an input difference, not a missing publication fade.

An initial apparent half-turn in same-source coarse UVs was **not** sufficient
to select a fix. Read-only sampling of original source18222, authenticated at
PTS 18240222/30000, reproduces Kjerag's saved bands within one byte code. Native
coarse coordinates instead sample stream 1 for native fusion ordinal 0 and
stream 0 for ordinal 1. This agrees with the already-authenticated original
source-pixel association in `studio-x4-video-reference-602.json`.

Mac static checks retain the positive source lookup rotation:

- `Init` at `0x32472c8..0x3247348` constructs `Ry(+pi/2)`.
- `SetFish2SphereMap` at `0x3247ba4` remaps through it into owner
  `+0x1c8/+0x228`, with no additional chart rotation.
- `StitchMapFromImageCPUData` at `0x168bb14..0x168bba8` converts dual-texture
  packed coordinates to `(2*x,y)` and `(2*(z-0.5),w)`.
- Frame-texture and CVPixelBuffer conversion preserve their native ordinals;
  the source sampler adds no image rotation or reflection.

Kjerag already performs the X4 native-to-delivered lens exchange and fixed
SPHERE `Ry(pi)` datum in `parent_inputs::x4_model6_static`. The color boundary
had omitted both conversions. Composing that existing datum with native
`Ry(+pi/2)` yields `Ry(-pi/2)` for sampling the Kjerag packed map, with native
fusion left sampling `packed.zw`/stream 1 and right sampling `packed.xy`/stream 0.
Against the retained native coordinates after source18222, that fixed
conversion reduces component mean absolute UV differences to 0.00046..0.00076,
with maxima 0.0015..0.0037. This is localization evidence, not exact upstream
geometry identity or a visual verdict.

The candidate applies the same camera conversion on publication: native
inverse `Ry(-pi/2)` composed with the X4 datum becomes `Ry(+pi/2)`, and native
ratio outputs are rebound to delivered stream order. The ratio grid uses 100
angular steps, unlike the geometric map's 99-step endpoints; reversing 99-row
indices would therefore be wrong. ONE X2 retains its original coordinates and
ordinals. No solve, admission, temporal policy, extra GPU pass or output fade
is added. The candidate is not yet qualified or installed; the moving owner
A/B remains the acceptance gate.

## Candidate verification and moving review

`april-camera-fusion-01/` freezes the candidate source and test executable.
Two 31-source actual-Scene passes succeed: native-aligned cold source18208,
and the owner's comparison-aligned cold source18209. Every packed map and
alpha remains byte-identical to its respective same-start baseline. The
candidate native-start bands now admit at **0/7/14**, versus Studio **0/7/13**:
the large excessive-update difference is removed in this prefix, but the last
update still differs by one source. Native band-byte mean absolute differences
are about 8.0/7.1 codes per lens, down from about 50.8/49.8 before conversion.
These are input diagnostics, not a claim of invisible residuals.

The 61-source ONE X2 riser sequence preserves all 305 prior artifacts exactly:
pictures, packed maps, alpha and both ratio maps. Both cameras' actual cold/warm
Scene tests pass, including the CPU-reference comparison on their own sampled
inputs. Focused color checks pass 58 tests, zero failures, one opt-in replay
ignored; optional captured-fixture/profile cases without supplied inputs are
not new native-data qualification. Workspace/all-target Clippy, formatting,
source-list and name checks pass. Full offline workspace tests now pass with
both real camera inputs and required GPU enabled: 1,269 passed, zero failures,
31 ignored. Three additional 31-source actual-Scene sequences cover the April
riser and the reported July/August views; all 93 packed maps and alpha maps
remain byte-identical to their respective same-start baselines. This preserves
geometry, not an owner verdict on the new colors or temporal behavior.

The release native player is frozen as `april-camera-fusion-01/kjerag`, SHA-256
`a06c28d8186c40a6451c98ec95d63604e50906971fa9bdd82d9e3fe9c0ec5d43`.
Its isolated native UI harness passes 47 checks, zero failures, including real
X4 playback, pause/resume and late scrubber seeking. The exact starting-view
backward seek and paired-file drop checks were skipped in this invocation.
Artifacts are retained under `native-ui-session/` and the full terminal report
in `native-ui.log`. These are functional checks, not a performance qualification
or flicker verdict. Installed-bundle qualification remains pending; nothing has
been merged or installed. Delivery preflight found only 1.1 GiB free, below the
existing packaging runner's 6 GiB build and 5 GiB qualification guardrails.

The first new output-grid regression failed because it treated a permutation
of already-rounded native tables as an exact geometric oracle. At row1/col0,
the composed table yields padded X=1, while the opposite table at row99/col100
yields X=200: f32 sin(pi) takes the other azimuth branch, whose native upper
clamp selects column199 instead of column0. These are adjacent periodic nodes,
not identical endpoints. The corrected regression compares **every node** to
the readable camera reference at the unchanged tolerance, and retains the
independent geometric permutation control away from those singular meridians.
No production arithmetic was changed to make that test pass.

The muted moving review uses the existing accepted Studio movie, not another
export: candidate left, Studio right, 31 sources starting18209 repeated eight
times. Private file `review/candidate-vs-studio-loop-muted.mp4`, SHA-256
`b7d74cdf02f5b40b62673cbceca05636a4ff0fd70be7270b43aa65c5280be4a1`.
An additional `candidate-vs-previous-muted.mp4` preserves the previous Kjerag
arm for comparison. Root inspected actual output pixels for framing and
picture integrity; the owner's moving-video flicker judgment is still pending.

## Installed candidate, 2026-09-09

The preceding packaging/storage hold is resolved. Source checkpoint
`9e193956cebb3034e18660911f8eeaddde11780d` was built offline with the 25.08 SDK,
then installed locally and tested through `dev.harding.Kjerag`, not an alternate
app path. The installed X4 harness passes 40 checks and ONE X2 passes 44, with
zero failures. Both include the exact reported starting view, backward seeking,
late scrubber seeking and real zero-copy playback; ONE X2 also exercises all
four paired-file arrival paths. Both skip the volume-popup check because the
isolated sandbox reports no sound device, and the intentionally injected
stuck-import check because it cannot preload into the sandbox. X4's paired-file
checks do not apply to its one-file capture. This does not qualify performance,
audio or the owner's flicker verdict.

The package source snapshot, build, prior-install identity and both complete UI
sessions remain in `scratch/flatpak-delivery-9e193956/`. Installed OSTree commit:
`a3287f9221a40cb7e6895ce8f52e721422a339feed2d64fb800a6cb5b4cb668c`.
Installed executable SHA-256:
`68dba116ca0a563d01085ffeb00a1d64bd7a963f504c45848726ca90e0dba056`.
Source, permissions, executable, installed commit and harness identities were
checked across qualification. The prior `31ea781d` bundle remains available
for rollback. Root inspected both installed reported-view captures; stills
establish picture integrity, not absence of temporal flicker.

To make delivery space, only the 11 GiB generated `target/debug/deps` cache
in the idle `gpu-warm-post-l1-join` agent worktree was removed. Its source,
top-level executables and all personal footage/comparison evidence remain.
The earlier native UI evidence was moved intact to `native-ui-session/` before
reusing the harness path. No merge or release occurred. The moving A/B verdict
is still required before declaring this candidate a visible fix.

## Owner rejects the candidate, 2026-09-09

Owner verdict on the candidate-left/Studio-right moving comparison:
"The left still flickers same way". The candidate therefore fails the visible
flicker acceptance gate. The camera-boundary finding and improved admission
counts remain diagnostic facts, not a solved defect. The build is not accepted
for merge; no rollback or additional product change is inferred from this
verdict alone.

The prior owner-confirmed fixed-coefficient control remains the stronger
causal discriminator: changing color coefficients is necessary for the reported
flicker in that comparison. Next inspect changing native/reference outputs on
the same already-captured bands, and the actual final color consumer. Do not
repeat sampling/mesh trials, infer success from update counts, or start another
Studio export to replace the accepted oracle.

## Same-input inner/outer split after the rejected candidate

The recorded native MGP return includes 212x100 prepared BGR images on updates
0, 7 and 13. A test-only spatial replay now substitutes those images for our
inner solver output, while keeping our ratio construction, retained boundary
row, blur and final native-chart remap. The denominator/current rows still
come from our area reduction of the same native bands; a native reduction
rounding difference is not excluded. No actual-player output changed.

| Update | Lens | Full reference maximum ratio error | Native prepared through our outer stages |
| --- | --- | --- | --- |
| 0 | 0 | 0.024082 | 0.001181 |
| 0 | 1 | 0.013807 | 0.001552 |
| 7 | 0 | 0.023475 | 0.001174 |
| 7 | 1 | 0.011313 | 0.002546 |
| 13 | 0 | 0.016965 | 0.000848 |
| 13 | 1 | 0.012234 | 0.002119 |

This localizes most of the measured coefficient discrepancy to the inner
solve or its inputs. It is not evidence that correcting it will remove the
reported flicker: native and reference outputs differ spatially, and a maximum
over ratio nodes is not a rendered seam verdict. The residual outer-stage
difference also remains unclassified. Independent same-band inspection finds
that native/reference discrepancies change at the admitted updates, rather
than being only a fixed spatial bias; both hold exactly between updates.

Raw replay reports and outputs are retained in `april-native-prepared-01/02/`
under the existing comparison directory. The second also records our prepared
and reconstructed current BGR images for a direct inner-stage comparison.
The opt-in test is
`image_fusion::spatial::tests::replay_captured_native_prepared_outputs`, using
`KJERAG_FUSION_NATIVE_PREPARED_REPLAY` for the original native capture and
`KJERAG_FUSION_NATIVE_PREPARED_OUTPUT` for a new output directory. A successful
run validates processing/shape, not numeric equality or visual acceptance.

Two further controls prevent selecting a superficial solver rewrite:

- Native `GenerateXIds` numbers the overlapping lens nodes interleaved per
  pixel, unlike Kjerag's complete lens blocks. The test-only
  `image_fusion::solve::tests::replay_native_node_order` changes that vector
  order while preserving physical equations, sparse accumulation, centering
  and warm history. All six prepared BGR images at updates 0/7/13 remain
  byte-identical to the ordinary reference. This particular ordering change
  does not explain the discrepancy and is not a production fix. The probe
  does not reproduce every native sparse/reduction arithmetic detail.
- Prepared images contain untouched current pixels outside each lens's
  correction mask. Native lens-0 rows 52..60 and lens-1 rows 39..47 are
  byte-identical to reconstructed current inputs at all three updates
  (5,724 bytes per lens/update). Thus the copied boundary input rows agree;
  this does not directly authenticate the other two central rows.

Independent correction-field comparison rules out a simple Cb/Cr exchange
or a one/two-cell spatial shift on these observations. The remaining field
difference includes local sign and amplitude differences before ratio
construction, reaching 8..9 byte codes on used support. These remain numeric
localization facts, not evidence of a new visible fix. Detailed prepared-field
and native application/node-order notes are retained under
`april-native-prepared-02/`; the permutation output/report is under
`april-native-node-order-01/`.

A bounded native physical-equation check finds the same ordinary open
four-neighbor penalty, exact `0.1f` edge coefficient and squared contribution,
with no added diagonal/anchor term. The selected ImageFusionMGPCpu constructor
disables the alternative triangulation branch. The byte evidence builder also
uses matching `+w/-w` matrix entries and unweighted Y/Cb/Cr differences; its
source-value multiplier is initialized to one. These checks do not authenticate
every evidence/control byte or the native finite-solve trajectory. The next
unresolved boundary is the actual evidence/control and solved fields, not
another output fade, source export or vector-layout rewrite.

## Same-input finite-solve arithmetic controls

The selected Mac `solveCorrectionColor` and its Eigen channel body use the
same exact tolerance (`0x38d1b717`), cold maximum 100, retained warm budget,
zero-cold/prior-centered-warm initialization, reciprocal diagonal
preconditioner, zero-RHS handling and strict pre/post residual tests as the
readable reference. The selected reductions and CG vector updates use separate
multiply/add/subtract instructions, not fused multiply-add. This finite-loop
check does not authenticate every sparse matrix or evidence entry.

Two arithmetic differences were read, rather than inferred from output:
Mac dot products finish their two four-lane accumulators with adjacent pairs
`(0+1)+(2+3)`, while `chromatic::dot` chooses `(0+2)+(1+3)`; native also centers
each channel with a binary32 vector reduction instead of `chroma::centre`'s
binary64 sum. Native retains the centered vector as the next warm guess.
The initial static note incorrectly equated the dot products from their
two-accumulator shape alone; the actual `faddp` finish corrects that claim.
Instruction addresses are in
`april-native-prepared-02/cg-semantics-audit.md` under the private comparison
directory; the recovered fixed-length centering is the test-only
`native_centre` helper in `solve_arithmetic_probe.rs`.

The opt-in `image_fusion::solve::arithmetic_probe::replay_native_arithmetic`
runs five independent histories on the same reconstructed current images at
native updates 0/7/13: the already checked native node-order control,
materialized normal-matrix multiplication, native centering alone, native dot
finish alone, and all three arithmetic changes together. Evidence, physical
equations, preconditioner and budgets remain unchanged. The sparse control
accumulates separate diagonal/off-diagonal products in ascending native column
order; it is not a claim to duplicate every Eigen sparse instruction. The
test-only CG copy is checked against ordinary reference outputs and exits by
the node-order control. Small synthetic tests pin the reduction discriminator
and the materialized matrix's diagonal/symmetry.

**Every arm leaves all six prepared BGR images byte-identical to the ordinary
reference.** All preserve cold exits 49/28/34 and warm truncations at five
iterations. The greatest pre-byte channel-field difference is approximately
0.000036 codes. Native prepared discrepancies still reach 8..9 byte codes.
None of these arithmetic controls explains the saved-output mismatch, and
none is selected as a production change or a flicker fix. This is a bounded
negative on these observations, not a guarantee for arbitrary videos or all
floating-point implementations.

Reports and output images are retained in `april-native-arithmetic-01/02/`.
The replay reads `KJERAG_FUSION_ARITHMETIC_INPUT` (prepared-02 current images),
`KJERAG_FUSION_ARITHMETIC_NATIVE` (original native capture) and creates
`KJERAG_FUSION_ARITHMETIC_OUTPUT` (a new directory). No new Studio export,
installed build, temporal smoother or playback code change accompanies this
test. Input/evidence construction remains the next unresolved boundary.
