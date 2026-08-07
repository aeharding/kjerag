# Owner reference views

The acceptance registry for seam work. Lines are runnable as CLI args and Ctrl+V targets.
Footage lives on the owner's test box under ~/Videos (owner ruling 2026-08-01: footage filenames
are fine in the repo). A bare filename is in ~/Videos or ~/Videos/Insta; a file that has moved out
of those carries its whole path on the line, quoted where a directory name has a space in it. A line
carrying a path like that is a CLI argument only, and is NOT a Ctrl+V target: `Framing::read_line`
takes the first whitespace word as the path, so a quoted directory splits mid-quote, and nothing
expands a tilde, which makes such a paste a silent no-op (issue #157).
Agents: read this at the start of any seam task;
add new owner references here with date, category, and status.

**Every `lock=1` yaw below was re-derived on 2026-08-06, and every line says what it used to be.**
The yaw in one of these lines is measured in the stabilized frame, and the owner's ruling that day
made that frame world-fixed (#165): its zero used to follow the aircraft's slow heading and is now
the heading the file opened on. The two differ by however far the old follow had been carried,
which is not small and is not one number per file. The rule is `new_yaw = old_yaw + carried(t)`,
where `carried` is the old filter's own low-passed heading at that instant: worth degrees a second
while the aircraft turns and nothing at all while it flies straight. On
VID_20260714_193252_00_006 it is 6.8 degrees at the first frame, 44 by 6.5 s and 157 by 36 s, so
two lines a second apart in one file get different corrections. `--bin carried` computes it per
line, by solving the same IMU track under both filters and differencing their headings at the
frame the line names:

```sh
cargo run --release -p kjerag-spike --bin carried -- <file.insv> time=36.303 yaw=3.78
```

Every line below went through that and was then checked in the picture, the commit before #165
(67a4bcf) rendering the old line against this build rendering the new one, registered by phase
correlation over the middle half of a 1024 px `--bin reframe` render. Fourteen lines: twelve match
at zero pixels and two at 2 px, which at fov 20 is 0.04 degrees, with correlation 0.92 to 1.0000
and 0.05 to 3.5 codes of mean absolute difference. The control that says the check can fail is the
same comparison with the yaw left alone, and it reads 1.7 to 20 degrees out; where the content is
smooth enough that a slid picture still correlates, the slide itself is the reading.
**A `lock=1` yaw saved anywhere else - a note, a screenshot, a shell history - still points
somewhere else and needs the same treatment.** `lock=0` lines are unaffected. Re-read the horizon
caveat in the Motion section, which is the same trap at a smaller size.

## Motion: what the band applies, and how much of it moves (`--bin shear`)
- 2026-08-05 `VID_20260714_193252_00_006.insv time=36.303 yaw=162.31 pitch=5.44 fov=20.00 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=3.78`, carried +158.53 deg. Match 0.04 deg, correlation 0.9994;
  the same comparison with the yaw left alone reads correlation -0.30. #165 published `yaw=160.63`
  for this line, which is the same derivation taken off a half-second grid and 1.68 degrees short;
  the numbers below are at the aim above, and where the two differ this line says so.
  The shimmer view, and the acceptance instrument for the seam epic's motion work. The seam
  sweeps 393 px down the picture over the three seconds this measures, 540 px of travel counting
  the way back, so everything here is measured against the seam's own row rather than the
  picture's. **The world-fixed lock did not stop that sweep and made it bigger**: the same run on
  67a4bcf at the old aim reads 329 px end to end and 483 of travel, because the body now turns
  under a parked view instead of carrying it round. The acceptance command is that
  line plus `frames=90 warm=6.0 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`,
  and the `seam=` is not optional: fitted from the file instead, the same view reads 0.028 deg
  of total displacement at -150 px rather than 0.340, because what the band applies is what the
  calibration left it. **Two columns and they are not the same number**: `size` is the whole
  displacement and `along` is its along-seam part, and every "applied" figure below is the
  `along deg` column unless it says otherwise.
  Live arm against the same frames with the band held off, 90 frames, READ AGAINST MAIN AT #165
  (see the horizon caveat below): `-150` px 0.3381 deg along (0.3397 total) at 0.0066 deg step
  rms over 87 pairs, worst single step 0.0187; `+0` 0.3356 along (0.3564 total) at 0.0687 with a
  worst single step of 0.4582 over 89 pairs; `+60` 0.0820 along (0.1871 total) at 0.1471 over 89;
  `+150` 0.0022 along (0.0738 total) at 0.0091 over 76. Those four offsets
  are -2.93, 0, 1.17 and 2.93 degrees off the seam, and at the 8 degree handover the pass draws
  now all four are INSIDE it, so `+150` is not an unbent floor and has not been one since #162.
  The band's own state moves 0.0449 deg rms between frames on the bend and 0.0008 on the
  along-seam field, which is where it was before the lock changed. `mode=profile` puts the
  along-seam plateau at 17.21 px (0.3362 deg), still 15.56 px at +0.47 deg, 1.19 px (0.0232 deg)
  by +1.41, and only out past +3.5 degrees does it reach the 0.09 px its far quarter averages;
  the handover is bracketed inside +24 to +72 px, 0.94 degrees of view. The instrument's own
  bracket line said "down to 0.09 px by +1.41 deg" until 2026-08-06, which reads as the value
  there and is not: +1.41 is where the field comes within a tenth of the plateau of that floor.
  `null=1` holds both arms and reads exactly zero at every probe on every frame; `mode=plant`,
  run with the same `seam=` as the rest of this line, yaws the second arm 0.05 and
  0.10 deg and reads the expected -2.534 and -5.068 px back to within 0.026 px, every band,
  ratio 1.9946 to 1.9971.
  READ THE `+0` STEP RMS AS ONE FRAME AND NOT AS A FLOOR (2026-08-06, this rebaseline). #165
  reported that number falling 0.0773 to 0.0099 across the lock change, an eight-fold improvement
  in the instrument's floor. It does not survive re-derivation: at the aim above it reads 0.0687,
  and all three readings are one frame's correlation failure. Drop the two worst steps of each and
  the same runs read 0.0097 before, 0.0088 at this aim and 0.0094 at #165's, which is the floor and
  it did not move. What the outlier is: at this aim frame 6 of 90 reads -0.67 px of along-seam
  displacement between two frames reading 18.4 and 18.5, at a correlation peak of 0.905 that the
  gate accepts. **What the lock change did move is how many frames yield a reading at all**: at
  `+0` the usable step pairs go 71 to 89 and at `+60` 30 to 89, because fewer frames are refused
  for a seam past `TILT_LIMIT`; `+150` goes the other way, 83 to 76. That is the real result and
  it is a yield, not a floor.
  CAVEAT, THE HORIZON: everything above except the null, the plant and the band's own state is a
  reading about where the seam lands in THIS view, and under `lock=1` that is decided by the
  orientation track. #158 reseeded it and moved the seam 23 to 45 px down this window, a mean of
  about 35, which took `-150` from 0.3641 deg to 0.3663 and `+60` from 0.0417 to 0.0578 with its
  readings from 43 to 30, on an instrument that did not change. The band's own state moved by 0.000002 across the
  same merge, because it is fitted in the body's frame and these bands are read in the view's.
  Re-read this line after any horizon-seed or lock change before treating a move as the band's.
  #165 is the second instance and the largest: the rows above are the third reading of one view,
  and the aim they are read at had to be re-derived before any of them meant anything.
  CAVEAT, the `+60` band is the fragile one and only it. It was fragile by yield before the lock
  changed: 30 of 90 readings correlated and 23 of them were on neighbouring frames, so its step rms
  rested on a quarter of the run. At the world-fixed aim it yields 89 pairs of 89 and steps by
  0.1471 deg, which is the same eighth of a degree over three times the run. What has not been
  re-measured is its response to the acceptance gate: sweeping `KEEP_PEAK` 0.78 / 0.80 / 0.82 moved
  it 0.1276 / 0.1350 / 0.1412 deg, a 10 percent span, with the other three bands unmoved to four
  decimals, and that sweep is a pre-#165 reading over the thin run. Read `+60` as a band that steps
  by about an eighth of a degree, not as a number, and re-run the sweep before quoting it.
  `mode=profile`'s `+24` to `+48` are the same species, thinnest at `+24` and `+36` (79 and 80
  pairs against 87 either side).
  CAVEAT, THE BRACKET IS A LOWER BOUND (2026-08-05, `research/handover-fade`): the 0.94 degrees
  above is the width the instrument resolves the handover to, not the width the map hands over
  across. The held arm carries the two lenses' whole 18.7 px disagreement as a double image over
  the same corridor, so the match has two peaks and reports whichever leads, which turns a ramp
  into a step. What the picture actually carries of the along-seam correction is lens 1's weight,
  and the Rust twin reads it with no correlation in the way: a smooth ramp over the whole 2.00
  degree crossover, nine tenths of it at +0.86 deg and one tenth at -0.76, so the applied shear is
  0.182 deg per degree of view and not 0.52. Widening the map's own handover from 2 to 4 degrees
  left this printed bracket at +24 to +60 px, unmoved, and 8 degrees moved it only to +12 to +72,
  which is 1.17 degrees of a ramp that is 8; at the 8 the pass draws now, re-read at the
  re-derived aim, it prints +24 to +72. Do not quote the bracket as the handover's width, and do
  not read a change in it as proportional to a change in the map.
  OWNER VERDICT, THE WIDTH (2026-08-05, `fade-ab.sh`, LABEL-BLIND): two arms of one binary, arm 1
  the shipped 2 degree handover, arm 2 an 8 he was not told about. **"2 is way better. Def not
  perfect but way better"** - said of arm 2, the 8. Shipped as the default on
  `feat/handover-width`.
  READ THIS BEFORE QUOTING A WIDTH NUMBER: every instrument that has an opinion on the handover's
  width is MONOTONE in it and not one of them picks 8 or anything else. Sharpness over the overlap
  falls with width at all five reference views, the doubled band grows, the corridor's own step
  statistics get worse, and only the epipolar shear improves. The width is a perceptual call and
  the numbers price it rather than make it.
  READ THIS BEFORE QUOTING A `mode=blend` NUMBER (2026-08-06): that instrument's `bands=` rows are
  a SYNTHETIC linear crossover it builds itself, so their doubled band is `0.8 * width` by
  construction and grows exactly linearly. The map's own is the `shipped` row, swept with
  `KJERAG_HANDOVER_DEG`, and it is not linear: at this view (`yaw 90 fov 60 fit=1`) the doubled
  band reads 1.50 / 2.79 / 3.89 / 4.78 / 5.41 and the sharpness 1.309 / 1.247 / 1.194 / 1.150 /
  1.120 at 2 / 4 / 6 / 8 / 12 asked for, so four times the width doubles 3.2 times as much picture
  and costs 12 percent of the sharpness, and the 12 column is really the 9.69 this file affords.
  AND EVERY ROW OF THAT MODE, `shipped` INCLUDED, IS DRAWN WITH THE PER-FRAME BEND OFF (empty
  cells, `Weighting::at`). Far field that is the picture to within the bend's own size, which is
  why the numbers above stand; near field it is not the picture at all, because the bend IS the
  near-field mechanism. Score a view with somebody's gear on the seam with `--bin band
  mode=render`, whose `share` column comes off the real pass with the band pass live: at the
  May-26 gear (0.99 m) it falls 1.387 -> 1.182 across 2 to 8 and at the May-01 under-pilot view
  (0.84 m) 0.725 -> 0.613, about 15 percent, against 9 to 10 percent at a far-field azimuth of the
  same two frames.
  What the numbers do settle is the ceiling: the handover reaches `width/2` off the seam plus the
  bend it carries, and past the two lenses' shared ring the crossover stops deciding the blend -
  the coverage depth takes the weight over and steps it to zero at the rim, and a bent ray that
  lands off a lens's picture is weighed zero and never sampled (this line said "a sample from off
  the end of a fisheye circle" until 2026-08-06, which is not what the code does). Measured per
  file with `--bin band`: six X4 Air files afford 9.36 to 9.82 degrees and the ONE X2 affords 3.99,
  so the width is clamped per camera and 12 is refused everywhere.
  What the owner still sees at 8 ("def not perfect") is the un-shrunk 0.36 deg disagreement, which
  stage 9's estimator owns and a width cannot reach.
  WHAT THE POOLED FIELD DOES TO THIS LINE (2026-08-06, stage9.md 8): with a five-term field fitted
  on the five flights that are not this one and composed with the same `seam=`, the `along deg`
  column reads 0.0286 / 0.0232 / 0.0085 / 0.0019 at -150 / +0 / +60 / +150, against 0.3204 /
  0.3189 / 0.0791 / 0.0008 with no field, and the `+0` step rms falls 0.0690 to 0.0167 with the
  worst single step 0.4475 to 0.0565. **That is not the picture's correction shrinking, it is the
  correction moving**: this instrument measures what the BAND applies, and the band now reads
  through the table and applies what the table still leaves. Read a plateau from this line only
  together with whatever table the run was given.

## Geometry: along-seam axis (stages 5+6, merged)
- 2026-08-01 `VID_20260714_193252_00_006.insv time=2.836 yaw=111.83 pitch=4.12 fov=20.00 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=93.99`, carried +17.84 deg. Match 0.04 deg, correlation 1.0000;
  with the yaw left alone the same comparison is 3.05 degrees out.
  The original horizon-offset report. main 32.8px -> stages 5+6 9.4 cold / 15.4 warm (wide window).
  Owner verdict: "def looks better, albeit not perfect". Residual is distance-dependent (local, not pose).
  READ THE rms COLUMNS BEFORE QUOTING A STEP FROM HERE (2026-08-05, PR #154). `--bin step` prints each
  fitted line's own rms beside the step, and at this view in the WARM state one side of one window
  follows scenery instead of the horizon and reads 13 to 14 px where a line that describes its own
  points reads 0.5 to 2.1. It still prints a step, and that step is not the seam's. Two builds are
  comparable only where both their fits are clean, which at this view is the cold pair.

## Geometry: the along-seam axis after stage 9 (VERDICT: no table)
- 2026-08-06 The two May-01 crossings below, `--bin crossing bins=180` under the pooled fit,
  read the along-seam median magnitude at **1.30** (GOOD) and **1.43** (BAD) view px, sensitivity
  0.03 to 0.04. Taken at 67a4bcf, before the world-fixed lock, at those views' pre-derivation
  yaws; the entry below re-measured the same views both ways and the along-seam medians agree to
  0.14 source px, so the lock change does not reach these two numbers. That is 0.071 and 0.088 degrees, and it agrees with `--bin table`'s reading of
  the same residual round the whole ring on six flights (0.064 to 0.128 deg rms per capture).
  **Stage 9 refused to fit a per-azimuth table for it**: the part above what the pass already
  applies is 3.7% of the leftover, it does not predict a held-out flight, and the best any static
  table reaches on one is +1.25% (docs/research/stage9.md). The refusal carries a size: a static
  per-azimuth field above 0.02 to 0.06 deg (0.37 to 1.1 view px here) is excluded, below 0.02 is
  not.
  **What DOES reproduce is the five-term field itself**, on nine captures of two cameras at full
  reading density (layer-2 preflight, `research/layer2-preflight`): fitted on other flights only
  it takes the pooled along-seam leftover 0.0536 -> 0.0211 deg on this camera, 9 of 9 improved.
  Measured at this very view: with a five-term field fitted on the Jul-14 flight and held out, the
  GOOD view's along-seam median magnitude goes **1.30 -> 0.07 view px** with the epipolar median
  unmoved, and the BAD view 1.43 -> 1.06. That is a pose-order field pooled per camera, which is
  layer 2, not a per-azimuth table. stage9.md 4.5 withdraws that document's "does not reproduce"
  sentences: they were the mean reduction, not the camera.
  **SHIPPED 2026-08-06, and re-read here through the shipped path** (stage9.md 8): the field is
  learned by watching, pooled per camera beside `SeamFit`, and composed with the pool's pose at
  open into `band::Table`. At these two views under `seam=roll:0.795,yaw:-2.310,pitch:-0.936,
  cx:-3.28,cy:-11.91`, with the field fitted on flights that are NOT May-01, the along-seam median
  magnitude reads **GOOD 1.29 -> 0.12 and BAD 1.47 -> 0.86 view px**, and the epipolar median moves
  0.02 to 0.15 view px against this instrument's own 0.01 to 0.17 of dither sensitivity, which is
  unmoved. **Both crossings improve and neither is traded for the other.** A field off the Jul-14
  flight ALONE reads 0.97 and 1.09 here rather than the 0.07 above, and the difference is reading
  density and not the code: that number was fitted on 1200 moments of that flight and this one on
  48 frames of it. Read 1.29 rather than 1.30 as this build's baseline for the same reason the
  entry above gives - the two agree to the instrument's floor.

## Geometry: local vs pose field (VERDICT PENDING - the "optimizing some parts not others" family)
- 2026-08-01 `VID_20260410_185407_00_004.insv time=43.143 yaw=93.36 pitch=-2.43 fov=33.95 lock=1`
  vs `VID_20260410_185407_00_004.insv time=45.112 yaw=-86.05 pitch=3.18 fov=38.28 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=127.56` (carried -34.20) and `yaw=-52.37` (carried -33.68).
  Match 0 px on both, correlation 1.0000 and 0.9999; with the yaws left alone the same comparison
  reads correlation 0.37 and 0.86. Two seconds apart and 34 degrees of carry apart, which is why
  the pair needed two corrections and not one.
  Two seconds apart, opposite yaws; one acceptable, one not. Skies photometrically clean (6x contrast) -
  pure geometry. Evidence: .worktrees/stage7/scratch/stage7/sky-apr{1,2}-warm.png
- 2026-08-01 `VID_20260501_183417_00_002.insv time=50.117 yaw=-80.28 pitch=0.06 fov=55.69 lock=1` GOOD
  vs `VID_20260501_183417_00_002.insv time=50.117 yaw=101.13 pitch=0.75 fov=62.79 lock=1` BAD
  RE-DERIVED 2026-08-06: both were 5.85 degrees of carry short, from `yaw=-74.43` and `yaw=106.98`.
  Match 0 px on both, correlation 1.0000 and 0.9999; with the yaws left alone the same comparison
  slides 5.38 and 5.27 degrees.
  Same instant, the two seam crossings: one matched, one mismatched where the lens intercepts the
  horizon on the other side. Owner: "not blending - this is a seam mismatch issue." The cleanest
  demonstration yet that a global field fits one crossing at the other's expense.
  STATUS 2026-08-06, and the contrast is intact under the world-fixed lock. `--bin crossing` at
  both re-derived views, `bins=180 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`:
  GOOD reads an epipolar median of -3.65 source px (magnitude 4.61, spread 2.20) over 19 accepted
  sites of 37, BAD -13.37 (magnitude 13.37, spread 1.91) over 37 of 41. The same views on 67a4bcf
  at the old yaws read -3.65 and -13.23, over 19 of 37 and 38 of 41, so **the lock change moved
  this reading by 0.14 source px and moved nothing about the verdict**. Sensitivity is 0.01 to
  0.06 view px at a thousandth of a degree of dither either side of the change, which is this
  instrument's floor and not an improvement on it. The along-seam medians agree the same way:
  -6.48 against -6.47 at GOOD, -6.81 against -6.95 at BAD.

## Calibration starved (issue #130, X2) - fitted on fix/130-x2-fit, owner test pending
- 2026-08-01 `VID_20251018_191318_00_002.insv time=77.978 yaw=5.62 pitch=-3.41 fov=70.80 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-13.77`, carried +19.39 deg. Match 0 px, correlation 1.0000;
  with the yaw left alone the same comparison slides 17.2 degrees.
  October X2, mid zoom. Ran the factory calibration: watch-to-calibrate matched only 2-3 of 72
  azimuths on every X2 capture (the search was centred on an extrinsic 2.8 deg out, and widening it
  was broken by rectangle sampling). Per-candidate refusal plus an acquired search centre
  (seam-two-axis 11) take the three captures to 50/42/65 azimuths and a pool entry. Round the ring
  at this moment: 2.570 deg along and 2.830 across on factory, 0.257 and 0.267 fitted. Local step
  where the horizon crosses the seam, `yaw=-2.11 pitch=3.6 fov=35` (was `yaw=-21.5`, the same
  +19.39 of carry; the picture verifies, 0 px and correlation 1.0000 against 67a4bcf at the old
  yaw, 15.9 degrees out with the yaw left alone): -3.11 deg factory, -0.61 deg fitted.
  **What is UNVERIFIED here is the lock flag and not the aim.** The fragment does not say whether
  it was locked; an unlocked pair of renders is byte-identical across this change (0.00 codes), so
  the picture cannot tell; and `--bin step` at this view answers "no horizon fitted on both sides
  of the seam" on either flag today, so nothing reproduces the two numbers above either way. If
  that fragment was `lock=0` it never moved and `yaw=-21.5` stands.
  Pictures: `scratch/x2fit/october-{factory,file}.png`.

## Photometric: brightness/color at the seam (stage 7 merged-in-draft, stage 8 in progress)
- 2026-07-31 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=488.855 yaw=-5.17 pitch=2.56 fov=218.99 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=67.24`, carried -72.41 deg. Match 0 px, correlation 0.9999;
  with the yaw left alone the same comparison slides 20 degrees.
  Stage 3's soil reference (sun lighting one lens). Step 2.265 -> 1.424 codes under stage 3.
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=-86.02 pitch=-17.08 fov=114.41 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-72.20`, carried -13.82 deg. Match 0 px, correlation 1.0000;
  with the yaw left alone the same comparison slides 9.8 degrees. The same correction applies to
  the SMOKE and ANTI-ACCEPTANCE entries below, which are this view again.
  The May wide "do a lot better with blending" view: ADDITIVE 6.5-code step on 17-24 code soil
  (28-38% perceptual), channel-uniform. Stage 7 moves 0.7 of 6.5; stage 8's primary target.
  Evidence: .worktrees/stage7/scratch/stage7/evidence-may-stretched.png
- 2026-08-01 `VID_20260501_183417_00_003.insv time=99.032 yaw=-66.27 pitch=-37.28 fov=101.47 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-73.42`, carried +7.15 deg. Match 0 px, correlation 0.9630;
  with the yaw left alone the same comparison slides 6.5 degrees.
  "Another color/brightness/whatever" - wide, pitched down, ground-dominated. In stage 8's set.
  STATUS after stage 8: 1 px excess over the same content elsewhere is under the JND; the long-lag
  Weber improves and what is left is the wide-matching ramp.
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=-86.02 pitch=-17.08 fov=114.41 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-72.20`, carried -13.82 deg, verified with the May wide entry
  above, which is the same view.
  The SMOKE view: the same May wide view whole-frame (no content window), which is the render the
  owner pointed at by name ("for example smoke3-2-drawn") when he ruled stage 8's first form not
  aggressive enough. STATUS after stage 8: the +-8 degree mismatch goes 7.55 codes to 2.93, and the
  1 px excess over the same content elsewhere is -0.73 percent, i.e. no line the content does not
  read everywhere. Evidence: .worktrees/stage7/scratch/stage8/evidence-smoke.png
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=669.369 yaw=-66.00 pitch=-16.05 fov=30.56 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-60.70`, carried -5.30 deg. Match 0 px, correlation 0.9626;
  with the yaw left alone the same comparison slides 5.1 degrees.
  Sent with no commentary straight after "to the eye, it still effectively looks like a line".
  Same May file, pitched down, fov 30.6 - a fine view, 0.0064 deg per pixel. STATUS after stage 8:
  1 px excess -0.82 percent and 2 px -0.69, so the line there is NOT photometric at any scale an
  edge lives at. Evidence: .worktrees/stage7/scratch/stage8/evidence-zoom30.png
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=95.00 pitch=0.00 fov=60.00 lock=0`
  The GEOMETRIC CONTROL, chosen by the instrument rather than by the owner: the azimuth his own gear
  crosses the seam at. `lock=0`, so this line means what it always did and was not re-derived. It reads a 1 px excess of +5.87 percent over the same content elsewhere, and
  the photometry moves it by 0.00 - before 5.94, after 5.94. Any view whose line survives the
  photometry belongs to the local-vs-pose verdict above, not here.

## ANTI-ACCEPTANCE: the artifact the acceptance layer was blind to
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=-86.02 pitch=-17.08 fov=114.41 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-72.20`, carried -13.82 deg, verified with the May wide entry
  in the section above, which is the same view.
  The owner's own screenshots at this area (his ~/Pictures/Screenshots/Screenshot_2026-08-01_20-24-45.png
  raw, _20-25-26.png with his red annotations) show dark STREAKS across the soil running away from the
  seam. Stage 8's per-direction offset over wide support painted each direction's own noise along that
  direction's sweep. **Every acceptance statistic in this campaign straddles the seam and could not see
  it.** Any photometric work must now also pass the FIELD-INTERIOR COHERENCE metric
  (`--bin colour`, the interior block): the applied correction sampled 4-60 deg OFF the seam on dark
  content, binned by azimuth, reported as the rms of what a five-term harmonic cannot describe.
  Rejected build reads ROUGH 1.01%; nulls 0.000%; planted 0.5 and 2.0 code ripples read 2.07% and 8.27%.
  A correction that is smooth round the ring reads zero however large it is.

## Photometric: a green cast on the sun-facing lens (owner 2026-08-02, OPEN - stage 10 gate)
- 2026-08-02 `VID_20260410_185407_00_004.insv time=594.027 yaw=-89.89 pitch=-62.95 fov=41.19 lock=1`
  and `VID_20260410_185407_00_004.insv time=602.368 yaw=-139.23 pitch=-37.74 fov=71.04 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-129.09` (carried +39.20) and `yaw=-132.22` (carried -7.01).
  Match 0 px on both, correlation 0.9244 and 0.9878; with the yaws left alone the same comparison
  reads correlation 0.06 and 0.15. The first of the two is the weakest match in the registry and
  the reason is measured, not guessed: sweeping the new build's yaw across the old build's picture
  peaks at -89.86 (r 0.9937, 0.87 codes) against the -89.89 the rule gives, so the rule lands 0.03
  degrees out here. That is about two milliseconds of this instant, which slides 0.565 degrees of
  picture per frame, and the difference image is FLAT with radius (mean absolute 3.29 / 3.54 /
  3.55 / 3.22 / 2.91 codes, centre out in five rings), which a residual roll about the view axis
  could not be. What is left is a sub-pixel residual on low-contrast fine texture, 13.9 codes of
  standard deviation on dark ground. Why the rule lands 0.03 out on the two fastest-turning views
  and 0.00 elsewhere is NOT settled: the stored track is interpolated between samples 5 ms apart
  that each carry their own carry, which is the right size of thing, but nothing here isolates it.
  A green cast on the lens facing the sun and not on the other, owner-reported on the April file and
  raised as an acceptance blocker. Colour at the seam is not new and this entry claims no novelty:
  insv-format.md 6.11 measured the per-channel step on 2026-08-01, including the sun isolated on a
  corpus X4 at +6.08/+10.39/+16.37 codes R/G/B with the sun in one lens against +7.23/+7.67/+7.70
  with it in neither, and the owner had already named "the change in colour at the seam, especially
  on the sky or when the sun is in one of the lenses" as the worst part left. Per-channel numbers
  have been inside acceptance decisions since then (seam-blending.md 6: the hue step 3.4-5.6 codes
  to under a code on his captures, which is why that view's line was ruled not colour). What is new
  is the cast being CONFINED to the sun-facing lens, named as a hue, on the owner's own capture, and
  arriving after stage 8's applied photometry was rejected: the measurement layer survived the
  rejection and nothing corrects this in the picture today. The first view is steeply pitched down at
  -62.95, the second moderately at -37.74, both onto dark ground, which is where an additive cast
  would show most. Veiling glare or an internal reflection is the hypothesis and not a reading: 6.11
  could not separate a gain from an offset on the owner's own sun-in-one-lens content. Eight seconds
  apart at 41.19 and 71.04 deg, so the pair asks a correction to hold across framing and across time
  rather than at one instant. Owner: "needs correction and/or blending."
  STATUS: OPEN, no evidence renders yet. Gates stage 10.

## Hard mode: geometry and photometry at once (owner 2026-08-02, OPEN - stages 9 and 10)
- 2026-08-02 `~/Videos/Insta/ab_testing/"clip 1"/VID_20260802_191029_00_002.insv time=31.064 yaw=-64.71 pitch=-31.44 fov=142.89 lock=1`
  RE-DERIVED 2026-08-06: was `yaw=-144.04`, carried +79.33 deg, the largest correction in the
  registry. Match 0 px, correlation 0.9997; with the yaw left alone the same comparison slides
  12.1 degrees and the correlation falls to 0.69.
  A same-day capture that is outside the pool as it stands, which is what makes it evidence about a
  correction rather than about the corpus the correction was tuned to. Nothing enforces that: a
  `SeamSample` is five angles, a patch count and a residual, the pool is keyed by camera and stores
  no file identity (`crates/app/src/config.rs`), so playing this file folds it in like any other and
  the hold-out is a habit, not a mechanism. At fov 142.89 the frame should hold both seam crossings
  and the sun at once, which would put glare, the colour cast above and azimuth-varying alignment in
  one picture; that has not been measured on a render yet. Hard-mode gate for stage 9 and stage 10
  alike. The file is clip 1 of the Studio corpus below, and ~/Videos holds no other copy of it.
  STATUS: OPEN, unmeasured.

## The owner's Studio oracle corpus (built 2026-08-02, ~/Videos/Insta/ab_testing, never committed)
Owner-built A/B material: Insta360 Studio's own render of a named position beside kjerag's, so a
defect can be scored against a stitcher that ships. Layout is `clip N/` holding the .insv, with
`snap N/` folders inside it, one per position, each holding `studio_config.png` (Studio's pan, tilt,
roll, field of view, distortion and timecode for that exact position), `studio_screenshot.jpg` and
`kjerag_rough_screenshot.jpg`. Paths only in this repo; no image out of it is committed.
- clip 1 `VID_20260802_191029_00_002.insv`, the hard-mode capture above. snap 1 puts the sun directly
  in one lens. Owner findings: Studio struggles in the glare, but its glared-to-sheltered edge is
  smooth and its seam is invisible outside the immediate glare; kjerag's seam is abrupt and extends
  well past the worst of the glare.
- clip 2 `VID_20260526_191025_00_004.insv`, the May file most of the photometric entries above are
  measured on, and now the only copy of it under ~/Videos. snap 1 is the dirt reference. Owner
  finding: Studio renders the seam invisible to the eye.

Purpose (owner ruling 2026-08-02): Studio is the comparative bar for mitigation-shaped defects, which
is stage 10 and near-field alignment. Far-field alignment answers to the absolute zero-defect bar and
not to Studio.
STATUS: two clips, one snap each, no measurement taken off either yet.

## Standing bars
- Pixel-perfect horizon at zoom is an acceptance criterion (owner, 2026-07-31).
- "Perceptually minimizing the seam as much as possible" is THE objective; sky is the hardest canvas
  (owner, 2026-08-01). Stage 8 bar, tightened by the coordinator after the owner's "still
  effectively looks like a line": max local Weber contrast at/below JND (~1%) at the 1 and 2 pixel
  lags on ALL content, not only flat. Measured as the EXCESS over the same statistic straddling a
  line a few degrees away in the same window, because texture reads a few percent everywhere and a
  raw number cannot tell a line from a scene.
- 60fps dual-stream full resolution realtime on the owner's device; research may exceed budget,
  shipped form needs the story (owner, 2026-08-01).

## Off the seam: other owner-reported defects
- 2026-08-02 the locked horizon leans while circling (issue #152, branch
  `research/horizon-lock-repro`). The owner reported it in the opening 30 s of
  `~/Videos/Insta/ab_testing/"clip 1"/VID_20260802_191029_00_002.insv`, and his "+-45" is the total
  swing rather than the amplitude: the repro fits a once-round sinusoid averaging 18.9 deg of
  amplitude over 90 instants, 16.3 to 35.5, which is about 40 deg peak to peak and about +-19 either
  side. Its azimuth walks round with the aircraft's heading rather than sitting at 0 and 180 deg.
  The fits are the branch's walk over a capture's opening, 0 to 45 s at half-second steps; which
  capture is not recoverable from the CSV, which carries no file identity. #152's own line names
  `VID_20260729_191815_00_005.insv`, which is not under ~/Videos on the test box, so the defect is on
  more than one capture.
  Evidence: .worktrees/horizon-repro/scratch/base-instants.csv
  STATUS: reproduction confirmed by the owner 2026-08-05; cause identified (the seed trusts the
  accelerometer's magnitude and not its direction, which is issue #45's second half:
  `closer_to_gravity` in crates/meta/src/orientation.rs scores a candidate window by
  `(magnitude_g - 1.0).abs()` alone, and a coordinated turn reads 1 g while pointing away from
  vertical); fix in progress, not yet merged. Tracked separately from seam work; it is not a seam
  defect and no seam bar applies to it.
