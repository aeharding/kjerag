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

## Motion: what the band applies, and how much of it moves (`--bin shear`)
- 2026-08-05 `VID_20260714_193252_00_006.insv time=36.303 yaw=3.78 pitch=5.44 fov=20.00 lock=1`
  The shimmer view, and the acceptance instrument for the seam epic's motion work. Under the
  lock the seam sweeps 350 px down the picture over three seconds, so everything here is
  measured against the seam's own row rather than the picture's. The acceptance command is that
  line plus `frames=90 warm=6.0 seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`,
  and the `seam=` is not optional: fitted from the file instead, the same view reads 0.025 deg
  at -150 px rather than 0.366, because what the band applies is what the calibration left it.
  Live arm against the same frames with the band held off, 90 frames, READ AGAINST MAIN AT #158
  (see the horizon caveat below): `-150` px 0.3663 deg applied at 0.0047 deg step rms over 89
  pairs; `+0` 0.3623 at 0.0619 with a worst single frame of 0.42; `+60` 0.0578 at 0.1350;
  `+150`, which is lens 0's picture and is never bent, 0.0003 at 0.0003, the instrument's floor.
  The band's own state moves 0.0449 deg rms between frames on the bend and 0.0008 on the
  along-seam field. `mode=profile` puts the along-seam plateau at 18.67 px (0.3646 deg) with the
  handover bracketed inside +24 to +60 px, 0.70 degrees of view. `null=1` holds both arms and
  reads exactly zero at every probe on all 90 frames; `mode=plant` yaws the second arm 0.05 and
  0.10 deg and reads the expected -2.534 and -5.068 px back to within 0.029 px, every band,
  ratio 1.9920 to 1.9963.
  CAVEAT, THE HORIZON: everything above except the null, the plant and the band's own state is a
  reading about where the seam lands in THIS view, and under `lock=1` that is decided by the
  orientation track. #158 reseeded it and moved the seam 45 px down this window, which took
  `-150` from 0.3641 deg to 0.3663 and `+60` from 0.0417 to 0.0578 with its readings from 43 to
  30, on an instrument that did not change. The band's own state moved by 0.000002 across the
  same merge, because it is fitted in the body's frame and these bands are read in the view's.
  Re-read this line after any horizon-seed or lock change before treating a move as the band's.
  CAVEAT, the `+60` band is the fragile one and only it: 30 of 90 readings correlate and 23 of
  them are on neighbouring frames, so its step rms rests on a quarter of the run. Sweeping
  `KEEP_PEAK` 0.78 / 0.80 / 0.82 moves it 0.1276 / 0.1350 / 0.1412 deg, a 10 percent span, and
  the worst single reading left out moves it 0.024. The other three bands do not move at all
  over the same sweep (unchanged to four decimals). Read `+60` as a band that steps by about an
  eighth of a degree, not as a number. `mode=profile`'s `+24` to `+48` are the same species, and
  at #158 one of them is refused outright on its pair count.

## Geometry: along-seam axis (stages 5+6, merged)
- 2026-08-01 `VID_20260714_193252_00_006.insv time=2.836 yaw=93.99 pitch=4.12 fov=20.00 lock=1`
  The original horizon-offset report. main 32.8px -> stages 5+6 9.4 cold / 15.4 warm (wide window).
  Owner verdict: "def looks better, albeit not perfect". Residual is distance-dependent (local, not pose).
  READ THE rms COLUMNS BEFORE QUOTING A STEP FROM HERE (2026-08-05, PR #154). `--bin step` prints each
  fitted line's own rms beside the step, and at this view in the WARM state one side of one window
  follows scenery instead of the horizon and reads 13 to 14 px where a line that describes its own
  points reads 0.5 to 2.1. It still prints a step, and that step is not the seam's. Two builds are
  comparable only where both their fits are clean, which at this view is the cold pair.

## Geometry: local vs pose field (VERDICT PENDING - the "optimizing some parts not others" family)
- 2026-08-01 `VID_20260410_185407_00_004.insv time=43.143 yaw=127.56 pitch=-2.43 fov=33.95 lock=1`
  vs `VID_20260410_185407_00_004.insv time=45.112 yaw=-52.37 pitch=3.18 fov=38.28 lock=1`
  Two seconds apart, opposite yaws; one acceptable, one not. Skies photometrically clean (6x contrast) -
  pure geometry. Evidence: .worktrees/stage7/scratch/stage7/sky-apr{1,2}-warm.png
- 2026-08-01 `VID_20260501_183417_00_002.insv time=50.117 yaw=-74.43 pitch=0.06 fov=55.69 lock=1` GOOD
  vs `VID_20260501_183417_00_002.insv time=50.117 yaw=106.98 pitch=0.75 fov=62.79 lock=1` BAD
  Same instant, the two seam crossings: one matched, one mismatched where the lens intercepts the
  horizon on the other side. Owner: "not blending - this is a seam mismatch issue." The cleanest
  demonstration yet that a global field fits one crossing at the other's expense.

## Calibration starved (issue #130, X2) - fitted on fix/130-x2-fit, owner test pending
- 2026-08-01 `VID_20251018_191318_00_002.insv time=77.978 yaw=-13.77 pitch=-3.41 fov=70.80 lock=1`
  October X2, mid zoom. Ran the factory calibration: watch-to-calibrate matched only 2-3 of 72
  azimuths on every X2 capture (the search was centred on an extrinsic 2.8 deg out, and widening it
  was broken by rectangle sampling). Per-candidate refusal plus an acquired search centre
  (seam-two-axis 11) take the three captures to 50/42/65 azimuths and a pool entry. Round the ring
  at this moment: 2.570 deg along and 2.830 across on factory, 0.257 and 0.267 fitted. Local step
  where the horizon crosses the seam, `yaw=-21.5 pitch=3.6 fov=35`: -3.11 deg factory, -0.61 deg
  fitted. Pictures: `scratch/x2fit/october-{factory,file}.png`.

## Photometric: brightness/color at the seam (stage 7 merged-in-draft, stage 8 in progress)
- 2026-07-31 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=488.855 yaw=67.24 pitch=2.56 fov=218.99 lock=1`
  Stage 3's soil reference (sun lighting one lens). Step 2.265 -> 1.424 codes under stage 3.
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=-72.20 pitch=-17.08 fov=114.41 lock=1`
  The May wide "do a lot better with blending" view: ADDITIVE 6.5-code step on 17-24 code soil
  (28-38% perceptual), channel-uniform. Stage 7 moves 0.7 of 6.5; stage 8's primary target.
  Evidence: .worktrees/stage7/scratch/stage7/evidence-may-stretched.png
- 2026-08-01 `VID_20260501_183417_00_003.insv time=99.032 yaw=-73.42 pitch=-37.28 fov=101.47 lock=1`
  "Another color/brightness/whatever" - wide, pitched down, ground-dominated. In stage 8's set.
  STATUS after stage 8: 1 px excess over the same content elsewhere is under the JND; the long-lag
  Weber improves and what is left is the wide-matching ramp.
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=-72.20 pitch=-17.08 fov=114.41 lock=1`
  The SMOKE view: the same May wide view whole-frame (no content window), which is the render the
  owner pointed at by name ("for example smoke3-2-drawn") when he ruled stage 8's first form not
  aggressive enough. STATUS after stage 8: the +-8 degree mismatch goes 7.55 codes to 2.93, and the
  1 px excess over the same content elsewhere is -0.73 percent, i.e. no line the content does not
  read everywhere. Evidence: .worktrees/stage7/scratch/stage8/evidence-smoke.png
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=669.369 yaw=-60.70 pitch=-16.05 fov=30.56 lock=1`
  Sent with no commentary straight after "to the eye, it still effectively looks like a line".
  Same May file, pitched down, fov 30.6 - a fine view, 0.0064 deg per pixel. STATUS after stage 8:
  1 px excess -0.82 percent and 2 px -0.69, so the line there is NOT photometric at any scale an
  edge lives at. Evidence: .worktrees/stage7/scratch/stage8/evidence-zoom30.png
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=95.00 pitch=0.00 fov=60.00 lock=0`
  The GEOMETRIC CONTROL, chosen by the instrument rather than by the owner: the azimuth his own gear
  crosses the seam at. It reads a 1 px excess of +5.87 percent over the same content elsewhere, and
  the photometry moves it by 0.00 - before 5.94, after 5.94. Any view whose line survives the
  photometry belongs to the local-vs-pose verdict above, not here.

## ANTI-ACCEPTANCE: the artifact the acceptance layer was blind to
- 2026-08-01 `~/Videos/Insta/ab_testing/"clip 2"/VID_20260526_191025_00_004.insv time=630.763 yaw=-72.20 pitch=-17.08 fov=114.41 lock=1`
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
- 2026-08-02 `VID_20260410_185407_00_004.insv time=594.027 yaw=-129.09 pitch=-62.95 fov=41.19 lock=1`
  and `VID_20260410_185407_00_004.insv time=602.368 yaw=-132.22 pitch=-37.74 fov=71.04 lock=1`
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
- 2026-08-02 `~/Videos/Insta/ab_testing/"clip 1"/VID_20260802_191029_00_002.insv time=31.064 yaw=-144.04 pitch=-31.44 fov=142.89 lock=1`
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
