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
  STATUS after stage 8: 1 px excess over the same content elsewhere is under the JND; the long-lag
  Weber improves and what is left is the wide-matching ramp.
- 2026-08-01 `VID_20260526_191025_00_004.insv time=630.763 yaw=-72.20 pitch=-17.08 fov=114.41 lock=1`
  The SMOKE view: the same May wide view whole-frame (no content window), which is the render the
  owner pointed at by name ("for example smoke3-2-drawn") when he ruled stage 8's first form not
  aggressive enough. STATUS after stage 8: the +-8 degree mismatch goes 7.55 codes to 2.93, and the
  1 px excess over the same content elsewhere is -0.73 percent, i.e. no line the content does not
  read everywhere. Evidence: .worktrees/stage7/scratch/stage8/evidence-smoke.png
- 2026-08-01 `VID_20260526_191025_00_004.insv time=669.369 yaw=-60.70 pitch=-16.05 fov=30.56 lock=1`
  Sent with no commentary straight after "to the eye, it still effectively looks like a line".
  Same May file, pitched down, fov 30.6 - a fine view, 0.0064 deg per pixel. STATUS after stage 8:
  1 px excess -0.82 percent and 2 px -0.69, so the line there is NOT photometric at any scale an
  edge lives at. Evidence: .worktrees/stage7/scratch/stage8/evidence-zoom30.png
- 2026-08-01 `VID_20260526_191025_00_004.insv time=630.763 yaw=95.00 pitch=0.00 fov=60.00 lock=0`
  The GEOMETRIC CONTROL, chosen by the instrument rather than by the owner: the azimuth his own gear
  crosses the seam at. It reads a 1 px excess of +5.87 percent over the same content elsewhere, and
  the photometry moves it by 0.00 - before 5.94, after 5.94. Any view whose line survives the
  photometry belongs to the local-vs-pose verdict above, not here.

## ANTI-ACCEPTANCE: the artifact the acceptance layer was blind to
- 2026-08-01 `VID_20260526_191025_00_004.insv time=630.763 yaw=-72.20 pitch=-17.08 fov=114.41 lock=1`
  The owner's own screenshots at this area (his ~/Pictures/Screenshots/Screenshot_2026-08-01_20-24-45.png
  raw, _20-25-26.png with his red annotations) show dark STREAKS across the soil running away from the
  seam. Stage 8's per-direction offset over wide support painted each direction's own noise along that
  direction's sweep. **Every acceptance statistic in this campaign straddles the seam and could not see
  it.** Any photometric work must now also pass the FIELD-INTERIOR COHERENCE metric
  (`--bin colour`, the interior block): the applied correction sampled 4-60 deg OFF the seam on dark
  content, binned by azimuth, reported as the rms of what a five-term harmonic cannot describe.
  Rejected build reads ROUGH 1.01%; nulls 0.000%; planted 0.5 and 2.0 code ripples read 2.07% and 8.27%.
  A correction that is smooth round the ring reads zero however large it is.

## Photometric: CHROMATIC (owner 2026-08-02, OPEN - stage 10 gate)
- 2026-08-02 `VID_20260410_185407_00_004.insv time=594.027 yaw=-129.09 pitch=-62.95 fov=41.19 lock=1`
  and `VID_20260410_185407_00_004.insv time=602.368 yaw=-132.22 pitch=-37.74 fov=71.04 lock=1`
  The first CHROMATIC defect on record: a green cast on the sun-facing lens only, not on the other.
  Every photometric reading before this one was brightness, and the steps quoted above are
  channel-uniform codes; a per-channel defect has never been in an acceptance number. Both views are
  steeply pitched down onto dark ground, which is where an additive chromatic cast (veiling glare,
  internal reflection) shows most. The second is the same defect eight seconds later in a wider
  frame, 41.19 deg against 71.04, so the pair tests view-independence and temporal stability of a
  correction rather than one framing of it. Owner: "needs correction and/or blending."
  STATUS: OPEN, no evidence renders yet. Gates stage 10.

## Hard mode: geometry and photometry at once (owner 2026-08-02, OPEN - stages 9 and 10)
- 2026-08-02 `ab_testing/clip 1/VID_20260802_191029_00_002.insv time=31.064 yaw=-144.04 pitch=-31.44 fov=142.89 lock=1`
  A same-day capture from outside every pool this campaign has fitted on, held out on purpose: it is
  evidence about a correction rather than about the corpus the correction was tuned to. At fov 142.89
  the frame should hold both seam crossings and the sun at once, which would put glare, the chromatic
  cast above and azimuth-varying alignment in one picture; that has not been measured on a render
  yet. Held-out hard-mode gate for stage 9 and stage 10 alike. The file is clip 1 of the Studio
  corpus below and lives only there: the path is relative to ~/Videos/Insta and wants quoting,
  because the directory name has a space in it.
  STATUS: OPEN, unmeasured.

## The owner's Studio oracle corpus (~/Videos/Insta/ab_testing, never committed)
Owner-built A/B material: Insta360 Studio's own render of a named position beside kjerag's, so a
defect can be scored against a stitcher that ships. Layout is `clip N/` holding the .insv, with
`snap N/` folders inside it, one per position, each holding `studio_config.png` (Studio's pan, tilt,
roll, field of view, distortion and timecode for that exact position), `studio_screenshot.jpg` and
`kjerag_rough_screenshot.jpg`. Paths only in this repo; no image out of it is committed.
- clip 1 `VID_20260802_191029_00_002.insv`, the hard-mode capture above. snap 1 puts the sun directly
  in one lens. Owner findings: Studio struggles in the glare, but its glared-to-sheltered edge is
  smooth and its seam is invisible outside the glare; kjerag's seam is abrupt and extends well past
  the glare.
- clip 2 `VID_20260526_191025_00_004.insv`, the May file most of the photometric entries above are
  measured on, and now the only copy of it under ~/Videos. snap 1 is the dirt reference. Owner
  finding: Studio renders the seam invisible to the eye.

Purpose (owner ruling 2026-08-02): Studio is the comparative bar for mitigation-shaped defects, which
is stage 10 and near-field alignment. Far-field alignment answers to the absolute zero-defect bar and
not to Studio.

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
- 2026-08-02 horizon lock dips about +-45 deg at 0 and 180 deg azimuth while circling, in the first
  30 s of `ab_testing/clip 1/VID_20260802_191029_00_002.insv`; reproduction in progress, tracked
  separately from seam work.
