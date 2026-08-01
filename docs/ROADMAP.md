# Roadmap (living doc)

Update this file in any PR that changes project status. Work queue is
GitHub issues; this doc is the map, issues are the tasks.

**Status 2026-07-31:** feasibility study complete (docs/research/), repo
bootstrapped, M0 done, M1 done, and the horizon holds still.
`cargo run --release -p kyerag-spike -- <file.insv>` decodes one 3840x3840
lens on VA-API, imports the dmabuf planes into wgpu with no copy, and
renders to PNG at 103 fps (3.4x realtime). `cargo run --release --
<file.insv>` plays the file in a libcosmic window, every frame imported
zero-copy onto the device iced created and reprojected inside iced's own
render pass: the shell, the shader widget and the wgpu-28 import all
confirmed on screen. M1 is under way: `crates/meta/` reads the trailer's calibration
(issue #2), the source tree is a workspace with one crate per layer
(issue #19), the picture is reprojected through the lenses' own Mei/UCM
models with drag to look around and scroll to zoom (issues #3, #26), and the
playback core plays it (issue #4): one demuxer, both lenses decoded in
lockstep and delivered as pairs, a presentation clock that paces 29.97 fps
content by due time, space to pause, and frames pullable by index or
timestamp. Measured over 60 s of real footage: 29.94 fps presented, zero
dropped, zero starved. **The sphere is closed** (issue #27): the shader
projects every ray into both lenses, so turning around shows the back
hemisphere, upright and unmirrored, and **the seam between them is
blended** (issue #7), which is the first of M2. The
window is a COSMIC app around it (issue #16, built to docs/UI.md): the menu
bar in the header, a welcome view, the portal file chooser, drag and drop,
recent files, the whole key map, fullscreen, Settings and About drawers, and
a control overlay that takes itself and the pointer away after 2 s of
stillness while playing and never while paused. The scrubber scrubs (issue
#5): dragging it seeks to keyframes, 21 ms each wherever in the 37.9 GB file
they land, and letting go seeks to the exact frame. **The view can be
photographed** (issue #15): `s`, the camera button and `File > Save frame`
write a 3840 px wide JPEG of the reframed view, at the window's aspect and
not its size, into the desktop's screenshots folder; `Ctrl+C` puts the same
picture on the clipboard as `image/png`. The capture is the window's own
pipeline and bind group drawn a second time into a texture of the surface's
format, so the numbers in the file are the numbers on the screen, and
everything after the submit runs on a worker thread: 13 captures over 20 s
of playback, zero dropped and zero starved in every report. **A capture says
so**, in a toast built the way cosmic-files builds its own: `Frame saved to
"Screenshots"`, `Frame copied to the clipboard`, or the reason it did not
happen (docs/UI.md, "The capture toast"). 11 captures over 30 s of playback
with the toasts in: zero dropped and zero starved in all six reports.

**M1 is done.** The seam blend (issue #7) is the first M2 quality item and
it has landed: where the two lenses overlap the pass mixes them by
longitude preference times coverage depth, with no feather width anywhere
in it, and the hard line and the tone edge where the hemispheres met are
gone. Near-field structure crossing the seam, which on this footage is the
wing and the lines, ghosts softly instead of stepping, which is parallax
and is the expected trade. Exposure is deliberately **not** corrected, and
that is the finding rather than an omission: the trailer's two shutter
records are parsed and kept apart (issue #7's other half, and the camera's
own frame clock for #8), but measured over two 30-minute captures the
brightness step at the seam is 0.9 to 3.5 percent while the shutter ratio
swings 0.54 to 1.81, and applying the symmetric split those records imply
makes the step four to twenty times worse.

**The horizon is locked** (issue #8), which is the second M2 item and the
one the feasibility study called the riskiest correctness surface. The
trailer's IMU is parsed and integrated into a `world_from_body` quaternion
every 5 ms of the file, and the reprojection pass composes its inverse
between the lens mounting and the camera, so the world stays put while the
camera swings. Roll and pitch are locked completely; yaw is high passed with
a 3 s constant, so a swing is cancelled and a deliberate turn still reads as
a turn. Drag to look around needed no change at all: the anchor it stores is
in whatever frame the camera rotation lands in, and with the lock on that
frame is the world. Measured on rendered frames: the horizon moves 0.23
degrees peak to peak over 120 frames of calm flight and 2.86 through a
61 deg/s roll, where with the lock off it leaves the picture entirely.
`View > Lock horizon` and `h` flip it live, and it is on by default.

Two conventions were settled on the way, both against pixels rather than
against other people's tables. The IMU's axis convention is `xZY` for the
X-series in Kyerag's own frame, picked out of all 24 rotations by comparing
the accelerometer's idea of up against the horizon in unlocked frames; it
wins every stretch of two captures by 15 to 36 degrees over the runner-up.
And the quarter-turn roll datum from issue #3 belongs to the **delivered
picture** rather than to the sensor, which the IMU could tell apart because
it is bolted to the sensor: that closes the last "what 4.8 does not settle".
**Rolling shutter is fused into the same map, measured, and on** (issue #9),
which is the third M2 item. For every output ray the landing row is solved
for and the orientation used is the one at that row's own readout instant,
one round of the solve, no extra pass and nothing resampled. The one thing
the file does not record is **which way the sensor reads**, and on an X4 Air
it reads **down the delivered frame**: 1.00 +-0.12 of a whole frame in the
trailer's own 15.883 ms, measured over five stretches of a 30-minute capture
by fitting one lens against itself a few frames apart, with the horizon
lock's rigid rotation and the camera's own translation fitted out alongside.
That direction is the one the seam cannot see, because two lenses reading
down their own pictures sweep the same world direction and it cancels between
them; that is both why #42 shipped it switched off and why switching it on
cannot disturb the seam #7 blended. It costs about half a millisecond per
redraw at 2560x1440, 0 dropped and 0 starved
(docs/research/insv-format.md 6.7).

**Hemisphere-aware decode gating is half done and half cut** (issue #10),
which is the fourth M2 item and the third time this project has built
something and then measured it out. The half that shipped is in the shader:
each lens's picture is one cap around its own axis, that cap comes out of the
calibration by solving the model's own coverage boundary, and a ray further
off the axis than the cap weighs exactly nothing, so the pass does not run
the Mei map for it. One dot product per lens decides. 1.74 to 1.54 ms per
redraw at a view inside one hemisphere, 1.81 to 1.66 across the seam, and
eight rendered views are byte for byte what they were.

The half that was cut is the decoder, and the reason is arithmetic. Gating
the invisible stream is worth 2.84 points of one core and 1.53 W of SoC power
against a 6.10 W idle, which is the halving the issue predicted. But the gate
can only be on while **no** ray of the view can reach the far lens, and at the
app's default 90-degree field of view that is 16.5% of the sphere; on real
flying footage with the horizon locked, which is the default, a parked view
is not a parked geometry, and the measured duty cycle is 21.6 to 24.3% with
no hysteresis at all and 8.9 to 9.4% with the 15 degrees of margin a release
would need. Releasing a cold gate costs 195 to 340 ms, six to eleven frames
of stale far hemisphere, because HEVC has no way into the middle of a GOP and
this camera's is 29 frames. Expected saving: **0.14 W**, for a state machine
and a packet ring through the frame path. `kyerag-spike --bin gating` is the
measurement and it stays runnable.

**The lock-defect work of PR #51 was reverted the same day** (issues #44
and #45). Its drag-relative follow passed a filter-level test (a pinned
view moved under 0.05 degrees across five minutes of wandering heading)
but the owner, on that exact build, still saw the view jump back seconds
into playback: whatever the app path does to the pin, the unit test did
not exercise it. Owner's rule applied: no code on main that is not doing
its job.

**The dip was a bad start, and the seed is fixed** (issue #45,
docs/research/insv-format.md 8.7). `Filter::solve` seeded the estimate from
the first tenth of a second of accelerometer whatever it read; on the April
capture that tenth of a second weighs 1.281 g, which the running filter
refuses outright, so the horizon started **48.9 degrees** off level and
walked back over a minute and a half. The seed now searches forward for the
first window the filter believes completely and carries it back to the start
of the track with the gyroscope. Measured through the projection pass with
`kyerag-spike --bin dip`: at 6 seconds 48.9 degrees becomes **1.9** on the
April capture and 14.7 becomes 8.1 on the June one, at 30 seconds 29.2
becomes 3.9 and 6.0 becomes 2.5, and at 300 seconds both files read what
they read before to three decimal places, which is the control. What is
left is the accelerometer's own disagreement with gravity inside the seed
window, 2.8 degrees on one capture and 9.8 on the other, and that is the
residual issue #57 is about.

**The instrument that missed it is repaired.** `dip` gated out any line more
than 20 degrees off level, so it never measured a defect that is 40 to 50,
and the 1.9 to 8.5 degree apparent-gravity attribution PR #51 reported was
that selection. Its injection control passed because 1 to 3 degrees was
injected where the baseline was already small: **a control has to span the
regime being measured**. The gate is off by default, every view it drops is
counted and printed, and the acceptance run injects 45 degrees and reads
back 45.101 - where the gate that shipped drops 48 of 60 views and reports
no fit at all. The apparent-gravity size and the GPS prescription (#57) are
withdrawn in place in 8.7 with pointers to the new numbers; #57 stays on
hold until the residual is re-measured.

**High-quality sampling at high zoom is half shipped and half measured out**
(issue #11), which is the last M2 item and the fourth time this project has
built something and then cut part of it. What ships is the **luma** plane:
where an output pixel has landed inside a source texel, the pass reads a
Catmull-Rom kernel instead of the hardware's bilinear tent, engaged smoothly
from 1:1 to 2:1 magnification and exactly off at 1:1 and wider. Sixteen
texels as nine bilinear fetches, which agree with sixteen point fetches to
0.14 codes RMS. How magnified a fragment is comes off the **map's own
Jacobian**, the hardware's quad derivative of the landing, so the fisheye's
uneven angular density and the output's own fall-off towards its corners are
both in it and neither had to be assumed. On real footage at the window the
player's numbers are taken at, zoomed to 50 degrees on ground and buildings:
detail (mean absolute Laplacian) 4.12 codes bilinear against 4.61 sharp,
**+11.8%**, 1.8 codes mean over 85% of pixels, and side by side at four times
life size the stone courses of a barn separate instead of smearing, with no
ringing on the roof line. It is worth **most in the middle of the zoom
range** and least at the end of it: at 5x, which is 25 degrees, the source has
nothing left to resolve and the two kernels draw the same ramp (+1.8%).

What is cut is the **chroma** plane. NV12's two planes are two grids, and
measuring them separately is what found it: chroma is half the size, so it is
magnified twice as hard and it is under 1:1 at **every** field of view this
player offers. Upgrading it is therefore not a cost paid at high zoom but a
cost paid always, and it is the larger half of the bill (0.69 to 0.90 ms
with luma upgraded, to 1.23 with both). What it buys, on 8-bit 4:2:0 chroma
that HEVC has already smoothed, is 0.41 codes on 40% of pixels and **no**
measurable change in detail at all: 4.606 either way. `Sampling::Sharp` keeps
it one line from shipping for footage that would change the answer.

**A scrub no longer waits for pictures nobody asked for** (issue #46). The
decode thread used to look at its command queue only between reads, so a
drag position arriving while it refilled the lookahead behind the last
landing waited out three pair decodes first: 33 of the 39 ms between a
keyframe seek at the reader (21 ms) and the same seek through the player
(59 ms). It now asks between packet reads and gives the read up, and a scrub
costs 26 ms: about 38 picture updates a second where there were 17. The read
a seek itself asked for is never given up, because drag positions arrive
faster than landings come out of them and a rule that always took the newest
would show no picture at all. Playback is untouched by construction and by
measurement: 29.97 fps presented, 0 dropped, 0 starved, the same decode rate
as before, and the sound goes with it, because a preempted read stops
without reading another packet and the seek behind it flushes the ring.

**The zoom goes all the way out to the blue ball** (issue #47, owner ask). The
scroll used to stop at 110 degrees, because that is where a flat window stops
being one. It now keeps going: past that threshold the output projection bends
out of perspective through **stereographic**, which is the **tiny planet**
Insta360 names and the owner calls the blue ball, and on until the whole
sphere is a ball with room around it. One
family does the whole range, `r = tan(shrink * theta) / shrink`, with `shrink`
running 1 to about 0.18, so nothing is switched over and there is nowhere to
pop. The far end is where the ball fills 0.8 of the window's **shorter** side,
which is 605 degrees of field of view on a 16:9 window and 406 on a square
one, and past 360 the frame is simply wider than the sphere: that extra is the
room the ball sits in, painted the same grey the pass has always painted where
no lens has the ray. Ctrl+0 comes back in one press.

Measured on real footage at 2560x1440 (`kyerag-spike --bin ball`): one scroll
from 20 degrees to the ball, a notch at a time, rendered, and the largest
single step is 64.3 codes at fov 402, with the largest growth of a step over
the step before it 1.32x at fov 173 - against 1.15x inside the flat range this
change did not touch, and nothing standing out at the threshold or at
stereographic. Cost, interleaved across the range and three runs agreeing
within 0.01 ms a cell, 0.71 ms/redraw at the default view against 0.81 at the
ball, on a 33 ms frame; the flat range costs what it cost, `--bin zoom` off
this branch against the same binary built off main, alternated on one box,
within 0.04 ms a cell in both directions. Playback at the ball, with three
3840 px captures taken during it: 29.97 fps presented, **0 dropped, 0
starved**, and a still of the ball is byte for byte the ball. The drag took no new mathematics: it was
already written against the projection's own rays rather than against a
tangent, so it inverts whichever map the view is in, and grabbing the ball and
turning it works because of that rather than despite it.

What is **not** fixed is aliasing. Out wide the map minifies rather than
magnifies (7.6 delivered texels to the output pixel at the middle of the
ball), so issue #11's kernel correctly switches off and the pass is plain
bilinear on a single mip level: against the same view supersampled 4x4 the
ball is 4.1 codes out over the pixels that have picture and 107 at worst,
against 1.1 and 11 at the default view. High-contrast edges will shimmer in a
moving ball. A prefilter is the fix and it is deliberately not built yet: the
imported dmabuf textures have one level and no room to generate another in
place, so it means a downsample pass per frame per lens, for a view the player
is in for a few seconds at a time. Numbers first, then the owner decides.
**A fast drag no longer freezes the picture** (issue #55). What #46 measured
and could not fix inside itself: `Player::pump` showed a frame only while its
own seek was still the newest, so a hand faster than a landing starved the
display instead of slowing it, and at 60 slider positions a second not one
picture reached the screen for the length of the drag. The pump now takes a
frame from any seek newer than the position on screen, so the pilot sees the
landings their finger has passed over rather than nothing, and picture
updates rise with the hand instead of falling off a cliff: 10.0, 15.0, 19.5,
29.0, 38.5, 45.5 and 43.0 a second at 10 to 90 positions/s, against 10.0,
15.0, 12.5, 10.5, 5.0, 0.0 and 0.0. The two questions the old rule answered
with one flag are now separate: which frames may take the screen, and which
seek is still owed one. The second is what keeps a paused window redrawing,
and it ends on the newest seek's own frame, so the release still lands the
exact frame under the handle (49 of 49 drags, both arms). At 60 positions/s
the picture now changes every 22.0 ms, which is about what one keyframe
decode costs on this camera (21.1 ms at the reader): the drag runs at the
decoder's rate, which is also the answer to #46's open question about a
100 ms drag cycle. There was no 100 ms cycle. Three landings in four were
being thrown away.

M2 is done. #44 and #45 closed against it (the seed fix, owner-verified),
and #48 has now reopened the seam.

**The seam is out by degrees, and it is calibration** (issue #48, phase 1,
measurement only). The owner supplied the one thing every earlier reading of
this was starved of: a capture from a camera that is **not moving**, and
Insta360's own export of the same capture as the parity benchmark. On a still
camera there is no parallax worth the name in the far field, no rolling
shutter and no motion blur, so what is left at the seam is the calibration.
`kyerag-spike --bin seam` measures it round the whole seam circle and splits
it into the axis parallax cannot reach and the axis it owns.

The along-seam residual #7 and #42 left open, -0.36 to -1.20 degrees, is real
and reads -0.30 to -1.25 here. It is not the problem. **Across** the seam the
two lenses disagree by **-2.4 to +2.7 degrees**, which is 43 px of the
delivered frame, and that is what the owner has been looking at: it draws a
tree trunk twice in a blended view and breaks the horizon in a hard cut, on
this capture and on his flight footage. Insta360's own export of the same
frame draws the trunk once.

The structure attributes it. Measured round the circle, a constant and one
cycle account for everything, leaving 0.012 degrees along and 0.055 across;
and the shipped map's own knob table says only a relative **lens tilt** can
put 2.7 degrees of one cycle across the seam while leaving 0.46 along it. The
fitted correction to lens 1 is a rotation of roll +0.80, yaw -2.29, pitch
-0.82 degrees plus a 15 px principal-point shift, and applying it takes the
along-seam residual from 0.766 to 0.077 degrees and the across-seam from 2.333
to 0.108. Applied and re-measured, every patch reads inside 0.02 degrees along;
rendered, **a hard cut with no blend at all is continuous** through content
that was visibly broken before, on both static captures and on flight footage
from five weeks earlier.

Controls, because #45's lesson is that an instrument which cannot catch the
failure is not an instrument: injected errors of the size being reported read
back at 0.99 to 1.06 (roll along, yaw across at r = 1.000, a 20 px principal
point on both axes); a second capture of a completely different scene, taken
minutes later with the camera moved, is fixed by the first capture's
correction; and the deck under the camera fails to pair at all exactly where
parallax says it must, because 5 to 30 cm of subject distance is 6 to 38
degrees of disparity in a 14 degree band.

Against the benchmark, each stitch scored on its own picture (gradient energy
in the seam band over the same statistic either side of it, so tone curves
divide out): Insta360 keeps **0.83 to 0.88** of its sharpness across the seam,
we keep **0.573**, and the correction takes us to **0.689**. Their export is a
square 1440x1440 reframe about 95 degrees across, mildly compressed rather
than rectilinear, not an equirect crop.

The blend width has a number now instead of a guess. Rendering the same
seam-crossing frame under crossovers of stated width and scoring each against
the front lens alone: a **2 degree** band takes 80 percent of what a hard cut
would give, against a 14 degree band today. But the two halves are not
independent, and this is the finding that orders the work: shear, the
disparity divided by the band, is 1.07 at 2 degrees with today's calibration
and 0.52 with the correction applied. **Narrowing the band before correcting
the calibration would trade a soft wide ghost for a hard visible tear.**

Nothing in the shader changed: phase 1 is measurement, and the fitted
parameters are for the owner to validate before phase 2 applies any of them.
Method and every number: docs/research/insv-format.md 6.8.

**M3 has started, and the player has sound** (issue #13). The file's AAC track
is decoded off the same demuxer as the two lens streams, resampled by
`swresample` into the device's own format, and written to a ring that a cpal
output stream drains. **The picture stays the clock.** Every device callback
asks the presentation clock where the picture will be when the samples it is
about to write are heard, and moves the sound to meet it: a splice when the
two are more than 30 ms apart, which only a start, a seek landing or a
recovered stall ever is, and a resampling ratio of a few parts per million the
rest of the time, which is what holds two crystals together over a half-hour
flight. A seek throws the ring away on the shell's thread rather than waiting
for the decode thread to reach the seek, so no stale sound survives a scrub,
and every start and stop is a 5 ms ramp rather than a step, so a pause does
not click. In the control row: a speaker button after fullscreen, opening
cosmic-player's own volume dropdown, with both settings remembered in
cosmic-config. The wheel stayed on zoom (docs/UI.md, "Conflict 2"). Nothing is
processed: a paramotor track is mostly wind, and it is played as recorded.

`kyerag-spike --bin sync` is the measurement, and it drives the real player
with no GPU in it: play, pause, resume, two scrubs and a frame step on a
printed schedule, with the app's own five-second report. Over a 325 s run of
real footage, past those, the sound sat **0.0 ms** from the picture in 40 of
44 windows and never further than **0.8 ms**, with zero underruns and zero
dropped chunks; the target was 40. The same binary with the drift correction
switched off and nothing else changed sits at **+28.1 ms** and stays there,
which is what the correction is for and is the control that says the number
above means something. Recording the output device's monitor and measuring the
sample-to-sample step at each join puts every one of the six below the 97th
percentile of ordinary playback, against 12 of 15 synthetic hard cuts spliced
into the same recording that land at the 99th or above: the fades hold.

**Looking around got three fixes at once** (issues #77, #63, #78, all owner
reported). **Fullscreen no longer resets the view.** The camera was living in
the shader widget's iced `State`, and iced rebuilds widget state whenever the
widget tree changes shape: libcosmic pushes the header bar into the same
column as the content, so hiding the bar moves the content up a place and
everything under it is built fresh. Fullscreen hides the bar, which is the
whole of the connection - and so does the two-second idle timeout, so the view
was being reset under a pilot who was only watching, too. The `Viewpoint`
lives on the `Scene` now, which is the shell's own and outlives its view. The
harness gained the checks that measured it: entering and leaving fullscreen by
both keys, and the controls hiding on their own.

**The pan goes over the top** (issue #63). The pitch had a wall at 89 degrees;
it is gone, and past a quarter turn the view is looking back over itself with
the world upside down, on round to a whole turn and round again. It is a pitch
and not a roll, so the horizon stays a level line either way up, and a
vertical drag never swings the yaw. Nothing was added to get there:
`rot_y(yaw) * rot_x(pitch)` was always the rotation and a pitch past 90 is a
perfectly good one. What the anchor solve needed is to count to the nearest
tilt **the short way round**. Checked through the pass on real footage and not
only in arithmetic: rendered at yaw 180 pitch 180, the pass draws exactly the
yaw 0 pitch 0 picture turned upside down, every pixel of it within one code of
255.

**And the wide end of the zoom drags calmly** (issue #78). Pinning the grabbed
direction to the cursor is what makes the flat range feel like a hand on the
picture and what makes the ball twitchy: the pinned rate at the middle of the
ball view is 900 degrees of world per width of window, against 164 at the
widest flat view, so a drag across the window out there turned the world two
and a half times. Past 110 degrees the pin comes off and the drag turns the
view at a fixed 164 degrees per window width, which is the rate the pinned
drag itself is going at in the last view before the handover, so the two meet
at the threshold with nothing to feel. Under 110 nothing changed at all.

**And the zoom key no longer lets go of the drag** (issue #83, pre-existing
and surfaced by the three above). `Ctrl+=` and `Ctrl+-` took a held drag's
hold again at the middle of the frame while the hand was somewhere else, so
the next move of the pointer hauled the picture over to whatever the middle
had been pointing at: 33 degrees of yaw for a cursor that did not move, in
the widget-level check that reproduced it. A held drag already knows where
the cursor is, because that is what the wide regime measures its travel from,
and the key zooms there now, which is what the wheel has always done. With
nothing held it still zooms about the middle, which is where a keyboard with
no hand on the picture is pointing.

**And nor does the wheel out in the room around the ball** (issue #92, owner
reported, the last of the same family). Zoomed out to the ball, a drag that
starts on the picture and wanders out into the grey keeps turning the view,
which is what issue #78 bought: the wide drag reads the hand's travel and not
what is under the cursor. Scrolling out there killed it stone dead. Every
zoom re-takes the drag's hold at the cursor, the room has no direction under
it to take hold of, and the whole drag was being dropped rather than the
hold: the button was still down, the pointer moved 200 px, and the camera
came back bit for bit identical in the widget-level reproduction. The hold is
kept now where there is nothing to replace it with. Nothing reads it stale:
the room only exists past 220 degrees of view and the pinned drag stops at
110, so the wide drag, which does not use it, is the only regime the room can
be seen from.

## Milestones

- **M0 Pipeline proof** — decode one lens via VA-API, import into wgpu
  zero-copy, render headless to PNG with timings. Done (`crates/spike/`,
  issue #6). Shell bring-up followed in issue #1: libcosmic window, shader
  widget, and the wgpu-28 port of the import.
- **M1 Reframing player** — dual decode, calibrated Mei reprojection,
  drag to reframe, scroll to zoom, play/pause/seek, screenshots. The MVP.
  Reprojection and the mouse are done (issue #3), and so is playback:
  dual-stream decode, the presentation clock and play/pause (issue #4).
  Full 360-degree look-around lands here too (issue #27): both lenses
  sampled, and the seam they left is blended by #7 in M2. The app shell
  around all of it is issue #16, seeking is issue #5, and screenshots are
  issue #15, whose toast has since landed in cosmic-files' idiom
  (docs/UI.md, "The capture toast"). What is left of the MVP's UI is the two
  Settings rows for the capture folder and resolution.
- **M2 Quality** — seam blend (issue #7, done: weight field in, exposure
  correction measured and rejected), gyro horizon lock (issue #8, done:
  complementary filter, `View > Lock horizon`, and a harness that measures
  the horizon in rendered frames because a Studio export was not available
  overnight), rolling-shutter correction (issue #9, done: fused into the one
  backward map, and on, the readout direction measured off the pictures
  because the file does not record it), hemisphere-aware
  gating (issue #10, done: the pass skips the lens a ray cannot reach, and
  the decode gate under the same test is measured and cut), scrub
  responsiveness (issue #46, done: a newer drag position takes the decode
  thread off the lookahead refill, 59 ms to 26 ms per scrub; and issue #55,
  done: a drag faster than a landing shows the landings it has passed over
  instead of freezing, 0 to 46 picture updates a second), high-quality
  zoom sampling (issue #11, done: a Catmull-Rom kernel on the luma plane
  wherever the map's own Jacobian says an output pixel has landed inside a
  texel, and the chroma half of it measured and cut), and the zoom out to the
  tiny planet and the whole ball (issue #47: one projection family from
  perspective through stereographic to a finite disc, capped where the
  ball clears the window's shorter side; the tiny-planet framing sits
  mid-scroll on the way there, and the owner chose to keep the extended
  range after trying a hard stop at the planet).
  Three of the quality issues under it are the interaction ones the owner
  found while flying the finished zoom: the view surviving fullscreen
  (issue #77), the pan carrying on through the poles (issue #63), and the
  wide end of the zoom dragging calmly (issue #78).
  **M2 is complete**, except that issue #48 reopened the seam: the two lenses
  are misaligned by up to 2.7 degrees across it, phase 1 has measured that,
  attributed it to a relative lens tilt and fitted the correction, and phase 2
  will apply it. Issue #79 opened a second camera: the ONE X2 writes one lens
  per file, and the player now pairs the two at open and holds an X2's horizon
  with that camera's own IMU convention.
- **M3 Export & sound** — clip export (reframed VCN encode, and lossless
  time-range remux), audio playback (issue #13, done: AAC off the same
  demuxer, cpal out, slaved to the video clock, volume and mute in the control
  row).

## Scope doctrine (owner, 2026-07-31)

Kjerag is a VIEWER: view, reframe, screenshot, and at most a simple clip
export (mark in and out, export the current view or a lossless cut). It
must be an awesome viewer before any of that export work starts, so
quality owns the roadmap until the owner says the bar is met. Keyframed
reframing and timeline editing are OUT OF SCOPE, not deferred: that is an
editor, a different product (Kdenlive's bigsh0t filters already cover
keyframed 360 export on Linux). The one editor-adjacent idea parked with
no commitment: export that follows the view the pilot actually flies
live, no keyframe UI ever.

## Decisions log

- 2026-07-31 **The app has an icon** (issue #67, seven workshop rounds
  recorded in docs/icon.md). A round teal world with a green coast and a warm
  rim, and a small figure entering it from the upper left, drawn by
  `scripts/icon-diver.py` from a joint skeleton rather than traced. The
  figure's size and how far its feet clear the rim are set together, because
  the rim crossing is what decides both: round 7 grew it 18 percent inward,
  holding the feet at the same 27.1 units past the rim.
  `resources/icons/hicolor/` is the theme tree: a scalable SVG, PNGs from 256
  down to 16, and a drawing of its own for 32, 24 and 16, because both COSMIC
  and the Pop theme redraw those sizes instead of exporting. The files are
  named for the application ID `dev.harding.Kjerag`, the one issue #66
  settled and issue #75 will put in the code; until that rename lands the
  binary still asks the theme for `app.kyerag.Kyerag` and will not find it.
- 2026-07-31 Seam bar raised (owner): "I want the best seam support out
  there." The prod gate is not good-enough but best-shipping, Insta360's
  stitcher included. Two tracks: the per-camera geometric foundation
  (static-capture 5-knob fit, #87 rework) ships first; depth-aware seam
  alignment (the overlap band is a 33 mm stereo pair, disparity gives
  metric depth - what dynamic stitching fundamentally is) is #80 phase A,
  research-first with owner-validated design before implementation.

- 2026-07-31 **The camera is the shell's state, not the widget tree's**
  (issue #77). iced keeps a widget's state in the widget tree and rebuilds
  it whenever the tree changes shape under it, which the header bar coming
  and going does on every fullscreen toggle and every idle timeout. Anything
  a pilot expects to survive the window changing shape therefore cannot live
  in an iced `State`, however natural a home it looks. Keeping it there and
  pinning the tree instead was the alternative and it is a trap: it makes
  every future layout change a chance to lose the view, silently, and the
  shell has to be free to change its layout.
- 2026-07-31 **The pitch runs all the way round, and the wall is gone**
  (issue #63, owner ask). The alternative reading of "keep looking up past
  the zenith" is to fold the crossing into pitch and yaw together - pitch
  turns back down and the yaw swings half a turn - which keeps the pitch
  inside a quarter turn and keeps the picture upright. It was rejected
  because it is not what was asked for: the owner asked to keep going
  **until he sees upside down**, and a fold never shows an upside down
  world. It also puts a discontinuity in the yaw exactly where the hand is
  moving. Letting the pitch continue is both the thing asked for and the one
  with no jump in it.
- 2026-07-31 **Past the flat range the drag is a rate, not a pin**
  (issue #78, owner ask). One threshold, `FOV_FLAT`, shared with the
  projection's own bend, and one constant: the rate the pinned drag is
  already turning at when it gets there. The alternative was to keep the pin
  and damp it - a speed limit on the solve - which reads well and breaks
  issue #63, because the same limit would have to bite hardest exactly where
  a pole crossing legitimately turns the view fastest. Two drags with one
  clean threshold beats one drag with a rule that has to know about poles.
- 2026-07-31 **A capture reports itself at the top of the window, in a toast
  drawn out of libcosmic's own pieces** (issue #15, docs/UI.md's open
  question 2). cosmic-files is the only first-party app that uses toasts at
  all, so its lines are the whole precedent and the wording, the 5 s, the
  five-line stack, the tooltip container and its spacings, and the refusal
  to carry an action unless it undoes something destructive are all its own
  (`src/app.rs:1344-1358`, `toaster/mod.rs:33-63`, `79-85`, `162-181`). The
  **placement is the owner's**, and it is a deviation from cosmic-files with
  a reason: it puts its toasts at the bottom because the bottom of a file
  manager is empty, and the bottom of this window is the transport. Shipped
  over the scrubber first, and the owner found it.
  `widget::toaster` cannot be moved: its overlay is laid out against the
  bounds iced hands every overlay, which are the window's
  (`toaster/widget.rs:199-215` against `user_interface.rs:228`), so it sits
  15 px above the bottom of the window whatever it is mounted over. Mounting
  it over a band at the top of the window was built and captured, and the
  toast did not move. So the stack is a `Stack` layer over the picture,
  which also gets the control row's overlay back
  (`overlay::from_children` rather than `Toaster`'s replace-the-content's,
  `toaster/widget.rs:137-162`). Two things that were measured rather than
  assumed: the layer is mounted even when empty, because a tree that grows a
  layer cost the toast five redraws before it reached the screen; and the
  five seconds is a sleep on the async runtime as libcosmic's own is, not a
  poll, because a 250 ms poll cost 3 to 6 redraws a second and dropped
  frames in 2 of 18 report windows against 0 of 18 without it.
  `scripts/uitest.sh` now asserts the placement instead of a reader having to
  notice it: transient chrome must leave the header band and the control-row
  band byte for byte identical, which the shipped-first placement fails.

- 2026-07-31 **A capture is not always one file, and the ONE X2's IMU is
  not mounted like an X4's** (issue #79, owner-reported). Three symptoms on
  the owner's X2 footage were two defects and one thing that was never
  broken. Half a sphere was the camera writing one lens per file: the two
  are paired at open, matched on frame index, and either file of a pair now
  opens the whole capture. The horizon being "way wrong" was the IMU axis
  convention, which fell through to the X4's `xZY` and is 121 degrees out on
  this camera; measured against pixels it is `Zxy`. "Upside down" was the
  same defect seen through a horizon lock that is on by default, and the
  delivered-frame datum it appeared to accuse turned out to be right: the
  unlocked picture is upright on a plumb reference, and the seam's own
  arithmetic agrees to 0.16 degrees.

  Two method notes worth keeping. The 24-way sweep **cannot** finish this
  job on a camera whose two best candidates are a half turn apart when the
  footage has no true horizon in it - a mountain ridge is not level - and
  what finished it was aiming the view along the accelerometer on a still
  frame and looking at whether the sky was there. And a wrong picture datum
  and a wrong axis convention are not separately observable in a locked
  view, because each cancels the other; only the unlocked picture pins the
  datum.

- 2026-07-31 **A saved still is a JPEG; the clipboard is still a PNG**
  (issue #15). Twelve encodings of five real 3840x2160 captures, plus
  libwebp and libjxl for reference, scored against those same pixels with
  ffmpeg's `psnr` and `ssim` filters. Nothing lossless got near the size a
  file that gets shared wants: PNG's own levels bottom out at 3.2 to 8.7 MB
  and take 3 to 7 s to do it, oxipng reaches 2.9 to 7.6 MB in 5 to 7 s, and
  lossless WebP, the best of them per second, 3.0 to 7.9 MB. Of the lossy
  ones only JPEG has a maintained pure Rust encoder: lossy WebP and JPEG XL
  are C libraries, and the one pure Rust JXL encoder does lossless only.
  Skipping them costs nothing measurable. At quality 93 with no chroma
  subsampling a still is 0.7 to 1.8 MB, a seventh of the PNG or less, and
  scores higher on SSIM than libwebp at quality 95 and libjxl at distance 1
  on all five captures, at 1.3 to 2.5 times their file size. The encode is
  65 to 74 ms against the PNG's 33 to 45 ms, on the worker thread that has
  already waited for the GPU and reads back 33 MB before it starts.
- 2026-07-31 **The UI harness builds the binary it drives, every run**
  (`scripts/uitest.sh`). It used to build only when `target/release/kyerag`
  was missing, so a binary left over from before a `git revert` is what it
  drove: the ball check failed twice on a tree whose source passes it four
  runs out of four, and the capture it filed was the reverted design rather
  than the restored one. A harness that reports on code it did not run is
  worse than no harness, and cargo costs nothing when the binary is already
  fresh. `KYERAG_BIN` stays the way to point it at a binary on purpose,
  which is how the stale one was identified.
- 2026-07-31 **The zoom out to the ball is one projection family, not a second
  projection** (issue #47). Perspective and tiny planet are two ends of
  `r = tan(shrink * theta) / shrink`: `shrink` 1 is rectilinear exactly,
  1/2 is stereographic exactly, and below that the sphere closes into a finite
  disc. Blending two separately-written maps was the obvious alternative and
  is worse in the way that matters, because the thing being asked for is that
  there be no seam in the scroll: a family has no crossover to hide. The
  schedule is `shrink = 110 degrees / fov`, which holds `shrink * fov / 2`
  constant past the threshold - the frame keeps the half angle of the widest
  flat view and the world shrinks into it - and that is not a taste: it is
  what makes zooming out zoom out at every point of the frame, where a
  smoothed schedule that overshoots hands back a scroll that reverses in the
  middle (`the_picture_only_ever_shrinks`).
- 2026-07-31 **The field of view is allowed past 360 degrees** (issue #47),
  rather than capping there or switching to a second control. At 360 the
  frame's edges are half a turn out and the sphere is exactly as wide as the
  frame; the owner asked for the ball to sit in frame **with room around it**,
  and room means the frame reaching further than the sphere does. Anything
  else needs a second zoom parameter with a different meaning at the far end,
  which is a worse thing to explain and a worse thing to test.
- 2026-07-31 **Which frames may take the screen and which seek is still owed
  one are two questions** (issue #55). `Player::pump` answered both with one
  epoch comparison: a frame was shown only while its own seek was the newest,
  and showing anything cleared `is_seeking`. That is what froze a fast drag,
  and the obvious repair breaks the other half, because `is_seeking` is what
  keeps a paused window redrawing and an intermediate picture would end the
  wait before the release's frame arrived. `Epochs` now carries `asked`,
  `shown` and a `Wait`, and the two questions are separate methods:
  `accepts` decides what may take the screen, and only the newest seek's own
  frame ends the wait. Three states rather than a flag, because the wait is
  not one thing: a **seek** wants a position newer than the one on screen
  (the reader is still handing over frames of the position being left, and
  they are a picture of nowhere the pilot asked to be, which the exact scrub
  measured at 79 ms of wrong picture), a **step** wants the very next frame
  of the position on screen and sends no seek at all, and **playback** wants
  whatever the clock is due. The landing is applied where the frame arrives
  rather than where the seek was asked for, so several outstanding seeks each
  get their own picture at their own time; `Presenter::advance` takes the
  seek's own frame however many pictures have already gone up, which is what
  makes the release's exact frame the last picture of a drag rather than a
  picture that never comes.

- 2026-07-31 **A picture from a seek the pilot has dragged past is better
  than a frozen one** (issue #55). Frames arrive in the order they were asked
  for, so a landing tagged after the picture on screen is a picture of
  somewhere the pilot has been since, and putting it up can only move the
  picture forwards. Sweeping the fixture end to end, 2 s a rate, interleaved
  arms, medians of 7 runs:

  | positions/s | 10   | 15   | 20   | 30   | 45   | 60   | 90   |
  | ----------- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | before      | 10.0 | 15.0 | 12.5 | 10.5 |  5.0 |  0.0 |  0.0 |
  | after       | 10.0 | 15.0 | 19.5 | 29.0 | 38.5 | 45.5 | 43.0 |

  Below 20 positions/s the decoder keeps up and both arms show one picture a
  position. Above it the old rule falls away to nothing while the new one
  climbs to the decoder's own rate and stays there: 45.5 pictures a second is
  22.0 ms each against a 21.1 ms keyframe decode at the reader. That also
  answers what #46 could not, which was what made a drag cycle cost 100 ms
  where a scrub through the same player cost 26. Nothing did: three landings
  in four were being decoded and thrown away. The release lands the exact
  frame in 49 of 49 drags on each arm, a median of 239 ms after letting go
  before and 281 ms after, which is not a difference the measurement supports
  (permutation p = 0.23): the per-drag spread is 24 to 446 ms on both arms
  and it is set by where in the GOP the release falls, since an exact seek
  decodes forward from the keyframe before it.

- 2026-07-31 **The orientation filter starts only from a reading it would
  believe completely** (issue #45). The rule that covers every other sample
  covers the first one: the seed searches forward for the first
  `accel_seconds` of accelerometer inside the whole of `trust_g`, and the
  gyroscope carries it back to the start of the track. Three alternatives
  were on the table and each was decided by a number rather than by taste.
  A **burn-in pass** would use every trusted sample of the opening instead
  of one window's worth, but it converges at `tilt_seconds` from wherever it
  started, so it needs a second time constant of its own; the forward search
  is one extra walk over at most 20 seconds of samples and no new constant.
  Accepting a **partly** trusted window, which is what the issue asked for,
  leaves the April capture 13.8 degrees off level at 6 seconds against 1.9
  for a fully trusted one, because the window it settles for is taken during
  the launch. And the search **stops at 20 seconds**, the default
  `tilt_seconds`, because past that the filter's own correction is worth
  more than a distant reading and the gyroscope has more of its own drift to
  carry back.

  A file that never reads gravity - a motor running from the first frame -
  gets the window closest to 1 g inside that search, which is a documented
  fallback rather than a panic or a silent identity, and which by
  construction is never a worse reading than the opening window the old code
  took unconditionally.

- 2026-07-31 **The sound goes out through cpal, and follows the picture's
  clock** (issue #13). cosmic-player was read first, as the doctrine asks, and
  it has no audio output code to copy: it is `iced_video_player`, which is
  GStreamer `playbin` with only the *video* sink replaced by an appsink
  (`src/video.rs:20-26`), so its sound leaves through playbin's default
  `autoaudiosink` and its volume and mute are playbin properties
  (`src/main.rs:1225-1235`). No COSMIC first-party binary on this box links an
  audio library for playback at all: `cosmic-settings-daemon` links
  libpipewire, and that is routing. GStreamer is already rejected here for the
  frame path, so the choice was issue #13's own pair, cpal or PipeWire
  directly, and cpal is the smaller by a wide margin. PipeWire still plays
  what it writes, through `pipewire-alsa`. The cost is one apt package,
  `libasound2-dev`, because cpal's Linux target links `alsa` whatever host it
  ends up using.

  The clock is **not** re-anchored on the sound, and that was not a
  free choice either: the pictures are paced by due time against a monotonic
  clock (issue #4), a reframing player must not judder, and a sound card is
  the one clock in the room that cannot be asked to wait. So the sound follows
  instead, in two corrections of different kinds. A **splice** when the ring's
  head is more than 30 ms from where the picture will be: sound whose moment
  has passed is dropped, sound whose moment has not come waits under silence,
  and the gain ramps down before the join and up after it. Only a start, a
  seek landing or a recovered stall is ever that far out. A **resampling
  ratio** the rest of the time, through `swr_set_compensation`, capped at
  0.5% and settling near 0.005%: that is the difference between the sound
  card's crystal and `CLOCK_MONOTONIC`, and without it a ring that is right
  now is tens of milliseconds out half an hour later.

- 2026-07-31 A magnified picture is sampled with a **Catmull-Rom kernel**,
  engaged on the map's own Jacobian (issue #11). The decision is per fragment
  and not per redraw because nothing about it is uniform: the fisheye carries
  1106 texels per radian down its axis and 948 radially at the rim of its
  picture, and a rectilinear output's density rises towards its corners, so
  the widest view the player offers is past 1:1 in the middle of a 2560 px
  window (1.23 texels to the pixel) and two thirds inside it at the corners
  (0.74). The shader reads that as the hardware's quad derivative of the
  landing the model just computed, which is the Jacobian by finite difference
  with the distortion, the mounting and the readout already in it, and needs
  no output size in the uniform block. Catmull-Rom rather than a B-spline
  because the kernel must pass **through** the texels it is given; a B-spline
  is what the usual four-tap trick is written for and it blurs a magnified
  picture. Sixteen texels as nine bilinear fetches, which measured 0.14 codes
  RMS and one code at worst against the same kernel as sixteen point fetches,
  on the highest-contrast view in this footage.

  Engaged **smoothly**, by mixing the kernel's weights from linear towards
  Catmull-Rom between 1:1 and 2:1, rather than by crossfading two sampled
  pictures: one kernel instead of two wherever the zoom sits in the band, and
  exactly the linear weights at the far end, so a view that is not magnifying
  takes the one fetch it always took and writes the bits it always wrote.
  Swept 70 steps of zoom across the whole band, the largest single step in
  which the sharp picture moved further than the bilinear one is 0.4 codes,
  against the 0.6 codes a kernel switched on rather than mixed would have to
  put somewhere.

- 2026-07-31 The **chroma plane is not upgraded** (issue #11), and, like the
  exposure match and the decode gate before it, the measurement is the
  reason. NV12's two planes are two grids, so they were given two thresholds
  and asked separately, which is what found the problem with upgrading the
  smaller one: chroma is half the size, so it is magnified twice as hard as
  luma and it is under 1:1 at **every** field of view this player offers at a
  window anyone uses. Its upgrade is therefore not a cost paid at high zoom
  but a cost paid always, and it is the larger half of the bill. What it buys
  is nothing anyone can see: 0.41 codes on 40% of pixels, and the detail
  metric does not move at all (4.606 with it against 4.606 without, on 4.120
  bilinear). Rendered at four times life size on the most saturated content in
  half an hour of footage, the two are indistinguishable. `Sampling::Sharp`
  is one line and stays runnable; footage with hard saturated colour edges,
  which paramotor flying does not have much of, is what would change the
  answer.

- 2026-07-31 **A scrub takes the decode thread off the lookahead refill**
  (issue #46). The thread used to look at its command queue only between
  reads, so a drag position that arrived while it was refilling the pipeline
  behind the last landing waited for three pair decodes of pictures nobody
  would ever see. `Reader::read_until` now asks an interrupt between packet
  reads and gives the read up when a newer command is waiting; nothing is
  thrown away, because the lanes keep what they decoded and the seek that
  follows is what clears them. Measured on the 37.9 GB fixture, the same 12
  places issue #5 used, medians of 10 runs per arm interleaved so that both
  saw the same box:

  | keyframe scrub             | before  | after   |
  | -------------------------- | ------: | ------: |
  | reader alone               | 20.6 ms | 20.6 ms |
  | through the player         | 59.2 ms | 26.4 ms |
  | picture updates per second | 16.9    | 37.9    |

  So 33 of the 39 ms between the reader and the player were the stale
  refill, and what is left is the thread handover plus one packet read of
  interrupt latency. The exact seek a release asks for came down with it,
  276 ms to 237 ms against a 230 ms reader. Both arms were measured before
  the sound landed (issue #13) and confirmed against it afterwards, three
  runs each: 59.2 ms to 26.5, and 276 to 236.

  **The read a seek itself asked for is never interrupted**, and that is
  load-bearing rather than an omission. A drag asks for positions faster than
  pictures come out of them (10 to 12 a second against 20 to 60 asked for, in
  the table below), so a rule that gave up whatever was newest would give up
  every landing too and a fast drag would show no picture at all.

  It composes with the sound (issue #13) without a rule of its own. A read
  that is given up stops before reading another packet, so it feeds the ring
  nothing more, and the seek it was given up for flushes the ring twice over:
  `Player::hush` on the shell's thread as the command is sent, and
  `Reader::seek` on the decode thread when it arrives. Preempting only
  shortens the gap between those two, which is the window in which the old
  position could still be decoded into the ring.

- 2026-07-31 **Skipping the map for a frame that will be overtaken is not
  worth its line** (issue #46, measured and rejected). Under
  newest-command-wins a refill frame can be decoded, mapped and handed over
  microseconds before the command that makes it stale, and the map is the
  expensive half (`av_hwframe_map` waits for the decode: 7.64 ms a frame in
  the M0 table, twice for a pair). Asking the interrupt before the map
  rather than after it recovers that. It cannot be worth much, and it is
  not: the window is one packet read wide, and interleaved runs of the two
  orderings sit inside each other's spread (26.4 ms against 26.5 for the
  scrub, 237 against 237 for the release, and the drag rates inside a
  picture a second of each other). The measurement is `--bin seek`; the
  ordering that ships is the one with the cheaper claim on it.

- 2026-07-31 **A drag is not a run of jumps, and the instrument now says so**
  (issue #46). `--bin seek` measured seeks one at a time, waiting for each
  picture before asking for the next, which is a hand that stops. A drag
  fires a position per pointer move whether or not the picture has caught
  up, and `Player::pump` shows a frame only while its own seek is still the
  newest, so a hand moving faster than a landing takes shows **nothing**.
  Sweeping the fixture end to end, 2 s per rate, medians of 8 runs:

  | positions/s | 10   | 20   | 30   | 45  | 60  |
  | ----------- | ---: | ---: | ---: | --: | --: |
  | before      | 10.0 | 10.2 |  9.0 | 3.8 | 0.0 |
  | after       | 10.0 | 12.0 | 10.0 | 5.5 | 0.0 |

  The release lands on the exact frame in all 105 of those drags, before and
  after. The interruptible read helps here too, but the ceiling is not the
  refill and this change does not move it: at 60 positions/s neither arm
  puts a single picture on the screen, and that is what a fast drag on the
  scrubber did until issue #55, whose entry above is where that ends.
  Whatever costs the difference between a 26 ms scrub and a 100 ms drag
  cycle was not found here, and an all-or-nothing epoch rule is what turns
  it into a frozen picture rather than a slow one. It is not the page cache:
  a sweep confined to one warm 36 s window of the file measures the same
  10.0, 11.5, 10.5, 5.5 and 0.0 against the full file's 10.0, 12.0, 10.0,
  5.5 and 0.0, interleaved on a quiet box. (#55's answer to the 100 ms: the
  cycle cost one keyframe decode all along, and the epoch rule discarded
  three landings in four.)
- 2026-07-31 The pass **skips the lens a ray cannot reach** (issue #10). Each
  lens's picture is one cap around its own axis; the cap is solved out of the
  calibration by finding where the model's own landing leaves the image
  circle, rather than written down as an angle that would be right for one
  camera. A ray further off the axis than that weighs exactly zero, so one
  dot product per lens replaces a Mei evaluation on the majority of the
  sphere that only one lens can see. The test is one-sided on purpose: false
  means the weight is exactly zero, true means it might not be, so a lens
  kept and weighed zero is multiplied by nothing and only a lens wrongly
  dropped would be a hole. The margin on the cap is half a degree, which is
  thirty times the worst error eight azimuths make on this fixture and costs
  0.4% of the sphere in projections that weigh nothing.

- 2026-07-31 The **decoder is not gated** on it (issue #10), and the
  arithmetic is the reason. What gating the invisible stream is worth,
  measured at playback pace on this box, two runs each, against a 6.10 W idle:

  | lanes                     | CPU, one core | SoC power |
  | ------------------------- | ------------: | --------: |
  | both decoded, both mapped |         7.06% |    9.38 W |
  | both decoded, one mapped  |         6.88% |    9.36 W |
  | one decoded, one mapped   |         4.22% |    7.85 W |

  So the full gate does halve decode power, as the issue predicted: 1.53 W of
  the 3.28 W decoding adds over idle. The cheap version that never goes cold,
  decoding both and mapping one, is worth 0.02 W and is inside the noise.

  What it is worth **on average** is the problem. A gate can only be on while
  no ray of the view reaches the far lens, which at the default 90 degrees of
  field of view is 16.5% of the sphere and 11.0% of yaw/pitch space; at 45
  degrees it is 45.6% and at 110 it is 8.4%. And with the horizon locked,
  which is the default and which the footage demands, a parked view is not a
  parked geometry: the body swings and turns under it. Measured over 40
  parked views and 60 s of two X4 Air captures, at 90 degrees, the gate would
  be on 21.6 to 24.3% of the time with no hysteresis and 8.9 to 9.4% with 15
  degrees of margin, releasing one to three times a minute with nobody
  touching the mouse. With the lock off it holds forever, but only 5 to 17.5%
  of parked views qualify. Expected saving at the default: **0.14 W of about
  10 W**, and 0.26 points of one core.

  Against that, releasing a cold gate is not free and cannot be made free.
  HEVC has no way into the middle of a GOP and this camera writes 29-frame
  ones, so the far lens has to be walked from a keyframe: measured at 195 to
  340 ms, six to eleven frames of stale far hemisphere, and it does not
  depend on how long the gate was on because the hold is bounded by the GOP.
  Three warm strategies were tried. Keeping the packets since the last
  keyframe and replaying them is the best of them and is what those numbers
  are. Decoding the keyframes as they pass shortens the replay by one frame
  of 29 and adds a decode a second. Re-seeking on release is slower (230 ms
  median and 447 worst, issue #5's table) and moves the demuxer the live lane
  is reading from, so it hitches the hemisphere that never stopped. No margin
  closes the gap either: the body alone reaches 551 deg/s on this footage,
  where 15 degrees of margin is 27 ms, and a drag solves for the direction
  under the cursor and so has no rate bound at all.

  A gate that is on a tenth of the time, saves a seventh of a watt, and can
  show a stale far hemisphere for a third of a second is not a trade this
  player makes. `kyerag-spike --bin gating` is the whole measurement and
  `Reframe::reaches` is the test it would have used, both kept so the loser
  stays measurable. What would change the answer is a shorter GOP or a
  format with cheaper random access, and the instrument would say so.

- 2026-07-31 An X4's sensor reads **down the delivered frame**, so
  rolling-shutter correction ships **on** (issue #9). Measured at 1.00 +-0.12
  whole-frame readouts down and 0.02 +-0.07 across, over five stretches of a
  30-minute capture, by one lens against itself a few frames apart. The
  trailer records how long a readout takes and nothing about its direction,
  and a direction applied backwards doubles the skew it should remove, so the
  bar was a control on **each** axis the fit answers on: injecting each of the
  four candidates reads back at 0.85 to 1.02 on its own axis and leaves the
  other where it was. Cameras nobody has measured keep `Sweep::Unknown`,
  which is a zero axis and no correction.

  Same day, and this is the transferable part: **#42 had the answer in its
  own tables and read it as noise**, because it controlled one of the two
  axes it fitted. The uncontrolled axis was reported as "does not repeat" on
  the strength of two stretches, one of which turns out to be a stretch where
  an injected control reads back at -0.10. An instrument that cannot see says
  so in a control column, not in a scatter.

  Same day, second lesson: **a still capture cannot answer a motion
  question.** The settling capture this issue waited on arrived with the
  camera standing on a desk (0.2 deg/s median, 1.5 worst), where a whole-frame
  readout displaces the picture by 0.02 degrees and the instruments' own
  controls can apply 0.003. `--bin rolling` now prints a `carries:` line
  before it decodes anything, which is the file's own rate distribution
  against what the measurement needs.

- 2026-07-30 License AGPL-3.0 (Alex; matches wingover). Unlocks Gyroflow
  GPL-3.0 shader reference with attribution.
- 2026-07-30 No LRV proxy dependence (Alex): full-res decode must stand
  alone; generate proxies only if ever proven necessary.
- 2026-07-30 Frame delivery via DRM_PRIME dmabuf, not hwcontext_vulkan
  (device-lost reproduced on target hardware) and not GStreamer (no
  wgpu/dmabuf sink).
- 2026-07-30 No optical-flow stitching: static calibrated warp measured
  equal or better for a player.
- 2026-07-30 Insta360 MediaSDK rejected: NDA, non-redistributable, no
  seek API, bundled cloud-calling codec.
- 2026-07-30 Primary target is AMD/Intel Mesa (VA-API). NVIDIA would need
  an NVDEC backend variant; out of scope until someone needs it.
- 2026-07-30 ffmpeg-next/ffmpeg-sys-next pinned to 6.1, matching the system
  ffmpeg. The 8.x APIs in the research notes are not present.
- 2026-07-30 Zero-copy import is not a hand-rolled ash routine: wgpu 30's
  `Device::texture_from_dmabuf_fd` (wgpu-hal Vulkan) imports the VA-API
  planes as they come, 0.12 ms/frame for both. On libcosmic's wgpu 28 the
  same import is written by hand (see next entry).
- 2026-07-30 Shell: libcosmic from day one (Alex). Native COSMIC chrome is
  part of the product identity. We accept the hand-rolled ash dmabuf
  import against libcosmic's wgpu 28, and delete it the day libcosmic
  reaches wgpu 30.
- 2026-07-30 PR policy (Alex): the coordinator self-merges once CI is
  green and the diff is reviewed. Alex steers via issues, the roadmap,
  and check-ins.
- 2026-07-30 The trailer is read directly (`crates/meta/src/trailer.rs`, ~45
  lines) instead of through `telemetry-parser`. The published 0.2.6
  aborts on our X4 Air footage: it serializes the metadata protobuf's
  enum fields with `unsafe { transmute }` of the raw i32, and the file
  carries a value no enum in that schema has. The fix exists only on an
  unpublished master that pulls two further git forks. Kyerag was already
  bypassing that crate's lens profile (wrong on the Air) and would have
  had to bypass its merged exposure records (M2), so what remained was
  the record walk. `prost` decodes the eleven fields we read.
- 2026-07-30 libcosmic still pins wgpu 28 (checked, not assumed:
  `pop-os/libcosmic@dc1cf9f` vendors `pop-os/iced@7346cff`, whose
  workspace `Cargo.toml` says `wgpu = "28.0"`; the lockfile resolves
  wgpu 28.0.0 / wgpu-hal 28.0.1). The hand-rolled import stands.
- 2026-07-30 The whole crate is on wgpu 28, spike included. Two wgpu
  majors in one graph would mean two sets of incompatible types for the
  same textures, and the spike's job is to measure the code the app runs.
- 2026-07-31 Carry a wgpu fork (Alex). Nothing exposes
  `VK_EXT_image_drm_format_modifier` on the device `iced_wgpu` creates, so
  the shader widget could not import a frame on stock dependencies. We
  carry the extension-enable hunk of gfx-rs/wgpu#9366 (shipped in wgpu 30)
  applied verbatim to the v28.0.1 tag, on
  `aeharding/wgpu@v28-drm-modifier-backport`, through `[patch.crates-io]`.
  The alternative was a second wgpu device and an external-memory handoff:
  roughly 200 more unsafe lines and a CPU stall per frame.
  **Deletion condition: the day libcosmic reaches wgpu 30, delete the patch
  entry, the fork, and `crates/render/src/dmabuf.rs`, and call
  `vulkan::Device::texture_from_dmabuf_fd` instead.**
- 2026-07-31 The patch entry names `wgpu`, not `wgpu-hal`. Patching
  wgpu-hal alone leaves the rest of the tree on the crates.io wgpu-types,
  and two wgpu-types in one graph is two incompatible `TextureFormat`s.
  `wgpu` is the only crate in that workspace anything outside it depends on.
- 2026-07-31 ~~The app turns libcosmic's content container off
  (`core.window.content_container = false`)~~ (issue #22, superseded by
  issue #93 the same day). It insets the view by `border_padding` on the
  right and, because `nav_bar.active` defaults to true even with no nav
  model, by nothing on the left. Video wants both edges, and turning the
  container off is one of the two ways to get them.
- 2026-07-31 The border padding is zeroed instead, and the content container
  stays (`core.window.border_padding = Some(0)`, cosmic-player
  `src/main.rs:895`, issue #93). `main_content_padding` is `[0, 0, 0, 0]`
  either way (`app/mod.rs:632-639`), so the video still has both edges. What
  the container is worth is the window background: libcosmic paints
  `background(theme.transparent).base` only on the container branch
  (`app/mod.rs:856-874`), and that colour is what makes a COSMIC window a
  darkened pane over the compositor's blur. Without it the welcome view was
  blur and nothing else, which is what the owner saw. cosmic-files paints no
  background of its own either; it just leaves the container on
  (`src/app.rs:2352-2367`, off in desktop mode only).
- 2026-07-31 One crate per layer, in a workspace (issue #19): `kyerag-meta`,
  `kyerag-media`, `kyerag-render`, `kyerag` (the app) and `kyerag-spike`.
  The layer diagram is now a build constraint, and `kyerag-meta` builds and
  tests with no libav headers anywhere on the box, which a CI job that
  installs nothing checks on every push. `[patch.crates-io]` moved to the
  workspace root, the only manifest cargo reads one from.
- 2026-07-31 `kyerag-render` depends on libcosmic, for one file. The three
  `iced::widget::shader` impls are a foreign trait on types `render` owns,
  and coherence forbids writing them in `kyerag`. The alternative, a set of
  forwarding newtypes in the app crate, is more code for the same wiring;
  they live in `crates/render/src/widget.rs` and nothing else in the crate
  mentions iced.

- 2026-07-31 The lens pose composes as `Rz(roll - 90) * Ry(yaw) * Rx(pitch)`
  in the delivered frame's own axes (x right, y down, z along the optical
  axis). The quarter-turn datum is measured, not assumed: applying `roll` as
  `offset_v3` writes it renders an X4 Air on its side, and dropping `roll`
  renders a ONE X2 on its side. Four candidate rotations were rendered
  against plumb references on both cameras
  (docs/research/insv-format.md 4.8). This closes the last open question
  from the format study's section 10.
- 2026-07-31 The Mei forward map is written from the model description
  (Mei/Rives, OpenCV `omnidir`) rather than transcribed from Gyroflow's
  `insta360.wgsl`, so `crates/render/src/projection.rs` is plain AGPL-3.0
  with no SPDX header. The Gyroflow route stays open and stays licensed:
  any file that takes it carries its own header.
- 2026-07-31 The forward map exists twice, in WGSL and in Rust, sharing one
  `Reframe` uniform block. The Rust copy is what `cargo test` can check
  against known angles on a box with no GPU and no footage, and
  `min_binding_size` is what makes wgpu reject a pipeline whose two
  definitions have drifted.
- 2026-07-31 The camera lives in the shader widget's own iced state
  (`Viewpoint`), not in the app's model. Panning is a widget concern, the
  shell has no opinion about it, and no message round trip happens per
  mouse move.
- 2026-07-31 The drag anchors and solves (issue #29). A press stores the
  world direction under the cursor, and every move solves for the view that
  puts that direction back under the cursor: height above the horizon fixes
  the pitch, bearing then fixes the yaw. Stepping yaw and pitch by the
  cursor's own movement is only grab-the-world near the middle of the view,
  because near the pole a yaw turns about an axis nearly along the view
  ray. The horizon stays level, so where an exact answer would need roll
  the pitch clamps and the drag reads as a wall; and where a view pitched
  near the vertical sees past the pole, the solve takes the tilt nearest
  the one it is already at rather than the mirrored view that fits equally
  well.
- 2026-07-31 A ray is in the picture only where the Mei map stays one to
  one, `cos(theta) > -1/xi`, as well as inside the image circle (issue
  #30). Past that turning point the map folds rays from behind the camera
  back inside the circle, which showed as a raw circular fisheye hanging
  behind the reframed view. The bound comes out of the calibration's own
  xi, so no maximum field of view has to be guessed at. It is per lens: a
  fold in one lens is now a ghost printed over a picture the other lens is
  drawing correctly.
- 2026-07-31 ~~The pick between lenses is nearest axis, and it is a branch
  rather than a blend~~ (issue #27, superseded by #7 the same day). One lens
  was sampled per output pixel, which halved the texture fetches; the cost
  was a hard seam. Nothing is left grey either way: the two 97.4-degree caps
  overlap by about 14 degrees. Sampling still uses an explicit mip level,
  because a `textureSample` needs uniform control flow to compute one and
  every imported texture has a single level anyway.
- 2026-07-31 The lenses are mixed by `cos^2(theta/2) * (image_radius -
  landing_radius)`, normalized (issue #7). Longitude preference times
  coverage depth, which is the field docs/research/insv-format.md 6.6
  recommends, and neither factor is a feather width: the band that gets
  blended is the overlap itself, 83.4 to 97.4 degrees off the front axis,
  and the rim of the image circle, where vignetting lands and where the
  distortion polynomial is least trustworthy, is down-weighted for nothing.
  The longitude factor is what puts the crossover on the seam great circle
  rather than wherever the two image circles happen to end, which is 8 px
  apart on the X4 Air.
- 2026-07-31 Exposure is NOT corrected from the shutter records, and the
  measurement is the reason (issue #7). The plan was the symmetric split
  the format study recommended, `front /= sqrt(g)` and `back *= sqrt(g)`
  for shutter ratio `g`. Measured first, on two 30-minute X4 Air captures
  by comparing the mean luma of each lens's overlap annulus, which holds
  the same world directions in both: the real step is 0.9 to 3.5 percent
  and `g` swings 0.54 to 1.81, uncorrelated, and the split makes the step
  14 to 20 percent. The two lenses trade shutter against sensor gain to
  reach the same brightness, so `g` measures how differently the two
  hemispheres are lit and not how differently they came out; the per-lens
  gain that would complete the sum is not in the trailer. What is left,
  three percent laid across a 14-degree band, is under the eye's threshold.
  A measured luma ratio off the overlap band is the fallback and costs a
  readback per frame: do not build it before a capture needs it.
  docs/research/insv-format.md 6.3 has the method and the table.
- 2026-07-31 The trailer is read through record 0's index, not by walking
  (issue #7). Walking is not merely slower on the X4 Air: its trailer
  leaves 163 to 250 KB of slack between records, so the chain stops making
  sense after the three records nearest the footer and the exposure records
  are unreachable. The ONE X2 writes no index and packs its records tight,
  so the walk is still there for it, and where both exist they agree.
- 2026-07-31 The blend's loop runs MAX_LENSES times whatever the file
  holds, and the lens count zeroes a slot rather than shortening the loop
  (issue #7). A loop the shader compiler cannot unroll indexes its local
  arrays dynamically, which puts them in scratch memory: 1.82 ms per redraw
  against 1.68 at 2560x1440, which is more than the blend's own second
  texture fetch costs.
- 2026-07-31 Lens 1's nominal arrangement is a half turn about the body's
  vertical, multiplied on the right of the block's own angles (issue #27,
  docs/research/insv-format.md 4.9). The file does not contain the flip at
  all, and the two orders differ by twice lens 1's roll residual, so this
  was measured rather than picked: each lens rendered alone across the
  seam, and the far-field content correlated between the two pictures.
  0.4 degrees of along-seam residual for this order against 1.5 for the
  other, and the half turn about x, which is a rear sensor upside down,
  correlates with nothing at all.
- 2026-07-31 The shell's design is written down before it is built, in
  docs/UI.md, from the cosmic-player / cosmic-files / cosmic-edit sources
  (issue #16). Every decision in it cites the first-party file it copies,
  and the places where no COSMIC precedent exists are listed as open
  questions rather than answered by us. The written guidelines turned out
  to cover almost none of this: `system76/hig` is one README about dialogs
  and copy that defers to the elementary HIG, which has no keyboard,
  header-bar or media page either. The source is the guideline.

- 2026-07-31 Frames are delivered in pairs, not one stream at a time
  (issue #4). `Frames` carries every video stream at one PTS, so the two
  lenses cannot drift apart: there is no code path that delivers lens 0
  without lens 1. Both are imported into wgpu, and since issue #27 both are
  sampled.
- 2026-07-31 Playback is paced by due time, not by counting refreshes. Each
  frame's due time comes off a monotonic clock anchored to the first frame
  presented, and the shell sleeps until it (`RedrawRequest::At`). The
  shell-side alternative, a `window::frames()` subscription pumping the
  clock on each redraw message, was built first and measured: 33-46
  redraws/s on a 60 Hz display and 1-18 dropped frames per 5 s, because the
  event has to leave iced and come back. The clock is pumped inside the
  redraw pass instead, in `kyerag_render`'s shader widget, which costs a
  `RefCell` in `Scene` and buys 30.0 redraws/s with nothing dropped.
- 2026-07-31 The engine holds container PTS as the frame clock for now. The
  trailer's `pts_type = 2` (`VideoPtsEexposureFile`) suggests the per-frame
  exposure records are the camera's authoritative clock; #4 is pacing, not
  gyro alignment, and #8's Studio-diff harness is what can tell. Only
  `Frames::timestamp` changes if it turns out otherwise.
- 2026-07-31 `Reader::lookahead` is 2 (issue #4). Mapping the oldest queued
  surface rather than the newest hides the `vaSyncSurface` inside
  `av_hwframe_map`: 2.19x realtime at depth 0, 2.46x at depth 2, 2.47x at
  depth 4, so depth 2 takes the whole win. docs/ARCHITECTURE.md's "2-3
  frames in flight" now has a number behind it.
- 2026-07-31 The shell is built to docs/UI.md, which reads the first-party
  COSMIC apps and cites one for every call (issue #16). Three things in it
  are ours rather than theirs, each for a reason recorded there: scrubbing
  seeks keyframes until release (#5), the video does not toggle playback on
  click because the press is the look-around grab, and there is no nav bar.
  Two more the implementation added: the auto-hide timeout is checked by a
  250 ms timer that runs only while playing, because this player sends no
  per-frame message to hang it on (that is the whole point of the pacing
  design), and the `text/uri-list` drop is handled while the portal's
  file-transfer mime is not, because that one is a D-Bus round trip rather
  than a payload and nothing here is sandboxed.
- 2026-07-31 There is no hand-built keyframe index, which is what issue #5
  expected. libavformat parses the whole of `stss`/`stco` out of `moov` when
  the file is opened, so the index is already in memory and `av_seek_frame`
  is a lookup in it: measured at 0.1 ms for the seek call itself, and 70.6 ms
  to open the 37.9 GB file. A second copy of that table would buy nothing.
  `cargo run --release -p kyerag-spike --bin seek` is the instrument.
- 2026-07-31 A drag on the scrubber seeks to keyframes and the release seeks
  exactly (issue #5, docs/UI.md's one deliberate deviation from
  cosmic-player). Measured on the 37.9 GB file: 21 ms to a keyframe against
  230 ms median and 450 ms worst to an exact frame, which is what an accurate
  seek per slider tick would cost. A keyframe lands 455 ms early on average,
  which is half a GOP.
- 2026-07-31 A seek hands its first frame over with no lookahead
  (`Reader::landing`). The lookahead is a pipeline and its depth is paid
  before the first picture comes out of it: 46 ms per scrub against 21 ms.
  Playback refills it behind the landing frame.
- 2026-07-31 `media::first_frame` is gone. Everything reads through
  `Reader`, which takes a `Cue` (frame index or timestamp), seeks to the
  keyframe at or before it and walks forward without mapping what it
  passes: 0.22 s cold to any frame in a 3 GB file, position-independent.
  This is the entry point #5's seek and #8's harness build on, and the
  `reframe` instrument now takes `frame=` and `time=`.

- 2026-07-31 Horizon lock is **on by default** (issue #8). The footage
  decided it: this camera is clamped rolled about a quarter turn and pitched
  down, so an unlocked view of a paramotor flight has its horizon running
  down the picture and swinging out of it, and the reframed view inherits
  every swing of a camera hanging under a wing. `View > Lock horizon` and
  `h` flip it live and the choice is remembered. `h` is bare and is ours,
  like `s`: no COSMIC app locks a horizon, so there is no precedent, and the
  owner asked to be able to flip it while watching.
- 2026-07-31 The camera-body orientation is a **complementary filter**, not a
  Kalman filter (issue #8). Integrate the gyroscope, turn the estimate
  towards the accelerometer with a 20 s constant, and believe the
  accelerometer only near 1 g. A Kalman filter estimates the same two states
  with a covariance nobody can populate from a file that records no noise
  figures. Every constant is measured on real footage and the reason for each
  is in docs/research/insv-format.md 8.5; the one that is a judgement rather
  than a measurement is the 3 s yaw constant, and the numbers either side of
  it are there too.
- 2026-07-31 Yaw is **high passed, not locked** (issue #8). A view welded to
  the heading the file starts on fights every deliberate turn; a view that
  follows the body exactly inherits every swing. At 3 s the view's worst
  heading swing inside a second is 29 degrees against 103 unstabilized, and
  it still follows 946 degrees of real turning a minute against 986.
- 2026-07-31 The IMU axis convention is **measured, not transcribed** (issue
  #8). A three-letter convention string is only half of a convention; the
  other half is the frame it lands in, which is whatever the project it came
  from composes next, and Kyerag's composition is its own. All 24 rotations
  were compared against the horizon in unlocked rendered frames; `xZY` wins
  every stretch of two captures, by 15 to 36 degrees over the runner-up.
- 2026-07-31 The quarter-turn roll datum belongs to the **delivered picture**,
  not to the sensor (issue #8, closing the open question in
  docs/research/insv-format.md 4.8). The IMU is bolted to the sensor and can
  tell the two readings apart: held level by its accelerometer alone, an X4
  Air comes out a quarter turn on its side through `Rz(roll - 90)` and level
  through `Rz(roll)`. `kyerag_meta::Pose` now carries both.
- 2026-07-31 The **gyro is aligned to the exposure records' clock**, and
  playback still paces on container PTS (issue #8). `pts_type = 2` means what
  it says: the camera's own timestamps drift from the container's nominal
  30000/1001 grid at 6.4 ppm, 11.5 ms by the end of a 30-minute file. What
  the choice is worth is 0.10 to 0.15 degrees of camera orientation on
  average and 0.95 to 1.48 at the worst instant; rendered, the two are
  indistinguishable, so the case for the camera's clock is that bound plus
  the fact that the gyro timestamps come off the same clock.
  `FrameClock::Container` is kept so the loser stays measurable.
- 2026-07-31 The orientation track is stored at **200 a second**, not per
  frame and not per IMU sample (issue #8). Per sample is 1.8 million
  quaternions and 72 MB on a 30-minute capture; per frame is too coarse for
  issue #9, which needs an orientation part way through a frame. 5 ms is
  three times finer than the 15.9 ms readout it exists to serve.
- 2026-07-31 Verification is **physics in the footage**, with a Studio export
  as a later drop-in (issue #8). The owner was asleep and no reference export
  existed, so the references are that a horizon is level and an accelerometer
  at rest reads 1 g. `kyerag-spike --bin horizon` measures the horizon's
  angle in rendered frames; its own tests are its positive control, and a
  deliberately wrong axis convention is the negative one, reading 54 to 65
  degrees of standard deviation against 0.04 to 0.68 for the right answer. A
  Studio export becomes one more row in the same table.

## Measured on the target box (AMD Phoenix, RADV, 3840x3840 HEVC)

Per frame, 300 frames, one lens, one frame in flight (`crates/spike/`):

| path      | demux | decode | deliver | import | render | fps   |
| --------- | ----: | -----: | ------: | -----: | -----: | ----: |
| zero-copy | 0.14  | 0.85   | 7.64    | 0.12   | 0.91   | 103.0 |
| copy      | 0.10  | 0.57   | 45.3    | 2.25   | 3.05   |  18.4 |

`deliver` is `av_hwframe_map` (zero-copy) or `av_hwframe_transfer_data`
(copy); `import` is the dmabuf import or `write_texture`. The map stage is
dominated by the `vaSyncSurface` inside it, so it is really decode wait: a
player that keeps 2-3 frames in flight gets that time back. The copy path
does not reach realtime for even one of the two lenses.

Playback, both lenses, 60 s of a 3840x3840 29.97 fps file, rendering
2560x1440 (`cargo run --release -p kyerag-spike --bin playback`):

| lookahead | decode        |
| --------- | ------------- |
| 0         | 2.19x realtime |
| 2         | 2.46x realtime |
| 4         | 2.47x realtime |

| what                        | measured |
| --------------------------- | -------- |
| presented                   | 29.94 fps |
| redraws                     | 30.0 /s   |
| dropped                     | 0         |
| starved                     | 0         |
| worst late                  | 6.6 ms    |
| reprojection pass           | 1.31 ms/redraw |
| CPU (decode + import + pass) | 9.1% of one core |

Sampling both lenses (issue #27) costs the pass about a quarter more, and
nothing else: measured back to back against the same binary before the
change, three runs each at 2560x1440, 1.26 to 1.48 ms/redraw with one lens
against 1.59 to 1.90 with two, still 0 dropped and 0 starved either way.
Every output pixel ran the Mei map twice, once per lens; **since issue #10
it runs it once wherever only one lens can have the ray**, which is most of
the sphere. Three 30 s runs each side of the change, same binary, 2560x1440:

| pass, ms/redraw       | before | after |
| --------------------- | -----: | ----: |
| yaw 0, fov 90         |   1.74 |  1.54 |
| yaw 90, fov 90 (seam) |   1.81 |  1.66 |
| yaw 45, fov 110       |   1.80 |  1.64 |

0 dropped and 0 starved in all eighteen runs, 29.92 fps presented and 30.0
redraws/s throughout, and eight rendered views are byte for byte what they
were before. The seam view saves nearly as much as the axis view because a
90-degree window on the seam is mostly not seam: the band is 14 degrees
wide and everything either side of it drops a projection. The saving is
smaller than issue #9's numbers suggest a Mei evaluation is worth, and that
is the correction rather than a disappointment: the 1.11 ms a readout round
costs is a `turned` (a sine, a cosine and a cross product), a normalize and
a Mei, and the Mei is the cheap part of it.

Blending them (issue #7) costs about a twentieth more again, and only a
little of that is the second texture fetch. Three 60 s runs each of the
same binary either side of the change, 2560x1440, at two views: yaw 0,
which is down the front lens's axis and holds no seam at all, and yaw 90,
which puts the seam down the middle of the picture.

| pass, ms/redraw | yaw 0, no seam | yaw 90, seam through the middle |
| --------------- | -------------: | ------------------------------: |
| hard pick (#27) | 1.61 to 1.62   | 1.61 to 1.63                    |
| blend (#7)      | 1.67 to 1.70   | 1.73 to 1.76                    |

0 dropped and 0 starved in all twelve runs, 29.94 fps presented and 30.0
redraws/s throughout. The seam costs 0.06 ms of that and the rest is
structure: an earlier shape of the same shader, whose loop was bounded by
the file's lens count and so could not be unrolled, measured 1.82 ms at
yaw 0, because a loop that is not unrolled indexes its local arrays
dynamically and they go to scratch memory. Away from the seam the pass
takes the one texture fetch it always did, and writes the same bits: a
one-lens ONE X2 file renders byte for byte what it rendered before the
blend, at three yaws, and so does the front hemisphere of a two-lens file.

The windowed app over the same 60 s: zero dropped and zero starved in
every 5 s report, 30.0-30.2 redraws/s, 13.4% of one core and 295 MiB RSS
for the whole libcosmic process.

Rolling-shutter correction (issue #9) costs what a second pass through the
lens model costs, because that is what it is: the landing row is solved for
rather than computed, and each round of the solve is one more turn of the ray
and one more Mei projection per lens per pixel. Two 60 s runs each at
2560x1440, yaw 90 so the seam is down the middle, forced on through the
harness hook before the direction was known:

| pass, ms/redraw | measured |
| --------------- | -------: |
| correction off | 1.84, 1.85 |
| one round of the solve | 2.96, 2.97 |
| two rounds | 3.99, 3.97 |

0 dropped and 0 starved in all six runs, 29.97 fps presented and 30.0
redraws/s throughout. Switched off it was not nearly free but exactly free:
the same binary rendered two of three test views byte for byte against the
pass before the change, and the third differed in one channel of one pixel of
a million by one code, which is the compiler's scheduling of a refactored
function and not the map.

**Re-measured when it was switched on** (2026-07-31, an X4 reads down the
frame): five 60 s runs of one build, the same view, alternating the file's
own readout against `playback ... off`, which is the pass as it was before
issue #9. **4.00 and 4.23 ms off, 4.28, 4.82 and 5.00 on**, so about half a
millisecond per redraw, and 0 dropped and 0 starved in all five with 29.97
fps presented. Both arms are dearer than the table above because issues #10
and #11 changed what a redraw does; what the table above is still good for is
the shape, which is that a second round would cost as much again as the
first.

Hemisphere gating (issue #10), `cargo run --release -p kyerag-spike --bin
gating`. How much of the sphere a view could gate a lens off for at all, with
the body held still, and what the view axis has to be within for it:

| fov | cone half-angle | view axis within | of the sphere | of yaw/pitch |
| --: | --------------: | ---------------: | ------------: | -----------: |
|  20 |          11.4 deg |         70.7 deg |         67.5% |        54.1% |
|  45 |          25.4 deg |         56.7 deg |         45.6% |        33.6% |
|  90 |          48.9 deg |         33.2 deg |         16.5% |        11.0% |
| 110 |          58.6 deg |         23.5 deg |          8.4% |         5.5% |

And what a **locked horizon** does to that, which is the finding: over 40
parked views and 60 s of each of two X4 Air captures, with nobody touching
the mouse, the body's own swing takes the gate off and puts it back.

| fov | margin | gated       | releases/min | median run  |
| --: | -----: | ----------: | -----------: | ----------: |
|  45 |  0 deg | 48.4, 49.9% |     2.6, 4.5 | 0.60, 1.43 s |
|  45 | 15 deg | 32.8, 31.1% |     1.5, 2.4 | 1.77, 2.30 s |
|  90 |  0 deg | 24.3, 21.6% |     0.9, 2.6 | 1.90, 2.10 s |
|  90 | 15 deg |   9.4, 8.9% |     1.4, 2.0 | 0.57, 1.17 s |
|  90 | 30 deg |   0.9, 0.3% |     0.9, 0.3 | 0.47, 0.40 s |
| 110 | 15 deg |   3.2, 2.5% |     0.5, 1.0 | 0.70, 0.90 s |

Releasing a gate, packets held since the last keyframe and replayed, nothing
mapped on the way:

| gated for | held packets | catch-up      | frames stale |
| --------: | -----------: | ------------: | -----------: |
|     0.5 s |           13 | 195 to 204 ms |       6 to 7 |
|     2.0 s |           28 | 237 to 335 ms |      8 to 11 |
|    10.0 s |           28 | 280 to 339 ms |      9 to 11 |
|    30.0 s |           28 | 293 to 340 ms |      9 to 11 |

Screenshots (issue #15), 3840 px wide at the window's aspect. In the
windowed app, 20 s of playback with a still saved every 2 s and copied
every 7 s: zero dropped and zero starved in all four 5 s reports, 28.9 to
30.0 fps presented. Headless, five captures over 10 s
(`playback <file> 10 60 5`): 29.77 fps presented, zero dropped, zero
starved, and `prepare` costs 2.19 ms on the redraw that takes a capture
against 0.57 ms on one that does not (worst 6.26 ms against 3.29 ms). What
is left is the readback and the encode, and both belong to a worker
thread: 45 to 53 ms to encode one 8.5 MB PNG.

High-quality zoom sampling (issue #11), `cargo run --release -p kyerag-spike
--bin zoom`. How far the view magnifies the source, down the front lens's
axis at 2560x1440, and how far each plane's kernel engages there:

| fov | centre | corner | luma engaged | chroma engaged |
| --: | -----: | -----: | -----------: | -------------: |
|  20 |  0.152 |  0.150 |         1.00 |           1.00 |
|  60 |  0.499 |  0.435 |         1.00 |           1.00 |
|  90 |  0.864 |  0.626 |         0.18 |           1.00 |
| 100 |  1.030 |  0.686 |         0.00 |           1.00 |
| 110 |  1.234 |  0.743 |         0.00 |           0.86 |

Texels per output pixel: under 1 is magnifying. Two things to read off it.
The player is **already magnifying this camera at its own default view**, by
16% in the middle of the picture and 60% at the corners, before anybody
touches the wheel. And the chroma plane, at half the grid, is under 1:1 at
every field of view on offer, which is what cut its half of the upgrade.

What the two halves are worth, at 2560x1440 on real footage, as the mean
absolute Laplacian of the luma ("detail") and as the difference between the
pictures:

| view                          | bilinear | luma          | both planes   |
| ----------------------------- | -------: | ------------: | ------------: |
| ground and buildings, fov 50  |    4.120 | 4.606 (+11.8%) | 4.606 (+11.8%) |
| wing and lines, fov 50        |    1.126 | 1.214 (+7.8%) | 1.217 (+8.1%) |
| wing and lines, fov 25        |    0.676 | 0.688 (+1.8%) | 0.694 (+2.6%) |

| view                         | luma moves            | chroma adds          |
| ---------------------------- | --------------------- | -------------------- |
| ground and buildings, fov 50 | 1.835 codes over 85.4% | 0.412 over 39.9%    |
| wing and lines, fov 50       | 0.475 codes over 42.5% | 0.259 over 25.6%    |
| wing and lines, fov 25       | 0.453 codes over 40.1% | 0.268 over 26.5%    |

The upgrade is worth most in the **middle** of the zoom range and least at
the end of it, which is the opposite of what the issue expected: at 5x, on a
white canopy, the source has nothing left to resolve and the two kernels draw
the same ramp. Textured content at 1.6x is where a bilinear tent is
measurably the wrong shape.

The pass alone, one process, a still frame, the three settings interleaved
render by render so a laptop that throttles throttles all of them, least of
39 renders a cell:

| pass, ms/redraw | bilinear | luma (ships) | both planes |
| --------------- | -------: | -----------: | ----------: |
| fov 20          |     0.56 |         0.76 |        1.00 |
| fov 25          |     0.53 |         0.69 |        0.93 |
| fov 35          |     0.54 |         0.70 |        0.96 |
| fov 60          |     0.61 |         0.79 |        1.07 |
| fov 90          |     0.69 |         0.90 |        1.23 |
| fov 100         |     0.69 |         0.87 |        1.19 |
| fov 110         |     0.70 |         0.79 |        1.07 |

And under playback, which is the same pass with a cold texture cache and two
decoders running beside it (`--bin playback <file> 20 60 0 0 file <fov>
<setting>`). Every row here presented 29.9 fps with **0 dropped and 0
starved**; rows the box spoiled are left out, and the box was shared for part
of this session, which is why the controlled table is the one above.

| pass, ms/redraw | bilinear | luma | both planes |
| --------------- | -------: | ---: | ----------: |
| fov 20          |     2.07 | 2.83 |        3.39 |
| fov 45          |     2.06 | 2.52 |        3.41 |
| fov 90          |     2.23 | 2.47 |        4.68 |
| fov 110         |     2.23 | 2.40 |        3.57 |

Three properties, measured rather than argued:

- **It touches only what is magnified.** At fov 110 and 2560x1440, 47.7% of
  the picture is under 1:1 and 20.1% of it moved; the least magnified pixel
  that moved sits at 1.0008 texels to the pixel, where 1.0 is the switch-off.
  Rendered small enough that the whole picture is past 1:1, at 640, 960 and
  1280 px wide, the shipped setting is byte for byte the picture bilinear
  drew. Upgrading chroma as well is not byte-identical even at 960 px wide.
- **It does not pop.** Over 71 one-degree steps of zoom, the shipped setting
  is 0.179 codes from bilinear at fov 110 and 1.838 at fov 40, and the
  largest single step in that difference is 0.047 codes, 2.5% of it, where a
  kernel switched on rather than mixed in would put all 1.838 in one step.
  Step for step the picture itself moves 1.050x as far sharp as bilinear at
  the median and 1.063x at the worst, spread along the whole sweep: a sharper
  picture moving, not a kernel arriving.
- **A still gets it without being told** (issue #15). A 3840 px capture off a
  2560 px window magnifies 1.5x harder (0.293 texels to the pixel against
  0.440) and is byte for byte a 3840 px render of the same view, because the
  magnification is read off the hardware's quad derivative in whatever target
  the pass is drawing into rather than out of a resolution in the uniform
  block.

Seeking, 12 places from 1% to 97% of the 37.9 GB file, warm
(`cargo run --release -p kyerag-spike --bin seek`):

| what                          | median   | worst    |
| ----------------------------- | -------: | -------: |
| open the file (moov, once)    | 70.6 ms  |          |
| `av_seek_frame` alone         | 0.1 ms   |          |
| keyframe seek, reader         | 21 ms    | 49 ms    |
| exact seek, reader            | 230 ms   | 447 ms   |
| keyframe seek, through Player | 26 ms    | 54 ms    |
| exact seek, through Player    | 237 ms   | 473 ms   |

The worst case is not the far end of the file: it is whichever seek runs
first after the decoders warm up, and 97% costs the same as 1%. The player
used to cost 59 ms against the reader's 21, because the decode thread
finished the lookahead it had started behind the previous landing before it
read the next command; issue #46 made that read interruptible and the two
numbers above are what is left.

The same instrument measures a drag, which asks for a position per pointer
move rather than waiting for each picture, sweeping the file end to end for
2 s per rate:

| positions/s     | 10   | 15   | 20   | 30   | 45   | 60   | 90   |
| --------------- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| picture updates | 10.0 | 15.0 | 19.5 | 29.0 | 38.5 | 45.5 | 43.0 |

A picture reaches the screen while its own seek is newer than the position on
screen (issue #55), so a hand faster than a landing sees the landings it has
passed over: the rate rises with the hand to the decoder's own ceiling of
about 45 a second, which is one keyframe decode each. Under the rule that
shipped before #55 the same sweep read 10.0, 15.0, 12.5, 10.5, 5.0, 0.0 and
0.0, the last two being a frozen picture for the length of the drag. The
release lands on the exact frame every time either way.

## Ideas parked (complexity needs an observed failure first)

- Decoded-GOP cache in GPU memory for instant reverse scrubbing
  (~44 MB/frame; a 30-frame window is ~1.3 GB).
- Vulkan Video decode (drops VA-API plumbing; blocked on wgpu exposure
  and Rust HEVC support anyway).
- Batch screenshot/export queue across multiple files.
