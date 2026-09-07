# Selected photometric spatial reference

This records the Windows 5.9.10.0 selected X4 CPU path, not proof of the Mac
6.0.2 producer or other-camera equivalence. The DLL SHA-256 is
`75801cc67d890d0769ed2182136e21d6dc3ca10281c5ec5dd8c8adc3ea95f6dc`.
The readable implementation is `crates/render/src/image_fusion/spatial.rs`.
It does not run automatically during playback. Its input is an already
aligned pair of 200x100 BGR8 images in the working chart, plus the inner
helper's extended 212x4 validity bytes. Source sampling and the outer content
gate remain separate obligations.

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

Qualification: 27 `image_fusion` CPU tests pass, including a non-gray pair
through inner solve, spatial maps and RGB correction; neutral repeated input;
the retained boundary recurrence; exact right-edge fill; and poisoning all
rows touched by the omitted operations across successive observations.
The full required-Radeon workspace passes 1,212 tests, zero failures, 30
ignored, with all-target Clippy and static checks. These are implementation
regressions, not new Studio video comparisons. Neither executable was rebuilt.
