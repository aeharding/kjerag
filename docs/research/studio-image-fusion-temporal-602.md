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

## Missing full-overlap equations: native domain finding

The next bounded native read found a physical-system difference, not an
arithmetic-order discrepancy. Studio uses two separate domains:

- Quantile/metric measurements: original rows 49/50 and columns `[18,194)`,
  selected by NCC and sticky invalidity.
- Final controls/equations: original rows `[48,52)` and **all** columns
  `[0,212)`, admitted by those quantile-derived color bounds. NCC and sticky
  invalidity are not applied again to this larger equation domain.

Native `getValidOverlap<u8>` creates and fills the entire 4x212 control image
at `0x32250d0..0x32252c8`; `fillAAndB<u8>` loads the matching overlap rectangle
at `0x3228714..0x3228748` and loops all four rows and 212 columns at
`0x322877c..0x3228844`. Its cropped y38..41 maps through the y10 crop to
original y48..51, then to solve-window rows 8..11. These are the four rows
shared by the two distinct lens-node blocks. Nonzero control alone admits an
equation. Endpoint slack/sign conditioning, inclusive inner/outer bounds,
positive-excess distance and retained-metric weighting match the prior code.
The error was using the quantile sample list itself as the equation list.

The reference fix separates these domains. It retains quantile selection and
budget/history unchanged, writes all 848 control bytes from the current raw
BGR differences, then samples every nonzero control at its own physical node
pair. New synthetic tests exercise both omitted outer rows and omitted edge
columns, verify that NCC/invalidity are not a second equation mask, and retain
the native inclusive color bounds and rejection rule.

On the same saved native bands at updates 0/7/13, **five of six prepared images
are byte-identical to Studio**. Observation0 lens1 differs at a single byte by
one code. All six final reference maps match the earlier native-prepared
substitution, leaving its already measured outer-stage discrepancy of roughly
0.00085..0.00255 rather than the former inner-driven 0.011..0.024 maxima.
This confirms the omitted equations explain the large saved inner-output
discrepancy. It does not establish that the owner's moving flicker is fixed.
Results are retained in `april-native-full-overlap-01/`; the address audit and
independent reference review are in
`april-native-prepared-02/support-control-domain-audit.md`.

The adapter audit also confirms the native 4x4 area reduction, boundary-row
replication and six-column periodic extension with no intervening central-row
overwrite. Exact OpenCV byte rounding was not separately authenticated, but
the nearly identical full prepared outputs now bound its combined effect on
these observations. Native global-versus-central-row NCC fallback counting is
a separate discrepancy: strict and correlation-only masks agree everywhere
on these saved inputs, so it cannot change their solve. It is not included as
an additional speculative change to this defect's candidate.

The corresponding GPU fix separates 424 quantile-support slots from 848
control slots, retaining 1,728 state words instead of 1,320. It emits controls
and evaluates RHS/diagonal/quadratic contributions over the recovered full
overlap. No geometry, output remap, content threshold, retained metric, warm
budget, source cadence, publication timing or extra GPU pass changes.

Focused GPU qualification passes all eight tests with the real X4 and ONE X2
band fixtures supplied, including identity, lens-swap and channel-swap decoys.
Those fixtures' old float4 outputs were generated by the pre-fix CPU reference,
not Studio; the test now recomputes its expected result from their unchanged
bands and invalidity. A temporary missing `.values()` in that test adaptation
failed compilation and was corrected. An attempted run after that failed build
still used the prior executable/old expected maps and failed that fixture
comparison; it is not counted as qualification. The successfully rebuilt test
passes, with worst native-GPU/reference ratio differences about 0.000000775
(X4) and 0.0000855 (X2). The new outer-row/edge synthetic test's maximum is
about 0.000000596. Existing transition and retention tests also pass.

The exact reported April Scene capture completes sources 18209..18239. All 62
geometric packed-map/alpha files match the rejected camera-boundary candidate
byte-for-byte. The 31 displayed pictures and computed color maps are saved in
`april-full-overlap-01/frames/`, with source patch, hashes and exact test
executable alongside. The existing accepted Studio half is reused without a
new export or fitting. The muted review loop is
`review/candidate-vs-studio-loop-muted.mp4` (candidate left, Studio right),
SHA-256 `016dddc5795192647c4c4c177ff0e91edbcc5845b8cf6762b62aeb1926d51568`.
It repeats the same 31-source comparison eight times (248 frames,
30000/1001 fps, 2560x756), not 248 distinct source frames. A previous-left /
candidate-right comparison is also retained. Root checked frame15 for image
and label integrity; this still check is not a moving flicker verdict.

Actual Scene frame-ownership, seek, reattachment and cold/warm tests run with
both original cameras: 18 tests report success, with the optional cold-stage
probe and opt-in review helper inactive. GPU-only timing on the saved fixtures
shows changed-warm totals approximately 2.65..2.69 ms in both old and corrected
versions; cold totals change from 17.09 to 13.31 ms (X4) and 16.71 to 10.88 ms
(X2). These are individual serialized color-stage measurements, not sustained
playback measurements or proof of 240 fps capacity. No performance tradeoff is
accepted on their basis.

The complete workspace test gate passes with required GPU, both actual camera
paths and the real band fixture: 1,274 passed, zero failed, 34 ignored. An
initial invocation failed only because Cargo resolved the relative fixture
path from the crate directory; the absolute-path rerun completes successfully
in `april-full-overlap-01/workspace-tests-02.log`. All-target workspace Clippy,
formatting, rename and Cargo-source checks pass. No ignored oracle probe is
counted as executed by that aggregate result.

The owner has been asked to review the new moving A/B. No new Studio export,
invented smoother, installation or owner acceptance is implied by these
results. The native numeric trace remains the recorded type2 diagnostic path,
distinct from the owner's accepted panorama comparison. Full native/Flatpak
delivery and the owner's moving verdict remain separate gates.

## Full-overlap installed delivery, 2026-09-09

Source `365cedf69eff54e81596d01ba11a2a5259ef8b27` was built as a native
release executable and an offline 25.08 SDK Flatpak. No further production
changes accompany delivery. A bounded independent GPU review found no
inconsistency in the 848-control buffer layout, full-overlap node mapping,
RHS signs or matching diagonal/quadratic terms; it did not re-audit unchanged
CG, ratio production or camera bindings.

The native player passes 50 headless harness checks at the exact April
flicker view, with zero failures. The four paired-file tests do not apply
to this one-file X4 capture. Its saved executable SHA-256 is
`19bcddf8dd009cb834fc2b23043e0b5d6b9e107a99130c561162caf38144920a`.
The binary, harness hashes, complete session and receipts remain in
`april-full-overlap-01/native-qualification/`. Root inspected the actual
reported-view pixels via lossless PNG conversion, not moving flicker.
Part of this functional run overlapped the CPU-only package build; it is
not isolated playback timing evidence.

The Flatpak build completed successfully from an immutable source snapshot,
whose file hashes verify against the exact commit. Independent package review
also verified dependency coverage, unchanged permissions, Freedesktop 25.08,
and FFmpeg 7 linkage. The candidate is installed through the usual user
`dev.harding.Kjerag`, with identities verified before and after qualification:

- OSTree: `17df9824f8f3f49a2dc7bc33b62ea86bceaf6e6d7f51a7b6e8e2dc585acd5a2a`
- Executable SHA-256: `2bda56861cbed4c507d5294b381ecdd193527f8ff41d2c299fa909bc751ea394`
- Bundle SHA-256: `bd48a2bb43d5a1a901853d55982c2ec94015f5e342fb02002b92fe781f18f714`

The installed-ID harness completes 40 X4 and 44 ONE X2 checks with zero
failures. Both exercise actual opening, zero-copy playback, the original
reported starting views, backward seeking, the real late-content scrubber,
controls and error surfaces; ONE X2 also passes all four paired-file arrival
paths. The harness's additional shader/Rust-twin test is compiled and run
natively from the same source, not extracted from the installed package.
Both sandbox runs retain the previous volume-popup skip because the isolated
PulseAudio connection is refused, and the import-failure injection skip
because there is no preload into the sandbox. X4's paired-file cases do not
apply. These results do not qualify audio, sustained performance, 240 fps
capacity or the owner's moving flicker.

Build/install receipts and both complete sessions are preserved in
`scratch/flatpak-delivery-365cedf6/`; root inspected both installed
reported-view captures for picture/layout integrity. The previous `9e193956`
bundle and its evidence remain intact and their hashes reverify for rollback.
No files were deleted for this delivery. The owner still needs to judge the
new full-overlap moving comparison, not the previously rejected camera-datum
candidate. Nothing was merged, tagged or released.

## Owner rejects the full-overlap flicker candidate, 2026-09-09

On the new full-overlap candidate-left/Studio-right moving comparison
(`016dddc5795192647c4c4c177ff0e91edbcc5845b8cf6762b62aeb1926d51568`), the
owner reports "Yep still flickers". This rejects source `365cedf6` as a
visible flicker fix. The saved native prepared-image agreement authenticates
the missing-equations correction, not the reported temporal defect. Neither
test coverage nor installation changes this verdict. The candidate is not
accepted for merge; no automatic rollback is inferred.

The next bounded discriminator is the already captured changing color maps
and their final consumption. In particular, the native/reference outer-stage
residual and the real Scene's input sequence are not yet excluded as causes.
Do not repeat the rejected sampling/mesh trials, infer success from update
counts, or replace this evidence with another export or invented smoother.

## Periodic prepared-image join and native-input control

The next Mac read identifies another omitted operation, after MGP and before
ratio construction. The selected u8 branch calls `FuseLeftAndRightSide<u8>`
at `0x321dad4` and `0x321daf4`, independently for the two prepared lens images;
ratio construction follows at `0x321db1c`. The callee at `0x32312d0` clones
the two six-column extensions and center-crops columns `[6,206)` into a
separate 200-wide destination. It then joins the extensions into the cropped
edges, rather than discarding them. For rows 40..60 inclusive and j in 0..6:

- Destination j mixes original prepared column 6+j with 206+j, using cropped
  weight `0.5+j/12`.
- Destination 194+j mixes original column 200+j with j, using cropped weight
  `1-j/12`.

The other weight is formed as `1-cropped_weight`. The scalar u8 three-channel
path uses an f32 multiply for the extension term, an FMA for the cropped term,
then truncation toward zero, not ties-to-even byte rounding. The source edges
are clones of the separately solved extensions, not the opposite cropped edge.
Rows outside this interval are just the center crop. Addresses
`0x3231300..0x32313a0`, `0x3231444..0x3231598` and
`0x3231808..0x32319ac` establish these bindings and loops. The selected caller's
default p=0.2 gives row bounds 40 and 60 at `0x321da2c..0x321da5c`.
An independent read confirms the source/destination ownership, u8 selection,
indices, weights and conversion. The 3x11 ROI blur and periodic/reflected
boundaries remain unchanged.

Adding this operation to the readable reference reduces the six same-native-
input final map maxima to at most 0.000006795, versus 0.00085..0.00255 before.
Both the full reference and native-prepared substitution agree after this
change. An independent saved-output replay localizes the former error to the
remapped working-chart edge columns; temporal error changes also fall below
0.00000442. This closes that measured outer discrepancy, not the owner's
flicker. Rust outputs are in `april-native-periodic-fusion-01/`; the independent
script and report are `april-native-full-overlap-01/analyze_outer_edges.py`
and `edge-fusion-analysis.json`. The ordinary GPU producer deliberately remains
unchanged while the next diagnostic distinguishes the larger input-history
difference. This is not a completed GPU port or a newly installed fix.

The test-only `native-color` Scene review consumes the original 15 native
observations through the corrected reference, including their admission holds
and warm history. It requires a contiguous zero-based native prefix, an admitted
100-budget cold solve, finite exact-size payloads, and both published maps at
every ordinal. Before drawing, each native-chart result must agree with its
saved native map to 1e-5; the camera-chart reference must make identical
admission/solve decisions. That tolerance is a diagnostic identity guard,
not a visible-quality acceptance threshold. The camera reference performs the
existing chart/lens conversion, avoiding an invented resampling of native maps.

The actual Scene then processes sources 18208..18222 normally at the reported
April view and draws a separate control with those replayed coefficients.
The original source/frame, packed geometry, alpha and ordinary producer remain
unchanged. Every diagnostic baseline is checked against the actual Scene pixels
within one byte, and the installed Scene map/ratios remain unchanged after the
control draw. All 15 sources complete; all 30 geometry/alpha artifacts match
the earlier same-start capture byte-for-byte. This is a native-like coefficient
sequence substitution: it changes the sampling/admission history and includes
the recovered periodic join, so a positive verdict alone would not isolate
which difference matters. It does not freeze coefficients or add a smoother.

The ordinary sequence updates at 18208/18215/18222; the native-input arm at
18208/18215/18221. Even at the first two common updates, maps differ by maxima
roughly 0.0125..0.0222, with different local amplitudes, not only timing. Rendered
same-frame differences peak at two codes through18220 and three thereafter.
These differences are not a flicker verdict. The native source association
remains timestamp/trim-derived, and the native capture remains type2 diagnostic
output, not proof of the accepted type11 panorama's internal sequence.

Evidence is in `april-native-input-control-01/`: exact source patch/hashes,
test executable, ordinary and control pictures/ratios, completion log and
unchanged installed identity. Test executable SHA-256:
`6a972f8e60bc4a11900f9287f707023bdc000aebb4a1204459786b0effd6b385`.
The moving review uses the 14 sources18209..18222 shared with the accepted
Studio movie, after both Kjerag arms consumed cold source18208. Each loop
repeats those 14 sources 16 times (224 encoded frames, 30000/1001 fps), not
224 distinct observations. Root inspected frame6's actual pixels and labels;
stills do not establish moving quality. The subsequent owner verdict is
"both side have it. why cant you see it?": both ordinary and recorded-input
Kjerag arms still flicker. This rejects the control as a visible improvement;
native coefficient agreement does not establish the accepted panorama's
temporal output. The owner approved testing a temporal output discriminator
against these negative labels and the previously stable Studio, color-off and
fixed-first-color controls before another speculative correction.

- `review/own-vs-native-inputs-loop-muted.mp4`: ordinary Kjerag left, Kjerag
  with recorded native color inputs right; SHA-256
  `03e1723395ee5742d245b1147fbf3efcf77de34785ef8bdb91484bae82a114f1`.
- `review/native-inputs-vs-studio-loop-muted.mp4`: native-input Kjerag left,
  existing Studio comparison right; SHA-256
  `f1506cacfeaf9706602d6e4e1b98002d063e2dc4d6dc058064d1cb34fa96468a`.

Focused CPU color tests pass 55 cases, four opt-in probes ignored; the native
prepared replay and actual Scene native-input control were additionally run
explicitly and passed. All-target workspace Clippy passes. This increment has
not rerun full workspace/GPU/UI gates and does not qualify a new player build.
The installed Flatpak remains source365cedf6. No new Studio export, installed
app replacement, merge or release occurred.

## Owner-labeled temporal output localization, 2026-09-09

The owner rejects both native-input control arms as still flickering. The
coordinator explicitly acknowledged that extracted stills and coefficient
numbers had not established continuous-playback perception. The owner then
approved building a check against the existing labels before another candidate:
original, full-overlap, ordinary-short and native-input-short Kjerag flicker;
Studio, no-color and fixed-first-color are the owner-stable controls. Stable
does not mean the controls' remaining line or color is an accepted final result.

The new measurement localizes color-update pulses in actual rendered output.
It is **not a universal flicker classifier or a merge threshold**. It uses
existing footage only and changes no player code or photometric policy.

### Measurement and controls

All evidence below is under `scratch/studio-seam-ab-20260908-01/` in the
authoritative worktree, not `/tmp`. `temporal-probe.py` measures the original
31-source controls and 14-source native-input controls. It estimates backward
dense texture motion from the no-color picture, transports the previous
picture into the current one, and measures the remaining signed RGB change.
The same motion is used for all Kjerag arms; Studio estimates its own motion.
Only consecutive unique source images enter the calculation, never encoded
loop resets or title pixels.

Locations are the computed alpha-trace anchors at sources 18209/18224/18239,
sampled at screen rows 150/300/510 and thirteen separate horizontal offsets
-96..96 in steps of 16. Between anchors, piecewise interpolation is explicitly
navigation, not an authenticated new alpha trace. The third full-overlap
update at 18239 is on an exact trace anchor. Each 17-high, 9-wide local sample
retains all three signed channels separately after Gaussian spatial averaging
at sigma 4, 8 and 16 pixels. Opposite sides and channels are never pooled.
These are diagnostic spatial scales, not proposed production smoothing.

`temporal-probe-robust.py` repeats the equal first-14-source comparison with
independently estimated motion for every arm and a shared intersection mask
requiring forward/backward consistency within one pixel. This prevents an
arm from improving just by omitting its own unreliable pixels. Re-estimating
motion and masking preserve the observed ordering at all three scales.
`test-temporal-probe.py` passes three mechanical nulls: exact known transport,
identical-frame zero innovation, and opposite-sign color pulses remaining
separate even though their whole-image mean is zero. The original duplicate
arm also matches exactly; no-color's color-only residual is exactly zero.

The first attempt to remove the comparison movie's additional compression
used ffmpeg trim without timestamp passthrough. The coordinator caught a
duplicated first frame in the receipt. That `april-temporal-studio-raw-01/`
sequence is preserved and marked invalid; no completed measurement used it.
The corrected `april-temporal-studio-raw-02/` uses the original registration
script's sequential OpenCV decode and exact retained projection, with no fit.
All 31 saved PPMs match a second independent decode/projection pass exactly,
there are no adjacent decoded duplicates, and the first image matches the
retained Studio anchor exactly. This removes extra A/B encoding, not the
Studio export's own compression. The inherited source association remains
index-derived, not independently authenticated.

### Results and the deliberately rejected scalar interpretation

`april-temporal-detector-04/measurements.json` contains the final equal-length
self-motion/common-mask run using the corrected lossless projection. At
sigma 8, the maximum local temporal RMS, in 8-bit RGB codes, is:

| Owner label / arm | Measured local RMS |
| --- | ---: |
| Flicker: original Kjerag | 1.475 |
| Flicker: full-overlap | 1.325 |
| Flicker: ordinary short | 1.409 |
| Flicker: native-input short | 1.261 |
| Stable: no-color | 1.092 |
| Stable: fixed-first-color | 1.070 |
| Stable: Studio | 0.550 |

This ordering agrees with the owner's labels, but **is not an acceptance
score**. Different arms peak at different places and transitions. Some peaks
are source-motion residuals also present in stable controls. Removing source
18222 alone nearly erases the earlier full-overlap/ordinary-short scalar
margin. The simpler color-only RMS also fails to order all owner labels.
Neither scalar is selected as an autonomous quality gate.

The useful result is the event-localized check in `temporal-events.py` and
`april-temporal-events-01/events.json`. Independent saved left/right map
hashes identify updates rather than choosing events from output peaks. This
event report uses detector01's shared no-color motion, not detector04's
self-motion/common-mask residuals; the robust scalar ordering above and the
example below are separate checks, not a masked rerun of these inequalities:

- Full-overlap: 18215, 18222 and 18239.
- Ordinary short: 18215 and 18222.
- Native-input short: 18215 and 18221.

For each same-motion local RGB residual, subtract the fixed-color control,
then compare each location against that same location's largest held-frame
residual. Every listed update has coherent, same-sign excess across at least
three neighboring samples at **all three rows and all three scales**. Each
listed update independently exceeds its held-only envelope; deleting another
update does not change that envelope. This is descriptive, not statistical
leave-one-out validation, and neighboring smoothed samples are not independent
replicates. The independent static-field control, no-color minus fixed-color,
does not show those widespread update-linked runs. Its occasional local
exceedances remain in the complete report rather than being suppressed.
The held maximum is a descriptive within-sequence reference, not a tuned
perceptual threshold; held frames cannot exceed their own maximum by definition.

For example, at source 18215 / t607.773833, row300, offset -48, the self-motion
native-input output changes about +2.16 green codes; fixed-color is +0.10,
no-color +0.28 and Studio +0.42. Adjacent offsets -64/-32 show the same native
brightening. Full-overlap instead darkens that region at the same update.
Both are owner-rejected: matching pulse direction is not necessary to share
the reported flickering symptom. At source18222 the ordinary/full-overlap
update darkens the region again, while the native arm holds its coefficients
and its local result is close to the stable Kjerag controls. Native's other
coherent update is one source earlier, at 18221. Thus the instrument can locate
real output pulses and distinguish update timing without declaring that any
native-number match solved the visible defect.

### Boundary of this finding

This is enough to target particular color-update events in subsequent output
checks. It does not prove exactly which pulses the owner notices, explain
every pixel, authenticate the native type2 capture as the accepted type11
panorama's history, or justify freezing/smoothing color in production. The
next comparison must explain why these updates become visible in Kjerag but
are lower in the accepted Studio output. More agreement with type2 solver
numbers alone cannot close that question. No new export, GPU run, build,
installation, playback-performance claim or final fix occurred in this step.

Evidence hashes:

- Final robust measurements:
  `df16f219aef3e1b63246eb6c1e7e27012a8ebd1f3eed5baa2f0ca292fad75c47`.
- Complete map-tagged output events:
  `8b920230ee424b765a01eb9a4b8d26b352b5c03258e18c5efe0c87f188fc45dc`.

The final robust evidence directory retains its exact analysis/base source;
JSON receipts seal rendered inputs and map files. An unmodified actual
source18239 PNG was inspected for picture/location context, not a motion
verdict. The original moving A/B and the owner's negative verdicts remain
the quality authority.

## Bounded type11-style consumer control, 2026-09-09

The next experiment changes only drawing, keeping source video, geometric maps,
alpha and both ordinary/native-input coefficient histories fixed. Its
test-only shader combines three known consumer differences: analytic
per-fragment chart/map lookup in place of coarse-mesh lookup, a 1x1 source
box footprint in place of 1.7881766557693481, and explicit four-load float-map
interpolation in place of optional hardware filtering. It retains Kjerag's
view projection and draw ownership; it is **not** a complete reproduction of
the native `map_plane_uv`/`tex2DBiLinear` implementation or accepted panorama.
The original-chart ratio lookup remains before half-texel packed-map/alpha
centering. Correction still applies `clamp((RGB+1)*own_ratio-1,0,1)` per lens
before the alpha blend.

CPU tests validate the diagnostic WGSL and analytic chart at every interior
mesh vertex. Default shader sources remain exact. Three actual-Scene runs
cover sources 18208..18222 in the reported April view:

- `april-type11-consumer-null-01/`: opt-in absent. Every ordinary/native PPM,
  packed map, alpha and both ratio pairs match the preceding saved control
  byte-for-byte.
- `april-type11-consumer-control-01/`: alternative drawing. Every packed map,
  alpha and both ordinary/native ratio histories remain byte-identical.
- `april-type11-consumer-fixed-02/`: identical alternative drawing, holding
  the first ordinary ratio pair from source 18208. Ordinary PPMs match the
  preceding alternative run exactly and the first fixed draw is an exact
  same-frame null. This is a mechanical negative, not the owner's earlier
  stable fixed control initialized at 18209.

The first fixed run stopped before rendering because its runner omitted the
existing required baseline; that test mode also hardcoded 31 sources. It is
preserved as incomplete in `april-type11-consumer-fixed-01/INVALID.md`.
The corrected test-only count and baseline setup produce the completed 02 run.
Each successful run retains the source patch, source/binary hashes and guards
showing the installed Flatpak unchanged. No playback performance claim follows
from an offscreen capture. An unmodified alternative source 18215 PNG was viewed
for picture context, not continuous-video acceptance.

`type11-consumer-temporal.py` compares these lossless frames with the previous
ordinary/native-input and owner-labeled fixed controls. Each arm estimates its
own texture motion; a common forward/backward-consistency mask retains
96.14..99.52 percent of the full image. Signed local RGB samples stay separate
at the previously defined rows, offsets and scales. The old fixed control has
no source 18208, so old/new residual comparisons start at 18210. The new control's
18209 transition is reported separately. The different fixed initializations
are disclosed; raw residuals are retained as well as fixed-subtracted values.

The result is negative: both new changing-color arms retain same-sign,
three-neighbor pulses above their own held-frame envelopes at every map update
and all three rows/scales. Native events remain 18215/18221 and ordinary
events 18215/18222. For the previously localized source 18215, row 300, offset -48,
sigma 8 green residual, old/new native is +2.1579/+2.2004 codes. Old/new fixed
is +0.1030/+0.2197. The ordinary source 18222 pulse at that location is
-1.3067/-1.0762, with old/new fixed -0.1325/+0.0506. This does not establish
perceptual equality or a new owner verdict, but supplies no basis for presenting
the drawing experiment as a flicker fix. The same held-envelope and correlated-
sample limitations apply as in the preceding measurement section. All three
draw changes are combined, so this does not isolate each one's individual
effect or rule out every possible consumer difference.

The rejected test-only Rust draw and count override are removed; their exact
patches, test executables and pixels remain in the durable evidence directories.
The measurement is `april-type11-consumer-temporal-01/events.json`, SHA-256
`85214e16021e8cf2b8fefff1a3473e8385fe1498d94a6f0c70ca582e6eef9587`.
No production change, install, new Studio export or owner-review movie occurs
in this control.

### Accepted-output history gap

A read-only local audit confirms the accepted April panorama retains the
output and project, not its per-frame type11 color-map history. The available
15-source history is the later 1080p type2 diagnostic export. Its existing
video was copied from the Mac without a new export; local and remote SHA-256
are `dd22c4e6969e0fd6d9d11f05c3cba8ecf28486fa9dbd8a26d53f86d2797fd6c0`.
It contains 63 1920x1080 frames at 30000/1001 fps. Its inspected first frame looks
toward the horizon, not at the owner's downward ground-seam view, so that
video cannot replace the accepted panorama comparison. The separate recorded
type11 ratio pair at t1152.417933 establishes one upload, not the accepted
t607.574 history. Next target the actual panorama consumer's per-source map
uploads/holds rather than further numerical convergence to this type2 history.

## Actual panorama consumer history, 2026-09-09

The follow-up closes the coefficient-history distinction for the first 15
sources. `mac-panorama-color-01/` retains the source, records and output.
The existing active April alias project is byte-identical to the accepted
Direction-Lock-Off project before and after observation, SHA-256
`043620056970fb578305399604da8862bb5fad62469fc836dd1227fb6a1f9ef6`.
Studio's checked export controls select ordinary 360, 7680x3840, source
30000/1001, Original bitrate and H.265, with Anti-Flicker, Dolby Vision and
APMP off. A new 63-frame research export goes to a separate directory; the
accepted output is never overwritten. No new registration fit is performed.

The observer selects directly at type11 `UpdateMediaTextures`, worker offset
`0x4aa2d8`, using each consumer call's 56-byte `IdxTimed`. It verifies the pinned
arm64 worker hash before installing numeric probes. Recorded panorama caller
return PCs `0x47fba8` and `0x8ed76c` distinguish the exporter route; these are
**caller PCs, not function starts**. Root corrected that guard distinction
before running. Disabling LLDB's prologue skip ensures entry argument registers
are read at the actual entry. Renderer/vtable and same-thread call context
are retained with each source's shared `FrameTextureData` identity.

At the two previously authenticated upload BLs, `0x4aae58` and `0x4ab1d4`,
each selected entry supplies both exact 320000-byte, 200x100 float4 payloads.
The observer refuses absent/duplicate pairs rather than inventing a hold and
detaches only after the 15th complete pair. All 15 source identities match
18208..18222 on the native 30000 timebase. The capture completes with 9600000
payload bytes, then removes its breakpoints and detaches; the debugger exits.
The 63-frame export also completes. This is an observed CPU upload boundary,
not a readback of GPU texture contents or final-encoder frame association.

Every RGB coefficient of all 30 uploads is **exactly equal** to the same-source,
same-ordinal previous type2 diagnostic's `published-{0,1}.f32x3`, after BGR to
RGB conversion. The float4 fourth lane is not consumed. Both maps update at
sources 18215/18221 and are byte-identical on all other successive sources.
The peak ratio steps are about 0.0211/0.0217 at the first update and
0.0178/0.0157 at the second. The observed panorama path therefore does not
receive an extra temporally smoothed coefficient sequence in this prefix.
This does not establish the behavior of uncaptured downstream stages or every
camera/history, and does not close the owner's flicker defect.

`compare-output.py` projects new export frames 1..14 using the old fixed
registration and sampler, and checks sequential decode/no adjacent duplicates
and lossless PPM round trips. The projected source 18215 PNG was inspected for
context. The new export is not pixel-identical to the older accepted output:
mean absolute full-image differences range 0.96..2.05 RGB codes, with maxima
19..27. These aggregate differences are context only, not a quality judgment.
The unchanged `temporal-probe-robust.py` is also run against these 14 projections.
Its inherited `studio-stable` key does **not** label this new movie owner-accepted.
At source 18215, row 300, offset -48, sigma 8, the new Studio green residual is
+0.2108 codes versus the prior accepted export's +0.4203 and replayed-native
Kjerag's approximately +2.16. At 18221 it is +0.3632 versus the prior +0.1120.
The sampled output pulses remain lower here despite the identical captured
coefficient sequence. Neither this local measurement nor the aggregate RMS
is a perceptual pass, and the output-source association remains index-derived.

Consequently the next boundary is how those same corrections are applied and
combined with source pixels and alpha, including the still-unverified actual
GPU resource contents. More convergence to the type2 producer or an invented
temporal smoother does not answer this result. No player code, installation,
merge or owner-review candidate changes in this step.

Hashes (remote/local file copies agree):

- Observer: `463369ea1a80c10b4acc4dcb9f854e80f053865ee69450161d63b5688cab435b`.
- Native session: `5a5f935786cc40cd06be46acc0c1d1485f91ec3897db4cf8eea462d862041fed`.
- New panorama: `3bb92046b26b9d7772c84668b585fc13c1486910b6678aa33dc595627841e4ec`.
