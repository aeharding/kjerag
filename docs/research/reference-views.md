# Owner reference views

The acceptance registry for seam work. Lines are runnable as CLI args and Ctrl+V targets.
Footage lives on the owner's test box under ~/Videos (owner ruling 2026-08-01: footage filenames
are fine in the repo). Agents: read this at the start of any seam task;
add new owner references here with date, category, and status.

## Geometry: along-seam axis (stages 5+6, merged)
- 2026-08-01 `VID_20260714_193252_00_006.insv time=2.836 yaw=93.99 pitch=4.12 fov=20.00 lock=1`
  The original horizon-offset report. main 32.8px -> stages 5+6 9.4 cold / 15.4 warm (wide window).
  Owner verdict: "def looks better, albeit not perfect". Residual is distance-dependent (local, not pose).

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
- 2026-07-31 `VID_20260526_191025_00_004.insv time=488.855 yaw=67.24 pitch=2.56 fov=218.99 lock=1`
  Stage 3's soil reference (sun lighting one lens). Step 2.265 -> 1.424 codes under stage 3.
- 2026-08-01 `VID_20260526_191025_00_004.insv time=630.763 yaw=-72.20 pitch=-17.08 fov=114.41 lock=1`
  The May wide "do a lot better with blending" view: ADDITIVE 6.5-code step on 17-24 code soil
  (28-38% perceptual), channel-uniform. Stage 7 moves 0.7 of 6.5; stage 8's primary target.
  Evidence: .worktrees/stage7/scratch/stage7/evidence-may-stretched.png
- 2026-08-01 `VID_20260501_183417_00_003.insv time=99.032 yaw=-73.42 pitch=-37.28 fov=101.47 lock=1`
  "Another color/brightness/whatever" - wide, pitched down, ground-dominated. In stage 8's set.

## Standing bars
- Pixel-perfect horizon at zoom is an acceptance criterion (owner, 2026-07-31).
- "Perceptually minimizing the seam as much as possible" is THE objective; sky is the hardest canvas
  (owner, 2026-08-01). Stage 8 bar: max local Weber contrast at/below JND (~1%) on flat content.
- 60fps dual-stream full resolution realtime on the owner's device; research may exceed budget,
  shipped form needs the story (owner, 2026-08-01).
