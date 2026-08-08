# Roadmap (living doc)

Update this file in any PR that changes project status. Work queue is
GitHub issues; this doc is the map, issues are the tasks.

**Status 2026-07-31:** feasibility study complete (docs/research/), repo
bootstrapped, M0 done, M1 done, and the horizon holds still.
`cargo run --release -p kjerag-spike -- <file.insv>` decodes one 3840x3840
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
with the toasts in: zero dropped and zero starved in all six reports. **And
the view can be quoted**: `i` copies one line naming the video, the frame
and the framing, written as `reframe`'s own arguments, so a report about a
360 video carries the direction it was pointing rather than leaving everyone
to guess it. Every capture prints the same line, because a still's name
carries the video and the moment and nothing carries the direction. The copy
carries the file's name alone and the terminal line carries the path.

**And the line is a place, not a label.** `Ctrl+V` goes there: the frame it
names, the direction it was pointing, the horizon it was held with, as a jump
and not an animation. A reference carrying a path opens the video it names
first; one naming a video that is not open says which video it is from; a
clipboard holding anything else does nothing at all, because `Ctrl+V` over a
video means nothing in any other player either. The command line takes one
too, so the terminal line is a complete launch command:
`kjerag flight.insv time=9.576 yaw=144.40 pitch=0.90 fov=24.10 lock=1`.
All three read it with the one parser in `crates/render/src/framing.rs`, which
is also where it is written; reframe's real parser reads the same line in a
test, so no two of them can drift. A `lock=1` yaw is a direction in the
stabilized world frame, whose zero moved on 2026-08-06, so a line copied before
that date lands somewhere else and says nothing about it (docs/UI.md, the view
line). Measured under the harness: copy a view,
seek ten seconds away, zoom out a notch, paste, and the copied line comes back
to the millisecond and the hundredth of a degree with the picture byte for
byte what it was.

**M1 is done.** The seam blend (issue #7) is the first M2 quality item and
it has landed: where the two lenses overlap the pass mixes them by
longitude preference times coverage depth, with no feather width anywhere
in it, and the hard line and the tone edge where the hemispheres met are
gone. Near-field structure crossing the seam, which on this footage is the
wing and the lines, ghosts softly instead of stepping, which is parallax
and is the expected trade. Exposure is **not** corrected from the shutter
records, and that is the finding rather than an omission: the trailer's two
are parsed and kept apart (issue #7's other half, and the camera's own frame
clock for #8), but the two lenses trade shutter against sensor gain to reach
the same picture brightness, so the ratio is not a brightness ratio and the
symmetric split it implies makes the step four to twenty times worse. It is
corrected instead from what the band measures on the pixels (issue #103,
stage 3, docs/research/insv-format.md 6.10).

**The horizon is locked** (issue #8), which is the second M2 item and the
one the feasibility study called the riskiest correctness surface. The
trailer's IMU is parsed and integrated into a `world_from_body` quaternion
every 5 ms of the file, and the reprojection pass composes its inverse
between the lens mounting and the camera, so the world stays put while the
camera swings. Roll, pitch and yaw are all locked, so the view holds a
direction in the world and a deliberate turn moves the aircraft rather than
the picture (owner ruling, 2026-08-06; it was a 3 s yaw high pass until
then). Drag to look around needed no change at all: the anchor it stores is
in whatever frame the camera rotation lands in, and with the lock on that
frame is the world. Measured on rendered frames: the horizon moves 0.23
degrees peak to peak over 120 frames of calm flight and 2.86 through a
61 deg/s roll, where with the lock off it leaves the picture entirely.
`View > Lock horizon` and `h` flip it live, and it is on by default.

Two conventions were settled on the way, both against pixels rather than
against other people's tables. The IMU's axis convention is `xZY` for the
X-series in Kjerag's own frame, picked out of all 24 rotations by comparing
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
would need. Those were read under the heading follow; with the world-fixed
lock of 2026-08-06 the body turns fully under a parked view and the same
measurement on one of those captures reads 15.7% and 3.9%, which makes the
answer below no less final. Releasing a cold gate costs 195 to 340 ms, six to eleven frames
of stale far hemisphere, because HEVC has no way into the middle of a GOP and
this camera's is 29 frames. Expected saving: **0.14 W**, for a state machine
and a packet ring through the frame path. `kjerag-spike --bin gating` is the
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
`kjerag-spike --bin dip`: at 6 seconds 48.9 degrees becomes **1.9** on the
April capture and 14.7 becomes 8.1 on the June one, at 30 seconds 29.2
becomes 3.9 and 6.0 becomes 2.5, and at 300 seconds both files read what
they read before to three decimal places, which is the control. What is
left is the accelerometer's own disagreement with gravity inside the seed
window, 2.8 degrees on one capture and 9.8 on the other, and that is the
residual issue #57 is about.

**And the window was the residual** (issue #152, 8.8). That fix tested the
magnitude of one second of accelerometer, which is nearly blind to the
horizontal acceleration that tilts it: the August 2 capture's launch weighs
1.039 g at 21 degrees off vertical and was believed completely, and the
horizon stayed 17 to 21 degrees off level for over a minute. The seed is now
the whole opening minute averaged, which is bounded by how much an aircraft's
speed can change in a minute rather than by what one second happened to read.
That capture now opens **3.33 degrees off level against 20.97**, inside the
4.05 it settles at four minutes in, and against a backward pass over each
file the seed error is better on all six owner flights, worst case **3.03
degrees against 24.18**. Not a clean sweep of everything: one sibling file
that was already right by a third of a degree gives up a quarter of one, and
April 10's opening reads better at the first frame and worse from 4 to 20
seconds, with the two instruments disagreeing about which.

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

Measured on real footage at 2560x1440 (`kjerag-spike --bin ball`): one scroll
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
`kjerag-spike --bin seam` measures it round the whole seam circle and splits
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

**Phase 2 applies it, per camera, and then narrows the band** (issue #48, the
owner's "2 deg looks good"). The correction is five knobs, a relative rotation
and a principal point, **fitted once against a capture from a camera standing
still and stored under that camera** rather than fitted per file. Which of
those two is right was measured rather than argued, and not on the number a
per-file fit minimizes for itself. **The per-file fits disagree with each
other**: fitted file by file, the same glued pair of lenses asks for yaws from
-1.69 to -2.58 degrees and principal points 13 px apart, which is 15 px of
picture at the seam between two answers for one camera that did not change
between April and July. **The static capture's answer, applied unchanged to
all five flights, reads the same along-seam number their own fits do** (0.15
to 0.22 degrees, within 0.002 of the per-file answer on three of five) and
well inside the 0.31 to 0.40 the per-file rotation left. That axis is the one
parallax cannot reach, so it is calibration and nothing else. On the
far-field control the per-camera answer leaves **0.022 degrees along and 0.106
across**, which is 1.8 view pixels, against 6.7 for the per-file rotation.

That deletes three things this milestone used to carry: the two second wait
before a first play was corrected, the cache under the file's own hash, and
the thin-file failure mode where a capture with seven usable azimuths got a
fit of its own. `View > Calibrate seam from this video` is the one action, on
a worker thread, about two seconds, with a toast when it lands; a camera with
nothing stored still gets a best-effort fit off the file being played, which
is the old path demoted to a fallback and labelled as one in the report line.
The store is cosmic-config **state**, keyed by a serial-free camera key: the
model, the delivered frame size and the factory calibration string, hashed.

**Then the crossover, 2 degrees instead of the 14-degree overlap.** On flight
footage the doubled band goes from 10.60 degrees to 1.50 and its sharpness
against the front lens alone from 0.723 to 1.074. Against Insta360's own
stitch, the benchmark: they keep 0.92 to 0.97 of their own sharpness across
the seam, we kept **0.579**, the calibration alone took us to 0.689 and the
band takes us to **0.871** -- four fifths of the gap, closed. The pass costs
what it cost (3.77 ms per redraw before, 3.59 after, interleaved at
2560x1440), which took two attempts: reading the two lenses' angles back out
of the `Blend` array after the loop that fills it costs 5.5 ms against 3.6,
and only `--bin playback` under live decode can see that -- `--bin zoom` reads
the two as equal.

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

`kjerag-spike --bin sync` is the measurement, and it drives the real player
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
  **M2 is complete**: issue #48 reopened the seam, and both halves of it have
  now shipped. The two lenses were misaligned by up to 2.7 degrees across the
  seam; phase 1 measured that and attributed it to a relative lens tilt, and
  phase 2 calibrates it per camera off a capture the pilot points at and hands
  the picture over in a 2 degree crossover instead of the whole 14 degree
  overlap. What was left at the seam on flight footage is parallax, which is
  depth rather than geometry, and **stage 2 of issue #103 now measures it on
  every frame the pass draws**: a compute pass over the two imported textures
  reads the overlap band as the stereo pair it is, along the axis the file's
  own 33 mm baseline names, and each lens's ray is bent by the other lens's
  blend weight times what the two disagree by. Far field and near field take
  opposite time constants, 2 seconds against a tenth of one, which is what
  makes a per-frame reading steadier than the per-clip table phase A
  recommended rather than noisier: flicker 0.008 to 0.023 degrees rms against
  phase A's 0.22 to 0.54 for a naive per-frame table. It costs 0.3 ms a redraw
  at 2560x1440. **Stage 4 then made the crossover itself an answer to that
  same measurement**: the clamp and the width are one inequality,
  `|disparity| <= 0.9 * width`, and stage 2 held the width at 2 degrees and
  solved it for the disparity, which threw alignment away on everything
  nearer than 1.06 m. Solving the same line for the width opens the band to
  exactly what the reading needs and returns the floor bit for bit
  everywhere else, so the far field is the picture it was. It recovers up to
  11.5 view pixels of doubled edge, on 0.02 to 0.19 percent of
  direction-frames of the owner's own flights and 0.2 to 2.3 percent of the
  handheld and selfie-stick corpus, for 0.06 ms a redraw. Issue #79 opened
  a second camera: the ONE X2 writes one lens per file, and the player now
  pairs the two at open and holds an X2's horizon with that camera's own IMU
  convention.
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

- 2026-08-07 **A DJI Osmo 360 `.OSV` plays, on an equidistant lens model and
  with no horizon lock** (branch `feat/osmo-osv`, MVP). `kjerag <file>.osv` is
  the whole of it. The calibration is in the file's own `djmd` telemetry track
  rather than a trailer, as a protobuf with no `.proto` anywhere, read by field
  number in `kjerag_meta::osmo`; both units' parsed intrinsics match the
  scoping pass's independent table exactly, to the last digit of the `f32`.

  **Equidistant only, `r = fx * theta`, and the four `k` coefficients the file
  carries are read past.** Each lens entry also writes a fourteen-point mask of
  where the camera body cuts the picture, and on all four lenses of the two
  units it sits 1804 to 1860 px out. Equidistant puts that at 98.6 to 101.7
  degrees off axis, so 197 to 203 degrees of coverage, bracketing DJI's
  published 199. The Kannala-Brandt theta-polynomial reading of the same
  coefficients turns over between 88.4 and 89.8 degrees at 1615 to 1640 px and
  folds back, so it cannot reach that ring at any angle and would make a 199
  degree lens a sub-hemisphere one. The inverse reading folds too. No candidate
  form was kept, and the refusal is a forward check anyone can redo from the
  file rather than an overlap score: the scoping pass's overlap scorer
  preferred a known 20 px principal-point error, so its preference is not
  evidence.

  **Horizon lock is off on this format and says so.** The file carries a fused
  orientation at about 1 kHz whose frame is not pinned, and applying it naively
  made the scoping stitch worse, so none is read. The menu item draws disabled,
  the key bind does nothing rather than flipping a setting that cannot move the
  picture, and the app prints one `level:` line at open. Manual pan is v1.

  **The seam is left to fit itself and refuses.** No inter-lens translation is
  recorded, so the parallax band switches off, and on both units the fitter
  found 0 of 72 azimuths with content it could match and kept the factory
  calibration. Far-field content joins cleanly; near-field shows a soft band at
  the handover. That is the accepted v1.

  Measured on the target box with `--bin playback`, rendering 2560x1440, on a
  box with nothing else running (VA-API, 10-bit; kjerag has no software decode
  path). Decode at lookahead 4, then 20 to 30 s of paced playback:

  | capture              | decode         | presented        | dropped | starved | pass    |
  | -------------------- | -------------: | ---------------: | ------: | ------: | ------: |
  | unit B 8k30p         | 2.56x realtime | 29.57 of 29.97   |      10 |       9 | 6.91 ms |
  | unit B 8k50p         | 1.60x realtime | 49.45 of 50.00   |       4 |       3 | 5.11 ms |
  | unit A 8k25p         | 3.00x realtime | 24.87 of 25.00   |       0 |       0 | 6.51 ms |
  | X4 Air `.insv` 8k30p | 2.44x realtime | 29.94 of 29.97   |       1 |       0 | 7.90 ms |

  So an `.OSV` plays at its own rate, and costs what the `.insv` in the last
  row costs: **hardware decode is not the bottleneck at 8k30p on this box**,
  which is worth saying because the scoping pass's ffmpeg software decode of
  the same file ran at 487 percent CPU for realtime. A 5 s run taken while the
  box was busy read 26.6 fps and 20.6 ms a redraw, and is what this line said
  before; a number measured under contention is a number about the contention.
  D-Log M is out of scope: those files play with the log look, and no transform
  for it is in the container.

- 2026-08-07 **An acceptance line names the pose instead of copying it**
  (`seam=pool`, docs/research/reference-views.md). Three acceptance commands -
  the shimmer line, the May-01 crossing pair, and the `--bin step` block under
  seam-two-axis's "How to run the two instruments" - carried
  `seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`. That string is
  the knob-by-knob **median** of the owner's five-sample pool and no member of
  it: roll and cx off one fit, yaw off a second, pitch and cy off a third. It is
  the combination `SeamPool::answer` was changed to stop shipping on 2026-08-05,
  so **those three commands ran a pose the app had not drawn for two days**;
  every acceptance line written since then names the drawn one. The pose it
  draws is
  `roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91`, confirmed both by
  `config`'s own fixture test and by a `--bin reframe seam=pool` run on the
  owner's Jul-14 capture.

  The durable half is `seam=pool`, which every instrument that takes a `seam=`
  now takes: it reads this box's saved state through the app's own reader and
  applies the app's own `SeamPool::answer`, so a line written with it cannot go
  stale against the app at all. It refuses loudly rather than falling back when
  the pool holds nothing for that camera. That needed one new edge in the
  workspace, `spike -> app` (docs/ARCHITECTURE.md), because the alternative was
  a second copy of the pool's format and rule in the instruments, which is the
  shape of the defect itself. Six instruments shared one `enum Seam` in the
  same change; there had been six copies of it and two of the knob parser.

  **The recorded readings on those three lines were measured through the old
  string and have not been re-read at the drawn pose.** They stay, flagged in
  place. Re-reading them is a job of its own.

- 2026-08-06 **The reference registry is re-derived into the world-fixed frame,
  and the seam ladder is re-baselined there** (`--bin carried`,
  docs/research/reference-views.md). Fourteen `lock=1` lines, one correction
  each, computed at the line's own frame rather than per file: `carried` runs
  from -72.41 to +79.33 degrees across the registry and moves by 3 degrees a
  second at the shimmer view, so no two lines in a file share a correction.
  Each was then checked in the picture, 67a4bcf rendering the old line against
  this build rendering the new one: twelve match at zero pixels of 1024 and two
  at 2 px, correlation 0.92 to 1.0000, against a control that leaves the yaw
  alone and lands 1.7 to 20 degrees out.

  **Two of #165's numbers do not survive that.** The shimmer view's re-derived
  aim is `yaw=162.31` and not the `160.63` published there, which was the same
  rule read off a half-second grid; the picture is 83 px out at 160.63 and 2 px
  out at 162.31. And the eight-fold improvement in `--bin shear`'s floor
  (0.0773 -> 0.0099 deg of step rms at the seam band) is neither a floor nor an
  improvement: at the corrected aim it reads 0.0687, and dropping each run's two
  worst steps leaves 0.0097 before, 0.0088 after and 0.0094 at #165's aim. One
  frame in ninety fails its correlation and decides the statistic. What the lock
  change really bought is yield, because fewer frames are refused for a seam
  past `TILT_LIMIT`: usable step pairs go 71 to 89 at the seam band and 30 to 89
  at `+60`. The seam itself did not move, `-150` reading 0.3646 -> 0.3381 deg
  applied, and `--bin crossing` at the May-01 GOOD/BAD pair moves by 0.14 source
  px across the change with its verdict and its sensitivity floor intact.

- 2026-08-06 **The horizon lock holds the world, heading and all** (owner
  ruling; `Filter::yaw_seconds` 3 s -> infinite). It supersedes the
  2026-07-31 entry below, which chose the high pass.

  The owner asked for what Insta360 Studio does, in his words hold the world
  still, and the oracle probe had already measured how far from that the
  shipped design was. Against a Studio export of the same July 14 window,
  registered chain-free frame by frame, kjerag's view swept **404.7 deg/min
  away from Studio's with r2 0.98** over ten seconds: a straight ramp, which
  is a design and not a defect. The same measurement of this build reads
  **4.4 deg/min with r2 0.04** over the same ten seconds, which is no ramp
  at all, only a 6.6 degree peak-to-peak wobble the two stitchers disagree
  by. Dense phase correlation, the second instrument, reads the picture's
  own slide over the probe's headline three seconds at **22.3 degrees before
  and 3.4 after**, against Studio's 0.1.

  **The accepted price is gyroscope drift, and it is the technique's floor
  rather than a shortfall.** Nothing observes heading: gravity cannot see it
  and no capture on the test box carries a byte of the trailer's magnetic
  record, so a locked yaw inherits the gyroscope's zero and nothing ever
  corrects it. Studio's own export drifts the same order, 2.2 deg/min on the
  probe's window.

  **It is not a steady creep, which is the thing to say out loud.** The locked
  frame turns about the world vertical at `bias . up_in_body`, so a camera
  hanging tens of degrees off vertical brings its horizontal bias components
  in, and those are the larger ones: on the July 14 file `--bin drift` walks
  the running error to -36 degrees by minute 3, +87 by minute 8 and +149 by
  minute 19, about 185 degrees peak to peak, against a signed mean of 2.08
  deg/min. Quoting the mean alone, or the body's own yaw-axis bias, describes
  a flight nobody flew. The shape is the finding and the size is not: this
  file has no still moment good enough to read a zero from, and the ten
  seconds `--bin gyro` picks instead give the same walk hundreds of degrees
  either way with a 1.40 deg/min mean.

  **The world frame's zero is the heading at the file's first IMU sample**,
  not its first video frame: 18.71 degrees of the body's own turning apart on
  that capture, and that is where `Ctrl+0` lands.

  What the pilot loses with all this is the fly-forward feel: the view used to
  settle back onto the nose within a few seconds of any turn, and now a turn
  leaves it pointed where it was, so
  a flight that turns round shows the way it came until somebody drags. What
  goes with the follow is its erosion of a pan against the world, and gyro
  drift takes that place: issue #44 was closed by the owner as the t0 seed
  transient and not as a defect in the drag or the follow (entry above), and
  the old design's own control says the same, 0.62/0.03/0.38/0.18 degrees per
  half second after a pan at `from=300`.

  **Every stored `lock=1` view line moved, and by a lot.** The yaw in one of
  those lines is measured in the stabilized frame, whose zero was the
  followed heading and is now the file's opening heading, so the two differ
  by however far the old follow had been carried: measured on the July 14
  file through `--bin lean`'s own heading column, 6.8 degrees at the first
  frame, 44 by 6.5 s and 157 by 36 s. The rule is
  `new_yaw = old_yaw + carried(t)`, confirmed in the picture at the shimmer
  view. **The rule holds and the number this entry first put on it does not**
  (corrected 2026-08-06): that view's re-derived aim is `yaw=162.31` and not
  the `160.63` written here, because 156.85 is `carried` at the 36.036 instant
  the half-second walk measured and 158.53 is `carried` at the frame
  `time=36.303` shows. The picture is 83 px of 1024 out at 160.63 and 2 px out
  at 162.31, which is where the "to 1.6 degrees" came from. The registry was
  re-derived line by line the same day, in the entry above.

- 2026-08-07 **The along-seam field is real on the unbent projection and worth
  nothing in the delivered picture, because the per-frame band had already taken
  it out. It is measured, guarded, stored and NOT applied; what ships is the
  per-reading trim and a new rule about acceptance** (issue #103, stage 9 layer
  2, docs/research/stage9.md 8 and 9).

  **What was built.** `seam::measure` reduced each azimuth's frames with a mean
  over a population that moves 0.22 to 0.48 deg by rms between frames; it now
  reduces them with `seam::left`'s own rule applied per frame, one function
  (`seam::tolerated`) shared by the ring gate and the per-frame trim. On the
  unbent projection the trim alone takes the pooled X4 leftover under the stored
  pose from **0.0828 to 0.0653 deg** and the corpus's cross-capture agreement
  from 2 pairs of 15 to 15 of 15. `seam::along_terms` then reads the five terms
  `band::Along` is written in, above the factory calibration and above no pose
  at all, and held out through the shipped functions it takes the pooled
  leftover **0.0644 -> 0.0375 deg on the X4 Air and 0.0414 -> 0.0140 on the ONE
  X2, 9 of 9 improved**. At the two May-01 crossings `--bin crossing` reads the
  along-seam median **1.29 -> 0.12 view px at GOOD and 1.47 -> 0.93 at BAD**,
  both improved, neither traded.

  **Why none of that is an applied result.** Every instrument above draws the
  **unbent** projection: `seam::measure` reads its ring through `Reframe` with
  no band, and `--bin crossing` builds its map with `Held::default()`. The app
  does not. Photographed out of the app itself, at the same clip and view, the
  delivered along-seam axis on `main` is **already at or under 0.6 view px at
  GOOD** - the band's own per-frame `Along` fit had taken the same leftover out -
  and the field arm matches it within 0.2 against an instrument shown capable at
  1 px. At BAD `main` reads **-0.11 view px** where the unbent projection reads
  1.47, and the field arm reads **-2.06: two view pixels the wrong way**. At the
  shimmer view the field arm is slightly worse on every probe.

  **The owner's blind A/B said the same thing first**: "same, both bad" at every
  view, with the `main` arm called slightly steadier at the shimmer view, which
  the instrument agrees with at 10.081 against 10.214 codes per frame over 60
  frames.

  **Why reading the table through the band did not save it.** With a table `T`
  applied and the band measuring through it, the delivered correction is
  `T + fit(L - T)` against `fit(L)` with none, so the two differ by exactly
  **`T - fit(T)`**. `Along::fit` reproduces `T` only where the ring has
  evidence, and a session's ring is an arc: planting the real pooled field,
  `T - fit(T)` reads 0.0007 deg rms with all 128 directions covered and
  **0.0333 rms, 0.0696 worst at the 27 of 128 `--bin step` reports on real
  footage** - 1.13 view px at the BAD view's scale, varying with azimuth, which
  is the size and the shape of what was measured. Reading through a table is
  necessary and not sufficient, and that binds anything that ever fills the
  `Table` uniform.

  **THE NEW BINDING RULE**: any change that applies something at the seam must
  include a **delivered-app-path comparison against `main`** in its acceptance,
  not only the unbent instruments. The A/B protocol is part of the battery and
  not only the owner's gate, and it has **two halves**
  (`~/kjerag-ab/delivered.sh`): a **difference** half, the app photographed at
  the view against the same binary run twice, which says whether two builds draw
  the same picture and cannot say which is better; and a **quality** half,
  `--bin step` and `--bin shear` with `seam=file` and the band live, which reads
  the seam itself. One control pair does not bound a spread, and a capture only
  counts if the fit landed before the shutter.

  **What ships.** The per-reading trim; `seam::along_kept`'s harvest guard,
  which refuses a sample whose own five terms compose to more than 1.2x the
  leftover they were fitted to (the July-25 flight, 170 deg of hole, reads 1.33
  and is refused outright at the app's plan); and the field stored dormant in
  `SeamSample::along_deg` against the one regime the delivered finding does not
  cover, the first frames of a session before the band has evidence. `Table`
  ships at `REST` as on `main` and the compute pass's read-through is removed
  with it. **The pool is not discarded**: what paid for that cost was the
  applied field. Measured against `main` in the delivered picture with an empty
  pool on both arms, the branch sits inside the same binary's own run-to-run
  spread at both May-01 views (2.275 codes mean against a 4.443 control at GOOD,
  7.343 against 6.740 at BAD) and outside it at the shimmer view (0.758 against
  0.105), where the trim moves that file's fit by +0.032 deg of roll, -0.079 of
  pitch and -1.31 px of `cy`. **And the trimmed fit is the better one in the delivered picture**, which the
  quality half settles: step at the seam -21.19 -> **-18.89 view px**, the
  band's own along-seam load 0.176 -> **0.159 deg** mean and 0.792 -> **0.498**
  worst, and `--bin shear`'s residuals smaller at all four bands with the
  steadiness unchanged, three runs each and deterministic; reproduced
  independently at the same aim from a different band state (no warm-up, 26 of
  128 directions against 47 to 48) at -21.97 -> -20.69 view px with the band's
  load 0.227 -> 0.199 deg mean. A cleaner pose leaves less step at the seam and
  less for the band to carry, and it does. **The claim is one camera, one flight
  (July-14), two views, two band states**: the two May-01 crossings cannot be
  read this way (line fits at 51 to 54 px rms) and the X2 view answers "no
  horizon fitted on both sides of the seam".

- 2026-08-06 **No along-seam table is fitted: above the five terms the pass
  already applies, what is left is not a static function of azimuth this corpus
  could have found** (issue #103, stage 9, docs/research/stage9.md). The
  mechanism is built, measured and ships at rest.

  Stage 9 asked for a fourth layer on the along-seam axis: one number per
  direction, pooled per camera, carrying what `SeamFit`'s five knobs and
  `band::Along`'s five harmonic terms cannot say. `kjerag-spike --bin table`
  measures the case for one, off the ring `seam::measure` already reads, and
  the case fails on every count that decides it.

  On the owner's X4 Air, six flights April to August, 299 gated readings: the
  pooled pose leaves **0.064 to 0.128 deg rms** along the seam per capture,
  which is the 1.30 and 1.43 view px `--bin crossing` reads at his two May-01
  crossings. A five-term fit takes the pooled leftover 0.0818 to **0.0739**;
  five more orders take it to 0.0712, which is **3.7 percent**. Pooled over all
  fifteen pairs of flights, the two captures' readings at the azimuths they
  share correlate at **+0.194** as they stand and **-0.014** once each flight's
  own five terms are taken off: all of the agreement between flights lives in
  the orders `band::Along` already applies, and nothing above them is shared at
  all. And the test that decides, each capture predicted by a table fitted on
  the other
  five: **a table costs the flight it was not fitted on at every width that
  resolves anything** (0.0836 to 0.0872 against a **0.0828** no-table baseline).
  The fitted column improves monotonically as the kernel narrows while the
  held-out column gets worse in step - the stage-7 striping lesson as a number.
  Swept past any width a per-azimuth field is interesting at, **the best any
  static table reaches on a capture it was not fitted on is +1.25 percent**, and
  that is the ceiling on what this corpus could ever have paid.

  **It is a refusal and not a blind spot.** A planted six-cycle table, an order
  above anything the pass applies, is put in the map and the corpus re-measured
  through it: read back at 0.894 and 0.910 of itself at 0.05 and 0.10 deg over
  107 azimuths, and through `--bin crossing table=` at the May-01 GOOD view at
  slope -1.07 and -1.26 per site with the **epipolar axis unmoved** (+0.006 and
  +0.023 src px, MAD 0.04 to 0.06) and the traced 50/50 contour unmoved. An
  order-6 field at half the size of the residual being looked for is plainly
  visible to this instrument. **And the refusal carries a size**: planting a
  static field of a known order and asking how much of its power comes back on
  a held-out capture, this corpus excludes order 3 and up above **0.02 to 0.06
  deg** of amplitude and says nothing below 0.02, which at the owner's May-01
  GOOD view is 0.37 view px against an along-seam error of 1.30. A static field
  of a few tenths of a pixel is not excluded; one large enough to be most of the
  defect is.

  The second camera says the same, **and it is the one place the answer depends
  on the gate.** The ONE X2 of issue #130, whose factory extrinsics are 2.8
  degrees out, has a leftover whose order-3-and-up structure does reproduce
  across three captures of one evening (0.0127 deg of azimuth structure against
  0.0116 of cross-capture scatter) - and held out it buys 2.7 percent at its
  best width (0.0692 against 0.0711) while a 4-degree kernel is already worse
  than nothing. With the along-seam plausibility gate **off** those same three
  captures support a table at **+10.0 percent** (0.2602 against 0.2890): what
  the gate removes is 12 to 14 readings per capture with a tail past two
  degrees, which is exactly what an ungated table would soak up, but the reader
  has to see that the sentence turns on it. **The X4 Air corpus, which is the
  one that decides, does not turn on it**: ungated it reads +0.03 percent
  (0.2985 against 0.2986), gated +1.25.

  **This corrects the #155 entry below.** "A static per-azimuth map is enough
  for along the seam" was read off the fact that the along-seam **median**
  reproduces across flights, 1.1 source px apart between May and April. It
  does. Its **per-azimuth structure above what the pass already applies** does
  not, and the median is what `band::Along`'s constant term is already for.

  **And the "does not reproduce" half of this entry is WITHDRAWN**, by the
  layer-2 preflight corpus run (`research/layer2-preflight`,
  `scratch/layer2/CORPUS.txt` and nine per-reading dumps under
  `scratch/layer2/corpus/`). It was a property of the estimator, not of the
  camera. `seam::measure` means over each azimuth's frames and the band's
  `off_epi` EMA does the same, over a population that moves 0.008 to 0.05 deg
  between frames by median absolute deviation and **0.22 to 0.48 by rms**.
  Reduced with `seam::left`'s own 4-MAD rule per reading, at full density, the
  same nine captures under the same pose and the same gate read **apart 0.0293
  against spread 0.0542 on all 15 X4 pairs and 3 of 3 X2 pairs** - where the
  mean managed 2 of 15. So "two flights disagree at one azimuth by more than
  either varies round the whole ring" and "the signal is under its own noise"
  are struck.

  **The table refusal survives, and one of its two cameras carries it.** Held
  out: **on the X2 a table costs 4 to 6 percent under every reduction** (mean
  +4.1, trimmed +5.6, median +5.2); **on the X4 the effect runs -1 to +2 percent
  depending on the estimator** (mean -0.1 and -0.6, median -0.6, trimmed +2.4, an
  independent trim +1.3), which is nothing either way. The kernel sweep is flat
  from 4 to 36 degrees on both in the table-alone arm. And what survives the five
  terms has an **amplitude of 0.004 to 0.005 deg** - the orthogonal part of the
  ladder's 0.0199 and 0.0195, not their difference - which is **0.13 to 0.16
  source px, an eighth of a pixel** and two to three times finer than `--bin
  crossing` resolves; removing all of it perfectly would improve the held-out
  residual by 1.8 to 4 percent, depending on which arm's residual it is measured
  against, and a fitted table does not get it. The
  certifying control is cross-capture, not within-session: the same test on the
  same partitions recovers the five-term field on 9 of 9 captures. So it is a
  refusal and not a blind spot. `Table::REST` ships.

  **What does reproduce is the five-term along-seam field, one harmonic order
  below where this stage looked.** Fitted on other flights only, held out on
  every capture of both cameras: X4 pooled leftover **0.0536 -> 0.0211 deg**
  (1.69 -> 0.66 source px), X2 **0.0606 -> 0.0249**, **9 of 9 improved**. That
  is the pose-order field pooled per camera, which `band::Along` computes per
  session and nothing yet carries between sessions. A pose refit on trimmed
  readings moves the pool materially (`cy` -11.91 -> -13.18, `pitch` -0.936 ->
  -1.096, per-capture leftovers 0.049-0.062 -> 0.028-0.039) but does not stack
  with it (held out 0.0208 against 0.0211): two removals of the same thing.

  **Why this stage's instrument could not see it, and the scope that follows.**
  The reproduction needs roughly ten readings per azimuth. `--bin table`'s 12
  places by 4 frames lands about two, and its `dump=` writes the ring after
  `seam::measure` has meaned it, so the artifact is baked into the recorded
  rows. Subsampling the peer's dumps to each depth and running this stage's own
  trim and gate: 12 moments reads apart 0.0938 against spread 0.0780 and 2 of 15
  pairs, 120 moments reads 0.0409 against 0.0531 and 15 of 15, all 1200 reads
  0.0254 against 0.0483. Everything this entry says about amplitude was measured
  at the thin end through the mean, so it bounds what a thin badly-reduced corpus
  could see and not what the camera has.

  **One consequence for the shipped code, for whichever stage takes it up**:
  `seam::measure` and the band's `off_epi` update average a population they
  should be filtering, and on the GPU that is one comparison against
  `held.off_epi` before the exponential average.

  What ships is `band::Table` at `Table::REST`: 128 numbers in the `Reframe`
  block, added to the band's own along-seam term before projection on the
  unwarped body ray, lens 1 whole and lens 0 not at all, tapering to exactly
  zero at any direction no reading reached. Empty, it is the picture
  `origin/main` draws, byte for byte at four registry views, and its cost does
  not measure: 0.04 ms of an 8.10 ms redraw on a quiet box, which is 0.24
  percent of a 16.6 ms frame, and a paired -1.66 to +0.72 ms interval under
  load. Nothing in the app sets one, and the loop that would is the open
  question at the top of the PR.

  **One process failure, recorded rather than tidied**: the charter's protocol
  says to freeze the kernel width before the hold-out partition is opened, and
  this stage swept it against the hold-out column instead. Nothing turns on it -
  every width is at or worse than no table, so the sweep chose nothing - but a
  corpus that had said yes would have needed the whole measurement retaken.

- 2026-08-05 **The handover is eight degrees wide, because the eye said so
  against every instrument that had an opinion** (`projection::CROSSOVER_DEG`
  2 -> 8, clamped per camera by `band::affordable`).

  The owner ran the two arms of `fade-ab.sh` label-blind on his own footage,
  arm 1 the shipped 2 degrees and arm 2 an 8 he was not told about, and said
  ***"2 is way better. Def not perfect but way better"*** of arm 2. Every
  number in this campaign says the opposite, and this entry is the record of
  both.

  **No instrument in the sweep can pick a width, and that is a measurement.**
  Five widths - 2, 4, 6, 8, 12 - at five owner reference views, on the four
  statistics that bear on the trade, and every one of them is **monotone** with
  no knee anywhere: sharpness over the overlap falls smoothly (0.686 / 0.657 /
  0.626 / 0.593 / 0.520 at the May-26 dirt view, and the same shape at all five
  views, `--bin seam mode=blend`), the doubled band grows (1.60 / 3.20 / 4.80 /
  6.40 / 9.60 degrees), the corridor's own step statistics get **worse**
  (0.0619 / 0.0673 / 0.0723 / 0.0773 / 0.0824 deg rms at the seam band,
  `--bin shear mode=probe`), and only the epipolar shear improves (disparity
  over width, so 1/width by construction). A monotone curve has no preferred
  point on it. **The instruments priced the trade; they were never able to
  choose on it, and one label-blind verdict did.** Recording that is the point:
  an agent that had waited for a number to justify 8 would have waited forever,
  and an agent that had read the numbers as a verdict would have shipped the
  arm the owner rejected.

  **Those first two rows are the instrument's own ramp and not the map's**
  (corrected 2026-08-06). `--bin seam mode=blend`'s `bands=` rows are a
  synthetic linear crossover the instrument builds itself (`Weighting::Band`),
  with the per-frame bend switched off, so its doubled band is `0.8 * width` by
  construction and grows exactly linearly. The shipped path is the same
  instrument's `shipped` row, which reads the map, so `KJERAG_HANDOVER_DEG` is
  what sweeps it. At the July-14 anchor moment (yaw 90, fov 60, the file's own
  fit), 2 / 4 / 6 / 8 / 12 asked for:

  | | 2 | 4 | 6 | 8 | 12 |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | doubled band, deg | 1.50 | 2.79 | 3.89 | **4.78** | 5.41 |
  | sharpness | 1.309 | 1.247 | 1.194 | **1.150** | 1.120 |

  So four times the width doubles **3.2 times** as much picture and not four
  times, the sharpness falls **12 percent** over that span and not 14, and the
  curve flattens above 8 because the ask is being clamped: this file affords
  9.69, so the 12 column is a 9.69. Monotone either way, which is what the
  paragraph above rests on.

  **Every row of that instrument is drawn with the per-frame bend off**, the
  `shipped` row included, and it is only the far field that makes that
  harmless: out there the bend is a fraction of a degree, so a weighting priced
  without it is the picture to within its own size. Near field it is not, and
  the near-field paragraph below uses `--bin band mode=render` instead.

  **What the sweep did settle is the other end.** 12 is refused by the optics
  on every camera in the corpus, and 8 is nearly the last width that is not.
  The handover reaches `width / 2` off the seam plus the whole bend it carries,
  and past the two lenses' shared ring it stops being a handover at all. Not by
  sampling off the end of a fisheye circle, which is what this entry said until
  2026-08-06: the coverage test is taken on the unbent ray and the bend then
  moves the sample, but a bent ray that lands outside a lens's boundary comes
  back `inside == false`, `projection::claim` returns zero for it, and the
  fragment shader reads a lens only where its weight is positive. What happens
  past the edge is that the **coverage depth** takes the weight over from the
  crossover's ramp and steps it to zero at the rim, so the picture is handed
  over by the optics instead of by the width that was chosen, and where both
  lenses miss it is transparent. The bound is conservative and it stays; the
  reason it stays is that.

  Measured with `--bin band` on the owner's own captures, re-measured
  2026-08-06: six X4 Air files overlap by **14.56 to 15.02** degrees and afford
  **9.36 to 9.82** (May-01 002 9.36, Jul-25 002 9.40, Aug-02 002 9.41, May-26
  004 9.48, Jul-14 006 9.69, Jul-25 001 9.82), the calibration fixture overlaps
  by 14.44 and affords 9.24, and the ONE X2 overlaps by **9.19** and affords
  **3.99**, under the 8 the picture asks for. The earlier "9.8 to 10.0" was one
  file's number read as a family's and was wrong on five of the six. So the
  width is the camera's now, not the picture's: `Reframe::crossover` is the ask
  clamped by `band::affordable`, it travels in the uniform block rather than
  being written into the shader source (the shader is compiled once, before any
  file is open), and the X2 hands over across 3.99 while the X4 Air hands over
  across 8. The margin inside the overlap on the fixture goes from 3.18 degrees
  a side to **0.62** (0.68 to 0.91 on the corpus files), which is the honest
  price of this and is why the bound is now asserted against the shipped width
  instead of against `WIDEST_DEG`
  (`the_widest_band_and_its_bend_stay_inside_the_overlap`). The X2 sits exactly
  on the bound, with 0.00 to spare, which is what "affords" means.

  **The width follows the calibration.** Every number above is under the file's
  own seam fit, which is what the pass draws with, and the factory calibration
  is a different answer: a fit moves the principal point, which moves each
  lens's coverage boundary, which moves the overlap. On the X2 the factory
  calibration affords 4.91 and its own pooled fit affords 3.99. So the drawn
  width is a reading and not a property of the file, and it is said twice
  rather than once: the app prints `blend:` under `seam:` at open, off whatever
  calibration has landed by then, and the render crate's own fit path prints
  `blend:  that fit moves the handover: 4.91 -> 3.99 deg` when a later fit
  moves it. The second line is not tidiness. On a camera with **nothing
  pooled** the first line is the FACTORY width, because the fallback fit lands
  a second after it: verified 2026-08-06 on the October X2 against an empty
  pool, which prints 4.91 at open and 4.91 -> 3.99 when the fit lands 1.2 s
  later. Before this branch there was no line at all, which is the state an A/B
  on the width must not be run in again.

  **Stage 4 is inert at this floor, and nothing it recovered is lost.** Its
  adaptive term opens the band to `|disparity| / FOLD` and cannot exceed 2.89
  degrees, so a floor of 8 is above every width it could ask for and
  `band::width` is a constant on every file in the corpus
  (`the_adaptive_width_is_inert_under_the_shipped_floor`). What stage 4 was for
  - a near-field reading being clamped by a band too narrow to carry it -
  cannot happen at 8 either: `carried` clamps at `FOLD * 8`, 7.2 degrees, and
  the search cannot report past 2.6. The mechanism stays because the floor is
  the camera's: a camera whose overlap forced it under 2.89 would put the
  reading back in charge.

  **Stage 4 did have work to do at 2, and the far-field views checked here are
  not where it did it** (corrected 2026-08-06). `--bin band` reports zero
  direction-frames over the floor at 8 on every stretch tried, and it reports
  zero at 2 on the same far-field stretches - but pointed at a stretch with the
  pilot's own gear on the seam it does not: at `KJERAG_HANDOVER_DEG=2` the
  May-01 file at `from=550` opens 2 direction-frames of 40 x 128 to 2.531 deg,
  recovering 8.0 view px of doubled edge on content at 0.84 m, and the May-26
  file at `from=30` opens to 2.144 deg on content at 0.99 m. Both are inside 8,
  which is why the claim above holds; what was wrong was the evidence offered
  for it.

  What is genuinely lost is stage 4's other half, that the band never opens
  further than it has to - **near-field content is now drawn twice across the
  same 8 degrees as the far field, and it pays more for it than the far field
  does**, where stage 4 would have given it at most 2.89.

  That is witnessed and not arithmetic. The pilot's harness, legs and machine
  sit at 0.8 to 1.5 m ON the seam in every corpus file, at phi 79 to 127, which
  is reached at yaw 90 or 270 and pitch -53 to -90 with the horizon lock off.
  The instrument is **`--bin band mode=render`**, whose `share` column is the
  seam band's own gradient energy over the same picture's 9 to 25 degrees off
  it, run twice per view under `KJERAG_HANDOVER_DEG`:

  ```sh
  KJERAG_HANDOVER_DEG=2 kjerag-spike --bin band -- <file.insv> mode=render \
    from=30.23 count=60 yaw=270 pitch=-53 lock=0 out=scratch/nf-w2
  ```

  | view, 2 -> 8 | share | fall |
  | --- | ---: | ---: |
  | May-26 004 gear, 0.99 m (`from=30.23 yaw=270 pitch=-53`) | 1.387 -> 1.182 | 14.8% |
  | May-01 001 under the pilot, 0.84 m (`from=550.15 yaw=90 pitch=-90`) | 0.725 -> 0.613 | 15.4% |
  | May-26 004 far field, same frame (`yaw=90 pitch=0`) | 0.695 -> 0.626 | 9.9% |
  | May-01 001 far field, same frame (`yaw=90 pitch=0`) | 0.341 -> 0.310 | 9.1% |

  So the near field pays about **one and a half times** what the far field
  pays, on the same instrument, the same frames and the same statistic. It has
  to be this instrument and not `--bin seam mode=blend`: that one draws every
  weighting with the per-frame bend OFF, including its `shipped` row, and the
  bend is exactly the near-field mechanism (the disparity under the pilot reads
  1.9 to 2.3 degrees there). Measured with the bend off, the same two views
  read a 12 percent fall and looked like the far field, which is how this
  paragraph first came to say "the same size". The `share` statistic's own
  window stops at 5 degrees while the handover reaches 6.6, so all four rows
  are floors under the effect rather than its size. Pictures in gitignored
  `scratch/near-field-witness/`.

  The along-seam findings the research arm came back with, which the width
  above rides on, follow.

  **There is only one support, and it is the crossover's.** The along-seam term
  goes to lens 1 whole and lens 0 takes none of it, and that is not a choice
  about width: it is the difference the fit measured, so it is what makes the
  two lenses draw one piece of content in one place. Wherever both lenses are
  in the picture that difference is pinned at one whole correction, so what the
  picture shows walks from none of it to all of it exactly as the weights do,
  and a ramp spread wider than the weights is a ramp that un-corrects the seam
  over the width it spread. Splitting the correction across both lenses near
  the seam, which was the other candidate, is that same un-correction written
  differently: it displaces lens 0's near-seam content by up to half the
  correction, which today is exactly zero, and leaves the crossover's own
  excursion where it was. So the knob is the crossover width, this entry is
  that knob with a name, and the width is what moved.

  **`mode=profile`'s 0.70 degree bracket is the instrument's readout and not
  the map's ramp.** What the picture carries of the along-seam correction at
  one distance from the seam is lens 1's weight, which the Rust twin reports
  with no correlation in the way: a smooth ramp over the whole crossover
  (`the_along_seam_correction_hands_over_across_the_whole_crossover`). The
  instrument reads a step because its held arm carries the two lenses' whole
  18.7 px disagreement as a double image across that same corridor: its match
  has two peaks and reports whichever leads. Doubling the map's ramp to 4
  degrees leaves the printed bracket at +24 to +60 px, exactly where 2 degrees
  put it; only 8 degrees moves it, to +12 to +72, which is 1.17 degrees of a
  ramp that is 8. **The bracket is a lower bound on the handover and not a
  measurement of it.**

  **And the ramp does not simply scale with the width.** The weights are
  cosines of the two lens axes and not a distance, so a wider corridor is a
  different slice of them: the walk from nine tenths of the correction to one
  tenth spends 0.75 of a 2-degree crossover, 0.70 of a 4, 0.65 of a 6, 0.61 of
  an 8 and 0.52 of a 12 (measured over 24 azimuths of the fixture). A handover
  four times as wide therefore spreads the correction over **3.2 times** as
  much picture, not four times, and the earlier 1/width arithmetic overstated
  what widening buys.

  What widening costs is measured and the trade has no free side. The plateau
  is unmoved at every width (0.3642 to 0.3657 deg), the null reads exactly zero
  at every probe on all 90 frames, and the plant reads its known displacements
  back inside 0.03 px, so the correction still corrects. What moves is the
  corridor: the step statistics above, and lens 0's floor, which stops being a
  floor - the band the correction never touched reads 0.0003 deg at 2 degrees
  and 0.0243 at 8, because the blend carries the correction that much further
  into lens 0's picture. Against all five reference views under the pooled
  answer, the along-seam median at the contour is unchanged with width at every
  view with the sites to say so (`--bin crossing`, `bins=180`); the 50/50
  contour itself moves about half a view pixel, because the depth term in the
  weights is not symmetric and the width scales it, and the sun view is refused
  outright on 5 to 6 accepted sites of 49.

- 2026-08-05 **What the band applies to a moving picture is an instrument now,
  and it says where its numbers came from** (`--bin shear`, issue #103's motion
  half). The shimmer campaign measured it out of tree, with two rendered frame
  directories and four Python scripts; this is the same method in one binary
  that renders both arms itself. A frame is decoded once and drawn twice
  through two `ScenePipeline`s, the delivered one and one held off by
  `hold_band` from its first frame, so the two pictures carry the same content
  by construction and what separates them is the applied field. Patches are
  placed against the seam's own row, walked onto out of the shipped map, because
  the seam sweeps 330 px down the picture over the reference window and a row
  pinned to the picture would be measuring that sweep.

  **Two modes, a null and a plant.** `mode=probe` reads four bands across the
  seam per frame with the step statistics under them; `mode=profile` walks a
  thin patch across it and brackets the handover; `null=1` holds both arms,
  which makes the two pictures one picture and every reading exactly zero; and
  `mode=plant` holds both arms and draws the second at a known yaw, so every
  band has a displacement it has to read back. Those last two are the only
  readings in the set whose right answer is known before the run, which is why
  there are two of them: 0.05 and 0.10 degrees of yaw are expected to displace
  -2.534 and -5.068 px, read back inside 0.029 px at every band, and double by
  1.9920 to 1.9963.

  **Against the reference view** (docs/research/reference-views.md, the shimmer
  line): 0.3663 deg applied inside lens 1 at 0.0047 deg step rms, 0.0619 deg
  step rms on the seam with a 0.42 deg single frame, and 0.0003 deg on lens 0's
  side, which is the floor. Those four are stated against a main and the
  registry line says which: the view is held by the horizon lock, so #158's
  reseeded orientation track moved the seam 23 to 45 px down this window, a
  mean of about 35, and took the first of them from 0.3641 to 0.3663 with no
  change to the instrument. The
  band's own state moved by 0.000002 across the same merge, which is the shape
  of the distinction: the band is fitted in the body's frame and these bands are
  read in the view's. It is reported beside them, at
  360 directions and through the shader's own `Reframe::reading_at` rather than
  a second lookup of ours, and it moves 0.0449 deg rms between frames.
  `research/freeze-dynamics` is an unmerged research branch; merged locally,
  its `KJERAG_FREEZE_DYNAMICS=0` takes that column to exactly 0.000000 while
  the bands still read the field the state is holding, which is the instrument
  telling a correction that stands still apart from one moving under the
  picture. What does **not** fall to zero under the freeze is the corridor's
  own step statistic, 0.07 and 0.12 deg rms with single frames at 0.38: the
  seam sweeps a standing field across the picture, so a band at a fixed
  distance from it reads a different part of that field every frame.

  **A step is between neighbouring frames and nothing else.** A band that drops
  readings has fewer steps than it has readings, and differencing across a gap
  reports the field's whole excursion over that gap as one frame's step, which
  on this view inflated the handover bands by 12 to 25 percent. The pair count,
  the breaks and the longest gap are printed beside every step statistic, and a
  band with fewer than twenty neighbouring pairs is refused rather than quoted.

  **Every CSV carries its source path and the whole command line that wrote
  it.** The instruments' tables outlive their terminals, and the older ones
  record no file identity at all, so a number copied out of one cannot be
  attributed to a video, a view or a calibration afterwards. That the
  calibration belongs in the stamp is not a guess: the same view fitted from
  the file reads 0.027 deg where the stored calibration reads 0.366, because
  what the band applies is what the calibration left it.

- 2026-08-05 **The seed is a mean of the opening minute, not a reading from
  inside it** (issue #152, docs/research/insv-format.md 8.8). The rule #45
  left behind tested the **magnitude** of one second of accelerometer and
  called that testing the reading, and a magnitude is nearly blind to the
  error that matters: a horizontal acceleration tilts the specific force by
  `e` and weighs it `1 / cos e`, so the whole 0.05 g of the full-trust window
  is spent by 18 degrees of tilt. The August 2 capture opens with a launch
  weighing 1.039 g at 21 degrees off vertical, which that rule believed
  completely, and the horizon stayed 17 to 21 degrees off level for over a
  minute; panning a circle swung it 40 degrees peak to peak. The seed is now
  the whole opening minute of accelerometer, every sample carried into the
  frame of the track's first sample by the gyroscope and averaged, with no
  test of any reading against anything. What makes a mean answerable where a
  reading is not is that it is bounded by flying: the mean specific force over
  a minute is gravity plus the aircraft's change in speed over that minute,
  0.025 g for a paramotor, against 3 degrees of gyroscope drift over the same
  minute. Measured against a backward pass over each file (the filter run from
  two minutes in back to the first sample with its rates negated, which moves
  0.02 to 0.83 degrees when its span goes from 120 to 240 seconds), the seed
  error falls on all six owner flights: 24.18 degrees to 3.03 on August 2,
  11.96 to 0.29 on May 26, 6.52 to 0.71 on May 1, 5.27 to 1.21 on April 10,
  3.89 to 1.48 on July 25, 3.06 to 2.66 on July 14, and on two of the three
  sibling files beside them (3.58 to 1.21 and 12.94 to 2.77, against 0.29 to
  0.56 on one that was already right). Through the render path the August 2
  capture opens at **3.33 degrees against 20.97**, and its first forty seconds
  average 3.75 against 18.86.

  **What selecting readings costs, as far as it is measured.** On that file,
  through the render path, the tilt grows with how hard the rule selects: 3.75
  degrees counting every sample, 6.73 weighting each second by
  `Filter::trust`, 9.32 keeping only the seconds inside the trust window,
  18.86 for the old rule's one chosen window. The backward pass does **not**
  agree about the middle of that ladder. In aggregate it prefers the
  trust-weighted mean, 1.61 degrees of worst case against 3.03; per flight the
  plain mean is the closer of the two on four of the six, by 0.04 to 0.09
  degrees, which is inside that instrument's own resolution, and the two the
  weighted mean wins it wins by 1.42 and 1.46. So what is settled is the
  render path's ordering on the reported file plus the shipped rule against
  the old one, which both instruments call better on every file. Picking the
  best window inside the minute is worse on both (a window can weigh 1 g by
  holding two things that are not gravity, which is what April 10 does).

  What it costs: a reading the running filter would refuse is no longer
  refused, only diluted to its share of the minute. And April 10 is not a
  clean win: its first frame improves 6.3 degrees while its 4 to 20 second
  stretch reads 3.5 degrees worse, and the two arms **do not converge inside
  the forty second walk**. The gap between their tilts is 1.53 degrees at 24
  seconds, first dips under a degree at 27.0, and is back at 1.16 by 38; the
  angle between the two measured verticals, which is the stricter reading, is
  4.25 degrees at 24 seconds, 3.47 at 38, and never below 3.28 anywhere in the
  walk. What is verified is that they are identical to three decimal places at
  240 seconds. Which instrument is lying over that stretch is unresolved.
  Nothing in the running correction changed; its gating under power is
  correct, and is why a bad seed survives so long.

- 2026-08-05 **The pool answers with a fit some capture actually took**
  (issue #103, docs/research/seam-two-axis.md 4). `SeamPool::answer` took the
  median of each knob separately, and the five knobs trade against each other
  inside one fit, so what shipped was a combination nobody had measured: roll
  and cx off one capture, yaw off a second, pitch and cy off a third. It
  answers with one of the pooled fits now, the one the rest of the pool agrees
  with most, scored as a sum of distances in probe steps
  (`seam::distance`, which was already the walk's yardstick). Re-read off the
  pixels of six of the owner's flights, at the three places in each file the
  app's own fit reads, that combination leaves **0.382 deg** along the seam on
  average where the fit now chosen leaves **0.273**, better on all six flights
  and on 15 of the 17 individual readings. In picture space, over every
  registry view (docs/research/reference-views.md) and both of `--bin step`'s
  windows, it is better on 15 of the 21 readings whose line fits describe their
  own points and worse on 6, the worst of those being 04-10 at 45.112 s, where
  the wide window's cold step goes 1.04 to 3.81 view px. A pool that is split
  evenly answers with the middle of what it is split between, which is the old
  rule's answer and is what a pool of two always is: no member of such a pool
  has the rest of it agreeing with it more, and choosing one would be choosing
  by which file was watched first. The pooling, the quality gate, the
  per-camera cache and the walk are untouched. Awaiting the owner's own test.

- 2026-08-05 **The seam can be measured where there is no horizon to measure**
  (`--bin crossing`, branch `research/crossing-instrument`). `step` needs a
  horizon and fits scenery at 51 to 86 px rms on the owner's 2026-05-01 views,
  so a seam-fix candidate could not be screened at the two crossings he
  actually looked at. This traces the pass's own 50/50 handover contour and,
  at fixed sites along it, registers the two **raw** lens pictures against
  each other on the seam's own axes. It reads the calibration's unbent
  geometry, so a reading carries no warm history.

  **It states its own floor, because the first version of it did not and was
  quoted to a precision it did not have.** Every run re-measures with the
  three angle knobs moved a thousandth of a degree each way and prints how far
  its medians travel, and how many sites each dithered run accepted: 0.00 view
  px over 36 and 36 sites on the null, 0.01 to 0.09 over equal counts on the
  four 2026-05-01 views. Nothing it says is worth more digits than that line,
  and a table taken at one `bins=` does not compare with one taken at another.

  The counts are on that line because a band is set two ways. Equal counts
  mean the dither moved the readings and the band measures that. Unequal
  counts mean it moved a site in or out of the accepted set, and then the band
  is a median stepping over a different population and is not a reproducible
  digit at all. The sun view under the pool member is the one recorded run
  like that, 12 sites against 13, and its band should be read as *at least*
  half a view pixel on the epipolar axis and not as a number. It is also an
  **angle** floor: the dither never moves `cx` or `cy`.

  That line exists because of the defect it now guards. The ported tracer
  kept, per azimuth bin, the root with the largest `min(blend.weights)`, on
  the argument that it was the one furthest inside both lenses. It is not:
  `Reframe::blend` normalizes the pair to sum 1, so at a 50/50 root that score
  is **exactly 0.5 at every candidate** and the twenty-odd candidates a bin
  holds were separated by the last bit of the bisection's `f32`. A
  ten-thousandth of a degree of calibration moved the reported medians by
  about a view pixel and a rerun of the same command did not reproduce its own
  table. A bin now keeps the root nearest its own centre, azimuth being what
  every comparison this instrument makes is keyed on, and three consecutive
  runs are identical to the last printed digit.

  **Null**: lens 0 against its own picture reads exactly `0.0000` on both axes
  at every accepted site, 36 of 37, 41 of 41 and 78 of 78 across the three
  views. **Plant**: a known calibration delta read back per site against what
  the perturbed map predicts, two sizes each - yaw 0.10/0.20 deg, roll
  0.10/0.20, cy 2/4 px. Median error 0.0006 to 0.0113 deg, which is 0.02 to
  0.36 source px, and it does not grow with the plant.

  **The owner's frame**, at `bins=180`, in view px at 1024 across. The two
  calibrations are his own pool of five resolved the two ways: the knobwise
  median that shipped until the entry above, and the member #154 now answers
  with.

  | crossing | calibration | epi | perp | accepted |
  | --- | --- | ---: | ---: | ---: |
  | the one he called good | knobwise median (was shipping) | 4.5 | 3.8 | 19/37 |
  | good | pool member (#154, ships now) | 6.1 | 1.2 | 19/37 |
  | the one he called bad | knobwise median | 12.0 | 3.3 | 38/41 |
  | bad | pool member (#154) | 10.1 | 1.4 | 38/42 |

  The two crossings differ 2.7x on the epipolar axis and are the same on the
  along-seam one. The change #154 landed cuts the along-seam error at both,
  3.8 to 1.2 and 3.3 to 1.4 view px, and moves the epipolar term by about a
  pixel, which is what "the bad one barely moved" looks like here. This is an
  independent read of that change: the entry above measured it as along-seam
  residual round the whole circle, and this measures it at the two crossings
  the owner actually looked at.

  **The bad crossing's excess is not parallax, and the first reading of this
  table said it was.** Epipolar is the axis a subject's distance displaces
  content along, so 12 view px was read off as content 2.4 m away. That
  inferred a distance from a magnitude without asking what the sites were
  looking at, and they were looking at a town, an estuary and a ridge line
  kilometres off, where a 33 mm baseline produces 0.015 view px. The excess is
  geometry.

  **Held at matched body azimuth over the whole 30-minute clip**, eight
  instants, every run's reference healthy (scatter 0.011 to 0.093 deg):

  | body azimuth | axis | across 30 min | swing |
  | --- | --- | --- | ---: |
  | +30 to +60 (the bad one) | epi | -14.5 to -13.5 view px | 0.99 px |
  | +30 to +60 | perp | -3.1 to -2.2 view px | 0.93 px |
  | -160 to -120 (the good one) | epi | -3.0 to -1.5 view px | 1.53 px |
  | -160 to -120 | perp | -4.8 to -3.7 view px | 1.11 px |

  The scenery behind those azimuths changes completely across those instants,
  from an industrial town and estuary to open farmland to clear sky, and the
  reading does not. The horizon lock is world-referenced, so a fixed view line
  looks at a **drifting** arc of the body-fixed seam; a per-time median taken
  without matching azimuth swings by 19 view px for that reason alone.

  **Three of those four rows are steady and one is not.** The good crossing's
  epipolar row swings 1.53 px on a signal of about 2, so at that crossing the
  epipolar term is **not established as constant** and nothing may be built on
  its being so. And the bad crossing's steadiness is one azimuth of one
  flight, not a property of the axis: the 2026-04-10 flight moves 7 source px
  on that same axis within itself at a different azimuth, and is the standing
  counterexample to reading that row round the whole circle.

  **Along the seam reproduces across flights and across the seam does not,
  and that is the split a residual map has to be designed around.** At matched
  body azimuth, over the runs whose reference stands and which have at least
  ten sites in the window, in source px: 2026-05-01 reads perp -10.7 to -8.8
  over 3 runs and 2026-04-10 reads -11.5 to -6.6 over 7, the medians 1.1 px
  apart, inside either flight's own spread. Epi over the same runs reads -11.6
  to -11.3 on 05-01 and -9.0 to -2.1 on 04-10, a 9 px gap between flights and
  a 7 px spread **within** the April one. A static per-azimuth map is enough
  for along the seam; across it needs a per-session channel and probably the
  per-frame one the band already is.

  What did **not** survive is the claim that the epipolar term shifted late in
  the May flight, from -9 to -3 source px. Every late-May run behind it is
  either reference-withheld or has three to six sites in the window; the three
  usable May runs are all early and all read -11.3 to -11.6. The shift was an
  artifact of contaminated and thin runs.

  **Perp is the honest broker, and it is a gate** (`perp-implausible`). No
  depth can reach the along-seam axis at any distance and a file's calibration
  does not change while it plays, so a reading far from its crossing's own
  along-seam value is a correlation that locked onto the wrong feature. It is
  refused whatever its agreement, and it catches what the agreement floor
  passes at 0.92.

  The reference is the crossing's own median over at least five readings, or
  one the caller declares, and it is **withheld** when the crossing's own
  scatter exceeds two fifths of the tolerance: sorted, 33 recorded runs scatter
  0.03 ... 0.35 then 0.53, 0.57, 1.03 of it, and the three past the gap are
  the ones whose middle meant nothing. It fires on the two runs that had put
  the honest core near refusal while keeping the junk.

  The tolerance is 0.40 deg, 12.6 source px, and it is a **chosen operating
  point in a populated continuum**, not a line between two populations: over
  750 accepted readings the departure from a crossing's own value runs p50
  1.85 px, p75 5.13, p90 22.13, and 11.9% sit in the 8-to-25 px stretch the
  cut is in. An earlier version of this entry called that stretch empty, which
  was false. What the data does say is that the choice barely matters: put the
  cut anywhere from 8 to 20 px and 4.0 to 5.7% of readings change side.

  **What the gate is not is validated.** The two-view control that was
  reported as validating it does not: `measure` builds its patches on
  body-fixed axes, so the view rotation cancels exactly and two views of one
  body direction agree to 0.0005 px whether the reading is any good or not.
  It validated the tracer's frame handling and nothing else. What stands
  behind the gate is the physical argument, a planted-site mechanism test, and
  one consequence it cannot have engineered: removing sites on the along-seam
  axis improves the **epipolar** axis's across-time reproducibility, a channel
  the gate never inspects (MAD 0.73 to 0.61 view px at one crossing, 0.65 to
  0.45 at the other). It cannot catch a mismatch that moved only across the
  seam, and the epipolar across-time range barely moves for that reason.

  **Glare refuses rather than guessing.** On the sun view, 78 sites trace and
  14 to 17 are accepted; the rest are `weak`, peaking at 0.11 to 0.48. The
  null run over the same 78 accepts every one at exactly zero, so the refusals
  are the two lenses genuinely disagreeing under flare rather than sites the
  sampler could not reach. Under the pooled fit that view's own scatter is too
  wide for a reference and the gate withholds one.

  **The search window is not a knob to widen.** At 2.60 deg the same frame
  reads a median magnitude of 18.7 source px with a spread of 19.8, because
  content two degrees away is allowed to win. A railed site is the honest
  answer. Patch, step and correlation floor are not like that: over their
  whole swept range the medians move under a pixel.
- 2026-08-01 **The descriptors describe the app, and the channel is named**
  (owner, from a screenshot of COSMIC Store). The `.flatpakref` carried
  plumbing keys only, so the page a Store draws before the remote is trusted
  had a placeholder icon, no summary and "Kjerag Developers" on it, and the
  repository summary carried no title, so an installed copy said its source
  was `kjerag-origin`. Both are filled in now from the metainfo by
  `scripts/pages-site.sh`, the repository calls itself `Kjerag (stable)`, and
  the ref file is named for the channel: `stable.flatpakref`. The half that
  was already right is the appstream branch, which had the icons, the
  screenshots and the developer name all along and was simply not reachable
  yet at the moment the owner was looking (docs/DISTRIBUTION.md 4.5).

  **And a dispatch republishes it without a tag**
  (`.github/workflows/site.yml`): the objects the last release published are
  fine, and rebuilding the app twice to fix a sentence is twenty minutes
  spent on nothing.

  **The summary is "360 video player"** and the keywords name cameras Kjerag
  refuses (GoPro, Osmo 360, Max) on purpose: somebody with one should find
  the app, meet the refusal that names their format, and send a clip toward
  support. The description says which cameras work today, mirroring the
  README's table, so that is an invitation rather than a bait.
- 2026-08-01 **The seam probe stops assuming the camera knows where its own
  lenses point** (issue #130, branch `fix/130-x2-fit`,
  docs/research/seam-two-axis.md 11). The owner's ONE X2 could never be
  calibrated: 3, 2 and 2 azimuths of 72 on his three captures against the 10 a
  five-knob fit needs, so that camera's pool stayed empty and zero-config
  playback delivered the factory calibration for good. Its two lens axes are
  recorded **2.835 degrees from opposed** where his X4 Air's are 0.308, and the
  seam reads 2.1 to 2.9 degrees along against a search window of 2.0.

  Two faults. The back patch was sampled as ONE rectangle grown by the whole
  search and refused entire if any corner left the picture, so a candidate near
  the truth was refused for where the widest candidate landed: 157 of 432 tries
  against 0 on the X4 Air, and widening the window made it **strictly worse**
  (at 3.0 by 6.0 degrees all 144 tries were refused and nothing reached the
  correlation). And the window was centred on the calibration itself, which on
  this camera is the thing that is wrong.

  The fix is one rule each and neither is a widening: a summed-area table of
  the holes makes the refusal a **candidate's** rather than the rectangle's,
  and a coarse wide pass acquires where the ring actually sits before the
  reading pass runs, along the seam only (parallax cannot reach that axis) and
  only where the offset is outside the window already searched. The three
  captures now fit 50, 42 and 65 azimuths and agree with each other to 0.06
  degrees of roll; round the ring at the owner's October reference moment the
  seam goes from 2.570 along and 2.830 across to **0.257 and 0.267**, which
  puts that camera in the same range as every other one in the corpus. Ten of
  the eleven two-lens captures on this box come back with the same fit to the
  last digit; the eleventh is the corpus X4, mildly starved too, whose fit
  improves on both axes when re-read off the pixels.

- 2026-08-01 **Kjerag has its own channel: a signed Flatpak repository at
  `kjerag.harding.dev`** (issue #137, owner). The same version tag that
  attaches two bundles to a GitHub Release now also builds, signs and
  publishes an OSTree repository on GitHub Pages, so installing is one click
  on `stable.flatpakref` and every release after it arrives
  through `flatpak update`. A bundle is a copy of a build; a remote is a
  subscription, and only one of those keeps a machine current.

  **This reverses 2026-07-31's "Flathub and nothing else"**, and not because
  the costs that ruling named were wrong. Issue #71 priced self-hosting
  correctly and the price is unchanged: no discovery, and key management and
  update delivery ours permanently. Flathub's contribution policy of
  2026-05-29 rules out this project's development process, so the route those
  costs bought is closed. The one thing issue #71 got wrong was the worst of
  it: it expected a remote with no AppStream data to be invisible in every
  software centre, and `flatpak build-update-repo` composes that data from the
  metainfo the app already ships. The listing exists; what it lacks is the
  screenshots (docs/DISTRIBUTION.md 5).

  **Nothing in it is ours.** Three published actions, the shape valent uses:
  `crazy-max/ghaction-import-gpg` for the key, `andyholmes/flatter` to build
  per arch and export an incrementally signed repository, and
  `JamesIves/github-pages-deploy-action` to push it to the Pages branch. The
  one step of shell writes the two descriptor files, and writes them rather
  than committing them because they carry the public half of the signing key:
  a committed copy is a file that names the wrong key the day the key rotates.

  **The branch is `stable`, in the repository and in the bundles alike.** It
  is the name a Flathub stable branch would have, so if that policy ever
  changes, moving is `flatpak install flathub dev.harding.Kjerag` and deleting
  our remote, with no reinstall and nothing lost. It also closes a smaller gap
  that was already there: a bundle install and a remote install are now the
  same app on the same branch, so `flatpak update` reaches a machine that
  started from a bundle.

  **The accepted cost is deltas.** flatter caches the repository in a GitHub
  Actions cache, and those are scoped to the ref that wrote them, so the two
  arch jobs of one tag share theirs and the next tag starts empty. Each
  release therefore publishes a repository holding that release alone: updates
  resolve and install correctly, and they download the app whole rather than a
  delta against the version already there. The app is 8 MB, which is why that
  is a note and not a problem.

- 2026-08-01 **Errors are the error** (owner ruling, on reading the funnel's
  own output). He watched the terminal say "trailer says lens frames are
  2880x2880 but the stream decodes 736x368" while the window said "That file
  could not be opened.", and ruled the raw message is what the pilot gets,
  everywhere, as a rule and not as a fix. So the alert's body is the
  failure's own message and the generic line is deleted rather than demoted.
  Nothing falls back to it, because there is no it: a failure nobody
  anticipated says what it says. Three lines of the app's own sit over an
  error and they are the whole list (`fail::refusal`): the format refusal
  (#107), the missing decoder (#69), the sandbox reach line (#118). Each
  names a fix the error does not know about, and each is one sentence away
  from being a mask, which is why the list is written down rather than left
  to judgement.

  **The stopped-video alert went the same way** (coordinator's call on the
  branch, applying the rule the branch had just written). Its body was "The
  picture could not be drawn, so playback stopped. Open the file again.",
  which knew less than the stall it stood over, so it did not qualify. The
  body is the stall's own line with the action on the end of it now: "61
  frames could not be imported over 2.0 s, last: Too many open files
  (os error 24). Open the file again." Added rather than substituted, which
  is the difference that makes a line of ours legitimate, and the only thing
  that half of it carries is the one fact the render layer cannot have, that
  this open is over. The terminal echo is unchanged, and so is everything
  about when the alert appears and what closing it does.

  **Two failures had no reason to show and now do.** A drop the document
  portal refused printed its answer to the terminal and put the generic line
  in the window; it carries the portal's own words now. A drop with nothing
  openable in it has no error at all to show, because libcosmic keeps only
  what converted (`dnd_destination.rs:119-120` calls `.ok()`, read at the
  pinned revision), so it says that instead of blaming a file nobody named.
  Nothing else in the shell was masking a reason; what the audit found
  besides was the opposite defect, failures with no surface at all (the file
  chooser's own errors, and issue #131's About links).

  **The engine's error strings are UI copy now.** They were always written at
  the failure site; what is new is that a person reads them, so the copy
  rules bind them and the tests do too. `kjerag-meta` checks every `Error`
  variant through a wildcard-free match, so a variant added later has to be
  looked at. The harness proves the words reach the screen rather than only
  the log: two files that fail for two different reasons must draw two
  different dialogs, which is a check that fails on the commit before this
  one.

- 2026-08-01 **The chooser hands back a document because the grant is read
  only, not because it is a chooser** (issue #123, measured, no fix yet). A
  file picked in `File > Open video` arrives as `/run/user/<uid>/doc/<id>/
  <name>`, a directory holding that one file, so a capture written as two
  files plays one lens. The reason is one permission wide. xdg-desktop-portal
  1.18.4 passes every picked file to the document portal with
  `AS_NEEDED_BY_APP` (`src/file-chooser.c`, `src/documents.c`), the document
  portal skips the store when `flatpak info --file-access` says the app
  already has the access being asked for, and the access being asked for
  includes write unless the backend answers `writable: false`.
  xdg-desktop-portal-cosmic never answers it (`FileChooserResult` has no such
  field) and xdg-desktop-portal-gtk answers it only when the pilot ticks "Open
  files read-only". So `xdg-videos:ro` says `read-only`, the request wanted
  write, and a document is registered.

  Both answers were produced, which is what makes the first one believable.
  `scripts/chooser-probe.py` makes the chooser's own `Documents.AddFull` call
  with each permission set: write asked returns a doc id for a file under
  `~/Videos` and for one on the file manager's network mount, read only
  returns an empty doc id for both, which is the real path. With the two
  grants temporarily made read-write through `flatpak override`, all four
  return the real path. `scripts/chooser-flatpak.sh` then drove a real dialog
  end to end: the whole portal stack runs a second time inside a headless cage
  session on its own D-Bus bus, so the backend draws its dialog there and
  nothing lands on the desktop, and the evidence is the portal's own
  `Request.Response` signal. Default: a document path, `not shown`. The same
  pick with the read-only box ticked: the real path, `sampling 2 of 2
  calibrated`, `2 lens streams from 2 files`.

  That harness needed one thing the repo did not have: `wtype` delivers
  character keys into a cage session and no named ones (Return, BackSpace, the
  arrows never arrive), so a dialog answered by a button could not be answered
  at all. `kjerag-spike --bin click` presses it with the same wlr virtual
  pointer `dragsource` uses.

  **The app cannot ask for less**, which was the owner's condition on widening
  anything. The finer-grained lever looked plausible: the portal's impl spec
  documents a `writable` option whose default is "no". It is documented under
  the **results the backend returns**, not the request the app makes, and four
  things say so, three of them measured:

  - ashpd 0.12.3's `OpenFileOptions` has no `writable` field, so our call site
    cannot express it at all;
  - xdg-desktop-portal 1.18.4 filters request options against an allow list
    (`open_file_options`) that does not contain it;
  - sending it by hand from inside the sandbox (`RAW=` in
    `scripts/chooser-flatpak.sh`, through `gdbus` in the bundle) is accepted
    with no error and **dropped in flight**. On the bus, the app's call carries
    `dict entry("writable", boolean false)` and what xdg-desktop-portal
    forwards to the backend is `array [ ]`. The dialog that opens has its
    read-only box unticked, which is the same fact in a picture;
  - and the GTK backend never reads such an option anyway. It only writes one,
    from a checkbox the pilot ticks by hand.

  So the write bit cannot be given up per request, and the owner's ruling is
  that it is not given up at all: **the grants stay read only and the manifest
  does not change**. The chooser keeps handing over a document, and what the
  app does about it is ask for what it cannot see. It takes more than one file
  now, the picked set is paired by name rather than by directory, a drop
  carries its set the same way, and a capture that arrived half says which of
  the two proper ways to open it the pilot has left: pick both files, or drag
  them in. Half is still played, because a pilot who asked for a file gets the
  file.

  Two facts are needed before any of that is said, and either alone is a lie.
  The capture is read as ONE lens, and the naming rule names a mate: an
  X4-class file names one too and carries both lenses in its container, so a
  name rule on its own would call every X4 capture half of one (measured: it
  stays silent). A readable folder with no mate in it earns the toast; a
  document directory earns the guidance instead, because it lists one file
  whatever is on the pilot's card.

  Composing the capture at the open was not the whole of it. The owner tested
  the branch bundle, picked both halves, and the log said both things at once:
  `2 lens streams from 2 files` from the player, and `this file carries one
  lens stream, so it has no seam` from the seam fit a line later. The fit
  reopened the capture from the picked path and looked **beside** it for the
  second lens, which is where a bare path's mate is and where a document's
  never is, so a capture opened the proper way could never be calibrated or
  harvested from. The fix is that a capture is its files: the reader hands
  back the ones it opened (`Reader::paths`), the scene keeps those rather than
  the one path it was named by, and the walk the fit reads takes them as given
  (`Walk::over`). The drop and command-line routes passed all along for the
  reason that hid this: two real paths sit beside each other, so looking
  beside worked by accident of the route rather than by anything the fit knew.
  The harness now drops both halves from a directory each, which is the
  chooser's shape without a chooser, and fails if a capture the app read as
  two files ever says it has one lens stream.

  The longer answer is issue #134, the owner's own: a folder-first shell in
  the cosmic-player idiom, where the app is given the directory and never has
  to ask. Not v1.

- 2026-08-01 **A transient import error costs a frame, and every failure the
  pilot meets goes through the alert** (issue #124, owner-reported at flatpak
  verification). One failed frame import set a flag on the pipeline for good:
  the picture was gone until the app was restarted, the sound played on over
  it, and the whole of what was said was one `eprintln!` on a terminal a
  launcher-started Flatpak sends nowhere. Measured on main's own binary under
  the headless harness, with `dup(2)` made to answer EMFILE on the app's main
  thread for 0.3 s: the picture froze for good, the clock ran on at 30.00 fps,
  and the null sink still carried the sound at 15745 of 32767. The flag is
  gone. A failed import costs that frame and the next redraw tries again; a
  run of failures that lasts two seconds stops the file, sound and all, and
  says so. The bound is time and not a frame count, because what the pilot is
  looking at is a picture that has been frozen for so long, and how many
  redraws went by inside that is a property of his display. Two seconds is the
  shell's own "long enough that a person has noticed", the same one the
  controls hide on. The run lives in the open capture's `Stalled` rather than
  on the pipeline, which iced keeps for the life of the window and which is
  why the old flag outlived every file.

  **The second half is the structure**, and it is the owner's ruling: error
  surfacing consistent by code design rather than by discipline. The alert's
  line is now private to `crates/app/src/fail.rs` and the only way to put one
  there is `Alert::raise(Failure)`, which prints the terminal echo with it, so
  a bare `eprintln!` at a failure site is strictly less than calling the
  funnel rather than an alternative to it. The engine has no way to report at
  all: a pass that gives up leaves a `Stall`, `Scene::pump` hands it out as a
  `Next::Stopped` arm every caller must match, and the shader widget will only
  give that arm to a message type implementing `From<Stall>`, so the shell
  cannot compile the video widget without a way to receive one. Adding the arm
  broke `kjerag-spike --bin playback`, which is the mechanism working: an
  instrument whose picture died was reporting a clean run.

  **A stop is final for that open** (owner ruling, on testing the branch with
  the fault left on permanently). The first shape re-armed as soon as the alert
  was closed and retried on its own, so a persistent fault meant an alert every
  two seconds: five of them in one sitting, from an app whose alert says to open
  the file again while quietly having another go behind it. Now the `Stalled`
  that gave up stays given up for the life of that capture. The pass stops
  importing into it and `Scene` hands out no player, so a play press cannot
  start the clock over a picture that is not coming back either. Reopening the
  file is a new `Scene`, a new `Stalled` and a fresh two seconds of patience,
  and it is what the alert asks for. Under the bound nothing changed: a hiccup
  still costs frames.

- 2026-08-01 **The volume popup closes on a press in the video, the way
  cosmic-player's dropdowns do** (issue #126, owner-reported). It was a
  hand-toggled bool that only the speaker button flipped, so the only way out
  of it was the button that opened it. cosmic-player's way out is
  `widget::mouse_area(video).on_press(Message::VideoAreaClick)`
  (`src/main.rs:1771-1773`), whose handler closes an open dropdown and
  otherwise plays or pauses (`1507-1513`); it also closes one on play/pause,
  on the scrubber and its release, and on fullscreen. All of that is ours now
  except the play/pause branch, which is the look-around grab here and was
  already resolved against it (docs/UI.md, conflict 1). Escape is unchanged,
  because cosmic-player's `on_escape` only leaves fullscreen.

  **A comment is what kept it out.** docs/UI.md said a press in the video
  "fires before a `mouse_area` around it could see it", which is not true and
  was checkable: the pass returns `ButtonPressed` uncaptured on purpose, and
  says in `crates/render/src/widget.rs` that capturing it would take the
  double click to fullscreen away. One line justifying a choice, read as
  settled by everyone after it (AGENTS.md, "comments record, they do not
  argue").

  **The harness grew a pointer**, because nothing in it could press anything:
  every check before this one is a key press. `wlrctl pointer` is the packaged
  tool for the job and cannot do it - cage advertises the seat's pointer
  capability only while a pointer device exists, and a one-shot client's
  device is gone before a client can bind `wl_pointer`. Measured 2026-08-01: a
  `wlrctl` wheel that should have zoomed the view did nothing, twenty in a row
  did nothing, and the same zoom off the keyboard reached the ball every time.
  `crates/spike/src/bin/pointer.rs` holds the device open for half a second
  before it moves anything, which is the whole of the difference, and the
  clicks land.

  **And a sound device**, because the speaker button is drawn disabled when
  the box has no output, and the session's own runtime directory has no
  PipeWire socket in it: every harness run until now said "playing silently",
  so the popup could not be opened there at all. The session now gets the
  desktop's socket, and the stream goes to the same null sink
  `scripts/quiet.sh` uses. Verified rather than assumed: the app's stream sits
  on `kjerag_quiet` while the harness runs, and `PIPEWIRE_NODE` is what puts
  it there, because pipewire-alsa is what plays what cpal writes.
- 2026-08-01 **The shipped Flatpak took no drops, and nothing could have
  caught it** (issue #118). A drop into a sandbox arrives as
  `application/vnd.portal.filetransfer`, which is a key the target exchanges
  with the document portal for paths it can open, and the app read only
  `text/uri-list`, whose paths belong to the source's filesystem and do not
  exist inside the sandbox. `dnd.rs` predicted exactly this in its own header
  and it shipped anyway, because there was no drop check anywhere: `wtype`
  presses keys and cannot drag. So the instrument came first
  (`kjerag-spike --bin dragsource`): a second Wayland client that performs a
  real drag with a virtual pointer and offers either shape. It found three
  things about the harness before it could find anything about the app, each
  of them something a desktop session has and a headless one does not.
  libcosmic creates the `wl_data_device` a drag is delivered over only while
  the seat has a **keyboard** (smithay-clipboard `src/state.rs:323-333`), and
  with none in the session neither this app nor cosmic-files ever asked for
  one. It reads the drop through the seat its last input event came from and
  gives up with "no events received on any seat" when there has been none, so
  a window nobody has clicked accepts a drop and then reads nothing from it.
  And wlroots drops on a button release only if the destination has already
  accepted, which is a round trip through another process, so a drag
  performed at machine speed is cancelled before the target has looked at it.
  With those three answered, the measurements: the dev build opens a
  `uri-list` drop and refuses a portal one; the released 0.1.1 bundle refuses
  the portal one, which is the owner's report, and reads a `uri-list` one and
  then cannot open the path it names ("No such file or directory" for a file
  that plainly exists). The fix is libcosmic's own two calls,
  `on_file_transfer` and `command::file_transfer_receive`, and it needs no
  permission at all: the portal exchange is what a sandbox is for.

  What the portal arm cannot fix is a source that never registers the files.
  cosmic-files 1.5.0 offers `text/uri-list` and nothing else (its source, and
  the shipped binary, which does not contain the string `vnd.portal`), so a
  drag out of the COSMIC file manager hands over a path on the host and that
  half of the exchange is the source's. It is also the owner's own workflow,
  so on his ruling the manifest grants `--filesystem=xdg-videos:ro`: the
  smallest grant that covers every way a bare path arrives, which is that
  drag, `kjerag ~/Videos/flight.insv` on a terminal, and a double click.
  Read-only because the player never writes to footage, and the videos folder
  rather than home because footage kept elsewhere is one `File > Open
  video...` away through the portal at no permission at all. The day
  cosmic-files registers its drags the grant can be reconsidered.

  What is left outside the grant used to get the words a corrupt file gets,
  and now gets its own line in issue #117's alert, only inside a Flatpak:
  "Kjerag cannot reach that file from inside its sandbox. Open it with File >
  Open video." The path decides it rather than the error, because libav's
  answer for a path with no mount behind it is "No such file or directory",
  which is a sentence about a file that is not there.

  Measured on the bundle built from the branch, with the grant, on real
  footage under `~/Videos`: a cosmic-files-shaped `uri-list` drop opens, a
  portal drop opens, `flatpak run <app> <path>` opens, and a double click's
  own `--file-forwarding` shape opens; the same `uri-list` drop of a path
  outside the folder is refused with the line above. And for the two-file
  captures of issue #123, every one of those four hands over the host path
  and pairs both lenses (`2 of 2 calibrated`, `2 lens streams from 2 files`),
  because both the document portal's `RetrieveFiles` and flatpak's file
  forwarding skip the document store for a file the app can already read.
  The file chooser is one permission short of skipping it too, which the
  2026-08-01 entry above measures: a file picked there arrives as
  `/run/user/1000/doc/<id>/<name>`, its mate is not in that directory, and
  the capture still plays one lens. Issue #123 stands for that path alone.

  And the grant was still not enough, which the owner found by testing rather
  than by reading: his footage library is on a NAS, mounted by the file
  manager, so the paths his drags carry are
  `/run/user/1000/gvfs/smb-share:server=...,share=.../...`, which a sandbox
  cannot see either. `--filesystem=xdg-run/gvfs:ro` is the standard grant and
  the read-only half is measured: the mount lists inside the sandbox and a
  file on it reads. Measured through a drop, on his own share: the file
  opens, and because the mate sits beside it there too, a two-file capture on
  the NAS plays `2 of 2 calibrated`, `2 lens streams from 2 files` over SMB.
  The instrument had to learn the shape as well: a mount's directory is
  called `smb-share:server=host,share=name`, and an argument parser that
  treats `=` as an option refuses to drag it.

  What is left outside every grant refuses once per drop rather than once per
  file, which is what a multiple selection sends: the app takes the first file
  and says one thing about it (measured: two files in, one refusal out).

- 2026-08-01 **A pane with no frame draws the backdrop, not a picture of its
  own** (owner-reported). The pass carried an animated test pattern from its
  first bring-up, a sine of the distance from the middle of the view on a wall
  clock, and drew it wherever the uniform block said there was no frame: every
  open from a window that was already up showed it until the first decoded
  frame landed, and any state where frames never arrive showed it for good. It
  is gone, and with it the clock that animated it, the two uniform fields that
  carried that clock and the flag beside it, and the redraw the widget asked
  for on every compositor refresh while nothing was open. What is left is the
  mechanism the room around the ball already uses (issue #100): no frame is no
  lens with a ray, which is transparent everywhere, which is the shell's
  backdrop - libcosmic's pane in a window, black in fullscreen. Opening a file
  is now a pane that is already there and a picture that arrives on it.

  **The harness had never looked there**, and the check that does is the
  interesting half. The command line cannot reach the state at all:
  `kjerag file.insv` opens the file while the window is still being mapped, so
  the decode thread has the whole mapping to work in and the first frame is
  there before the first pixel is drawn (measured over 80 captures from
  launch, none of them in the gap). A paste over `Ctrl+V` opens the file from
  a window that is already up, which is the pilot's own path and the only one
  a keyboard has - `Ctrl+O` opens a portal dialog cage has no portal behind -
  and naming a time 90% into the file widens the gap from one capture to
  twenty, because the first frame then comes off a keyframe walk rather than
  off the head of the stream. The pattern is told from a picture by its own
  symmetry: it scales into red by the horizontal place and into green by the
  vertical one over a blue that is neither, so two patches at mirrored places
  read the same green and the same blue to the byte and different reds, which
  no frame of video does. Against the build this branch started from the check
  fails on 41 98 211 against 170 98 211; with the pattern gone it passes on
  the backdrop, five captures of it before the frame.

- 2026-08-01 **Another camera's 360 format is refused by name, and nothing
  in the app grades the camera it does take** (issue #107, alongside #88).
  Two halves of the same honesty. The refusal: a GoPro `.360` and a DJI
  `.osv` are ordinary MP4s, so before this they opened, failed the trailer
  read, and got the line a corrupt file gets. `kjerag_meta::Format::sniff`
  now names the maker off the container before the decoder is asked for
  anything, in this order: the Insta360 trailer magic, GoPro's own `udta`
  boxes (`FIRM` `GPMF` `CAME` `MUID`), DJI's `djmd`/`dbgi` tracks, and
  Google's spherical metadata for a stitched MP4. Only where the bytes say
  nothing is the name asked, so a `.360` off a firmware that writes the
  container differently is still named, and no `.insv` can be refused for
  what it is called. The search walks the box tree instead of the bytes,
  because a raw grep for `st3d` over the sample corpus hits two genuine
  Insta360 captures inside their compressed video, and refusing a pilot's
  own footage is the one failure this must not have. Verified on 18 real
  files from seven cameras; the spherical arm alone has hand-built fixtures
  only, because no such file exists here and ffmpeg 7.1 cannot write one.
  **What says it is an alert, not the welcome view** (owner, 2026-08-01, on
  the first cut: "a normal alert, not some weird bespoke string on the splash
  page"). Every cannot-open line moved with it, the missing-decoder one of
  issue #69 included: `Application::dialog` returns the stock
  `widget::dialog` shaped the way cosmic-files shapes its failed-operation
  dialog (`src/app.rs:5665-5678`), one title, the reason as the body, the
  `dialog-error` icon, and one button, dismissed by that button or by Escape.
  A failed open now takes nothing away either: whatever was playing carries
  on playing behind the alert, where before the shell dropped the open file
  to show a line on the welcome view.
  The other half is what was **not** built: an `.insv` from a camera outside
  the verified set opens and plays with nothing said about it. The support
  tiers are the README's and the listing's, where somebody deciding whether
  to install reads them; in the app a tier is a label the pilot cannot act
  on, it would fire on files that play perfectly, and it would go stale the
  day #88 verifies a model. What the app says about a camera stays what it
  already said: the `lens:` line naming model and firmware on the terminal.
- 2026-08-01 **aarch64 is a second runner, not a second recipe** (owner
  directive). The gates that compile run on `ubuntu-24.04` and
  `ubuntu-24.04-arm`, natively, and a version tag publishes
  `kjerag-<version>-aarch64.flatpak` beside the x86_64 bundle, each built on a
  runner of its own arch. What decided the shape was the ffmpeg PPA: the
  workspace pins 7.1, Ubuntu 24.04 ships 6.1, and a PPA is often amd64 only,
  so the question was put to the arm runner before anything was written.
  `ppa:ubuntuhandbook1/ffmpeg7` publishes an arm64 index, `libavcodec-dev
  7:7.1.1-0build1~ubuntu2404` installs out of it, and `pkg-config` then reports
  61.19.101. So the provisioning block is untouched and both arches read the
  one copy of it. That left the alternative unbuilt, which was moving the
  compile jobs into the Flathub SDK container the bundle already builds in: it
  does carry ffmpeg 7.1 dev for both arches (measured), but it has neither
  rustup nor clang, and taking it would have traded two cached toolchain jobs
  for a container pull to solve a problem the PPA does not have. The release
  half was checked the same way rather than assumed: the
  `flatpak-github-actions:freedesktop-25.08` tag starts on the arm runner and
  reports `aarch64`, and `org.freedesktop.Platform`, `Sdk`, `rust-stable` and
  `llvm21` all resolve at 25.08 for `--arch=aarch64` on Flathub. One trap is
  recorded in the workflow, because it fails as a cross build rather than as a
  missing thing: the flatpak-builder action's `arch` input defaults to the
  literal `x86_64`, so the arm runner has to be told. Measured on the throwaway
  tag that proved it (`0.1.1-armtest1`, published and then deleted): a 7.2 MB
  aarch64 bundle carrying `app/dev.harding.Kjerag/aarch64/master` and
  `runtime=org.freedesktop.Platform/aarch64/25.08`, beside the 8.1 MB x86_64
  one. The two bundle jobs run side by side and the arm one finished first,
  7m22 against 9m10, so the second bundle costs no wall time of its own; the
  tag run as a whole went from 11m31 to 14m43, and that difference is one cold
  arm cargo cache in the gates. Warm, the two legs land together: on the second
  push to the branch the arm gate took 1m33 and the x86 one 1m52. What this is
  not, and the README and
  docs/RELEASING.md say so where a person reads them, is a verified build: a
  runner has no GPU, decode is VA-API against `/dev/dri/renderD128`, and most
  arm devices decode through V4L2, which this app does not use. The aarch64
  bundle is compiled and unit tested and has been run by nobody.
- 2026-08-01 **A version tag is the release, and nothing about it is ours**
  (issue #106). `cargo release patch --execute` on main bumps the version,
  stamps a dated entry into the metainfo changelog, tags the plain version
  with no `v` in front of it, and pushes; the tag makes a workflow build the
  Flatpak and publish `kjerag-<version>-x86_64.flatpak` and its `.sha256` as a
  GitHub Release. The owner's rule for the whole path was that Kjerag is the
  simplest project Flathub will ever see and its releases should be too, so
  every piece is either an upstream tool used as its documentation intends or
  it is gone. cargo-release owns the version, the changelog entry, the commit
  and the tag; Flatpak's own GitHub action owns the build, in the image
  Flathub builds with, which took a hand-written `apt-get` block and its two
  archaeology comments (`eu-strip`, gdk-pixbuf's SVG loader) out of the tree
  the day it arrived; softprops/action-gh-release and GitHub's own generated
  notes own the release. What that left of our own is four lines: a tag
  pattern, a bundle name, a `sha256sum`, and `release.toml`. An earlier
  version of this work had a `scripts/version-check.sh` holding three copies
  of the version in agreement, and it was deleted rather than kept: with one
  tool writing all three from one number, the thing it verified cannot
  disagree, and cargo-release's own `exactly = 1` on the changelog stamp is
  the guard that the metainfo was really written. The release workflow calls
  `ci.yml` rather than restating it, so a tag runs the gates a pull request
  runs, and `scripts/uitest.sh` is in neither: a runner has no
  `/dev/dri/renderD128`, so it is a cargo-release hook, which means the dry
  run that precedes every release is also the harness run. Measured on the
  pipeline's own test tags: about ten minutes end to end, an 8.1 MB bundle,
  and `flatpak install --user` of it followed by `flatpak run
  dev.harding.Kjerag --version` printing `kjerag 0.1.0`. The channel question
  is not reopened by any of this: Flathub is still where a published app goes
  (docs/DISTRIBUTION.md 4.1), and a single-file bundle is a file rather than a
  channel.
- 2026-08-01 **The room around the ball belongs to the window** (issue #100).
  The pass wrote a flat 0.10 grey wherever no lens has a ray; it now writes
  transparent black through a premultiplied blend and paints nothing there at
  all, so what fills the room is the one layer behind the video
  (`app::backdrop`). In a window that layer is empty, which leaves libcosmic's
  own pane showing: darkened translucency over the compositor's blur with a
  frosted theme, the same colour opaque without one, and no fallback of our
  own for the no-blur case, because it is the same line of libcosmic either
  way. In fullscreen the layer is black, on the owner's call: there is no
  desktop behind a fullscreen window to frost. A still is black too, for a
  different reason: the capture pass clears black and the transparent room
  flattens onto that, so a JPEG carries no alpha and needs no channel for one.

  **Nothing with a picture in it moved.** Measured over `reframe` renders from
  both builds, three fields of view (40, 90, 150) in both target formats:
  byte-identical PNGs. At the ball every differing pixel is the room and every
  one of them the same substitution, 25 25 25 to 0 0 0 on a linear target and
  26 26 26 to 0 0 0 on an sRGB one (193,084 of 262,144 pixels, 73.7%). The
  blend's cost is under this box's noise: the median ms/redraw over six
  interleaved `ball` runs a side moves between -0.03 and +0.15 ms at 2560x1440
  on a Radeon 760M, against a spread of 0.4 ms between runs of one build and a
  33 ms frame.

- 2026-08-01 **The project is Kjerag** (issue #75), in one mechanical sweep:
  five crates, the binary, `App::APP_ID`, the four `resources/` and
  `flatpak/` file names, the cosmic-config identifiers, the report prefixes,
  the harness and every doc. The previous spelling is gone rather than
  aliased (owner: "doesn't exist in any files or filenames, folders,
  anything"), so no compatibility name is read anywhere and
  `KJERAG_TEST_MEDIA`, `KJERAG_BIN`, `KJERAG_TEST_INSV` and `KJERAG_FFMPEG7`
  are the only names the scripts answer to. `scripts/name-check.sh` is the
  lock, and CI runs it: a tracked path or file carrying the old name fails
  the build. The transcripts in docs/DISTRIBUTION.md had their identifiers
  rewritten with the rest, and that document's preamble says so, because a
  transcript nobody re-ran is evidence for what it measured rather than for
  what it prints.

  **cosmic-config moves with the ID and nothing migrates.** The stores live
  under `~/.config/cosmic/<id>/` and `~/.local/state/cosmic/<id>/` and
  cosmic-config has no name-migration path, so settings, recent files and the
  seam pool are all discarded. Pre-release, and sanctioned in #75. The pool is
  a cache by construction (see the entry below), so watch-to-calibrate refills
  it silently over the next few files played; the settings are four values and
  the recents are ten paths.

  The icons kept their bytes. The name mismatch that issue #93 worked around
  is gone and an installed build resolves `dev.harding.Kjerag` through the
  icon theme, but a `cargo run` out of this tree installs no theme, and
  libcosmic answers a miss with an empty SVG rather than a placeholder. All
  three cases were measured with the harness before deciding
  (`crates/app/src/app.rs`, `APP_ICON`).

- 2026-08-01 **The calibration menu action is deleted, and the store with it.**
  Zero-config playback (AGENTS.md) leaves no room for it, and the reason is
  stronger than the doctrine: the action fitted whichever file was open, so on
  this box it stored the May 1 flight's fit and then the April 10 flight's,
  reported "Seam calibrated for this camera" both times, and never once fitted
  the static capture it existed for. A fit taken through a flight's seam
  absorbs that flight's parallax (6.8), so both answers were wrong and nothing
  on screen could show it. That is the whole explanation of the owner's "I have
  never once seen a before and after that improved". The single-entry
  `seam_calibration` is replaced by a per-camera `seam_pool` of quality-gated
  fits, medianed, filled by watching; the old key is discarded rather than
  migrated, because its contents are exactly the contamination the pool exists
  to average out. The correction is no longer landed once either: a
  `Correction` walks from what is drawn to what is asked for at 0.25 deg/s, so
  a fit that lands mid-playback is never a jump.

- 2026-07-31 **Distribution settled** (docs/DISTRIBUTION.md). A `.insv` gets a
  MIME type of its own, `video/x-insta360-insv`, glob only: the bytes that
  identify one are the last 32 and shared-mime-info offsets are
  start-relative, so the good magic rule is unreachable and the near miss
  makes `gio` segfault. The desktop entry, the metainfo, the MIME package and
  the icon theme tree install out of one `resources/` root, and the Flatpak
  builds offline from the committed `flatpak/cargo-sources.json`. **The
  channel is Flathub and nothing else** (owner): a self-hosted repository was
  worked out in full and declined (issue #71), and Flathub is reached under
  AGENTS.md's one scoped exception, owner-coordinated, previewed here before
  any outward step. The licence spelling is `AGPL-3.0-only`. The app ID is
  `dev.harding.Kjerag` (issue #66) and the whole tree carries it since issue
  #75: the ID is the cosmic-config path, the icon name, the D-Bus name, the
  Wayland `app_id` and four file names at once, so the sweep moved all of
  them in one commit rather than leave the entry naming one ID and the binary
  registering another. What is left before a submission is the owner's:
  screenshots, the X11 question, and whether `xdg-config/cosmic:ro` costs
  persisted settings.

- 2026-07-31 **`flatpak/cargo-sources.json` was stale on `main`**, and the
  rule that should have prevented it could not: it is written per commit and
  the failure is per merge. Issue #90 regenerated the sources on a branch cut
  before issue #95 bumped the ffmpeg pin; both merged clean because they touch
  different files, and `main` then held a lock file wanting ffmpeg-next 7.1
  and a source list offering 6.1.1. Found by building the Flatpak rather than
  by reading it, which is the only way it can be found. Fixed by
  regenerating, and `scripts/cargo-sources.sh --check` now compares the two
  package sets with no network and no generator, in CI and by hand. The check
  was shown able to fail before it was believed: against the stale file it
  names all four crates, ffmpeg-next 7.1.0 among them.

- 2026-08-01 **The seam's photometry: the measurement layer survives, the
  application does not** (issue #103, stage 8 final,
  docs/research/seam-blending.md 16). The owner tested the applied correction
  twice and rejected it twice, the second time on dark STREAKS across his soil:
  *"I don't think this approach is valid."* PR #138 ends as measurement
  infrastructure - the shipped crates are main's byte for byte, and the whole
  branch is one instrument file and the record.

  **The process finding is the durable part.** Every acceptance statistic this
  campaign has ever used STRADDLES THE SEAM. Stage 8 found the statistic was in
  the wrong units and fixed that; the replacement straddled the seam too. So
  nothing ever measured what an applied correction does to the picture it is
  painted OVER, and two builds were rejected on an artifact class the whole
  acceptance layer was structurally unable to see - a per-direction field over
  wide support painting each direction's own noise along its whole sweep, which
  is stage 5's scalloping on the photometric axis. **The rule: a field applied
  over an area is accepted on the area, not on the boundary.**

  **What ships is three instruments and their plants.** A perceptual lag ladder
  in Weber contrast at 1 to 128 pixels of the delivered view (a planted step
  reads back exactly at every lag; the same step spread over 64 pixels reads a
  64th of it locally). An excess-over-the-same-content statistic that names a
  line's author. And the field-interior coherence metric that was missing, which
  reads main at 0.03 percent, the rejected build at 1.01, and its own nulls at
  0.000. It is registered as the anti-acceptance for photometric work.

  **And one finding no rejection touches:** at every reference view the owner
  has given, the residual line's excess over what the same content reads a few
  degrees away is at or under the JND at the one and two pixel lags, while at
  the azimuth his own gear crosses the seam it is +5.87 percent and the entire
  photometric stage moved it from 5.94 to 5.94. **What still reads as a line is
  geometric**, which makes the local-warp-versus-pose verdict the campaign's
  next question.

- 2026-08-01 **Symmetric wide matching, and the measurement that names the
  line's author** (issue #103, stage 8 second form,
  docs/research/seam-blending.md 14-15). The owner viewed stage 8's first form
  twice. *"I dont think its aggressive enough with blending"*, and then *"to the
  eye, it still effectively looks like a line"*. Both are answered, and only one
  of them by building something.

  **The correction is split between both hemispheres and carried to the pole.**
  The first form eased it to nothing seven degrees off the seam, to keep a
  player from moving a hemisphere's black level; the symmetric split dissolves
  that objection the way stage 3's gain split did, since each hemisphere moves
  HALF the mismatch towards the other. At the owner's own wide view the
  difference between eight degrees either side of the seam goes **7.55 codes to
  2.93**, and the halo that was the priced risk of going wide did not appear:
  the long-lag Weber contrast goes 44.7 percent to 24.4 at 64 pixels. A count of
  pixels of the delivered view came out with the old shape - it decided nothing
  at any field of view the player offers, and where it bit it made the handover
  narrower than the content would bear.

  **The line that is left is GEOMETRIC, and that is measured.** The same
  statistic straddling a line a few degrees off the seam, in the same window and
  the same content, separates a photometric step (a difference in level, present
  on content with no gradient at all) from a misregistration (a difference in
  position, present only where there is content to draw twice). At every
  reference view the owner has given, the seam's excess over what that content
  reads anywhere is **at or under the 1 percent JND at the one and two pixel
  lags**; at the azimuth his own gear crosses the seam it is **+5.87 percent and
  the entire photometric stage moves it from 5.94 to 5.94**. So the photometric
  half of "no line" is done to the bar and the geometric half is the
  local-warp-versus-pose decision already pending. No local warp is built here.

- 2026-08-01 **The seam's blend, in the space an eye reads it in** (issue #103,
  stage 8, docs/research/seam-blending.md 9-13). The owner viewed stage 7's
  branch at a wide May reference view and said *"we need to do a lot better
  with blending"*, and the verdict written for him found three reasons no
  amount of tuning could: the correction was multiplicative where the
  difference is additive, the estimator weighted brightness squared so the
  content the artifact shows on carried one percent of the weight, and the
  loss was in codes while the eye reads ratios. **All three are one mistake -
  the metric - and stage 8 changes the metric first.**

  Acceptance is now the steepest local **Weber contrast across the seam**, at
  lags of 1 to 32 pixels of the delivered view, with controls that read a
  planted step back exactly and separate a step from a ramp of the same size
  to three decimals. At the owner's own wide view it goes **42.3 percent to
  16.6**, and the step the two sides differ by goes **+32.5 percent to -2.1**.
  On flat content it is at the one percent just-noticeable difference at the
  one and two pixel lags, which is where an edge lives; what is left past four
  pixels is a ramp the correction itself makes, because a player may not move
  a hemisphere's black level and the correction has to end somewhere.

  Five moves, each from a measurement: **ratio space** (the codes-space
  estimator deleted, not switched off); **a gain and an offset fitted
  jointly**, because sequentially the gain comes out at 1.15 and ruins the sky;
  **the offset per direction**, because a constant plus one cycle plus two -
  the basis stage 7 fitted through - leaves 4.2 to 5.5 codes rms against a
  frame noise of 0.8 to 1.0, so what varies round a seam is not a shape;
  **one width** in pixels of the delivered view, gated per direction by what a
  wider handover would cost in ghost, absorbing stage 4's crossover and stage
  7's colour region; and **a handover profile with no corner** plus dither
  inside it, which is the residual physics a photometry cannot reach. One hole
  was most of the improvement: stage 7 read a photometry only where the
  correlation had established what it was looking at, and that left 50 of 128
  directions blank in a continuous arc - the arc the complaint was in.

  **The debt went down.** `band::Tint` is deleted whole - its fit, its shader
  twin, its compute entry point, its pipeline and its readback - and three
  notions of "near the seam" became one function. Three new constants, one new
  derived one, one whole mechanism gone. The photometry costs +0.28 ms per
  redraw, less than stage 7's +0.38 for more work.

- 2026-08-01 **The seam hands over a colour, and one number could never have
  reached it** (issue #103, stage 7,
  docs/research/insv-format.md 6.11). The owner's verdict on the merged
  geometry work: *"the worst part now is the change in colour at the seam,
  especially on the sky or when the sun is in one of the lenses."* Stage 3
  corrects one gain for all three channels, so the SPREAD between the channels
  survives it exactly however well it is fitted, and that spread is 3.4 to 5.6
  codes on the owner's own six captures and 1.5 to 31 across four camera
  models - over the one code an 8-bit picture can carry on every one. On a
  corpus X4 it is **10.29 codes with the sun in one lens and 0.47 with the sun
  in neither**, which is the owner's own sentence measured in somebody else's
  footage. So `Tone` carries three gains in the same sixteen bytes.

  Two findings changed the design. **The pass had never read the content the
  complaint is about**: the band refuses a patch under its contrast gate, so
  20 to 64 percent of a real seam - the sky - carried no reading at all, and
  the gain was measured on the ground and applied to the sky. A flat patch has
  no geometry and the best colour on the ring, because what a displaced window
  costs a photometry is the content's own gradient across it: measured at 0.33
  to 0.76 codes rms at the residual the pass leaves, against differences of 2
  to 33. And **the difference is not one number round the ring**: a
  per-channel constant leaves 1.0 to 6.2 codes rms on the owner's captures
  against a frame-noise floor of 0.4 to 3.2, and the same five-term basis
  stage 5 fits the geometry through takes another third to a half off it. The
  null says that shape is not a window that moved: 0.15 to 0.25 codes of
  one-cycle amplitude against the measurement's 1.4 to 4.1.

  The cycles are applied as a FIELD near the seam, whole across every
  crossover the band can open and faded out by `Reframe::overlap` - the one
  angle in the problem that is a property of the cameras. Carrying it over the
  whole hemisphere the way stage 5 carries its rotation was measured and
  refused: lens shading would read the same on every scene of one file and
  this moves by 3 to 27 codes between five places in one capture, so it is
  glare, and glare has no business being painted over half a sphere. The
  glare OFFSET stage 3 priced is answered by measurement rather than by a
  build: on the content that can tell a gain from an offset the two are
  indistinguishable and the pair together buys under a tenth of a code, so no
  black level is moved.

  Eight narrow views round the seam at the owner's own reference instant:
  hue step 8.48 codes mean before, 5.22 after, seven of eight improved. It is
  not under one code and does not claim to be; what is left is the part of the
  ring that is not a constant, one cycle or two.

- 2026-08-01 **The band's cost was the fetch, not the solve** (issue #103,
  stage 2). The obvious optimisation was to score each candidate shift on a
  quarter of the patch's samples, which is what `seam::best_shift` does. It
  made the pass **slower**: 9.1 ms a redraw against 8.4. What the pass spends
  its time on is filling the two correlation grids, 3733 taps of a tiled
  3840x3840 decoder surface per direction per frame on an iGPU that is
  decoding at the same time. Shrinking the grids instead - a 0.10 degree step
  against 0.08, a search that stops at 2.6 degrees rather than 4.0 because the
  fold clamp cannot carry more than 1.8, and half the ring read per frame -
  took the whole per-frame measurement from +3.2 ms to **+0.3**. The first two
  are resolution the parabola and the seconds of averaging give back; the
  third is free because the filter is paced in seconds of media time, so a
  direction read at 15 Hz and one read at 30 settle in the same wall time.

- 2026-08-01 **The two instruments disagreed because the band was wrong twice,
  and the ruler was wrong once** (issue #103, stage 6,
  docs/research/seam-two-axis.md sections 9 and 10). Stage 5 was capped by the
  band's along-seam channel reading +0.06 to +0.20 deg where `--bin seam
  mode=residual` read -0.41 to -0.46 on the same directions of the same file,
  while the two agreed to 0.01 deg on the far side of the ring. Three faults,
  each needed for one half of that. **(a)** `Ring::perp` was built `centre x
  epi`, the negative of `seam::ring`'s own axis: the pass drew correctly for it
  because it measures and applies through the same axis, but every number it
  printed was the probe's with the sign turned over. **(b)** `reset` was a
  property of a FRAME and the state it throws away is per DIRECTION, and a
  frame reads every `SLICES`-th direction - so a seek reset half the ring and
  the other half crept toward the new content at `TAU_FAR`, reaching 0.56 of
  the truth after 120 frames. **(c)** `--bin band` had no way to be handed a
  stored fit, so the two instruments were read under different calibrations,
  which differ by 0.04 deg on the far side of the ring and 0.32 on the arc
  carrying the step. After (a) and (b) the band reads **0.99 of the probe on
  both parities**, and `--bin band` takes `seam=` so (c) cannot recur.

  **The ruler was wrong too.** `--bin step` extrapolates a straight line to the
  seam from four degrees out, on the premise that a horizon is a great circle.
  What it traces is a ridge, and the same frame with the band held off reads
  10.4, 20.9, 30.5, 32.8 and 37.8 view px at `guard` 1.2, 1.6, 2.0, 2.5 and
  3.5. Every DIFFERENCE between two builds survives that - the correction
  rotates one hemisphere and moves its whole trace by a constant, 23.2 px in
  all three windows - so the campaign's deltas stand and its absolute numbers
  carry the hill. It prints a `close:` column now, over the two degrees just
  outside the frame's own crossover, with each fit's rms beside it.

  At the owner's reference view, close-in column: **+17.3 view px on `main`,
  -5.2 cold and +8.1 warm on stage 5, -6.0 cold and -5.8 warm here**. The
  campaign's own wide column reads 32.8/30.2 on main, 10.1/23.3 on stage 5 and
  9.4/15.4 here. What stage 6 buys is that **cold and warm now agree**: 0.2 px
  apart where stage 5 was 13.3, because the reset reaches every direction.

  **Cost, priced with the box divided out** (`--bin band mode=cost`, the slope
  over sixteen extra dispatches, minimum of several runs at 1440x1440):
  **0.58-0.71 ms per redraw in steady state, 3.5 to 4.3 percent of the 16.6 ms
  a 60 fps frame has**, and 1.3-2.9 ms once on the frame a seek lands on -
  which now sweeps the whole ring where stage 5's swept half of it. Stage 5's
  form measures 0.89-0.93 ms; its reported +2.55 ms was `--bin playback`'s
  whole-redraw delta on a box building four worktrees, and six alternating runs
  of two builds under a load average of 21 came back 5.1 to 20.3 ms with the
  builds interleaved. The whole saving is the flat-sky gate, which used to be
  reached only after a candidate's entire double loop had run: a direction of
  blank sky, which on a real seam is most of the ring, paid for the whole table
  to be told there was nothing in it. A narrow re-acquisition search was built
  on top of that and **measured out** - 0.631 against 0.632 ms on a sky seam
  and 0.714 against 0.700 on a seam full of near ground, inside the run-to-run
  spread on both - so it is not in the branch. The cadence the cost ruling
  asked for has been in the pass since stage 2: `SLICES` reads half the ring
  per frame.

  **The owner's second reference view is a different defect** (issue #130). His
  October capture is a ONE X2, and that camera refuses its own fit on every
  file: `only 2 of 72 azimuths on the seam had content both lenses could be
  matched on`, 3 / 2 / 2 across three captures against the 10 a five-knob fit
  needs, so it can never build a pool entry and plays on the factory
  calibration forever. The reason is a trap: the residual there reads 1.1-1.6
  deg along the seam and 0.9-2.8 across, which is larger than the probe's
  window (and those four readings are themselves clipped by it: with the window
  moved onto the ring it is 2.1-2.9 deg along - see 2026-08-01 above),
  and widening the window makes it strictly worse because the back patch is
  sampled as ONE rectangle grown by the whole search - at `along=3.0
  across=6.0` every single try is refused for leaving the overlap. At that view
  one degree epipolar moves the horizon 12 rows and one degree along the seam
  moves it 3, the content at the seam is half a metre away, and the step is
  5 to 6 DEGREES. Measured on `main`, on stage 5 and here it moves by about a
  pixel in each direction, which is the right outcome for a fix aimed at
  another axis.

- 2026-08-01 **The seam has two axes and the campaign had only ever measured
  one** (issue #103, stage 5, docs/research/seam-two-axis.md). The owner
  rejected the horizon on `main` after stages 1 to 4 all merged on good
  numbers, and the reason is that every acceptance number those stages carry
  is a statistic of the **epipolar** axis, which is the axis a horizon cannot
  show. At his fov-20 reference view one degree epipolar moves that horizon
  **0.6 rows** and one degree **along the seam** moves it **53**, and the whole
  band campaign moves that view by 2.6 view px of 32.8. `Cell::off_epi` had
  measured the other axis since stage 2 and never applied it, and its search
  saturated: three offsets at 0.30 degrees, with 44 percent of measured
  directions cold and 67 percent warm sitting ON the limit against a corpus
  range of 0.17 to 0.67.

  Stage 5 measures it properly and puts it in the picture. The search is now
  nineteen offsets at 0.90 degrees on the same 0.10 grid the epipolar axis
  uses, with the same parabola between whole steps; nothing rails on any
  camera tried. The channel has its own confidence, refused on its own,
  because a reading pinned on the along-seam limit is a camera outside
  anything measured and refusing the epipolar channel for it would throw
  stage 2 away on that footage. One time constant, `TAU_FAR_S`, wherever the
  direction looks: parallax cannot reach this axis at any distance, so what it
  holds is the camera, and the camera does not move.

  **Two things it had to learn by being built first.** Applied per direction
  it scallops - far fewer than 128 directions correlate on a real frame, and a
  field with holes in it applied over a hemisphere warps a horizon instead of
  moving it (18.5 view px of correction at one end of a four-degree fit and
  4.7 at the other). So the ring is fitted to the shape the phenomenon has:
  constant, one cycle and two cycles, which are relative roll, principal point
  and focal aspect, the decomposition `--bin seam` has printed since #48. Five
  numbers, a ridge of one direction's worth of evidence, no time constant of
  its own. And applied only across the band it does nothing - 0.03 view px of
  32.8 - because a pose error is wrong everywhere and not only at the
  handover, so it goes where the calibration it belongs to goes: to lens 1,
  over its whole picture, scaled by the ray flattened into the seam plane,
  which is exactly the `cos(elevation)` a relative roll produces.

  The owner's reference view goes **32.8 to 10.1 view px cold** and **30.2 to
  23.2 warm**, which is short of the low single digits the ruling asked for
  and is capped by the measurement rather than by the application: at the
  azimuths carrying his step the band reads 0.06 to 0.20 degrees where
  `--bin seam mode=residual` reads 0.41 to 0.46 on the same directions of the
  same file, while the two agree to 0.01 degrees on the opposite side of the
  ring. That disagreement is the next thing to diagnose and it is what stands
  between this and a pixel-perfect horizon. Cost is **+2.55 ms per redraw**
  under live decode, which is out of the campaign's class and is the search's
  and not the application's: the same width at a 0.30 grid is +0.86 ms and
  reads 15.9 cold, and a two-pass coarse-to-fine search measured worse on both
  counts on this GPU because sixty-four workgroups' worth of extra barriers
  cost more than the candidates they save.

- 2026-08-01 **The near end of the seam correction is now the search window,
  not the fold** (issue #103, stage 4). The crossover width and the shear
  clamp turned out to be one inequality read two ways, so the band now opens
  to carry what was measured instead of the measurement being cut to fit the
  band. What that exposes is where the remaining bound sits: the clamp is
  inert for every disparity the pass can report, and what stops the
  correction at 0.73 m is `NEAR_DEG`, the search window. Measured by widening
  it to 4.0 degrees on a branch-local build: the direction-frames the band
  opens for go from 175 to **513** on the X3 sample (2.28 to 6.68 percent),
  the worst doubled edge recovered goes from 11.3 to **31.9 view px** on
  content at 0.42 m, and the pass costs **+0.20 ms** rather than +0.06. Not
  taken here, because it changes the measurement rather than the crossover
  and it halves the margin the widest band has inside the lenses' overlap
  (1.04 degrees a side on the X4 Air, against 3.22 today). It is priced and
  it is one constant: the ceiling follows `NEAR_DEG` on its own, so there is
  no second number to keep in step.

- 2026-08-01 **The seam's exposure difference is a gain, and only on the far
  field** (issue #103, stage 3). The whole earlier exposure corpus was refused
  by the audit, so it was re-instrumented from zero
  (`kjerag-spike --bin expose`). What made a trustworthy measurement possible
  is stage 2: the band's alignment is what makes two samples the same content,
  and the correlation that finds it is invariant to a brightness change, so
  the two questions are orthogonal by construction. Three findings, each with
  its own control:

  **The additive term is a near-field artifact.** Fitted across patches
  spanning 17 to 243 codes, a gain-plus-offset model beats a gain alone by 47
  percent when near-field directions are included and by nothing at all when
  they are not. What read as veiling glare in the lens with the sun in it was
  the alignment of a boot. The one exception is the X3, where an offset near
  -9 codes survives the cut; that is the priced follow-up.

  **This is what the old corpus's three inconsistent gains were.** A
  two-parameter difference read with a one-parameter estimator returns an
  answer set by the brightness of whatever content it weighted, and how much
  near-field content a seam holds is a property of the capture. Three
  captures, three numbers, no bug.

  **Pooling per-patch ratios was measured out.** The reading's slope against a
  deliberate misalignment is 0.0370 ln per degree for an average of per-patch
  ratios and 0.0013 for a pooling of totals, on the same frames: a displaced
  window's error is a boundary term and falls as the window widens. Least
  squares in codes then beat both on all nine captures, and an equal-weight
  average of log ratios is worse than doing nothing on four of them.

  What ships is one gain, far field only at the band's own knee, least squares
  in codes, smoothed at the constant the far field already has, split
  symmetrically. **No constant was added except the runaway guard**, which is
  four times the widest gain measured. Cost 0.03 ms a redraw; frame-to-frame
  flicker 69x under one code; one-lens files byte-identical.

- 2026-08-01 **The pinned seam benchmark named the wrong file** (issue #103).
  #87 and #103 both give `VID_20260501_183417_00_001.insv` as the source of
  `~/Videos/TEST.mp4`, the camera maker's export the 0:09 wing dip is scored
  against. It is part **003**: cross-correlating the two files' own audio as
  10 Hz energy envelopes over every offset gives r = 1.000 at offset 0.0 s on
  003 against 0.64 and 0.67 on the other two parts. The export's `comment` tag
  is the CAPTURE's start time, which is part 001's name, and it is not the
  clip's offset. Scored against the wrong part the projection fit never locks
  and the share reads anywhere from 0.497 to 0.932 across half a second, which
  is how a number that is not a measurement got into the record as one.

- 2026-07-31 **The sound reads on a demuxer of its own** (issue #97, owner
  defect). One file handle for all three streams was the simpler design and
  the owner's April capture disproved it: the camera left 67 MB of picture
  between the audio sample ending at 4.907 s and the next one, and
  libavformat lets a stream fall a whole second behind before it seeks out
  of file order, so the sound for that region arrived after its moment had
  passed and the splice dropped it. Measured on main: silent from 4.87 s to
  8.21 s. A second capture on this box has the same gap at 4.480 s and a
  third has one at 1445.8 s, so it is a camera behaviour rather than one bad
  file. The alternatives are all worse: a deeper ring cannot hold sound that
  has not been read, reading the pictures a second ahead needs 60 more
  surfaces than a decoder pool holds, and buffering the packets instead
  means carrying 25 MB of undecoded picture at all times. A demuxer of its
  own with the pictures discarded reads the sound at its own 190 kbps and is
  immune to any interleave, for one file handle and 0.2 s of open. **And the
  underrun count was lying**: a ring that ran dry while its head was behind
  the picture took the splice's fade-down path and counted nothing, which is
  why the hole measured 2.4 s when it was 3.3 s, and why issue #95 read 227
  underruns as a burst at startup when they were this hole in the middle.
- 2026-07-31 (late) Seam architecture revised by three owner rulings: the
  app targets ANY 360 footage (near-field moves in general, so per-frame
  band alignment is the MAIN path and the per-clip table is a prior);
  the horizon bar is pixel-perfect (calibration brings residual inside
  the band search's capture range, per-frame alignment snaps it to zero,
  far field included - which is how Insta360's own horizon is perfect);
  and correction is calibrate-by-watching (seam readings harvested from
  playback's own decoded frames, slerped in below perception, pooled
  per camera, cached per file, no user surface at all).

- 2026-07-31 **ffmpeg pin moved 6.1 -> 7.1** (owner: "Bump to 7"), which
  supersedes the 2026-07-30 entry further down. Issue #65: the Flatpak
  could not be built from the tree at all while the pin said 6.1, because
  every freedesktop runtime ships ffmpeg 7 and the 25.08 one is forced by
  libcosmic's rustc floor. The port is one file. ffmpeg 7 replaced the
  bitmask channel layout with `AVChannelLayout`, which holds raw pointers
  and so is not `Send`, and a `Track` rides its `Reader` onto the decode
  thread; it now derives the layout from the channel count it already
  keeps rather than storing one. The bill goes to the dev box: Ubuntu
  24.04 has no ffmpeg 7 and will not get one, so ffmpeg comes from a PPA
  (AGENTS.md, and the same one in CI) or, without sudo, from
  `scripts/ffmpeg7-local.sh`.

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
  settled and issue #75 put in the code, so an installed build now resolves
  its own icon by name. The app still draws the scalable SVG out of its own
  bytes, because a source-tree run installs no theme to look a name up in.

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
  (`scripts/uitest.sh`). It used to build only when `target/release/kjerag`
  was missing, so a binary left over from before a `git revert` is what it
  drove: the ball check failed twice on a tree whose source passes it four
  runs out of four, and the capture it filed was the reverted design rather
  than the restored one. A harness that reports on code it did not run is
  worse than no harness, and cargo costs nothing when the binary is already
  fresh. `KJERAG_BIN` stays the way to point it at a binary on purpose,
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
  touching the mouse. (Both under the heading follow; the world-fixed lock of
  2026-08-06 turns the body fully under a parked view and takes those to 15.7%
  and 3.9% on one of the same captures.) With the lock off it holds forever,
  but only 5 to 17.5% of parked views qualify. Expected saving at the default: **0.14 W of about
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
  player makes. `kjerag-spike --bin gating` is the whole measurement and
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
  ffmpeg. The 8.x APIs in the research notes are not present. **Superseded
  2026-07-31**: 7.1, see the top of this log.
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
  unpublished master that pulls two further git forks. Kjerag was already
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
- 2026-07-31 One crate per layer, in a workspace (issue #19): `kjerag-meta`,
  `kjerag-media`, `kjerag-render`, `kjerag` (the app) and `kjerag-spike`.
  The layer diagram is now a build constraint, and `kjerag-meta` builds and
  tests with no libav headers anywhere on the box, which a CI job that
  installs nothing checks on every push. `[patch.crates-io]` moved to the
  workspace root, the only manifest cargo reads one from.
- 2026-07-31 `kjerag-render` depends on libcosmic, for one file. The three
  `iced::widget::shader` impls are a foreign trait on types `render` owns,
  and coherence forbids writing them in `kjerag`. The alternative, a set of
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
  redraw pass instead, in `kjerag_render`'s shader widget, which costs a
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
  `cargo run --release -p kjerag-spike --bin seek` is the instrument.
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
  than a measurement is the yaw constant, and the numbers either side of it
  are there too.
- 2026-07-31 Yaw is **high passed, not locked** (issue #8). A view welded to
  the heading the file starts on fights every deliberate turn; a view that
  follows the body exactly inherits every swing. At 3 s the view's worst
  heading swing inside a second is 29 degrees against 103 unstabilized, and
  it still follows 946 degrees of real turning a minute against 986.
  **SUPERSEDED 2026-08-06**, top of this log: a deliberate turn carrying the
  picture round is the thing the owner wanted gone, and the swing this entry
  worried about was never what the high pass caught. On the July 14 file the
  3 s constant took the view's worst swing inside a second from 239.9 degrees
  only to 178.6 (`--bin gyro`), while following 986.8 deg/min of the turning.
- 2026-07-31 The IMU axis convention is **measured, not transcribed** (issue
  #8). A three-letter convention string is only half of a convention; the
  other half is the frame it lands in, which is whatever the project it came
  from composes next, and Kjerag's composition is its own. All 24 rotations
  were compared against the horizon in unlocked rendered frames; `xZY` wins
  every stretch of two captures, by 15 to 36 degrees over the runner-up.
- 2026-07-31 The quarter-turn roll datum belongs to the **delivered picture**,
  not to the sensor (issue #8, closing the open question in
  docs/research/insv-format.md 4.8). The IMU is bolted to the sensor and can
  tell the two readings apart: held level by its accelerometer alone, an X4
  Air comes out a quarter turn on its side through `Rz(roll - 90)` and level
  through `Rz(roll)`. `kjerag_meta::Pose` now carries both.
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
  at rest reads 1 g. `kjerag-spike --bin horizon` measures the horizon's
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
2560x1440 (`cargo run --release -p kjerag-spike --bin playback`):

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

Hemisphere gating (issue #10), `cargo run --release -p kjerag-spike --bin
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

**Read under the heading follow that shipped until 2026-08-06, and the world-
fixed lock makes every row of it worse.** The body now turns fully under a
parked view instead of partly, so the gate comes off more. Re-run on
VID_20260714_193252_00_006, before against after: at fov 90 and no margin
18.2% gated becomes 15.7%, at 15 degrees of margin 5.2% becomes 3.9%, at fov
45 and no margin 49.1% becomes 44.7%, and the longest single run over the file
falls from 14.21 s to 9.41 s at the default field of view. Releases a minute
barely move (5.5 to 5.6 at the default). **The decision the table gated is
unchanged and better supported**: an expected saving of 0.14 W was already too
little for a state machine and a packet ring, and there is less of it now.

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

High-quality zoom sampling (issue #11), `cargo run --release -p kjerag-spike
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
(`cargo run --release -p kjerag-spike --bin seek`):

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
