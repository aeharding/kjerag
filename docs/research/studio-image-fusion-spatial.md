# Selected photometric spatial reference

This records the Windows 5.9.10.0 selected X4 CPU path, not proof of the Mac
6.0.2 producer or other-camera equivalence. The DLL SHA-256 is
`75801cc67d890d0769ed2182136e21d6dc3ca10281c5ec5dd8c8adc3ea95f6dc`.
The readable implementation is `crates/render/src/image_fusion/spatial.rs`.
It does not run automatically during playback. `observe_bands` takes two
aligned 800x16 BGR8 source bands and extended 212x4 validity bytes, performs
content admission and reduction, then runs the spatial reference. The direct
`observe` entry still accepts already-aligned 200x100 working images and
explicitly bypasses content admission. Neither byte-image API establishes
source identity or chooses playback scheduling.

## Source boundary clarified by direct binding review

The selected outer caller produces persistent **800x16** BGR sources at
`+0x2e0/+0x340`, then invokes content predicate `0x183beaf10` before resize.
The predicate reads each source's full 16-row height, but derives strip width
from destination width 200 divided by three. Its three X ranges are `[0,66)`,
`[66,132)` and `[132,198)`; columns 198..799 do not affect admission. The
18 per-channel mean differences are binary64 and trigger on strict absolute
difference greater than 3.0. Empty retained means also trigger. Every trigger
copies all current mean vectors into the retained baseline before MGP; a
non-trigger keeps both the baseline and cached ratio maps.

On admission, `0x183bf4650..0x183bf4778` resizes each full source with
interpolation value 3 into `Rect(0,48,200,4)` of separate retained 200x100
working images at `+0x3a0/+0x400`. This is not a later mutation of the source
header into a 200x4 Mat. Older notes asserting that degeneration are wrong.
The recovered reduction is 4:1 in both dimensions. OpenCV's integer-area
implementation averages the aligned source blocks before converting back to
bytes ([OpenCV resize source](https://raw.githubusercontent.com/opencv/opencv/4.x/modules/imgproc/src/resize.cpp));
this does not identify Studio's exact optimized dispatch. The reference uses
integer sums followed by ties-to-even byte conversion. Surrounding working
rows may start at zero: the subsequent row replication supplies every row
which can affect these selected outputs. A poisoned-surroundings regression
checks this over successive observations.

The native means use binary64 arithmetic. Replacing their comparison with
an integer sum threshold is not bit-equivalent: sums 9 and 3177 differ by
exactly `3*1056`, but their separately rounded means differ by
`3.0000000000000004`. The CPU reference preserves that trigger. This is a
boundary case to disclose when qualifying a cheaper GPU admission policy,
not a reason to emulate binary64 throughout the GPU solver.

The detached sampler now replaces full-chart point sampling with the source
band construction below. `stitch-layers ... estimate-fusion` passes its bands
through `observe_bands` and renders the resulting ratios. A separate GPU
producer and capture-owned automatic integration are now implemented on the
working branch; real playback qualification is pending. The CPU reference
remains available rather than being replaced by shader-specific arithmetic.
Raw binding evidence is in ignored `photometric-static/review-content-gate-*`
and `review-selected-getter-*` files. No new runtime capture was needed.

## Source-band construction

`0x183bec4a0` accepts two 200x100 normalized lens-local UV maps. For each lens,
`0x183beb290` pads one periodic column on either side, then calls OpenCV remap
with interpolation 1 and border 0 through the four-row lookup ROI
`Rect(0,48,200,4)`. The lookup generator `0x183beb560` uses the same spherical
form as the ratio coordinates below, with **positive** `Ry(PI/2)` and
`coord_x = 1 + upper(200*az/TAU,199)`. Its theta denominator is 100, not 99;
its four absolute rows are 48 through 51. This produces two 200x4 source-UV
bands, not source colors yet.

The source sampler interpolates these UVs at endpoint-aligned positions
`(out_x*199/799, out_y*3/15)` to produce each 800x16 band. It then bilinearly
samples the source planes and converts their values to BGR bytes. The native
mode dispatch is `0x183bf5bc0`; the planar sampler and plane workers include
`0x183bee510`, `0x183bf1690` and `0x183bf1800`.

Kjerag uses its already-qualified final packed map, converting the two atlas
X coordinates to lens-local coordinates once. The detached GPU implementation
has separate map-composition and source-sampling dispatches. Map composition
uses five fractional bits, corresponding to OpenCV's
[`INTER_BITS=5`](https://raw.githubusercontent.com/opencv/opencv/4.x/modules/imgproc/include/opencv2/imgproc.hpp);
the subsequent endpoint expansion is continuous. Each lens is sampled locally,
without the drawing consumer's box filter or cross-lens atlas interpolation.
Container-driven float RGB conversion followed by clamping and ties-to-even
byte conversion is Kjerag policy. The native runtime conversion branch and
upstream packed-map producer were not authenticated by this bounded CPU trace.
This is therefore not a claim of bit-identical native source-band bytes.

The native coordinate mask is initialized to zero at
`0x183bec0eb..0x183bec118`. After a packed-map update, ordered `<0` or `>1`
tests on any lens-local UV component mark invalidity; a valid value never
clears it. These ordered comparisons do not reject NaN. Kjerag accumulates
the resulting four rows, periodically extended by six columns, before content
admission, including observations whose colors do not trigger a solve. This
does not identify Studio's map-update or video-frame cadence. Shape validation
precedes every reference state mutation.

Raw lookup, matrix, mask-lifetime and sampler evidence is retained in
`photometric-static/review-upstream-band-mapping.md` and its named disassemblies.

## Verified sequence

The selected method is `0x183c01450`. Its MGP helper returns constructor
fraction `p=0.2` through `0x183c1cb10`; this is not the retained color metric.
The fixed dimensions below are part of this reference's contract.

| Stage | Selected operation |
| --- | --- |
| Current image preparation, `0x183c00bb0` | Copy row 48 to rows 39..47, and row 51 to rows 52..60, inclusive, in each image. Other rows stay unchanged. |
| Extension, `0x183c0e630` | Wrap six whole pixels at each horizontal edge, producing 212x100. |
| Inner MGP, `0x183c1c990` | Correct same-ordinal BGR images with the retained solve. See `image_fusion/solve.rs`. |
| Crop, `0x183c0f510` | Copy `Rect(6,0,200,100)` back to 200x100. No resize. |
| Ratios, `0x183c0d990` | For `d in [0,10)`, left row `50-d` and right row `50+d` receive `(prepared+255)/(current+255)`, independently per BGR channel and own ordinal. |
| Extension within ratios | For the same offsets, left row `50+d` copies left row 49; right row `49-d` copies right row 50. |
| Neutral fill | Set left rows `[35,40)` and right rows `[60,66)` to one. |
| Blur, `0x183c11260` | Normalized 3x11 box over left `[35,55)` and right `[45,65)`. X is periodic; Y is REFLECT_101 about each 20-row ROI. Copy filtered pixels back in place. |
| Remap, `0x183c03470` | Read full 200x100 ratios through one periodically padded column per edge, with absolute Y inclusive gates `[35,55]` and `[45,65]`. Outside writes one. |

The ratios are initialized to all ones at `0x183c0317e` and `0x183c03210`,
then retained. Left row 40 is outside the fresh ratio writes and neutral
fill, so its previous blurred value contributes to the next blur. The
reference retains this row's history; it does not recreate a neutral map
every observation. The other rows used by either blur are freshly written.

Two surrounding operations have no path to the final selected output:
give-back `0x183c0e470` receives the **full** ratio headers and touches left
rows `[0,25)` and right `[75,100)`. After remap, endpoint normalization
`0x183c0e2b0` receives the **original** ratio headers, not the separate remap
destinations, and touches their first 20 rows. None of these rows enters a
selected blur, remap gate, or retained row 40 on a later observation. The
fixed-size reference omits this unconsumed state. Applying either operation
to the blur ROIs or final outputs would be a different algorithm.

## Coordinate map

Init `0x183c02fc0` creates the final 200x100 coordinate pair at owner
`+0xf8/+0x158` with `Ry(-pi/2)`. The generator `0x183c0fba0` uses:

```text
theta = y * PI / H
phi = x * TAU / W
q = (sin(theta)*cos(phi), sin(theta)*sin(phi), cos(theta))
v = Ry(-PI/2) * q
az = acos(v.x / sqrt(v.x*v.x + v.y*v.y + epsilon))
if v.y < 0: az = TAU - az
coord_x = upper((W*az/PI)*0.5, W-1) + 1
coord_y = upper((H-1)*acos(v.z)/PI, H-1)
```

`epsilon` has f32 bits `0x2edbe6ff`. `upper` is an ordered greater-than
branch, preserving NaN. There is no half-pixel offset. The actual sine and
cosine of the rotation are evaluated, not replaced with ideal-axis constants.
The remap's Y values are absolute full-map row coordinates; they are never
rebased by subtracting the gate's lower bound. The reference changes BGR
channel order to RGB only when constructing the renderer's ratio maps.

## Corrections and limits

Direct caller, import, data-literal and Mat-binding review corrected several
claims in the archived ledger and initial scratch notes:

- ROI fills are `+1.0`, not zero.
- The ratio builder receives margin 40, not camera model id 23. Its ten-row
  dispatch leaves the retained left row 40 described above.
- Blur X wraps; it is not edge replication. Only blur has a 20-row source.
- Give-back receives 100-row maps, not 20-row ROIs. Endpoint normalization
  is not applied to final remapped outputs.
- Coordinate azimuth spans TAU, not PI; X is upper-clamped before adding one.

Raw disassemblies and independent reviews are retained privately under
`scratch/source-color-20260907/photometric-static/`. No binary or personal
footage is committed. Readable box and bilinear reductions and Rust libm
are not claimed bit-identical to OpenCV/Windows. The inner CG traversal also
has its previously disclosed numerical-order differences. This reference
does not establish source/output video parity or authorize enabling an
unaligned estimator in playback.

Initial spatial qualification: 27 `image_fusion` CPU tests passed, including a non-gray pair
through inner solve, spatial maps and RGB correction; neutral repeated input;
the retained boundary recurrence; exact right-edge fill; and poisoning all
rows touched by the omitted operations across successive observations.
The initial full required-Radeon workspace passed 1,212 tests, zero failures, 30
ignored, with all-target Clippy and static checks. These are implementation
regressions, not new Studio video comparisons. Neither executable was rebuilt
for that initial spatial increment.

Source-band qualification now passes 45 focused CPU/GPU tests, including the
outer gate, exact area blocks, sticky invalidity on skipped color updates,
all 800 nonconstant composed GPU nodes, opposite-rotation and repeated-UV
negative controls, and textured source samples from both lenses. Normalized
texture filtering is tested at one BGR code, not claimed bit-identical to
scalar f32 interpolation. Both actual reported X4/X2 views render from the
new bands, with finite ratio maps and byte-identical correction-disabled PNGs.
The coordinator viewed both source pairs, corrected views and computed seam
overlays. These do not establish Studio output parity or temporal quality.
The native player and frozen owner review package are unchanged by this
diagnostic build. Evidence is ignored
`scratch/fusion-sampling-20260907/attempt-08/`.
The subsequent full required-Radeon workspace passes 1,231 tests, zero
failures, 30 ignored; all-target Clippy and static gates pass. The tested source
files and both player executables remain unchanged across that run. Logs are
in `scratch/fusion-sampling-20260907/gates-03/`.
