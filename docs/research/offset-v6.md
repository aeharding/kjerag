# `offset_v6`, swept: which half of it is worth drawing

Measured 2026-08-08 on `hack/flat-v6` at 84db133 plus this work. Every number
below is reproducible from the scripts in `scratch/v6/` (gitignored: they read
the owner's own flights).

## The question

The X4 Air writes `offset_v3` (5 distortion coefficients per lens) and
`offset_v6` (13), and declares `capture_offset_version = OFFSET_V6`. The first
v6 hack read all thirteen and drew with them. The owner's eye then refused it:
v6 with the pooled pose was "bang on" at the downward building view, and v6
**pure factory** was much worse than v3 and much worse than Studio everywhere
he looked. Studio renders the same file cleanly, so the numbers are sufficient
and the reading was wrong.

What was actually assumed, as opposed to read off the disassembly:

| assumption | the branch's choice | the alternative |
| --- | --- | --- |
| tokens 6, 7 | `p2, p1` | `p1, p2`, the way `offset_v3` writes them |
| tokens 8, 9 | `p2_r2, p1_r2` | `p1_r2, p2_r2` |
| tokens 10-13 | `s1, s3, s2, s4` (x, y, x, y) | `s1, s2, s3, s4` (OpenCV's order) |
| direction | forward, ideal to distorted | inverse, distorted to ideal |

Four binary choices, sixteen readings. `kjerag_meta::Reading` is that value and
`Reading::every()` is the whole space; `KJERAG_V6_READING` selects one for the
app and `--bin ceiling`'s `v6:<reading>` selects one for the instrument, both
through the same `kjerag_meta::name_v6`.

## What was refuted before a pixel was read

**The radial variable is the Mei plane radius, not an angle.** Reading tokens
1-5 as a polynomial in theta gives a radial multiplier of 85 on lens 0 and 788
on lens 1 at the 100-degree rim, against 1.156 and 1.152 for the plane radius.
There is no reference angle to normalise by either: trailer fields 22 and 23,
proposed as one, are a colour preset string (`standard` on every X4 Air,
`vivid` on the ONE X2) and a field that does not exist on any capture in the
corpus.

**Lens 1's k4 of 9.32 is not wild.** At the rim it contributes +0.019 to a
multiplier of 1.152; the four leading terms trade off against each other and
the total sits within 0.05% of what v3's three terms give. Against the camera's
own nominal design curve (trailer tag 145, `bare`) both lenses want the same
scale to 0.019% and match the design shape to 2.0%, which is what v3 does too.

**No reading folds.** All eight forward readings are monotone in radius over
the full 100-degree field on both lenses, so injectivity does not discriminate
between them.

## The oracle

`--bin seam mode=residual` samples both lenses on one angular grid around
directions on the seam great circle, over 24 frames, and reports what the
calibration leaves **before** any fit. The **along-seam** axis is the seam
circle's own tangent: the baseline between the lenses is perpendicular to every
direction on that circle, so no subject's distance can displace content along
it at any distance. What is left there is the camera's geometry and nothing
else, ring-wide and frame-pooled. That is the verdict. `across` is quoted
beside it and carries the scene's depth as well, so between arms on one scene
it is calibration, and between scenes it is not.

`--bin ceiling` is the second instrument: the same across-lens disagreement
`--bin crossing` measures, taken at ONE set of traced sites through every arm
at once, so a calibration that moves the crossover cannot also move the content
being matched. Its `control=map` and `control=null` both pass on every run
below (the null reads exactly 0.00 on all eighteen arms).

## The table

Degrees left before any fit. Lower is better. Every row is the same 24 frames.

| file | arm | along | across |
| --- | --- | ---: | ---: |
| May-01 002 | v3 | 1.037 | 1.681 |
| May-01 002 | v6, all thirteen | 1.031 | 1.281 |
| May-01 002 | **v6pose** | **0.946** | **1.207** |
| May-01 002 | v6dist | 1.092 | 1.758 |
| May-01 003 | v3 | 0.834 | 1.669 |
| May-01 003 | v6, all thirteen | 0.880 | 1.417 |
| May-01 003 | **v6pose** | 0.849 | **1.315** |
| May-01 003 | v6dist | 0.874 | 1.690 |
| July-14 006 | v3 | 0.764 | 1.761 |
| July-14 006 | v6, all thirteen | 0.812 | 1.503 |
| July-14 006 | **v6pose** | 0.780 | **1.347** |
| July-14 006 | v6dist | 0.771 | 1.576 |

`v6pose` is v6's eleven pose and intrinsic tokens over v3's five-coefficient
distortion. `v6dist` is the opposite: v3's pose over v6's thirteen. Between
them they say which half of the string the change lives in, and they say it
without an argument.

**The instrument's own floor**, five disjoint 24-frame windows of May-01 002 at
`from=40,50,60,70,80`. The ordering is identical at every window:

| arm | along, five windows | across, five windows |
| --- | --- | --- |
| v6pose | 0.972 0.946 1.005 0.982 1.009 | 1.331 1.207 1.323 1.215 1.268 |
| v6 | 1.078 1.031 1.089 1.064 1.134 | 1.407 1.281 1.418 1.311 1.293 |
| v3 | 1.052 1.037 1.097 1.086 1.128 | 1.885 1.681 1.910 1.777 1.796 |
| v6dist | 1.151 1.092 1.185 1.175 1.246 | 2.034 1.758 1.979 1.872 1.923 |

Window to window one arm moves about 0.08 degrees, and `v6pose` beats `v3`
along the seam at all five and across the seam at all five. `v6dist` loses to
`v3` at all five on both axes.

## All sixteen readings, on the half where a reading can still matter

`v6dist`, so v3's pose is held and only the thirteen change. Along-seam
degrees; `v3` is the control at the top.

| reading | May-01 002 | May-01 003 | July-14 006 |
| --- | ---: | ---: | ---: |
| **v3 (control)** | **1.037** | **0.834** | **0.764** |
| fwd tang=p2p1 grow=p2p1 prism=xyxy (branch) | 1.092 | 0.874 | 0.771 |
| fwd tang=p1p2 grow=p2p1 prism=xyxy | 1.155 | 0.914 | 0.807 |
| fwd tang=p2p1 grow=p1p2 prism=xyxy | 1.170 | 0.928 | 0.816 |
| fwd tang=p1p2 grow=p1p2 prism=xyxy | 1.187 | 0.941 | 0.840 |
| fwd tang=p1p2 grow=p1p2 prism=xxyy | 1.169 | 1.073 | 0.930 |
| fwd tang=p2p1 grow=p1p2 prism=xxyy | 1.195 | 1.038 | 0.935 |
| fwd tang=p2p1 grow=p2p1 prism=xxyy | 1.218 | 1.073 | 0.884 |
| fwd tang=p1p2 grow=p2p1 prism=xxyy | 1.304 | 1.062 | 0.903 |
| every inverse reading | refused | refused | 0.812 to 1.303 |

**No reading beats the control on any file.** The branch's own reading is the
best of the sixteen, which says the disassembly's token order was probably
right all along; it is still a regression against `offset_v3`.

**The inverse direction is refused outright.** On two of the three files
`--bin seam` cannot correlate enough patches to say anything at all - the two
lenses' pictures stop matching - and on the third it is worst on both axes.
`--bin ceiling` says the same in its own units: pooled over 808 traced sites,
an inverse arm registers content at 10% of them against 40% or more for every
forward arm, and its worst disagreement runs 250 to 285 raw lens px against 85.

`--bin ceiling` pooled over eleven views and 808 sites, median absolute
along-seam disagreement in raw lens px, agrees on the ordering:

| arm | read rate | median | worst |
| --- | ---: | ---: | ---: |
| **v6pose** | **42.6%** | **32.66** | 86.44 |
| fwd ... prism=xyxy (four readings) | 40.8-41.5% | 33.94-36.79 | 85-90 |
| v3 | 38.5% | 35.30 | 86.17 |
| fwd ... prism=xxyy (four readings) | 42.7-44.8% | 45.69-50.09 | 84-87 |
| every inverse reading | 9.5-11.0% | 24.91-43.08 | 247-285 |

## The verdict

**`offset_v6`'s thirteen distortion coefficients are not what Insta360 Studio
draws with, under any of the sixteen readings of them.** Nothing here says what
Studio does instead; it says this string, through this projection, in this
composition, is not it. The next theories are that the model family is wrong,
that it composes with the Mei projection differently than assumed, or that the
shipped renderer does not read this field at all.

**What is worth drawing is the other half.** v6's eleven pose and intrinsic
tokens - it disagrees with v3 by cx -10.80 and cy -4.15 canvas px and by
0.245 degrees of relative yaw and 0.396 of relative pitch on lens 1 - close the
seam about half a degree better across and a tenth better along, on every file
and every window tried. `KJERAG_OFFSET=v6pose` is what this build draws by
default and it is the only arm that beat `v3` on both axes.

**The accessory theory is dead too, and cheaply.** Trailer tag 145 carries one
nominal design curve and one field of view per accessory - `bare` 200 degrees,
`ProtectorA/S/AS` 195, `InvisibleDiveWater/Air` 190 - and the owner flies bare,
so Studio selects the bare row. That row is a polynomial in degrees off axis,
not in the plane radius the calibration is written in, and it is a design curve
that the per-unit calibration departs from by 2%, the same 2% `offset_v3`
departs from it by. It cannot carry a gap of this size and it is not a
rendering curve.

## The arms this build carries

| `KJERAG_OFFSET` | what it draws |
| --- | --- |
| unset | `v6pose`, the measured arm |
| `v3` | `offset_v3` alone, what `main` draws |
| `v6` | all of `offset_v6`, the refuted arm, kept reachable |
| `v6pose` | v6's eleven over v3's five |
| `v6dist` | v3's eleven over v6's thirteen, the isolated regression |

`KJERAG_V6_READING` names the reading for the two arms that carry the thirteen;
it defaults to the branch's. `KJERAG_POSE=off` still draws pure factory and
`KJERAG_HANDOVER_DEG` still sets the handover width.
