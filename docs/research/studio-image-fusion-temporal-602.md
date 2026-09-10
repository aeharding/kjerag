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

## Bound textures and exact texture-coordinate rebase, 2026-09-09

The next bounded observation selects source18215 at the type11 consumer,
native ticks18233215/30000. `mac-panorama-consumer-03/run-01/` retains the
exact source record, CPU packed/alpha/color uploads, submitted FS/Texture
uniforms, actual fragment texture bindings and four Metal `getBytes`
readbacks. The renderer, source thread, exporter invocation/CFA, private
texture-array pointers and apply-call stack identities are checked together.
The selected exporter project remains the accepted Direction-Lock-Off setup.

The recovered ordering matters: `UpdateMediaTextures` supplies the source;
the later same-exporter `UpdateParams` applies texture bindings **before**
FS uniforms. `TextureParam::apply` is a sibling after UpdateParams returns,
not its descendant. The final all-stop is OnRender `+0x4d0998`, after that
uniform upload and before the next virtual draw call at `+0x4d09a8`.
Stale global uniforms or an ancestor requirement on UpdateMediaTextures would
not authenticate this draw.

The full observer stops with one explicit missing-slot9 fatal. A separately
recorded **core-only** readback then admits slots0..3, not a silent full-pass
override. All four are 200x100, one mip, managed storageMode1; slots0..2 are
RGBA32Float/125 and slot3 is R32Float/55. Each readback is byte-identical to
its corresponding CPU upload:

| Slot / payload | Bytes | SHA-256 |
| --- | ---: | --- |
| 0 / left RGB ratios | 320000 | `dd3c9cafcf5bd298123966004764c21e0ca81c0f0d62ab0cc0c0e4fd65540498` |
| 1 / right RGB ratios | 320000 | `c3a312716902796f64b4fb3d4e64a916665951ead82917aea42ca111fbc88565` |
| 2 / packed UV | 320000 | `88ecedc4b878c07df635da82fd02f91c2b361bcba6a3832d3486f211b439b421` |
| 3 / left alpha | 80000 | `721ae03df12faec7b0a7766028038c3972112515524d59d22ef800b8014d578d` |

This establishes CPU-visible contents of the exact bound objects, **not**
managed-resource GPU coherence, slot9's fisheye texel, the final framebuffer
or encoder-frame identity. The submitted uniforms disable separate color
adjustment and cubemap output, give the source filter a 1x1 footprint and
the fisheye mask a 1x1 size. The ordinary shader corrects each own lens with
`clamp((RGB+1)*ratio-1,0,1)` before blending. It reweights alpha by fisheye
coverage; a uniformly sampled 1x1 value cancels, but the missing actual texel
and sampler binding are not asserted as authenticated.

`mac-panorama-consumer-01/run-01` is an earlier partial attempt that missed
the texture array because it assumed the wrong apply ordering. Its run02
armed a later source but observed none after LLDB command replacement failed.
Neither is combined with the successful core receipt. The initial Metal
expression also failed to compile; the completed core uses the SDK's exact
48-byte region layout and an explicit void-cast Objective-C message. All
debuggers detached and quit. Two new research exports were used in this
consumer investigation; no new export is used by the tests below. The locally
copied run03 movie is not output evidence because copying overlapped export
completion. Core event receipt SHA-256:
`0208c53febf8d4ac38f99fe85620eb1c8992e6791db3819f21d02dfadac57d26`.

### Camera basis versus texture centers

Normalize native packed atlas X to lens-local X before checking lane order:
`2*x` for the first lens and `2*(x-.5)` for the second. The source18215 packed
map supports the existing physical lens exchange and reflected camera chart;
the valid-lens mean UV difference is about0.000293. An initial contrary
same-lane result omitted the atlas offset and was rejected before a code
decision. The rotation basis is not the newly identified error.

For ratio textures the consumer transform is
`uv_native=((.5-u_K) mod1,1-v_K)`. Bilinear **texel-center** sampling therefore
commutes with this exact texture permutation:

```text
K_left [r,c] = native_right[99-r,(99-c) mod200]
K_right[r,c] = native_left [99-r,(99-c) mod200]
```

Recomputing `Ry(+pi/2)` through the producer's endpoint lattice instead of
permuting the final native texture is not that operation. At source18215
the maximum coefficient discrepancy is0.021325; for the 18214→18215 update
the maximum delta discrepancy is0.021725. At that peak, the current
camera-recomputed node changes while the exact-rebased native node holds.
The asymmetric synthetic Rust test checks the complete permutation and
bilinear equivalence, including wrapping/clamped endpoints. A shift100 ratio
control is not equivalent. Centered packed UV and alpha instead require
shift100; alpha also complements physical ownership.

The explicit `KJERAG_REVIEW_NATIVE_COLOR_EXACT_REBASE=1` test mode reads the
authenticated published maps after retaining all native replay validation.
It changes only diagnostic drawing, not the production coordinates/producer.
`april-exact-rebase-null-01` and `april-exact-rebase-01` each process15 actual
Scene sources18208..18222. Ordinary pixels, packed maps, alpha and ratios
are byte-identical to the prior control; the null's native-color outputs are
also identical. All30 exact diagnostic maps equal the exact permutation of
the corresponding actual panorama uploads, bit-for-bit.

The previous self-motion/common-mask output check, with the additional arm
explicitly marked unreviewed, gives source18215/row300/offset-48/sigma8 green
residual2.15793→1.38491, versus Studio0.42033 and fixed-color0.10301. At18221
the same location is1.18125→0.54839 versus Studio0.11196. Other local residuals
remain, including increases in some bins. These values do not establish
uniform improvement or perceptual acceptance. An unmodified source18215 PNG
was inspected for context, not a motion verdict. Exact-rebase measurements:
`96b3b19924ac49889d29b27746193c286da9a70896e1ec0d50f7928ff6dace5c`.

### Frozen native alpha and same-source coefficient counterfactual

`april-exact-native-alpha-01` keeps those exact ratios and adds only the
source18215 captured native alpha, converted as
`K[r,c]=1-native[99-r,(100-c)%200]`. Holding this one map across15 sources is
explicitly a **frozen proxy**, not an observed native alpha history. The test
requires X4/exact-native-color mode and rejects wrong-size, non-finite and
out-of-range payloads. Every original ordinary/exact-native picture and map
is still byte-identical to the preceding capture; the installed app is
unchanged. First-frame current/previous coefficient draws are exact nulls.

The frozen alpha does not eliminate the first localized temporal pulse:
its green residual is1.37879 versus1.38491 with Kjerag alpha. More directly,
two extra arms apply **previous and current ratios to the identical current
source and packed map**. This subtraction needs no optical flow or cross-frame
source-image comparison; selecting the prior coefficients still relies on
their authenticated source association. At18215/row300/offset-48/sigma8, the signed RGB increments
are `[.79140,1.07076,.62048]` with Kjerag alpha and
`[.79722,1.08827,.64506]` with captured native alpha. This isolates a real
coefficient-update effect not removed by native fractional blending there.
All13 held transitions/initial null are exactly zero. At18221 the same-source
green increments at this location are only−.01987/−.02108, so the raw temporal
residual there must not be described as all color-update pulse.

The first analysis attempt refused a relative input path before reading
diagnostic frames; `april-same-source-color-update-01/INVALID.md` records it.
The corrected02 analysis retains the full signed rows/offsets/scales and
input hashes, SHA-256
`3ada1deff87811c2f75a72ee4932470ae5f774bdc63b5f36787b6e8434f92e26`.
Frozen-alpha temporal measurements SHA-256:
`ca7aeb9ce0fcd6894bb16114ebf7616cbe9b8d9daf01eeb5a8b57e172c945612`.
These remain diagnostics, not another presumed fix for owner testing. No
production smoothing, player change, installation, merge or performance claim
occurs in these controls.

### Corrected pixel/view association and selected native V

`april-view-coordinates-01` retains the actual prepared `Reframe` and a
9-f32 view-to-body matrix for every source in the unchanged Scene diagnostic.
The matrix is constructed by calling `body_ray` on the three unit axes, not
by guessing uniform offsets. All prior ordinary and exact-native images and
maps remain byte-identical. Source18215 matrix SHA-256:
`19bff679dca4708395f851c153e4a034f5d4d39a5d2a6be9b305178b7a1f9623`.

The first `native-consumer-coordinate-01` analysis selected row300 but used
row150's center872.2. The shared trace helper actually gives centers
`[872.2,732.6,537.4]` at18215, so row300/offset-48 is pixel(685,300),
not(824,300). Its `INVALID.md` rejects that earlier reported-bin conclusion.
The coordinator also retracts the resulting apparent opposite-sign or
different-correction-region claim. This error is separate from the verified
texel-center discrepancy above.

Native type11 uses `BuildPlane`, not the common `FullScreenQuad(bool)`.
`InitRenderResource` at worker+`0x4a880c` calls BuildPlane with identity UV
transform at+`0x4a8884`. BuildPlane+`0x4f3ae8` maps row fraction to object
`y=2*(row/H-.5)` and texture V=`row/H`. The selected VS forwards that UV and
applies MVP to position. All ten older exact-class submitted VS receipts
have no X/Y transform; Metal raster top therefore receives V=1, not V=0.
The established panorama path uses native V=`1-top-left-image-V`. Run03
did not retain its own VS submission, so this is static plus earlier
selected-class evidence, not exact-run VS authentication.

`native-consumer-coordinate-02` imports the common trace helper, verifies the
centers/pixel, and uses the actual K matrix. Kjerag's view ray has Y down;
the existing registration helper uses Y up. With those conventions explicit,
native fusion UV at the selected pixel is(0.2390540,0.3858023), versus
(0.2391438,0.3860701) from Kjerag. The composed bridge has determinant
0.9999997 and differs from `diag(1,-1,-1)` by at most0.001098. After atlas
normalization and stream exchange, the physical lens UVs also agree within
the existing registration discrepancy. There is no evidence here for a
second camera-basis correction.

Native left owns the center with alpha1 and its green-ratio increment is
+0.00347458. A native-left-only approximate sigma8 patch effect is+1.04113
green codes, consistent with Kjerag's actual same-source+1.07076, not an
opposite sign. This estimate uses encoded panorama color and omits the
separate right-lens term where patch alpha reaches0.9215; it is not an exact
native framebuffer replay. A zero coefficient-update delta would therefore
not be a justified parity requirement. The corrected initial02 receipt is
`050d05e87543ddd1d6b6d8de4a1a319ede43584641c48b9deffefbf0708b2422`;
its then-pending static-V note is resolved only to the limited extent above.

### Production texture rebase and GPU periodic-edge join

The branch now applies the exact X4 output-coordinate permutation in
`coordinates::for_camera_output`. Source-band coordinates, physical stream
exchange and ONE X2's native coordinate table are unchanged. The table is
prepared once for the capture and shared by the readable and GPU paths; no
per-frame remapping pass, smoothing or new history policy is added. Nine
coordinate unit tests pass. The real GPU X4 test now requires all20,000
nodes/all4 lanes to equal the native opposite-lens permutation bit-for-bit,
including poles and meridians previously excluded by its approximate check.

The sealed pre-change `april-view-coordinates-01/render-tests` exposes an
existing GPU/reference failure: the X4 fixture's maximum ratio error is
0.0017317533 and ONE X2's is0.0030160546, above the unchanged1/510 tolerance.
The reference already contains the recovered periodic join, whereas GPU
`make_ratios` still used the unjoined prepared image. This failure predates
the output-coordinate patch.

The GPU port corrects cropped and extension bytes separately, joins the six
columns at each edge with the recovered row40..60 bounds, extension indices,
FMA order and truncation, then constructs ratios with the unchanged current
denominator. It adds no dispatch, buffers, admission or temporal state. On
RADV PHOENIX/AMD760M the X4 and ONE X2 fixture errors become0.000001013279
and0.00008547306. All eight GPU test entries pass; the optional profile entry
returns early because profiling is unset. Synthetic cold/warm, failure/skip,
full-overlap and exact X4 permutation checks pass without relaxed tolerances.
These are arithmetic checks, not a visual or performance gate.

`april-color-coordinate-only-01` and `april-color-coordinate-edge-01` each
run15 sources18208..18222 through the actual Scene with separately sealed
binaries and source patches. Original executable instructions/read-only data
are compared against the stripped evidence executable before each run.
Both finish with15 worker-ready wakes, unchanged packed maps/alpha and
bit-identical exact-native diagnostic pictures/maps. Production color maps
retain the original update/hold sequence: initial18208, updates18215/18222.
Adding the edge join changes map values but none of these15 view pictures;
the joined and coordinate-only PPMs are byte-identical. No installation changes.

The existing robust temporal check is rerun on both production sequences
with the same lossless Studio projection, motion policy and shared masks.
An optional `--no-snapshots` only omits regenerable dense arrays to conserve
disk; it retains every measurement and input hash. At row300/offset-48/sigma8,
ordinary old/new green residuals are0.54307→0.23034 at18215 and
-1.30673→-0.99610 at18222. Studio is0.42033/-0.21307 respectively. At18222
the new red residual remains-3.09778 versus Studio-0.35199; fixed-color is
also-1.86299 there, illustrating why total residual is not all color pulse.
Some other bins still differ. Neither lower numbers nor closer GPU/reference
agreement is treated as a flicker pass or a reason to override the owner.
Measurement SHA-256s:

- Coordinate-only: `12153f1f5aa54fc22a1450fa628ff19bc2eddf5a4d4cae2987f4858609ed6887`.
- Coordinate plus join: `eb3be703bb1570db26cb725452c322fa8fccb4fe873e1e5f16b283e0b9731232`.

A focused GPU regression dispatches the production ratio stage with distinct
solved extension bytes: cropped100 and extension111 at the half-weight edge
must give truncated105, not unjoined100 or rounded106. This passes along with
the full workspace qualification in `color-coordinate-qualification-02`:
1,282 passed,34 ignored; all-target workspace Clippy, formatting, name and
crate-source checks pass with required GPU and both real cameras. Tests log
SHA-256 `b7df349046de129c68f8b2bf22e688b3279a15c8203a497d0e3ca11b08d02dc8`.
Attempt01 ran out of disk during Clippy and never reached tests; its
`INCOMPLETE.md` explicitly rejects a gate pass. The retry uses the established
compact test-profile settings. Only regenerable compiler artifacts were
discarded. Seven older sealed debug executables were losslessly archived to
adjacent `render-tests.zst` files, each checked against its original decompressed
SHA-256; `EXECUTABLE-ARCHIVED.txt` beside each gives exact restoration steps.
No captured source, maps, output pictures or executable bytes were lost.

`april-color-coordinate-edge-01/review/coordinate-vs-studio-loop-muted.mp4`
shows production Kjerag left and the unchanged fixed Studio projection right.
The14 shared source-associated frames repeat16 times, at30000/1001 with no
audio; labels are outside the original1280x720 pixels. Frame6 was inspected
for layout/context, not motion acceptance. Movie SHA-256:
`737b9f3441c2db793c1eff07777f976fbc8c6088f699ef16de2d0d7ccc1e31b1`.
It has been linked for owner review without claiming the flicker is fixed.

### Native and installed-player qualification, 2026-09-09

The implementation checkpoint is `f5be77cd6f8d8d1adc1d024f9b915ccd3fdd323b`.
`color-coordinate-native-01` builds that exact source in release mode and
retains the executable before running the isolated UI harness at the reported
April607.574/yaw-76.84/pitch-55.87/fov108.79/lock1 view. All50 native checks
pass, including the exact held frame, backward seeking, late-content scrubber,
audio-control and injected import-failure handling. The captured window was
inspected for context, not a temporal flicker verdict. Native executable:
`cbb18671a38fb0505eab8e74988afeb13bca1f84382a7b5c0e6ba9d00c15665e`.

After retaining the final qualified renderer test executable byte-for-byte in
`color-coordinate-qualification-02/render-tests`, the disposable Cargo debug
profile was removed, recovering10,947,309,568 bytes. This was not a removal of
source, native captures, movies or sealed research evidence. Release output
was retained. Other debug build artifacts can be regenerated; the retained
test executable and its hashes allow rerunning the completed renderer suite
without rebuilding. The earlier seven research binaries remain in their
SHA-verified lossless archives. This creates working room for packaging rather
than repeating a build into a full filesystem.

`scratch/flatpak-delivery-f5be77cd` archives the exact source commit and builds
with the cached25.08 SDK/offline dependency inventory. The package links
`libavcodec.so.61` and runs its version query inside the sandbox. Installation
replaces the previous rejected365cedf6 candidate, not a main/release update.
The installed permissions metadata and executable match the package exactly.
Both remain unchanged through the complete installed-ID qualification:

- X4 at1153.452/yaw132.05/pitch3.55/fov63.63/lock1:40 checks,0 failures.
- ONE X2 at212.512/yaw71.13/pitch-13.99/fov57.95/lock1:44 checks,0 failures,
  including four paired-file delivery forms.

Sandbox audio-control and preload-injection checks explicitly skip, as in the
previous installed harness; they are not silently counted as passes. Captured
reported-view windows were inspected, not used to infer motion quality. Both
steady playback reports show30.00 fps with zero dropped/starved frames, with
worst lateness14.7ms X4/14.8ms ONE X2. The low partial-interval report after
pause/seek is not a sustained throughput measurement. These reports do not
establish the240 fps active-rendering capacity target or human smoothness.

Receipts:

- Installed OSTree: `e0b140d99e9598ac03187ed964fe85d9acd62db4ec71b2bc5a6d10d477a53f75`.
- Installed executable: `f89d46a2526585785bd67c24f66caa6a20fc87a019ee1f0b297100439af36a9b`.
- Bundle: `b22edf3e71365129e73d9cc08639d28a8fda58db183679929c2e4b2eeabafe66`.
- X4 UI log: `a4de3489553d1a745c870eb3162b5aad04a52b281321b40da846b1df58e838b0`.
- ONE X2 UI log: `cdc9a3b60f2a514698c28a5f62ec0a4b33d190d3a6f33ac2509f7b40496c2d2d`.

At this qualification checkpoint the coordinate-corrected movie and installed
candidate had no owner verdict. The subsequent installed-candidate rejection
is recorded below. No flicker-free claim, merge or release follows from these
functional checks; the active visual objective remains incomplete.

## Installed candidate rejected at a new April view, 2026-09-09

The owner reports "Yes still flicker" on the installed `f5be77cd` candidate,
with April004 at612.078/yaw-80.71/pitch-48.46/fov95.45/lock1. The implemented
coordinate and periodic-edge corrections are not a fix for the reported
visible defect. This verdict concerns production coefficients, not the
test-only exact-native-rebased coefficient arm, which still has no motion
verdict.

`scratch/studio-seam-flicker-612-20260909-01/seek-components-01` retains31
actual Scene sources18344..18374, source times612.078133..613.079133,
ordinary packed maps, alpha, both published ratio textures and the ordinary,
no-color, no-flow and individual-lens pictures. It runs the sealed renderer
test executable from the full qualification above, SHA-256
`efeae910f92d94daea336af0bbf7dd6d2e8d662ab3fdf7e49b3a2cd70f687b41`.
The source/binary/installed-app guards pass and the test completes with31
worker-ready waits. Test log SHA-256:
`99230ec8fc741aa9696a0dbbe03bd4e510fb5ad6ae28ecb95aab8382ba53ebe3`.
This is a fresh exact-seek history, not authenticated uninterrupted playback;
the owner has been asked which history led to the report. No production code
or installation changes accompany the capture.

The local Studio coverage audit finds no trustworthy movie for this new
interval. The accepted `april-studio-direction-off.mp4` has63 frames, with
the saved trim starting at18208. Its justified index-derived interval ends
at18270/time609.609000; source18344 is74 frames beyond that endpoint.
Other saved April panorama exports use the same trim. The older verified
1152..1156-second export and intermediate project states do not supply
missing612-second output. This coverage check does not upgrade the earlier
index-derived association into an encoder-PTS authentication.

The new `seek-components-01/review/current-vs-no-color-loop-muted.mp4`
shows current Kjerag left and identical source/geometry with color correction
bypassed right. It preserves1280x720 per arm with labels above the footage,
31 frames at30000/1001 repeated12 times, and no audio. It is an isolation
control, not a candidate fix or Studio comparison. Movie SHA-256:
`05fa6caec7a17a09a2c805f99b8efbf9daaa4e8b4cf05a8c41b57d88b7209da7`.
The first actual frame and a computed trace anchor were inspected for
view/context, not temporal acceptance. Owner motion feedback is pending.

The saved ratio hashes identify updates at18350/18355/18358/18360/18362/
18370/18372, with exact holds between them. Three new-view computed alpha
trace anchors come from the retained `stitch-layers` helper and these exact
saved maps. At row360 their marked spans are612..616,547..551 and631..635
for18344/18359/18374 respectively. The helper's detached ray pictures are
not accepted as exact Scene replacements:307..382 pixels exceed one code,
with maxima19..27. Only its computed alpha trace is used for navigation;
all temporal picture comparisons use the actual Scene/component captures.
The bounded `analysis-01/receipt.json` records this limitation and update
inventory (SHA-256 `8b2879d3e9c309a0c0e68f8cf3e2f0d95abc08639726bf3c3d291900a148977c`).
Its raw adjacent-frame RGB averages include source motion and pool channels
and trace pixels. They do not discriminate the reported flicker, establish a
new cause or contradict the owner's verdict. No fix is selected from them.

## Final CPU-visible output versus encoded movie, 2026-09-09

This bounded observation returns to the accepted earlier607-second view, not
the new612 report. Its purpose is to distinguish a stitch-consumer discrepancy
from an effect introduced after the final output callback. It does not choose
another production change or alter the owner's negative verdict.

### Capture and the failed downstream provenance assumption

The existing Studio main/exporter processes were reused without restart.
The original360 export settings were checked:7680x3840,30000/1001, Original
bitrate, H.265, with Anti-Flicker, Dolby Vision, APMP and Direction Lock off.
The saved project remains SHA-256
`043620056970fb578305399604da8862bb5fad62469fc836dd1227fb6a1f9ef6`
before and after. The normal Software Update dialog was closed without
installing an update. An optional screen-capture helper was denied by macOS
privacy; it was not retried or bypassed. Granted Accessibility controls and
the native export path remained separate. No playback audio was started and
the owner's system volume was not changed.

The hash-pinned worker observer retains exact type11 IdxTimed/map pairs for
sources18214/18215 and associates each by the active RenderExporter frame
and thread with its final notifier at worker offset`0x8ee474`. The earlier
`0x8ee46c` address is the argument setup, not the BLR. Both completed samples
contain a VideoToolbox AVFrame (format158) with a CVPixelBuffer in data[3],
7680x3840, format`420f`. The fixed accepted registration selects panorama
ROI x2244/y2756/w442/h306, covering view region[640,738)x[250,352) and the
existing event pixel(685,300). No registration or local warp was fitted.

Both read-only CoreVideo lock/unlock pairs return0. Actual returned luma and
chroma pitches are7680 bytes; logical ROI rows are copied verbatim twice and
match on each duplicate read. No lock or unlock uncertainty occurred. The
selected OffscreenRender flags at+8/+9/+0xa are[1,1,0], selecting the
ReadableTextureReader, not the Oryol reader. Static inspection finds its
source-draw wait before Core Image rendering, but no explicit post-Core-Image
fence. These are CPU-visible final output pixels, not a captured Metal
framebuffer or proof of all GPU resource coherence.

The export finishes all63 frames. The observer's final hook is hit63 times,
but `VideoWriter::AppendVideoSample` at`0xfa03a8` and the presumed encoder
call at`0x1e29fe8` are each hit **zero** times. Both captured final AVFrames
have AV_NOPTS_VALUE. Consequently the intended final-to-writer timestamp
authentication failed; no successful session manifest is synthesized. The
incomplete receipt records the exact hook counts and captures. After verifying
both balanced snapshots and no pending lock transaction, the debugger was
interrupted, checkpointed and detached. Studio remains running normally.

The bounded static follow-up identifies a separate F-writer family:
`FMediaFrameWriter::AppendVideoSample` at`0x8032f0`, F-input append at
`0x867e98`, and encoder call`0x868448`. It reads the sample's metadata time,
rescales it, and writes AVFrame PTS at`0x8033b4`/`0x803424`. These are
prospective observation sites, **not runtime-authenticated selected hooks**.
Any future observer must also permit the legitimate unset-to-assigned PTS
transition and authenticate wrapper changes while tracing the underlying
AVFrame identity. The hardware F-writer may wrap the original AVFrame in a
new MediaSample, so sample-pointer equality cannot be assumed throughout.
No further export
is started to close that link at this checkpoint.

A further bounded static audit finds no denoise or temporal filtering inside
either F-writer function. The optional CVPixelBuffer conversion is a
stride-aware plane copy through `av_image_copy`, not a filter. However, the
earlier MediaRender worker invokes an unobserved concrete virtual target at
`0x7edb14` (MediaRender+0x38, slot+0x10). Therefore the complete post-callback
pixel path is still not authenticated, and the writer audit does not license
an HEVC-only attribution. Its local `handoff-downstream-audit-01/REPORT.md`
is SHA-256 `e098490c31713d6b2230c34fc2774b86fce59f8dd31078e58399305de649c4cb`.

The durable capture root is
`scratch/studio-seam-flicker-612-20260909-01/mac-handoff-20260909-01`,
copied from the Mac repository with the same name. Relevant SHA-256 values:

- Frozen observer: `9ed70be6a095e72c19dc9fb3955b934dfd396ba0238f682e572bebe96f621472`.
- `run-01/session-incomplete.json`: `35225c3762a2244aff360b4a9db75c00e9673175d8ab217c08b7d19d8b79c957`.
- New63-frame movie: `0bbfdfb8fadfc9e19435e7581404b609752da2d0dacd6f75c07c777f7f322a76`.

### Pixel correspondence and same-conversion control

The local CPU-only `handoff-pixel-match-01` compares each captured Y/UV patch
against all63 decoded movie frames at the unchanged location, and a separate
vertically flipped candidate. Both luma and chroma independently select
movie6 for captured18214 and movie7 for captured18215. Combined NV12 mean
absolute differences are4.194/4.236 codes; the respective runner-up frames
are7/6 at6.527/6.787. Vertical-flip candidates exceed60 codes. Visual
inspection shows the same slanted ground/foliage structure and orientation,
with noticeably more high-frequency noise in the captured patch. This is
strong **inferred pixel correspondence**, not encoder-PTS authentication.
The crop streams retain every FFmpeg-returned NV12 byte; no color conversion
is used to rank them. Pixel-match receipt SHA-256:
`0beaad2ee2726b953ea011794b4358f30740c6468807639e845d305d7eb80eeb`.

`handoff-temporal-02` reuses the accepted rotation, existing event pixel,
sigma8 and17x9 patch without selecting a new favorable location. A common
motion field is applied to both arms; zero-motion and same-export-motion
controls are also retained. Its crucial conversion control runs the captured
and decoded NV12 through the **same** crop dimensions, full-range BT.709
conversion, and projection. A direct-Y arm bypasses RGB/chroma conversion.
The BT.709 interpretation comes from movie metadata; native CV attachments
were not captured, so it remains an explicit assumption for native RGB.

Using the exact-native Kjerag diagnostic's common motion field:

| Local signed event residual, codes | Studio captured output | Same export, decoded | Kjerag exact-native ratios |
| --- | ---: | ---: | ---: |
| Green, identical NV12 conversion | 1.273177 | 0.405090 | 1.384912 |
| Direct full-range luma | 1.222739 | 0.416803 | not compared |

The exported-minus-captured green delta remains about-0.87 codes with the
same-export motion field and-0.88 with zero motion. Thus neither a choice
of optical flow nor the different OpenCV/FFmpeg RGB conversions accounts for
this reduction. This comparison uses two different source pictures and still
includes motion residual; it is not a same-source correction counterfactual.
It also cannot identify which downstream export operation attenuates the
signal. The earlier ordinary-production arm has a different update history
and a lower residual at this particular event; do not present this one bin as
an explanation of all its owner-visible flicker.

Temporal receipt SHA-256:
`f91c44bccf70ed05ad1f9f00b24855b5f9dfd2ea1ec2024440e791982e913814`.
The preceding `handoff-temporal-01` retains the full-panorama OpenCV conversion
comparison;02 adds the controlled identical-conversion and direct-luma checks.
All analysis, raw patches and rendered context are worktree scratch, not/tmp.

### Decision boundary

For this localized event, attenuation occurs between captured CPU-visible
output and the exported movie. Calling it specifically HEVC suppression would
be premature while the selected downstream processing and writer identity are
unobserved. The accepted compressed movie remains the owner's visible target;
it is not by itself evidence that Studio's stitcher interpolates these color
updates. Do not assume missing stitch arithmetic explains a difference now
observed downstream of the final callback.

The new612 report, broader temporal behavior, actual installed playback and
owner acceptance remain unresolved. No smoother, codec round-trip, production
change, installation, merge or release follows from this two-frame finding.

### Actual downstream chain and incomplete second handoff capture

The same accepted project and63-source interval were used for
`mac-handoff-20260909-02`. Unlike the first run's wrong writer family, the
F-input hook at`0x867e98` and preencoder call at`0x868448` both received19
calls before detachment. These counts include unrelated inputs; neither hook
matched either selected final AVFrame. The final observer reached13 source
calls before detachment, not the whole63-frame export. Both selected final
Y/UV patches, sources18214/18215, are byte-identical to capture01, with
balanced read-only CoreVideo locks and duplicate reads agreeing.

Runtime vtables and Process targets identify the actual successor chain:

| Node | Vtable address-point offset | Process offset |
| --- | --- | --- |
| ImageAlgoNode, MediaRender's successor | `0x46bb238` | `0x7d32e4` |
| Defringe, internal head | `0x46ba968` | `0x7a7850` |
| BlockDenois | `0x46ba398` | `0x7968c0` |
| AlgoFrameEnd, internal tail | `0x46b9f78` | `0x7725a8` |

The internal list terminates with a null successor. These are instantiated
runtime nodes, not evidence of their per-pixel effect or temporal filtering.
No SequenceDenois or Deflicker node appears in this observed list. The result
rules out treating the route as an already-proven direct stitch-to-codec path;
it does not establish denoising as the cause of the earlier event attenuation.

The static ABI audit corrects two misleading interpretations. The return of
`ImageAlgoNode::Process` is status/error, not a replacement VideoFrameInfo.
VideoFrameInfo+`0x48` is FramePosition; its sample is at`0x80`. The retained
AlgoFrameInfo instead holds MediaSample at`+0x8`, FramePosition at`+0x28`, and
the original VideoFrameInfo at`+0x48`, each a shared-pointer pair. Its head call
at`0x7d37c8` receives the AlgoFrameInfo pair through`x1=sp+0x40`.
AlgoFrameEnd may materialize a new AVFrame and MediaSample, replacing the
sample with `SetMediaSample` at`0x77289c`, before its callback at`0x772960`.
Consequently original AVFrame identity cannot simply be presumed downstream.

A small separate supplement was armed while source18215 remained stopped at
the final notifier. At the image-chain head it matched that source's original
MediaSample/control **and** AVFrame/control/CVPixelBuffer, then recorded the
AlgoFrameInfo shared pair. Source18214's head was not captured. A later head
reused the18214 sample/control addresses but had a different AVFrame; the
guard stopped and rejected it. After preserving that rejection, head selection
was disabled, retaining only the authenticated18215 record. This illustrates
why stale sample addresses alone are insufficient source provenance.

The post-filter hook received six calls but never matched the sealed18215
AlgoFrameInfo pair. No post-filter or selected preencoder pixels were captured.
That absence does **not** prove object replacement: static dispatch inspection
shows retained AlgoFrameInfo shared pointers, while asynchronous execution and
the incomplete observation window prevent counts from establishing that the
selected source passed the callback. Do not relax the identity guard, infer
pixel filtering from the class names, or claim a final/post/encoder comparison.

The observer was interrupted and detached with all pending snapshots empty
and no uncertain CoreVideo transaction. The existing export subsequently
finished63 frames at7680x3840, duration2.102100s. No additional export was
started for the supplement. The project SHA-256 remained
`043620056970fb578305399604da8862bb5fad62469fc836dd1227fb6a1f9ef6`.
The live debugger was exited; no debugger remains attached at this checkpoint.

Durable worktree evidence root:
`scratch/studio-seam-flicker-612-20260909-01/mac-handoff-20260909-02`.
Frozen observer SHA-256:
`581cd2ae9e9be4f50afa29d1f46d881f18eec4a466fbbeada5cf218504afba10`.
Separate supplement SHA-256:
`a9d73f3d93007cd5ab0bdd2d33bafecc749f5635b2284256aae6dcec9310d589`.
`run-01/session-incomplete.json` SHA-256:
`5d9b2c1e85efc940b790c3f2a1bf67507469b5f80160289e544b81a37af968f4`.
The event log includes the raw node identities, successful18215 head link,
rejected address reuse and explicit incomplete detachment. The frozen original
records were not rewritten to substitute a different downstream frame.

This remains an unresolved final-output-to-post-filter pixel boundary, with
no Studio coverage for the owner's new612.078 view and no new player candidate.
The installed build remains rejected. No invented smoother or further broad
optimization reverse engineering is justified by this incomplete capture.

### Final-to-post-filter pixel boundary captured, 2026-09-09

The next bounded run, `mac-handoff-20260909-03`, reuses the unchanged frozen
observer and supplement, the same project hash, source interval and export
settings. The supplement is armed at the first selected final stop rather
than the second. Both sources18214/18215 match original MediaSample/control
and AVFrame/control/CVPixelBuffer at the ImageAlgo head. Their exact retained
AlgoFrameInfo pointer/control pairs, FramePosition and original VideoFrameInfo
then match at the AlgoFrameEnd callback. This closes the association even
though the post-filter MediaSample and AVFrame are different objects.

The earlier detachment was premature for a queued pipeline. The complete
static callback audit finds only the one normal AlgoFrameEnd callback site,
`0x772960`, not another already-AVFrame route. BlockDenois stores incoming
AlgoFrameInfo shared pointers in a queue and later associates results with
queued objects at`0x797208..0x797318` or`0x7979d4..0x797b1c`. The new run
actually observes both selected post-filter callbacks on the BlockDenois
thread. This is stronger than inferring selected-frame progress from unrelated
hook counts. A later duplicate head address was rejected; once both valid
head associations existed, further head collection was disabled.

All four selected post-filter planes were captured with duplicate logical-row
reads agreeing, read-only lock/unlock return values zero, no uncertain
transaction, format420f and unchanged7680x3840 dimensions. Both final-output
patches still exactly match capture01. The same-source post-filter patches
are different: Y mean absolute changes are3.680/3.767 codes and UV changes
1.768/1.786, with maximum absolute byte change10 for each plane. Those are
byte-domain differences, not a perceptual severity estimate. Inspection of
the actual saved post-filter patch confirms the expected ground structure;
it does not constitute a moving-video acceptance test.

`analyze-post-filter.py` and `handoff-post-filter-02/receipt.json` reuse the
unchanged accepted rotation, view pixel(685,300), sigma8,9x17 event patch and
common motion field from the exact-native-coefficient Kjerag diagnostic. Both
stages use identical full-range NV12 conversion. Native RGB still assumes
the movie's BT.709 matrix; direct Y bypasses that assumption.

| Selected signed event residual, codes | Final stitch output | Post-filter output |
| --- | ---: | ---: |
| Green, common texture motion | 1.273177 | 0.400292 |
| Direct luma, common texture motion | 1.222739 | 0.420283 |
| Green, zero-motion control | 1.207120 | 0.323580 |
| Direct luma, zero-motion control | 1.145891 | 0.330988 |

Thus the already-observed final/movie attenuation is present at the end of
the image-processing chain, before encoding. The evidence localizes it to
the combined Defringe/BlockDenois stage for this event; it does not separate
those two filters, identify their complete arithmetic, establish a general
flicker detector, or explain every ordinary-production update. In particular,
the ordinary producer has a different history and an already smaller pulse
at this particular bin, as recorded in the earlier analysis.

Post-to-encoder identity remains unresolved. The unchanged selected preencoder
site receives158 calls through completion but never the captured post-filter
AVFrame/control pair. No selected encoder pixels were read and no encoder-PTS
link is claimed. Independently, NV12, Y and UV matching over all63 movie
patches select movie indices6/7 for the two captured post-filter patches.
That is pixel correspondence, not an upgrade of the missing identity link.
The newly encoded movie's green/luma residuals at those inferred indices are
0.200221/0.366463 with the common motion field; they are not identical to the
post-filter bytes or the earlier movie. The post-filter-stage finding rests
on the directly authenticated pre/post pair, not on assuming codec identity.

The export finished63 frames, the project hash stayed unchanged, and the
debugger detached and exited with no pending or uncertain snapshots. The
durable capture's `run-01/session-post-filter.json` explicitly separates
`final_to_post_complete=true` from `post_to_encoder_complete=false`.
SHA-256 values:

- Post-filter session: `b8c618d559b2303e43f3ce573619382622185dd280a0955ee92432a672ab2801`.
- Final event log: `8141c863f57c6d0988bb3f07266a71362faaa1ea0c8072cfa8573872868d3daf`.
- Movie: `4fe80b6498408a63c1a2bd220bb47957477828a76b1bf09ff0ccb37f5d89595c`.
- Analysis02 receipt: `16b5dda76437015dc2c603d07edd3bda2c8533b4002e3a735cef30fed2a5154e`.

Analysis01 ran while the event log was still growing. Its exact sealed input
prefix was recovered and saved as `events-input-snapshot.jsonl` alongside its
original script, rather than treating the later larger event log as the same
hashed input. Analysis02 consumes the final detached-session artifacts.
All captures and analysis remain in worktree scratch, not/tmp.

### Captured warm GPU/reference regression

`image_fusion::gpu::tests::captured_warm_sequence_matches_reference_on_every_publish_and_hold`
adds missing multiframe implementation coverage without new source sampling
or altered arithmetic. It reads existing saved800x16 BGR input bands and
212x4 invalid bytes in manifest order, advances one GPU Producer and one
readable camera-specific Reference, compares every output map with the existing
ratio tolerance and requires bit-exact retention on every reference hold.
It also requires a warm update and a hold, so a cold-only fixture cannot pass.

With `KJERAG_FUSION_SEQUENCE_FIXTURE` pointing to
`scratch/studio-seam-ab-20260908-01/april-full-overlap-01/frames/fusion-inputs`,
the real RADV GPU run covers31 contiguous X4 sources18209..18239: four
admissions,27 holds, maximum GPU/reference error0.0000667572. The filtered
image-fusion suite also passes70 tests with both saved camera fixtures; five
opt-in tests are ignored in that suite, and the new ignored warm test is run
separately and passes. The initial sandbox run correctly refused a missing
Vulkan adapter; the reported passes are the subsequent real-GPU runs, not
that failure or a no-adapter skip. The release test binary links ffmpeg7.1.

Logs/source patch/binary hash are retained in
`scratch/studio-seam-flicker-612-20260909-01/warm-sequence-qualification-02`.
The warm log SHA-256 is
`3686e0a0df3662cf5bf7d0a8763b1d1883124095c924002b28c2d9170f4f572b`.
This rerun follows a lint-only assertion rewrite; the earlier01 receipt is
also retained. Focused release Clippy for the render tests, formatting, name,
crate-source and diff checks pass. Existing vendored dependency warnings are
still reported; no dependency cleanup is included.
No full workspace/UI/Flatpak gate is claimed for this test-only addition,
and no production arithmetic or installed executable changed.

The owner has been asked whether to evaluate gradual changes of the color
correction itself, preserving image detail but allowing roughly0.1s of color
catch-up, instead of implementing full-picture filtering. This is explicitly
a possible Kjerag design choice, not a recovered Studio constant or approved
tradeoff. No such policy is implemented or installed at this checkpoint.
The exact612.078 report still has no Studio output coverage or owner-accepted
fix; the full flicker-free objective remains unmet.

### Owner rejects an invented gradual-update policy, 2026-09-09

The subsequent owner answer is "no gradual unless it matches studio". This
supersedes the pending-choice status above. The roughly0.1s proposal was an
alternative Kjerag policy, not a recovered Studio behavior, and is not selected.
The X4 Air publication observations still show admitted updates and exact holds,
not gradual coefficient interpolation. The authenticated final-to-post-filter
attenuation identifies the combined processing stage only. Separating the
responsible filter and recovering its relevant behavior remains unfinished;
it must not be described as an already-understood temporal filter or a fix for
the later612.078 report. No production code or installed build changed.

### Defringe / BlockDenois split on the selected event, 2026-09-09

One further export of the same63-frame project captures the intermediate
surface at `BlockDenoisAlgoFilter::Process` entry `0x7968c0`. The frozen final
observer and head/post bridge are unchanged. A separate reviewed supplement
requires exact selected AlgoFrameInfo/control, FramePosition, original
VideoFrameInfo, MediaSample/control and AVFrame/control/CV identity from the
head, plus state1 and the existing7680x3840/420f/ROI guards. Its CPU-only
validator rejects Mat/unknown state and every retained object/control/surface
mismatch. All six final/middle/post snapshots use balanced read-only CoreVideo
transactions and duplicate logical-row reads; all complete successfully.

The static Defringe audit establishes the selected route: the asynchronous
worker calls `RunDefringe(CV,CV)` at `0x7a9b7c`, waits for its Metal completion,
and forwards the same state1 AlgoFrameInfo to the next filter. Thus this
intermediate snapshot is after Defringe and before any BlockDenois work.
Defringe includes current-frame detection/inference and color postprocessing;
its detector has a retained status and it has a configurable bypass gate.
Neither fact establishes gradual color coefficients or cross-frame pixel
averaging. Its full model and exact pixel semantics remain unread and are not
needed to label the measured intermediate boundary.

The current final ROI bytes reproduce the original final capture exactly.
Defringe changes13,150/12,398 UV bytes for18214/18215, but **zero luma bytes**
in both selected patches. The unchanged fixed607 event, rotation, sigma8,
9x17 patch at view pixel(685,300), shared motion and conversion give:

| Selected signed event residual, codes | Final stitch | After Defringe | After BlockDenois |
| --- | ---: | ---: | ---: |
| Green, common texture motion | 1.273177 | 1.287654 | 0.392611 |
| Direct luma, common texture motion | 1.222739 | 1.222739 | 0.423762 |
| Green, zero-motion control | 1.207120 | 1.221195 | 0.316699 |
| Direct luma, zero-motion control | 1.145891 | 1.145891 | 0.334562 |

Defringe therefore does not attenuate this selected pulse; attenuation occurs
in the following BlockDenois interval. This remains a local event attribution,
not a general perceptual flicker detector or a claim about the newer612 view.

The post callback is not necessarily a conversion boundary. Both selected
post states are1, and their stacks traverse the CV-buffer result overload at
`0x797b20`. `SetMediaSample` writes state1 at `0x771d8c..90`; End tests that
state at `0x772624` and branches directly to `0x772900`, skipping all Mat
materialization. The initial static audit's unqualified concern that the End
callback must include conversion was corrected after reading this branch.
For18215, the still-active result overload's callee-saved x20 recovers the
unchanged first CV result in its nonempty vector. It equals the post AVFrame's
data[3] exactly. That explicitly closes the adapter backing for18215; a direct
vector alias was not captured for18214. No extra export was made to obtain it.

The saved project explicitly requests `enableMultiFrameDenoise=true`, with
single-frame denoise and motion blur false. A runtime pointer chain from
Block+0x248 through Algo+0x10 and facade+8 authenticates the Metal backend
vptr at slid`0x480e690`. Backend+0x138/+0x139 are0/1, +0x140 is8, and +0x35c
is7; a later stop also records +0x354=2 and +0x358=3. The NAP code compares
its queued-frame count with +0x35c. This is a seven-frame window/admission
setting, not proof of seven additional neighbors or specific source indices.

The pinned worker contains readable Metal source for temporal fusion. The
initially located packed-C4 variant (not yet authenticated as selected at this
checkpoint) aligns reference pixels using block motion, combines them with a
current
sample of weight256, bounds reference weights by pixel difference and noise
level, performs rounded normalization, and clamps the result against
luma/chroma-dependent limits. Specializations take one through six references;
optional guided chroma processing follows. This is actual pixel-history
filtering, not a100 ms recurrence over lens-color coefficients. Static source
and the observed backend narrow the relevant implementation work, but do not
close the exact selected reference arrangement, per-source ISO/noise/limit
mapping, active shader variant, optional branches or full flow semantics.
No replacement algorithm is selected from incomplete parameters.

The post pixels differ from run03: luma differs at35,296/51,927 bytes and UV
at3,776/6,736 bytes. The analysis deliberately compares only this new run's
three stages, not its middle against old post pixels. The reason for this
run-to-run variation is unassigned. The measured attenuation reproduces in
direction and approximate size, not as byte-identical post output.

Durable root is `scratch/studio-seam-flicker-612-20260909-01/`:

- `mac-filter-split-20260909-01/run-01/session-filter-split.json`, SHA
  `0c985b7d7c194329d18475ac1d05a7fd2dc8d2c450d87ccff4e892d9680eb6cd`.
- Detached event log, SHA
  `9356be4af386e67ff58eabed40da56deb53150b31ffcc00d0ea4e8ef2342cb75`.
- `filter-split-analysis-01/receipt.json`, SHA
  `61f80f4b39fc671c76cff457159a70da37ff0006930f5841eae5de5268c0aa82`.
- `analyze-filter-split.py`, SHA
  `f28524f23bef2078bc6268f99361fd8ac730028a1832245757b6ad7a74ae21a0`.
- New movie SHA
  `705c507b6821a31dd33871a764faf0f9a5eb6bfc7e5abb85af6e76e6161d600c`.
- `defringe-audit-01/` and `blockdenoise-audit-01/` retain the static ranges,
  kernel excerpts and corrected boundary interpretation.

Local and Mac ffprobe confirm63 frames,7680x3840 and2.102100s. The original
project hash is unchanged. The observer deleted its breakpoints, detached and
quit; a separate process check finds no LLDB and the original exporter still
running. No encoder observation/PTS link is claimed. The coordinator viewed
the saved post patch to check image content, not as a moving-video verdict.
No production arithmetic, installed app, merge or acceptance status changes.
The next relevant RE is the selected denoiser's remaining operating semantics;
the newer612 Studio comparison and an actual tested implementation remain due.

### Selected normalized denoiser kernel, 2026-09-10

A bounded local read now distinguishes the actually selected kernel family
from the nearby packed-C4 source summarized above. No export or native session
was started for this step. The existing observed mode 8 selects the two-plane
path in `ConfigFuseNormEncoder` at `0x2c24998..b8`. Its Y pipeline comes from
map+0xa20, keyed by reference count, at `0x2c249f4..4a28`; after successful Y
dispatch, UV comes from map+0xa38 at `0x2c25594..5c8`.

`EnsureNapResources` constructs those maps at `0x2c1c5a4..740`, using names
`nap_fuse_y_N` / `nap_fuse_uv_N`, N=1..6, library key
`block_denoise_kernel_srcs_nap_norm_base`, and source pointer `0x4123e90`.
The creation lambda passes that source and function name to
`MetalContext::NewPipelineState` at `0x2c1d718`. This authenticates static
host/source selection for the captured mode, not compiled GPU instructions.

Both kernels bind the current plane first, then ascending reference-ring
planes excluding current, then the flow-texture deque, destination and luma
grid. Y uses constant buffer+0xbe0; UV uses+0xbf0. Selected dispatch sites are
`0x2c2530c` and `0x2c25f7c`. Other interleaved three/four-plane blocks are not
the selected bindings.

The selected arithmetic differs materially from the packed variant:

- Current sample has weight 1, with normalized floating samples. Luma/chroma
  motion weights are respectively signed-short flow.z/255 and flow.w/255,
  clamped to [0,1]. One thread handles one Y value or one UV pair.
- Motion addresses 16x16 luma blocks or 8x8 chroma blocks. Reference coordinates
  use integer flow offsets, arithmetic-halved for chroma, and clamp per pixel.
- For absolute difference d and normalized noise level n, a reference keeps
  its motion weight w when d<n. Otherwise the weight is
  clamp(1.5*n/max(d,1e-6)-0.5,0,w). U and V are evaluated independently.
- Weighted accumulation uses floating fused multiply-add and division, with
  no explicit integer-rounded normalization. The result is limited relative
  to current by normalized limit times the current-block luma-table entry,
  then converted to the selected output range and clamped to [0,1]. Texture
  storage can still quantize; exact selected storage formats remain separate.

This is pixel-history filtering, not coefficient interpolation. It rules out
using the earlier weight-256/integer-round source as the selected implementation;
no production code was written from either source. The proprietary source
bytes remain only in ignored scratch. Kjerag must implement the recovered
behavior independently, not ship extracted Studio source.

Evidence is under
`scratch/studio-seam-flicker-612-20260909-01/denoise-shaders-01/`.
`README.md` records exact bindings and limits; the hash-checked extractor
preserves the 18,091-byte selected NUL-terminated source, SHA-256
`f033ec64c489eebd39330821c703e972695b3f3edd8074e2bd7e713ca552cf15`.
The initially extracted units at `0x4128600` and `0x41392d7` remain labeled
unselected evidence. No input/output equality, visible flicker pass, 612 Studio
coverage, installation or merge follows from this static result. Per-source
parameters, flow/luma construction and temporal boundary behavior still need
their own evidence before a complete reproduction can be claimed.

### Selected denoiser inputs and effective parameters, 2026-09-10

One further export of the unchanged earlier607 project captures the inputs
to the selected normalized fuse and the same output buffers after the filter.
The frozen final/middle/post source observer remains the association base.
The new observer follows each destination CV buffer through the live
ConvertToSample/ProcessDenoiseResult call, requiring the same thread, exact
caller CFA and selected post AVFrame data[3]. This associates packet6 with
source18214 and packet7 with18215; it does not infer source IDs from ordinals.

All eight observed parameter packets select backend mode8, override word0,
radius3, ISO100, noise integer700 and limit integer10. The consumed first
two UBO floats are0.04289215803146362 and0.03921568766236305, matching the
recovered f32 scaling of700/16320 and10/255. Y's256 limit entries are all1;
UV's are all0.5. Range flags are zero and guided-UV is false. The guided
field is byte+0x28; interpreting its three padding bytes as a32-bit boolean
would give a false result. The constructor's400/15 values are not effective
values here. These observations do not provide another ISO's calibration.

Packets0..3 show relative current0,1,2,3 and reference counts3,4,5,6. Later
packets remain centered at3 with references0,1,2,4,5,6. This authenticates
the selected seven-frame scheduling window and effective radius for this
run, not all startup/flush cases or source identities of the unselected
packets. In particular, six references means three earlier and three later
frames, not seven past frames or gradual lens-color coefficient updates.

The selected input snapshots contain21 textures each: current Y/UV, six
reference Y/UV pairs, six signed-short motion grids and one luma-index grid.
The unchanged442x306 panorama ROI is bounded by actual flow offsets for
each reference. Full image dimensions are7680x3840 Y and3840x1920 UV;
sample formats are R8Unorm/RG8Unorm. Motion/luma grids are480x240,
RGBA16Sint/R8Uint. All are Metal managed storage. An explicitly named
CPU-shadow read copies each requested region twice and requires byte equality;
it makes no diagnostic blit, synchronization or GPU-completion claim.
The four current-plane patches are independently byte-identical to the
same-source middle CV snapshots. Reference and motion provenance remains
scoped to the captured retained owners and binding order.

The bounded upload read identifies an ordinary populated host-Mat upload
via `replaceRegion`, with a separate empty-Mat error fallback. Reaching an
upload call alone would not authenticate that populated branch. The actual
capture records subsequent fuse configuration and output association, not a
new runtime observation of every upload or the original motion producer.
Do not turn this static read into an unwarranted device-coherence claim.

Capture corrections are retained openly. The original observer delegated
1068/1080-byte UBO reads to a helper limited to256 bytes; the same paused
call was recovered using its existing duplicate-read chunker. An event-key
collision then occurred after the first Y receipt was saved; the callback
was corrected without resetting state or rereading that payload. The initial
shared-only pixel reader refused managed storage before copying bytes.
The explicit CPU-shadow option retained the remaining guards. Replacement
command/declaration guards also refused before copying; the final helper
reuses its checked48-byte region declaration. Offline checks now cover the
read-size delegation and event-key collision. No uncertain pixel-copy or
CoreVideo lock transaction occurred.

All selected final/middle/post snapshots and both input sets are saved. Safe
state assertions passed before detach. A final supplementary log line was
corrupted by the debugger PTY and rejected with SyntaxError after both
session files were written; the actual detach succeeded and is recorded.
The movie finished63 frames at7680x3840,30000/1001 fps,2.1021s; the project
hash stayed unchanged. Encoder provenance is not collected or claimed.

Evidence root:
`scratch/studio-seam-flicker-612-20260909-01/mac-denoise-inputs-20260910-01/`.

- Events: `bc2d46eddae36677fc955e7474eee59023178ed6f066a9ed059591cfc5699231`.
- Denoise session: `b4e85aa906e80fd6f6a71b40e7b0c408ddd24cd45e15abfabad7a4c28f731216`.
- Source/post session: `9ae297defced74d33d1c917c28c79008ce45faec3f030d0024ab0643b74dc0dd`.
- Finished movie: `ecec8a28f9abecd089a86c0acf799a5449c2f3b0f024f031930942c511b2a6d8`.
- Sibling parameter analysis: `f319e651c5077b0e952f62138bbbde0ca8bd6a46e76093fd31fa86fcb823f3aa`.

The original standalone Rust reference has eight passing algebra tests. A
separate receipt-driven driver now connects it to these exact saved inputs
and same-source post-filter outputs. It validates payload hashes, observed
storage/formats, rectangles/strides, six references, range flag and guided
gate, then applies the read Y/UV addressing and original fuse algebra. Input
UNorm conversion is f32 byte/255. Output code comparison uses explicitly
diagnostic ties-to-even rounding of255 times the clamped normalized result;
this is not an authentication of Metal's exact storage conversion.

| Source | Y differences / samples | U differences / samples | V differences / samples | Maximum code difference |
|---|---:|---:|---:|---:|
|18214|18/135252|2/33813|2/33813|1|
|18215|9/135252|1/33813|1/33813|1|

Thus405,723 of405,756 component codes are exact, and the remaining33 differ
by one code. This supports the selected normalized fusion law and the
captured CPU shadows for these two patches. It is not exact shader identity,
a complete motion estimator, general camera/ISO coverage or a moving-video
flicker verdict. No output-semantic constant was fitted to the comparison.

`denoise-offline-01/` contains the independent driver, input packer and
receipts. The initial run put reproducible executables/packed containers in
`/tmp`; all original evidence and sources remained in durable scratch. The
coordinator corrected the commands, preserved the original driver/packer,
strengthened input guards and repeated both runs entirely under
`denoise-offline-01/root-run/`. Both packed-container hashes and every
component result reproduce the initial receipt. The original reference
remains unchanged, SHA-256
`550cc32a1aa97733bb4640a74e787e330202070e94f83b5a41c9c0d10a18e820`.
The motion/confidence producer, broader ISO-dependent calibration and a
standalone Kjerag temporal path remain unclosed, as does moving output
verification. No player change, installation, owner retest,
merge or612 Studio coverage is added here.

### GPU normalized fusion primitive, 2026-09-10

`crates/render/src/temporal_fusion.rs` and its original WGSL now implement
the selected full-range normalized fuse with explicit current/reference
layers, motion/confidence grids, luma indices and effective parameters.
There are no inferred defaults, lens-color interpolation, history updates
or player selection. Y and UV use separate render passes into R8Unorm and
Rg8Unorm targets; this differs from Studio's compute execution to avoid
requiring optional narrow storage-texture formats. Encoding performs no
submission, CPU synchronization or pixel readback. Output allocation and
small parameter-buffer construction are not yet pooled.

On the owner's AMD Radeon 760M / RADV, three ordinary tests pass: explicit
layer ordering and separate Y/UV confidence, negative odd UV displacement
with independent U/V rejection and luma-table limits, and invalid-input
rejection. The opt-in native-packet test verifies the sealed packet/source
associations and replays both captured input sets into their original
full-size texture coordinates, including the nonzero output ROI.

| Source | Y differences / samples | UV component differences / samples | Maximum code difference |
|---|---:|---:|---:|
|18214|2119/135252|2343/67626|1|
|18215|2145/135252|2545/67626|1|

Thus 396,604 of 405,756 components match exactly; 9,152 differ by one code.
This GPU result is less numerically exact than the preceding CPU reference.
The difference's cause is not isolated, and the one-code test tolerance is
not an owner-approved visible tradeoff. No coefficients or thresholds were
fitted to these differences. This is a same-input kernel check, not full
image quality, 240 fps capacity or a flicker acceptance. Durable test receipt:
`scratch/studio-seam-flicker-612-20260909-01/denoise-gpu-01/results.json`.

The integration boundary must remain view-independent: prepare a body-space
panorama once per exact source/map/color result, retain the input window,
and publish the denoised current frame with its own source stamp. Redraws
must not advance temporal history. The existing media lookahead provides
two future sources, whereas this captured centered window needs three.
At the captured 7680x3840 geometry, seven NV12 inputs and one output alone
occupy 337.5 MiB, before motion grids, retirement or decode resources.
Those resource and scheduling changes are not implemented here.

A bounded static read also locates independent current-to-reference calls
through `BlockFlow::ComputeFlow`, raw three-i32 motion/cost records and
Studio's separate confidence packing. Its `PlaneOfBlocks::PseudoEPZSearch`
and recalculation routines strongly match the MVTools family: the primary
[motion core](https://github.com/pinterf/mvtools/blob/mvtools-pfmod/Sources/PlaneOfBlocks.cpp)
and [parameter documentation](https://github.com/pinterf/mvtools/blob/mvtools-pfmod/Documentation/mvtools2.html)
offer a source-reference route instead of reconstructing the entire search
from assembly. Matching names and structure do not authenticate Studio's
runtime geometry or output. No upstream code is imported in this change.
The bounded source/license comparison and static receipts are in sibling
`temporal-flow-upstream-01/` and `temporal-flow-law-01/`.

One static-note correction matters: the selected fast-resolution level was
not captured. Init reads a shift from backend+0x464; the saved backend slice
does not include that field. DenoiseInfo+0x14 is a boolean and +0x18 is the
radius, neither establishes that shift. Do not infer a one-level downscale
from those fields or the final motion-grid dimensions. Motion generation,
broader ISO calibration, live scheduling, moving output and the new 612
Studio comparison remain open. No new native session, export, installation
or merge occurred for this GPU check.

The subsequent bounded constructor read finds the default geometry words
`(16,16,1,10)` at 0x4118430, loaded into backend+0x45c..+0x468.
Thus +0x464 has constructor default 1, not a selected-runtime receipt.
`BlockDenoiseMetal::Init` also forces block size 16. Overlap is a higher
caller's stack argument, stored at +0x44c/+0x450, and remains unread for
this instance; exact pyramid depth consequently remains unread too.

Qualification for the isolated change: the full render suite passes 887
tests with 29 opt-in tests ignored. After an equivalent Clippy-requested
even-size validation change, all four temporal checks pass again, including
the native packets with identical difference counts. Workspace all-target
Clippy, formatting, name and crate-source checks pass. Full workspace tests
and the UI harness were not run for this unconnected primitive; this is not
a player delivery qualification.

### Selected motion producer inputs, 2026-09-10

The bounded constructor-to-consumer audit corrects the earlier scratch
Recalculate interpretation. Base Init passes both BlockFlow booleans false;
the selected constructor installs Analyse at+0x58 and leaves Recalculate
at+0x68 null. ComputeFlow therefore calls Analyse::GetFrameSuper, then
GroupOfPlanes::SearchMVs and the selected PseudoEPZ TBB worker. MVTools
lineage remains supported, but the alternate Recalculate constants are not
selected-path evidence.

One further export of the unchanged earlier607 project captures complete
current/reference super images, raw and packed motion, confidence tables and
luma for ComputeFlowFast invocation6. This is an invocation ordinal, **not
an authenticated source-frame number**. The observer associates all six
references by thread, caller CFA, backend and retained input/output owners.
It uses duplicate-equal bounded CPU memory reads only, with no target
expressions, CV locks, Metal calls or source-buffer writes. The loaded worker
slice and five hook instructions are verified before selection.

The receipt confirms original7680x3840, fast shift1, zero overlap,16x16
blocks, one gray super plane3840x3810 and seven pyramid levels. Raw motion
is240x120 CV_32SC3; packed motion is480x240 CV_16SC4. Thus this run selects
the resampling weight producer, not the earlier described direct-grid body.
The ordinary-init overlap constant and constructor shift are now independently
confirmed at runtime. Analyse is present and Recalculate is null.

| Selected search parameter | Observed value |
|---|---:|
| Hex2 internal flag |16|
| Non-finest / finest radius |2 /1|
| Lambda |0|
| Effective LSAD |1600|
| New / zero / global penalties |50 /50 /0|
| Pyramid lambda scaling |0|
| Global predictor |enabled|
| Effective badSAD / badrange |40000 /24|

These agree with the selected constructor and consumer trace. Every
non-finest plane uses exhaustive/radius2 search; the finest uses Hex2/radius1.
This is a hybrid of public MVTools defaults, not a stock preset. The pinned
source comparison also finds native predictor-boundary and parallel-loop
differences; shared names do not establish a drop-in bit-identical port.
The read-only comparison clone is vapoursynth-mvtools commit
`17250aa979616ac48dfb0e18abfdcf2bd4e3afc0`; no upstream code is imported here.

The confidence lookup override is0. Captured scale factors are4 and700,
with temporal factor1.25. All256 Y confidence entries are1 and all256 UV
confidence entries are2, distinct from the preceding final-fusion limit
tables1/0.5. The six double phases are
`(1,0.49999999999999994,0,0,0.49999999999999994,1)`.
The static phase law is a raised cosine over reference position; this is
pixel-reference weighting, not a gradual lens-color coefficient update.

The current luma grid is independently reconstructed by the recovered
packed-pyramid extraction: skip heights1920,960,480, then copy the480x240
crop at row3360. No resize is selected here. All115,200 output bytes match
the captured luma grid exactly. This tests extraction from the native pyramid,
not production of that pyramid from stitched pixels.

The selected Super path is also reconstructed independently. GetFrame calls
GroupOfFrames::Reduce, whose selected Plane::Reduce directly calls the
MVTools-family RB2BilinearFiltered function. It has no runtime filter-choice
dispatch here. Reduction is vertical then horizontal, each into8-bit storage:
interior taps `(1,3,3,1)` with bias4 and division8; first/last outputs use
pair averaging with bias1 and division2. Independent rounding of both axes
is required; one combined two-dimensional convolution is not this law.
Starting from each captured base image, all17,199,000 coarser logical pixels
across the seven pyramids match exactly. The separate diagnostic also
reproduces retained vertical-pass workspace and zero slack, so complete
allocation hashes match. The copied base pixels are not newly reconstructed
evidence. `temporal-pyramid-check-01/` preserves source lineage, native call
sites and the independently repeated comparison.

The confidence/resampling reference now reproduces all six packed matrices
exactly,2,764,800 signed-short lanes. Selected coordinates are `(x/2,y/2)`
without a half-pixel offset, with right/bottom neighbor clamping. Bilinear
sampling retains native f32 operation order. Displacement is divided by the
ratio, truncated and landing-clamped to full-image block bounds; interpolated
match cost is truncated without ratio scaling. Thresholds use the captured
lookup tables and reference phases. Their squares wrap as signed32-bit
integers before f64 conversion, whereas match cost is squared in f64.
The rational confidence result is truncated into packed lanes. No smoothing
constant is fitted. `temporal-confidence-law-01/` records exact FMA/conversion
boundaries and the selected-path verifier. Root's independent repeat matches
the saved zero-difference receipt.

The capture closes normally with six complete reference records, disabled
hooks, successful detach and LLDB exit0. All22 payloads (110,133,248 bytes)
pass independent size/SHA checks. One earlier unselected breakpoint stop
was continued with no active capture, pending copy or failure. The pre-arm
review corrected an eight-byte BlockFlow metadata overread; all14 offline
observer tests passed before use. No uncertain native resource transaction
occurred. Detailed correction and GUI receipts remain in ignored scratch.

Evidence root:
`scratch/studio-seam-flicker-612-20260909-01/temporal-motion-capture-01/`.

- Observer SHA256: `a63feeb4686da9a5aaaf2833f764ed65f2ad281d3d186edac2ebd147cd4b9882`.
- Complete/detached events: `72d7233f89e3ff9749c33eaa405cc408787ae9299260fa15a9f1cf1fba4560c4`.
- Native and reconstructed luma: `43968e5468fd2efde00160dc6b7ec377d1ea91a32009720b00cb34d3abcfa9ab`.
- Pyramid verification: `c3e96456098b37d49c79d2bb09a7ecc4ed2c4ae702d42337058dc0ea18221965`.
- Confidence/resampling verification: `097be25a5210c615dde864056dd1fd039afccb9b7fe818a57a7ee3179a5552c8`.
- Finished movie: `13103cb18554eeb290d63bcad71ce0fe7d28a86a3b173c4688fd04365c036c26`.

The movie finished63 frames,7680x3840,30000/1001 fps,2.1021s; the project
hash remains unchanged. This is a same-call motion oracle, not source/encoder
provenance, a new612 oracle, visible flicker acceptance or a performance
result. Raw motion search, preparation of base images from stitched pixels
and source-stamped player integration remain unfinished. No installed build
or merge changes.

The verified calculations now have readable Rust CPU references under
`render::temporal_fusion::{pyramid,motion}`. They require explicit inputs,
reject unsupported geometry/numeric conversions and own no history or
player selection. Pyramid storage is independent logical levels, not native
scratch bands. Motion packing preserves signed fractional displacement
before truncation, f32 FMA order, wrapped scale/threshold multiplication and
separate Y/UV confidence. Unit checks cover borders, per-axis rounding,
fractional costs and motion, threshold equality, independent tables,
overflow semantics and invalid inputs. The native tests seal inputs by hash
and reproduce all six packed motion matrices and all coarser logical pyramid
pixels exactly. No upstream implementation source is copied into these Rust
modules.

All19 temporal tests pass with both new private fixtures and the preceding
GPU-fusion packets. The AMD run retains the preceding GPU result (9,152
one-code differences, none larger), not a new perceptual verdict. The first
combined invocation omitted the older denoise-fixture environment variable;
its new reference tests passed, but the older fixture test correctly refused.
That sandbox invocation selected llvmpipe. The corrected run supplied both
fixture roots and used the actual AMD760M/RADV adapter, with no skipped tests.
This is reference qualification, not an installed-player test.

The final render suite passes900 tests with31 opt-in tests ignored; the two
new private fixtures and existing GPU-fusion fixture passed in the separate
19-test run above. Workspace all-target Clippy, formatting, name and crate-source
checks pass. Full workspace tests and the UI harness were not run for these
unconnected CPU references. No frame-path or UI change is being delivered.

### Standalone motion-search comparison, 2026-09-10

The captured inputs now drive a standalone CPU diagnostic built from the
unmodified, pinned MVTools core at commit
`17250aa979616ac48dfb0e18abfdcf2bd4e3afc0`. A thin harness supplies native
gray pyramid bands directly, with their actual stride and logical sizes.
It selects the captured search parameters above without applying the public
wrapper's block-area scaling a second time: the direct core receives effective
LSAD 1600 and badSAD 40000. Unselected pel-2/pel-4 and DCT entrypoints explicitly
refuse. This is not a production import or a new playback path.

Independent pixel checks establish that every native raw cost is exactly the
16x16 sum of absolute luma differences at its supplied integer displacement:
172,800 blocks across six references, with no out-of-bounds landing. The same
check passes for every standalone result. This closes displacement/cost units
and input association within this call, not source-frame identity or selection
of the same winning vector.

The unmodified baseline differs from native at 1,874, 1,728, 1,933, 2,003,
5,839 and 8,792 of 28,800 vectors, in reference order. These are not just border
differences; interior counts are 1,602, 1,420, 1,622, 1,707, 5,557 and 8,511.
The independent root repeat reproduces all six complete output matrices and
the report byte-for-byte. No parameter is adjusted to fit these counts.

The selected group-level audit establishes the same coarse-to-fine contracts
as the public core. Grids from finest to coarsest are 240x120, 120x60, 60x30,
30x15, 15x7, 7x3 and 3x1. Interpolation preserves the 9/3/3/1 stencil and its
separate vector/SAD rounding; global prediction uses per-axis histogram modes
and the joint `abs(component-mode) < 6` neighborhood mean before doubling.
Native vector
storage is not cleared on each call, but the selected 3x1 coarsest task cannot
read a previous call's predictors, and interpolation overwrites every finer
array. Earlier reference-pair vector state is therefore not another missing
input for this selected geometry. This does not equate native parallel order
with the serial public-core order.

Two concrete native per-block differences are read independently of the
comparison numbers. Studio copies its supplied global predictor afresh for
each block; the public core clips and retains that scalar across blocks.
Studio also clips seeded/coarse predictors to an inclusive upper landing
bound, whereas the public core subtracts one. Candidate validation is a
separate rule and must not change with seed clipping: the selected Hex2/radius1
dispatch skips the hexagon and uses ExpandingSearch/radius1, whose inspected
side candidates retain upper-strict validity. The native smallest-plane
forward-predictor suppression is another read difference, but is inactive
for this one-row coarsest grid. Working-area row bounds are whole-plane
bounds, not evidence of partition-local predictor gates.

Separate native-read adapters retain the public source and baseline untouched.
Their differing-vector counts, each out of 28,800, are:

| Diagnostic | Ref 0 | Ref 1 | Ref 2 | Ref 3 | Ref 4 | Ref 5 |
|---|---:|---:|---:|---:|---:|---:|
| Unmodified baseline |1874|1728|1933|2003|5839|8792|
| Fresh global only |1874|1728|1933|1187|2595|2535|
| Inclusive seeds only |535|546|1136|1382|4226|7632|
| Combined native-read changes |535|546|1136|512|756|682|

The smallest-plane-only variant is identical to baseline, consistent with
its inactive guard in the selected coarsest geometry. The combined variant
also includes that guard; its total mismatch is 4,167 rather than 22,169 of
172,800 vectors. Root independently reproduces all six combined matrices and
its report byte-for-byte, and recomputes every output SAD exactly from the
captured images with all landings in bounds. These results support the read
differences, not visual equivalence. No remaining error is assigned solely
to parallel scheduling, and no parameter tuning is selected.

The next useful product gate is moving output from Kjerag's own pixels, not
an indefinite pursuit of zero differing motion vectors. The existing type2
mesh adapter fixes the body-equirectangular texel-center raster and the exact
current Kjerag stitch-through-gamma-RGB calculation. An offline RGB panorama
materializer can therefore reuse those operations. The selected native
RGB-to-420f matrix, quantization and chroma sampling phase are still unclosed.
The nearby BT.601 conversion source is an unselected variant, not authority
for this path. Feeding the NV12 temporal primitive requires either a recovered
selected conversion or an explicitly disclosed Kjerag diagnostic conversion
with an identical unfused A/A round-trip control. No conversion or visible
tradeoff is silently accepted here.

Durable diagnostic roots under
`scratch/studio-seam-flicker-612-20260909-01/`:

- `temporal-search-reference-01/`: harness, sealed inputs, complete baseline
  outputs, independent repeat and output-SAD verifier. Baseline report SHA256
  `251dbf18495c726884cf575fa931c15c07bd605e4aaec8529da37efce15bba05`.
- `temporal-search-contract-01/`: group/native/public instruction receipts and
  the root working-area audit, including exact clipping/dispatch addresses.
- `temporal-motion-capture-01/raw-sad-verification.json`: native raw-cost check.
- `temporal-search-adapter-01/`: isolated patches, generated source diffs,
  executables, six raw outputs per variant and reports. Combined report SHA256
  `c147fe708c0df60dd8887d3f5af86287fae040dbd099a67c15bf61b2c5e1a32d`.
- `temporal-search-contract-01/materializer-prerequisites.md`: existing raster
  and RGB authorities, selected NV12 conversion gap and source-stamp contract.

This checkpoint adds no Studio export or native session. Motion-search
adoption, actual Kjerag panorama preparation, image-history scheduling and
moving-video acceptance remain open. Numerical motion identity is not a
shipping requirement. The earlier 607 capture does not cover
the owner's 612.078 report. There is no installed-player change or invented
gradual color update.

### Actual-Kjerag panorama and temporal candidate, 2026-09-10

The test-only Scene review now materializes the exact displayed source/map/
fusion into a body-equirectangular gamma-RGB texture. It projects that texture
through the same prepared locked view and separately performs an unfiltered
NV12 round trip. The latter is explicitly Kjerag's inverse source matrix,
full-range Y, neutral128/255 UV, exact2x2 chroma footprint average and centered
bilinear reconstruction. It is not the still-unclosed native RGB-to-420f law.
The ordinary type2 color helper was factored without changing its arithmetic.

At the exact612.078 view, all155 ordinary artifacts across sources18344..18374
are byte-identical to the retained pre-change baseline. Panorama resampling
itself has mean absolute RGB change1.4838 codes and maximum39; the NV12 control
adds mean0.376268 and maximum12. These are representation diagnostics, not
accepted visual differences or flicker metrics. Source18344 was viewed in all
three arms; still-image similarity is not a moving-seam verdict. The first
run's obsolete ONE-X2-only preparation guard refused X4; the corrected shared
map/source consumer passed31 sources on AMD. Both failure and rerun are retained.

The Rust serial search now matches all six complete combined-adapter arrays
from the saved capture. The known4167/172800 differing native vectors remain.
The implementation preserves the pinned MVTools source notices, elects its
later-version GPL-3.0-or-later option and includes the license text. Review
caught a component-wise global-average translation error before fixture
qualification; its regression now requires the source's joint X/Y inlier set.

The selected half-size gray input uses CPU8-bit INTER_LINEAR before Super.
The worker imports OpenCV4.7.0; the saved407 dylib SHA256 is
`00fdb0624c467644e9c47a5db534fdd473b33b4bfe6adb09aa806330cbd2be1e`, with custom
version-control string `4.0.1-6877-g6285f95df2`. At exact2x reduction,
[OpenCV4.7.0 resize.cpp](https://raw.githubusercontent.com/opencv/opencv/4.7.0/modules/imgproc/src/resize.cpp)
dispatches INTER_LINEAR to fast-area reduction (lines3635..3665). Each gray
output averages the nonoverlapping2x2 input footprint with upward half rounding
(lines2730..2735). The new bridge implements that law on a tightly packed GPU
Y readback, with no assumption about decoder pitch. It does not reuse the
pyramid's differently rounded two-axis filter. The native motion capture
begins after this bridge and lacks its full-size Y input, so byte equality to
that custom binary remains unverified; this is selected/static source authority.
The similar formula in an unselected embedded shader is corroboration only.

With the explicit ISO100 diagnostic flag, the same review retains seven
consecutive NV12 sources and their CPU pyramids in one decode epoch. It searches
and packs the six references in chronological order around center3, uses that
center's pyramid level3 as luma, copies source planes into GPU arrays and runs
the previously qualified normalized fuse. The emitted panorama is converted
and projected through the center's own Reframe. The oldest source is removed
only after completion. No correction interpolation, source substitution,
startup padding, end padding or ISO selection is introduced.

Sources18341..18377 produce31 outputs18344..18374. This starts the diagnostic
three sources before the reported time to provide real past inputs; it is not
a claim about the installed player's cold-seek history. A complete second
run without the temporal flag reproduces all259 unfiltered pictures/maps/
fusion and panorama-control artifacts byte-for-byte. The filtered still has
less fine grain; neither that observation nor any pooled score establishes
that the moving flicker is resolved. The side-by-side candidate uses the
identical unfiltered NV12 panorama on the left, filtered on the right, not a
Studio export. No authenticated Studio oracle covers612.078.

The final binary SHA256 is
`6b4af2dcda43f8684b4c32b979380a507339f9faaa40ee57e3c41d58a59671ea`.
All34 temporal tests, including the saved native pixel/pyramid/motion fixtures
and combined search outputs, pass on AMD760M/RADV. The render suite passes921
tests with32 opt-in tests ignored. Seven ONE X2 panorama-control sources at
the reported riser view pass, without selecting the X4 temporal regime for
that camera. Workspace all-target Clippy, format and crate-source checks pass.
Full workspace tests and UI harness were not run; no player build is installed,
pushed or merged by this checkpoint. CPU search averaged896ms per output and
packing51ms; the GPU allocation/copy/fuse/projection/readback section averaged
100ms in this diagnostic. These include deliberate offline work and shared
machine activity and are not player-capacity measurements.

Durable receipts: `scratch/studio-seam-flicker-612-20260909-01/panorama-controls-01`
and `temporal-panorama-01`. The latter holds the moving candidate, exact source
window CSV, same-arm repeat, qualified logs and pending integration boundaries.
No new Studio export/native session or gradual color coefficient policy was
added. Moving owner acceptance and production integration remain unfinished.

### Existing-Studio moving comparison, 2026-09-10

The same qualified binary now runs the earlier April 607.574 view with 35 inputs
18208..18242, producing 29 complete centered outputs 18211..18239. It uses the
unchanged captured-ISO100 offline implementation, not a new parameter choice.
The actual-Scene run passes on AMD760M/RADV in 36.92s. Each filtered output keeps
its center's source/map/color/view and exact six-reference source/time receipt.
The independently retained unfiltered NV12 panorama is the representation
control. CPU execution remains offline and no production path is selected.

The direct Studio comparison reuses the corrected 31-frame
`april-temporal-studio-raw-02` cache. Its receipt SHA256 is
`7fef48595b747ecce204fdac55dcf6156e7e9fc2bafd2b46dac934c8a3231f62`; all 31 current
PPM hashes match it. The selected labels 18211..18239 are decoded movie frames
3..31 under the retained index-derived offset, **not independently authenticated
camera-source identities**. The original Direction-Lock-off Studio movie SHA256
is `c544cb559bc80e306b97ea02af68eb6593471cd9e70b6e3ea4abf5c2a8e2787b`.
Its existing fixed whole-panorama display registration and 108.79-degree HFOV
are unchanged. This avoids another export, projection run or registration fit;
it does not remove Studio's original HEVC/decode differences or authenticate
equal internal history. The invalid duplicate-bearing raw-01 is not used.

Two one-cycle 29-frame movies put Studio left and either the temporal candidate
or its unfiltered control right. Both use lossless RGB ultrafast coding at
2560x720/30000-over-1001 fps, with no audio or spatial scaling. All 116 decoded
frame-body hashes match their source PPMs below the intentionally overwritten
38-pixel labels. Software four-thread decode is 0.322/0.326s, approximately
90/89fps, which qualifies delivery files, not player capacity. The launcher
loops the one-second interval; the loop cut is expected.

Root inspected the decoded 18215 preview for labels and corresponding field
boundaries. That is not a moving-flicker or smearing verdict. The owner has
been asked to compare the filtered side with Studio; this 607 review and the
prior 612 candidate remain unaccepted. There is no 612 Studio oracle, invented
gradual color update, new native/export session, installation, push or merge.
No full workspace/UI tests were rerun for this artifact-only checkpoint.

Durable root: `scratch/studio-seam-flicker-612-20260909-01/temporal-studio-607-01`.
It retains the render log, source windows, compose/verify/loop scripts, two
movies, body hashes, decode/probe receipts and caveats. Filtered-movie SHA256:
`f183393800234ce88e022b7ce2091c2080b38943519a6a8fb543925e177ed513`.

The earlier fixed event at view pixel (685,300), sigma 8 and patch 9x17 is
also evaluated on sources 18214/18215 without a new location or threshold.
One motion field from the current unfiltered control is shared by all arms.
The signed green residuals are 0.180343 codes unfiltered, 0.000161 filtered
and 0.401656 in the cached Studio images. The zero-motion control gives
0.086815, -0.096701 and 0.300952 respectively. These signed local averages
can cancel and are not a flicker or quality ranking. In particular, the
unfiltered event is already much weaker than the earlier native-coefficient
diagnostic. The fresh seek root, representation and shared-motion inputs differ;
this cannot establish numerical parity with the prior filter-split capture.
Both current coefficient arrays do change at 18215, confirmed by their saved
hashes. Direct Y and spatial-null results are not invented from missing inputs.
The moving owner gate remains required. Root independently reproduces the
complete event-analysis receipt SHA256
`0be7f16d7cbc6f8f79f7a57e60f43c95d196c916ada7d40f945ee0277bc53ccd`.

### GPU brightness-pyramid preparation, 2026-09-10

The already-read preparation arithmetic now has a GPU implementation beside
the CPU reference. `pyramid::gpu::Builder` accepts full-size R8Unorm Y or an
explicit half-size R8Uint base. The former recovers byte values, applies the
single rounded 2x2 mean, and then uses the same reduction path as the latter.
Vertical and horizontal render targets are distinct R8Uint textures, so the
intermediate byte rounding cannot be fused away. Core integer render targets
require no optional narrow storage-texture feature. Geometry and input usage
are validated before encoding. The builder records passes only and returns
owned levels; it selects no source history, temporal parameters or frame clock.

Tests cover all 256 normalized source codes, every possible four-byte sum,
multiple encodes before submission, explicit rounding/border discriminators,
odd terminal levels, unsupported-input refusal and a complete synthetic
full-Y-to-seven-level path. The shared native fixture loader retains all seven
original SHA seals and strips only native packed-row padding. Every logical
pixel in all 49 GPU levels matches those captures: 68,808,600 exact bytes.
As before, native capture level zero is already half-sized. It does not
authenticate the initial full-Y bridge, whose authority remains the selected
OpenCV/static contract plus independent CPU/GPU arithmetic checks.

The optional `KJERAG_PANORAMA_GPU_PYRAMID=1` actual-Scene diagnostic disables
the CPU full-Y readback/reduction route, encodes the new GPU preparation, and
reads the logical levels for the unchanged CPU search. This is still offline
execution, not the completed GPU temporal pipeline or a playback speed claim.
The ordinary unflagged candidate remains the readable reference. Source stamps,
maps, color coefficients and all temporal parameters stay unchanged.

At the exact 612.078 view, the same 37 inputs 18341..18377 produce all 31
filtered outputs 18344..18374 byte-identically to the previous candidate.
All 259 ordinary/map/alpha/color and panorama controls also match. The complete
center/reference index and timestamp columns and output RGBA hashes agree;
only diagnostic timing fields and the new route receipt differ. This closes
the actual opt-in route's execution check, not the owner's flicker verdict.

All 12 pyramid checks, including both native CPU and GPU fixture tests, pass
on AMD760M/RADV. All 40 temporal checks pass with the saved motion/search/fusion
fixtures. Workspace all-target Clippy, formatting, crate-source and name checks
pass. Standalone rustfmt initially disagreed with Cargo's workspace formatting;
`cargo fmt --all` resolves the formatting-only differences. No full workspace
tests or UI harness were rerun, and nothing is pushed or installed. The Scene
run took 38.79s while CPU Clippy work overlapped part of the run; it is not a
controlled timing comparison or a 240fps capacity result.

Durable receipts are in
`scratch/studio-seam-flicker-612-20260909-01/temporal-gpu-pyramid-01`: build and
test logs, optional-route source artifacts, and `verify-sequence.sh` with its
290-artifact comparison result. The exercised binary SHA256 is
`2c1052b0fa65a95efa525b096234d580f0cce6a4df663b1ba90a788ab67a688a`;
the subsequent source formatting changes no arithmetic. Both delivered review
movies retain their prior hashes. Moving owner acceptance, GPU motion search,
automatic parameter selection and production history/integration remain open.
No new native session/export or gradual lens-color update is introduced.

### GPU motion packing and independent-reference concurrency, 2026-09-10

The next port preserves the candidate's motion expansion/confidence arithmetic.
`motion::gpu::Builder` records one Rgba16Sint render pass per supplied raw
field, retaining the native top-right multiply followed by TL/BL/BR FMA order,
displacement expansion before truncation, cost truncation and image-border
clamp. Host scalar preparation keeps the reference's wrapping i32 scale
product, f64 phase FMA, f32 scale FMA and threshold conversion. Y/UV tables
remain independent and indexed by the explicit luma input. No shader f64,
submission, CPU wait, history selection or gradual color update is added.

The GPU subset explicitly requires full dimensions at most32768, raw dx/dy
within i16, costs0..65280 and each positive derived threshold at most46340.
Negative and zero thresholds retain the signed rejection gate. Larger positive
thresholds are rejected, not silently changed: the CPU reference still supports
native wrapping threshold squares, including65536/cost1 yielding-256.
These limits cover the saved capture, not automatic parameter selection for
every ISO/camera. Raw vectors and luma are still CPU uploads.

For the accepted branch1<=T<=46340 and0<=C<T, confidence is
`floor(256*(T*T-C*C)/(T*T+C*C))`. The denominator is at most4294698521,
within u32, although the numerator can need40 bits. Cost0 is exactly256.
For positive cost, eight binary long-division steps avoid that wide numerator:
start with remainder=T*T-C*C; compare it to denominator-remainder before
doubling, subtract that gap and emit1 when greater/equal, otherwise double
and emit0. Each step remains within u32. The pre-division values are exact
f64 integers in the CPU reference; a nonintegral rational is at least1/denominator
from an integer, larger than its f64 rounding error. The integer floor therefore
preserves that reference's final truncation throughout this bounded domain.

Synthetic checks exercise every possible16x16 byte SAD at thresholds0/1/2/3,
all six captured thresholds2800/3150/3500/5600/6300/7000 and upper bound46340
(with an independent UV threshold one lower). Other checks cover all256 luma
indices, signed thresholds/displacement extremes, fractional interpolation,
border clamp, scale wrap, input refusal and multiple encodes before submission.
The native test reads the same hash-sealed inputs/expected bytes as the CPU
test and matches all2,764,800 signed lanes exactly. The first run rejected a
WGSL scalar/vector type composition before rendering; the explicit conversion
was corrected. The final49-test temporal suite passes on AMD760M/RADV.

Separately, `search::selected_six` runs the six independent CPU searches in
bounded scoped workers. It prevalidates in supplied ordinal order and retains
that same result order. Serial versus concurrent vectors match both synthetic
inputs and all six saved combined-adapter outputs. There is no parallelism
inside a reference: coarse-to-fine state, searched-neighbor predictors,
row-major bad_count and strict-improvement candidate order remain unchanged.
The previously disclosed native vector gap remains. This is not GPU search.

The optional actual-Scene flags `KJERAG_PANORAMA_GPU_MOTION=1` and
`KJERAG_PANORAMA_PARALLEL_SEARCH=1` require the existing ISO100 diagnostic.
Neither selects ordinary playback. With both plus GPU pyramids enabled, the
same37 inputs18341..18377 preserve all31 filtered outputs18344..18374 and259
ordinary/map/alpha/color/panorama controls byte-for-byte. Center/reference
indices, timestamps and output hashes also match. The diagnostic records
which execution routes were selected and the timing scope. `pack_ms=0` means
no separate CPU packing stage, not zero-cost GPU motion work.

The15.20s offline run averages177.378ms search (max246.737) and89.174ms
GPU preparation/upload/packing/fusion/conversion/projection/readback completion
(max92.160) per centered output. No build/test job overlapped this run. These
are diagnostic wall times, not shader-only timing, an A/B speedup measurement,
real-time playback or240fps capacity. Motion search remains the dominant
cost and this execution is not suitable for installation.

Workspace all-target Clippy, formatting, crate-source and rename checks pass.
No full workspace test or UI harness is claimed for this uninstalled primitive.
Both moving owner verdicts remain pending and both delivered movies retain
their hashes. Root inspected a rendered18344 preview but makes no moving
flicker verdict from it. No native session, export, installation, push or merge.
Evidence is in `scratch/studio-seam-flicker-612-20260909-01/temporal-gpu-motion-01`.
The final exercised binary SHA256 is
`95b2edec868efd7848f2b1ba758beff0fa494df97ea7a255e9aba8dc91608b7d`.

### Exact finest-level GPU controller fails the performance gate, 2026-09-10

A bounded implementation tested whether preserving the reference's serial
decisions was useful on the GPU. The CPU reference now exposes the exact
preparation boundary: search levels six through one, derive the same global
predictor, and interpolate the finest plane's unsearched seeds. Full `selected`
search retains its original operations and matches all six sealed adapter
outputs. A separate replay test verifies that preparation plus the original
finest controller equals the complete search.

The GPU experiment used one 256-lane workgroup per reference, six groups per
row, and a separate ordered compute pass for every row. Each group processed
the row's blocks serially. Eight 32-lane teams evaluated fixed candidate sets
cooperatively. A leader replayed strict improvements in the CPU's ordinal
order: three inclusive starting seeds, four exclusive-upper-bound predictor
checks, then the fixed eight-candidate ring. UMH's fixed lists were batched in
order and its adaptive Hex decisions retained. The in-place raw field kept
unsearched future-row seeds intact; prior rows supplied searched neighbors.
The plane-wide bad count persisted between rows. Core i32/u32 arithmetic is
sufficient for the selected byte SADs and penalties. No subgroup feature,
per-block CPU readback or single long whole-plane dispatch was introduced.

All six GPU tests pass on AMD760M/RADV. They include a nonconstant case where
one future diagonal seed changes the first match and propagates through left
and up dependencies, an adaptive UMH match beyond the fixed initial range,
partial image-edge blocks, ties, reference order and rejected input followed
by valid encoding. Three saved-input repeats match every one of the 172,800
raw vectors against the sealed combined-adapter outputs. This does not close
the existing 4,167-vector gap to Studio's native raw fields.

The performance result rejects this implementation. For the saved 3840x1920
inputs, serial CPU coarse preparation takes 343.344 ms. GPU finest setup and
encoding take 21.356/7.494/7.258 ms across the three runs; submission through
completion and the single 2,073,600-byte readback takes
222.584/158.538/159.758 ms. No other agent build or GPU job overlapped this
measurement. These are host-observed diagnostic regions, not timestamp-query
kernel times or an actual-player test. The warm finest stage alone is far
beyond the 33.37 ms source interval, without filtering or view rendering.
The separate 177 ms prior CPU measurement used actual Kjerag inputs and
parallel full searches, so these are not a controlled CPU/GPU speedup ratio.

Do not continue the complete serial-decision GPU port. The prototype is
archived at `0cff5ad1` and retired from active source under MANDATES.
The next candidate will evaluate independent block refinement with immutable
coarse predictors. That changes motion-search semantics and must be tested on
filtered moving output; it is not presumed visually equivalent or accepted.
It does not introduce gradual color-coefficient updates or alter the recovered
pixel-fusion calculation. No new native session/export or installed change.

Evidence is in
`scratch/studio-seam-flicker-612-20260909-01/temporal-gpu-search-01`:
build, shader validation, 11 CPU search tests, six GPU tests and timing logs.
Both existing moving comparisons remain the outstanding human quality gate.
After retirement, all 50 remaining temporal checks pass on AMD, including the
native fixtures. Workspace all-target Clippy, formatting, crate-source and
rename checks pass. No full workspace tests or UI harness is claimed for
this uninstalled experiment. The remaining tree keeps only the useful CPU
finest-preparation boundary and its reference/fixture tests; no rejected GPU
controller is linked. Its exercised test binary SHA256 is
`48d3b4bde4c1f0a61092c406c754453d163023a0328d93a9611ce2ff3ae34d9a`.

### Independently parallel finest-refinement candidate, 2026-09-10

The replacement is explicitly a changed Kjerag motion-search algorithm, not
another exact port. Every finest block reads immutable interpolated coarse
seeds for its spatial predictors and writes a separate record. It retains the
ordered zero/global/own/median/left/up/diagonal candidates, inclusive start
clipping versus strict upper bounds for checked candidates, strict-lower ties,
SAD penalties and fixed radius-one ring. It omits serial bad-block/UMH recovery.
The shader dispatches one 256-thread workgroup per block/reference, evaluating
two candidate batches. The readable CPU oracle remains beside the shader.

The existing packer now also accepts the typed, validated GPU result and one
of its six reference ordinals. It selects that raw-buffer slice without a
readback, then uses the unchanged phase/threshold preparation and integer
confidence law. An arbitrary unchecked GPU buffer is not an input API.
The optional Scene route retains each GPU base with its exact source stamp,
NV12 image and CPU levels. CPU levels six through one still prepare each
reference, optionally in six scoped workers. Refinement, six packing passes,
fusion, conversion and projection share one encoder. This remains diagnostic
execution with explicit waits, not selected playback or a new color policy.

All 60 temporal tests pass on AMD760M/RADV. The new checks include patterned
1031x1027 images, immutable predictor and tie/boundary CPU tests, input refusal,
and six distinct refined-to-packed references before one submission. All
172,800 saved-input records match the new CPU oracle on three GPU repeats.
They differ from the old sealed serial reference at 45,789 records (per
reference: 8058/8021/7641/6028/7836/8205). These differences can affect reference
weights and temporal picture quality; no acceptance follows from oracle identity.

The first measurement has serial CPU coarse preparation 352.069 ms, pipeline
construction 0.881 ms, encode/upload 9.716/7.676/7.530 ms, and submit through
completion/readback 65.809/36.918/31.144 ms. The readback is 2,073,600 bytes.
These are diagnostic host regions, not kernel timestamps or actual-player
capacity. No other build/GPU job overlapped the run. Even warm finest work
alone consumes approximately one 29.97 fps source interval.

The actual Scene uses the same 37 sources at 612 and 35 at 607 as the existing
offline reviews, with the captured ISO100 regime and unchanged conversion,
source-specific stitching and color corrections. It produces 31 and 29 centered
outputs. All 259 plus 245 unfiltered image/map/alpha/color controls are exact;
CSV center/reference indices and timestamps also match. The output pictures
change as expected. At 612, CPU coarse search averages 75.672 ms (max 99.718),
and the GPU/upload/refinement/packing/fuse/conversion/projection/readback region
averages 134.222 ms (max 160.219). At 607 these are 82.207/112.123 ms and
132.368/146.790 ms respectively. These regions exclude earlier panorama
preparation and diagnostic controls; whole runs take 13.20 and 12.59 seconds.

The new 612 movie places the previous offline filter left and the parallel
candidate right. Neither arm is Studio. The 607 movie places the unchanged
cached Studio export left and this candidate right, using the existing fixed
registration. Existing index-derived source labels remain unauthenticated;
this does not supply a Studio oracle for 612. Both movies are lossless RGB,
2560x720 at 30000/1001 fps, with only the top 38 pixels replaced by labels.
All 120 decoded frame bodies match their source pictures exactly. Four-thread
software decoding takes 0.344/0.321 seconds for 31/29 pictures. Root inspected
the rendered 612/607 previews for labeling and picture placement, not a moving
flicker verdict. Owner acceptance remains required.

Evidence, build/test logs, controls verifier, composition commands and movies
are under `scratch/studio-seam-flicker-612-20260909-01/temporal-parallel-refine-01`.
The exercised render binary SHA256 is
`3b33205053a6d2559c141f7124ceaa89de8564d16e26cbe150910485a334e8c6`.
Movie SHA256 values are `d55be279c4c910ef9f4668db37d854443d28df8a7e1fa614a83a0cc56efbd737`
(612) and `ff10e50c5fe5152ca8b86f15eac4edc459e7da9f1a1169976a4e9baee9a56155` (607).
The first test build failed on diagnostic type inference/Debug requirements;
the corrected build ran the recorded tests and pictures. Clippy subsequently
requested explicit parentheses in one synthetic pattern expression, without
changing its arithmetic. Workspace all-target Clippy, formatting, source-lock
and rename checks pass. No full workspace tests/UI harness, new Studio session,
export, app installation, push, merge or accepted gradual update is claimed.

After the test-only parenthesization correction, the final binary SHA256 is
`4ad97650e2382c7ecdefb20f5599f599a66b52868c545f2b12f9ad5a1167fe66`.
All 60 temporal tests pass again in `final-temporal-tests.log`; the three
submit/completion/readback regions are 66.206/42.908/33.837 ms. Report the
observed warm range as 31.1 to 42.9 ms across both runs, not a guaranteed
31 ms stage. The movies above were made by the earlier named binary and are
unchanged; the only later source edit parenthesizes the same synthetic-test
expression for Clippy.
