# Architecture

## Layers

One crate per layer, in a cargo workspace (issue #19). A layer cannot use
a dependency it does not declare, so the diagram below is enforced by
`cargo build` rather than by good intentions.

```
crates/app      kjerag         libcosmic shell + window. The view is an
                               `iced::widget::shader` around a Scene, and
                               the mouse reaches it through that widget.
crates/render   kjerag-render  wgpu: dmabuf import, one WGSL pass (NV12 ->
                               RGB + Mei reprojection + seam blend),
                               camera state (drag = yaw/pitch, scroll = FOV),
                               offscreen render for screenshots
crates/media    kjerag-media   ffmpeg demux, dual VA-API HEVC decoders in
                               lockstep, presentation clock, play/pause,
                               frames by index or timestamp. One demuxer per
                               file of the capture, which is two on the
                               cameras that write one lens per file. No UI.
crates/meta     kjerag-meta    .insv trailer, read directly: per-lens Mei
                               calibration, gyro track, per-frame exposure.
                               No UI, no ffmpeg, no wgpu.
crates/spike    kjerag-spike   the headless instruments, kept out of the
                               app's dependency graph: `spike` (M0 frame-path
                               timings) and `reframe` (the projection pass to
                               a PNG, no compositor needed)
```

`app` -> `render` -> `media` -> `meta`, `render` -> `meta` as well for the
calibration the shader runs on, and `meta` depends on nothing but `prost`.
That last one is the point of the split: `cargo test -p kjerag-meta` passes
on a box with no libav headers, and a CI job that installs nothing proves it
on every push.

`media -> meta` is one function wide and it is issue #79's: a capture is not
always one file, and which file holds the other lens is a fact about `.insv`
naming, which `meta` owns. `media` owns what a container is and does the
verifying half with it (`Shape::pairs_with`), because the second file of a
per-lens pair carries no trailer to check against.

`media` and `meta` know nothing about the shell. `render` names libcosmic
for exactly one file, `crates/render/src/widget.rs`: those three
`iced::widget::shader` impls put a foreign trait on types `render` owns, and
Rust's coherence rules will not let the shell crate write them. The
alternative was a set of forwarding newtypes in `app`, which is more code
for the same wiring. Nothing else in `render` mentions iced.

The shell is libcosmic, which pins wgpu 28, so `render` is written against
28 and owns the one module that wgpu 30 would delete
(`crates/render/src/dmabuf.rs`).

`Size` and `Fallible` live in `media`: they are frame types, and `render`
depends on `media` rather than the other way round. `render` re-exports both
and adds the `Extent` trait, which is the `wgpu::Extent3d` half of `Size`
that cannot live in a crate with no wgpu.

## Failures the pilot is told about (issue #124)

There is one way a failure reaches the pilot and it is the alert, and that is
a property of the types rather than a rule anybody has to remember, which is
what the owner asked for: "why is this not consistent by code design?"

Three pieces hold it up.

- **The line is private.** `crates/app/src/fail.rs` owns the alert's title and
  body in a struct nothing outside that module can build, and the only way in
  is `Alert::raise(Failure)`. A new failure site cannot write its own sentence
  into the window; it adds a `Failure` variant, and the compiler then makes it
  give that variant a title, a line for the pilot and a line for the terminal.
- **The terminal echo comes with it.** `raise` prints as well as shows, so an
  `eprintln!` at a failure site is not a shortcut past the funnel, it is
  strictly less than calling it.
- **The engine cannot report at all.** `render` has no way to say anything to
  a person. A pass that has given up leaves a `Stall` in the open capture's
  own slot, `Scene::pump` turns it into a `Next::Stopped` arm that every
  caller has to match, and the shader widget will only hand that arm to a
  message type implementing `From<Stall>`. The shell cannot compile the video
  widget without a way to receive one.

**What the alert says is not the funnel's to write** (owner ruling,
2026-08-01, AGENTS.md "Errors are the error"). The funnel decides the surface
and the title; the body is the failure's own message, shown word for word. A
sentence written in the shell can only ever say less than the one written
where the failure happened, and the shape that proved it was live: the pilot
read "That file could not be opened." while the terminal read "trailer says
lens frames are 2880x2880 but the stream decodes 736x368".

Three lines of the app's own survive over an error, and they are the whole
list (`fail::refusal`): another camera's format (issue #107), a codec whose
decoder is one install away (issue #69), and a path the sandbox was never
shown (issue #118). Each is allowed because it knows something the error does
not and turns it into something the pilot can do. There is no line under them
for a failure nobody anticipated, which is the part that makes this hold:
there is nothing to fall back to, so nothing can be masked by falling back.

A fourth line of ours is added rather than substituted, and the difference is
the whole test: a stopped video reads the stall's own line and then "Open the
file again.", because that open being over is a fact about the shell that the
stall has no way to know. An addition hides nothing. The sentence it replaced
("The picture could not be drawn, so playback stopped.") was a substitution,
and it knew less than the stall underneath it.

That makes the error messages in `meta`, `media` and `render` pilot-facing
copy. They are still the layer's own to write, and they are still written at
the failure site; what changed is that they are read by a person, so plain
words and no em dashes bind them the way they bind `strings.rs`.

A stop is final for the capture it happened to, which is the owner's second
ruling on the issue (2026-08-01). The `Stalled` that gave up says so for the
rest of that capture's life: the pass stops importing into it, and `Scene`
stops handing out its player, so nothing the pilot presses can run a clock over
a picture that is not coming back. Opening the file builds another `Scene` with
another `Stalled`, which is the whole of the recovery and the only thing the
alert asks for. The shape before it re-armed on the next redraw and raised the
same alert every two seconds.

Deliberately not in the funnel: a capture that could not be written says so in
a toast, because the picture is still there and the pilot is still watching it
(docs/UI.md). The funnel is for the failures that leave him with no video.

## The frame path (zero-copy)

```
VA-API decode (two 3840x3840 HEVC streams, one demuxer)
  -> av_hwframe_map(dst = AV_PIX_FMT_DRM_PRIME, MAP_READ | MAP_DIRECT)
  -> AVDRMFrameDescriptor { fd, offset, pitch, format_modifier } per layer
  -> two single-plane wgpu textures per frame:
       R8Unorm  from layer 0 (luma,   DRM_FORMAT_R8)
       Rg8Unorm from layer 1 (chroma, DRM_FORMAT_GR88 - note GR, not RG)
  -> single fragment pass: Mei-project the ray into each lens that can have
     it, weigh them, sample each lens that carries any of the pixel,
     YUV->RGB, to swapchain at display resolution
```

The shader consumes both lenses (issue #27), so a view anywhere on the
sphere has a picture in it, and it mixes them across a 2-degree crossover on
the seam (issues #7 and #48). Outside that crossover one lens weighs exactly
1 and the other exactly 0 and only the first is fetched: a pixel away from
the seam costs what it cost before the blend, down to the bits it writes.

Since issue #10 the second lens is not projected there either. Each lens's
picture is one cap around its own axis, and how wide that cap is comes out of
the calibration by solving the model's own coverage boundary
(`coverage_floor`), so a ray further off the axis than that weighs exactly
zero and one dot product says so before the model runs. The test is
deliberately one-sided: a lens kept and weighed zero is written and
multiplied by nothing, which is what it was before, and only a lens wrongly
dropped would be a hole. Worth 0.20 ms of a 1.74 ms pass at a view inside one
hemisphere and 0.15 of 1.81 across the seam, and every picture it writes is
byte for byte the one the pass wrote before it.

No queue-family EXTERNAL acquire step is needed, on either wgpu version:
`create_texture_from_hal` offers no hook for it, `TextureUses::
UNINITIALIZED` works, and the spike's output is byte-identical to the
copy path's. wgpu 28's `create_texture_from_hal` has no `initial_state`
argument at all (that is wgpu#9496, new in 30), so the layout-discard
hazard is unavoidable there; it stays benign for the same two RADV
reasons, and the byte-identical PNGs are the check.

There is no viable fallback. The copy path measured 45.3 ms/frame of
delivery (18.4 fps) in the M0 spike: it cannot sustain realtime for even
one lens. Zero-copy import is a requirement, not an optimization. (An
earlier research note put `vaDeriveImage` at 0.53 ms/frame; that was the
map call alone, with nothing reading the pixels through it.)

## Playback (issue #4)

One demuxer per file feeds every decoder and hands out `Frames`: every lens
at the same instant, mapped and ready to import. A lens is never delivered
without its partner, so the two cannot drift apart; if a head ever lacked a
partner the reader drops it rather than pairing two instants.

Two demuxers is the ONE X2 case (issue #79), where a capture is two files of
one lens each. They share a frame grid exactly, so the lanes are matched on
frame **index** - the one number that means the same thing in two timelines
with their own `start_time` - and the reader pumps whichever file is behind
so neither runs away with the memory. The lens 0 file is a frame longer than
its partner in every pair measured, and that frame is dropped: it has no
partner, and half a sphere is not a picture.

**The sound has a demuxer of its own** (issue #97): the same file, opened
again with the pictures discarded. The reason is measured, not stylistic.
Two of the four large captures on this box have one place where the camera
left tens of megabytes of picture between two audio samples, both of them
about four and a half seconds in, and libavformat reads a file interleaved
like that by letting one stream fall up to a second behind
(`mov_find_next_sample` keeps to file order until the timestamps differ by
more than `AV_TIME_BASE`). Sound a second late is sound the splice drops, so
the owner heard three seconds of silence. Nothing on the picture side can
fix it: no ring can hold sound that has not been read, and the pictures
cannot be read a second ahead of themselves when a decoder's surface pool is
20 frames deep. On its own demuxer the sound reads 190 kbps of a 180 Mbps
file, seeking straight to each audio chunk (40x realtime over the whole 30
minute capture, measured), and the pictures no longer cross the file for
packets nobody takes from them. It costs one more file handle and one more
open of the container, 0.2 s on the 36 GB capture.

`Player` runs that reader on its own thread behind a two-deep channel and
answers one question per redraw: which frame belongs on screen at this
`Instant`. Nothing counts ticks. 29.97 fps divides evenly into no refresh
rate anyone ships, so a frame has a due time and the shell sleeps until it
(`iced`'s `RedrawRequest::At`, requested by the shader widget in
`kjerag_render::widget`). Pumping the clock from a shell-side
`window::frames()` subscription instead was written and measured first: 33
to 46 redraws a second against a 60 Hz display, and 1 to 18 dropped frames
every 5 s, because the redraw event has to leave iced and come back before
the next redraw can be asked for. The clock must be pumped inside the
redraw pass, which is why `Scene::pump` takes `&self` and holds the player
in a `RefCell`.

Playback still paces on container PTS, which is what a presentation clock
wants: a monotonic grid to sleep against. **The gyro does not.** `pts_type =
2` turned out to mean what its name says, so a frame's orientation is looked
up at the camera's own timestamp for that frame
(`ExposureTrack::frame_time_us`), which drifts from the container's nominal
grid at 6.4 ppm and is 11.5 ms away from it by the end of a 30-minute file
(issue #8, docs/research/insv-format.md 8.6).

## Trap list (each verified in the 2026-07 study)

- Use descriptor `pitch[]`/`offset[]` verbatim. Chroma pitch is
  `align(width, 512)`: at 3840-wide that is 4096 != 3840, and computed
  strides shear chroma on real footage while passing on 1920/2560 tests.
  The M0 spike saw exactly this on X4 Air footage, plus the other half of
  the trap: luma pitch is 3840, NOT padded. Padding is per-plane, so no
  single computed rule is right for both. (Chroma offset 14745600,
  modifier `0x200000010401b04`.)
- radeonsi exports ONE fd; later planes reference object 0. `dup()` per
  plane use; caller closes every fd. Spike saw `nb_objects` 1 with both
  layers on `object_index` 0.
- Pre-flight the format modifier via
  `vkGetPhysicalDeviceImageFormatProperties2` before image creation;
  an unsupported modifier is UB, not a clean error.
- Do NOT use ffmpeg's `hwcontext_vulkan` (AVVkFrame) route: handing an
  imported frame to a second Vulkan consumer produced
  `VK_ERROR_DEVICE_LOST` on the target box, twice. DRM_PRIME only.
- `av_hwframe_map` with `MAP_READ` calls `vaSyncSurface` first (ffmpeg
  6.1 `hwcontext_vaapi.c:1337`): the map waits for the decode to finish.
  The spike's 7.64 ms "deliver" is mostly this wait with one frame in
  flight; keep 2-3 frames in flight to hide it. Measured on the player
  (`kjerag-spike --bin playback`): mapping the oldest queued frame rather
  than the newest takes dual-stream decode from 2.19x realtime at depth 0
  to 2.46x at depth 2, and 2.47x at depth 4. `Reader::lookahead` is that
  depth and the engine sets it to 2.
- The decoder's VA-API surface pool is fixed at `avcodec_open2` and is 20
  surfaces per stream here (`Reader::pool_size`, read from the
  `AVHWFramesContext` after the first frame). Every held frame, mapped or
  not, holds one: the engine holds at most 9 per stream (2 lookahead, 2
  queued pairs, the one on screen, the one peeked, and 3 retained on the
  GPU). Nothing checks this at runtime; the count is the budget.
- An imported texture aliases the decoder's surface, so dropping the
  `Frames` while the GPU is still reading hands live memory back to the
  decoder. `ScenePipeline` keeps the last 3 pairs alive behind the one it
  binds; iced submits after `prepare` returns and presents later still, so
  "the draw call was recorded" is not "the GPU is done".
- Reference import code: `ez-ffmpeg` 0.17 `wgpu_filter/hw_interop.rs`,
  `iroh-live` `rusty-codecs/src/render/dmabuf_import.rs`, `bevy-dmabuf`.
- GStreamer was evaluated and rejected: no wgpu or dmabuf-to-Vulkan sink.
- System ffmpeg is 6.1 (Pop!_OS): pin the ffmpeg-next major that matches,
  or vendor a newer ffmpeg; do not assume the 8.x APIs from the research
  notes are present.
- wgpu-hal 28 enables `VK_KHR_external_memory_fd` and
  `VK_EXT_external_memory_dma_buf` whenever the adapter has them, but never
  `VK_EXT_image_drm_format_modifier`, and `iced_wgpu` builds its device from
  a fixed `DeviceDescriptor` with no hook. The `[patch.crates-io]` entry in
  the workspace root manifest is what turns the third one on, for iced's
  device and ours alike; the spike additionally forces it through wgpu-hal's
  `open_with_callback` because it builds its own device. `dmabuf::import`
  still checks `enabled_device_extensions()` and refuses: creating an image
  with a disabled extension's structures is UB, not an error, so the day
  someone drops the patch entry the failure must be loud.
- libcosmic's content container insets the app's view by `border_padding`
  on the right and, because `nav_bar.active` defaults to true even with no
  nav model, by nothing on the left (`app/mod.rs`, `main_content_padding`).
  Measured at scale 1.25: 1 physical px of border left, 10 right. The app
  sets `core.window.border_padding = Some(0)` and keeps the container
  (issue #93): the same `[0, 0, 0, 0]` around the video, and the window
  background comes with it. That background is painted only on the
  container branch (`app/mod.rs:856-874`,
  `background(theme.transparent).base`), so an app that turns the container
  off to reach the window edges is transparent behind its own content, and
  under the desktop's blur that reads as blur with no window over it.
- `iced_renderer` silently drops shader primitives when the tiny-skia
  fallback is chosen (`fallback.rs`: a `log::warn!` and nothing drawn), so
  a blank widget can mean "wrong renderer", not "wrong shader". libcosmic's
  `wgpu` feature is not on by default.
- iced's surface is sRGB when it gamma-corrects, while the spike's offscreen
  target is `Rgba8Unorm`. The same WGSL writes different numbers to the two:
  gamma-encoded video has to be linearised before an sRGB target re-encodes
  it. `TextureFormat::is_srgb()` decides at runtime. A screenshot therefore
  renders into a texture of the *surface's* format rather than a format of
  its own, which is what makes the bytes it reads back the bytes the
  compositor was handed. Measured both ways on real footage: the two agree
  to within one code on 5% of channels and exactly on the rest, which is the
  8-bit rounding of the round trip and nothing else.

## Projection

Insta360 stores a full Mei/UCM camera model per lens in the trailer
(`offset_v3`): xi, fx/fy, cx/cy, k1-k3, p1/p2, per-lens extrinsics
(~33 mm baseline). The X4 Air fixture: xi = 2.31494. The forward map is
~20 lines of WGSL, written from the Mei/OpenCV-omnidir description in
docs/research/insv-format.md 5.1; Gyroflow's
`distortion_models/insta360.wgsl` (GPL-3.0, AGPL-compatible) remains an
available reference for anything the description does not cover, and a file
that takes it carries an SPDX header. Nothing does today. Static calibrated
warp with a smooth blend; no optical flow (measured to not help). A reframed
view centered near a lens axis contains no seam at all.

`kjerag-meta` turns that string into a `CalibrationSet` whose pixel numbers
are already in delivered-frame coordinates (3840x3840 per lens), not the
15360x7680 side-by-side calibration canvas the file writes them on. The
shader consumes them as they come; nothing downstream rescales.

The rotation is `Rz(roll - 90 deg) * Ry(yaw) * Rx(pitch)`, in a frame whose
axes are the delivered frame's own (x right, y down, z along the optical
axis), times a half turn about the body's vertical for lens 1. The IMU is
bolted to the sensor rather than to the picture and wants the same three
angles **without** the quarter-turn datum (`Pose::sensor_from_body`), which
is how issue #8 settled where the datum comes from. That
quarter-turn datum was measured against rendered frames from two cameras,
not assumed: applying roll as the file writes it puts the world on its
side. So was the half turn, which the file does not contain at all (lens
1's recorded yaw is 0.039 degrees, not 180) and which the pictures of the
two lenses have to agree across the seam to settle.
docs/research/insv-format.md 4.8 and 4.9 have the frames, the method and
the tables.

Every ray is weighed against both lenses (issue #7), and the weights sum
to 1 wherever anything has it. Each lens's claim is its **share of the
crossover** times its **coverage depth**, zero where the ray is not in that
lens's picture at all. The crossover is **2 degrees wide** and centred where
the two lenses are equally far off their own axes (issue #48), which is the
seam wherever the calibration puts it; the coverage depth is a distance
transform from the lens's own validity boundary, so a lens fades out as it
runs out of picture and the rim of the image circle, where vignetting lands
and where the distortion polynomial is least trustworthy, is down-weighted for
nothing.

The width is the one number here a person chose, and it is a trade with
measurements on both sides (docs/research/insv-format.md 6.8): a wider band
draws whatever the two lenses disagree about twice, and a narrower one folds
the picture where they disagree at all. It could only be narrowed after the
calibration was corrected, and it takes the doubled band on real footage from
10.6 degrees to 1.5 while the band's own sharpness goes from 0.723 of one
lens's to 1.074.

A file with **one** lens stream takes no crossover at all: it has no seam to
hand over at, and its picture runs to the edge of its own coverage, 7 degrees
past where a seam would have been.

### The seam is calibrated per camera (issue #48)

The camera's own calibration is out by degrees at the seam on the owner's
unit: 2.4 across it, which is 43 px of the delivered frame and reads as a
doubled tree trunk. It is a relative lens tilt with a principal-point error
under it, and `kjerag_render::seam` measures and corrects it **per camera**,
because that is what it is: fitted file by file the same pair of lenses asks
for five answers 15 view pixels apart, while one answer fitted on a capture
from a camera standing still reads the same along-seam number on three and a
half months of the owner's flights as their own fits do
(docs/research/insv-format.md 6.8).

The fit is the phase-1 instrument's own measurement, in the shipped map's
units: both lenses sampled on the same angular grid at 72 azimuths on the seam
circle over frames spread through the file, each calibration field turned by a
probe amount to build the design matrix, three Gauss-Newton rounds because one
is 2 percent short at this size. Five knobs, a relative rotation and a
principal point, with the point held towards zero by a ridge and a fit refused
below twice the knob count in azimuths: those two are what keep a capture with
little far-field content from asking for 54 px of principal point.

**Nothing is fitted at open.** The answer is five numbers under a serial-free
camera key (`CalibrationSet::camera_key`) in cosmic-config state, so a file
opens corrected before its first frame with nothing to decode. `View >
Calibrate seam from this video` is what puts one there, on the file the pilot
has open, off the main thread, about two seconds.

A camera with no stored calibration falls back to fitting off the file being
played, on its own thread, landing a second or two in and saying so in the
report line. That is weaker for a measured reason and not just in principle:
a flight's across-seam column carries that flight's parallax, and a fit
through it absorbs some into a number that is then applied to the whole
sphere. A file the fit cannot read -- a legacy one-stream capture, a seam with
nothing far-field on it, an answer too big to be a calibration -- keeps the
factory calibration.

Nothing is shown from neither lens: the two 97.4-degree caps overlap by
about 14 degrees, which is checked over the whole sphere by `cargo test`
and over a 40-view sweep of real footage by counting the pixels the shader
painted grey. Where one lens carries the ray alone its weight is written
rather than divided out, because a GPU `x / x` is a reciprocal multiply
and lands an ulp short: on RADV that ulp reached the picture as one code
on 6 pixels of a million, which is enough to stop a one-stream file from
rendering the bytes it used to.

Exposure is corrected, but not from the shutter records (issue #103, stage
3). The trailer carries both lenses' per-frame shutter (records 4 and 12,
parsed by `kjerag-meta` and kept apart) and it is **not** a brightness ratio:
the two lenses trade shutter against sensor gain to reach the same picture
brightness, and applying the symmetric split that ratio implies makes the step
across the seam four to twenty times worse (6.3).

What is corrected is what the band **measures**. The same compute pass that
lines the two lenses up on the same content reads how much brighter one of
them drew it, which is a question only that alignment makes answerable: the
correlation that finds the shift is invariant to a brightness change, so the
shift is not moved by the exposure and the exposure read at that shift is not
moved by the shift. One workgroup then pools the ring into a single gain and
`Tone::split` halves it between the two lenses, so neither hemisphere carries
the whole change.

Three things about it are measured rather than chosen, and
docs/research/insv-format.md 6.10 has the tables: it is pooled from the **far
field only**, at the band's own knee, because a near-field direction's
photometry reads the alignment rather than the exposure; it is a **least
squares in codes**, which is the only one of three poolings that lowers the
step on all nine captures tried; and it is smoothed at the constant the far
field already uses, so nothing was added for it. A file with one lens stream,
and every frame before the first reading, take a gain of exactly zero and draw
byte-identical pictures.

The forward map exists twice, in `crates/render/src/projection.rs`: once in
WGSL for the GPU and once in Rust so `cargo test` can check known angles
with no GPU and no footage. Both read one `Reframe` uniform block, and the
bind group's `min_binding_size` makes wgpu reject a pipeline whose two
definitions have drifted apart.

Reframing, stabilization, and rolling-shutter correction fuse into ONE
backward mapping per output pixel. No intermediate equirect, ever.

## The output projection: flat, then bent, then a ball (issue #47)

The map above answers "which pixel is this ray"; something else has to say
which ray a point of the **output** is, and until issue #47 that was one line
of rectilinear projection with a hard 110-degree cap on it. Past there a flat
window stops being one: it stretches the corner by `1 / cos` of the angle out
to it, 3.1x at the corners of a 110-degree 16:9 view, and runs away to
infinity at 180. So the frame bends instead, and keeps bending until the whole
sphere is a ball with room around it.

One family does all of it (`projection::Screen`). A plane radius `r` from the
middle of the frame is the direction `theta` off the view axis with

```
r = tan(shrink * theta) / shrink
```

`shrink` of 1 is `tan(theta)`, the flat window, arithmetic for arithmetic what
it always was. 1/2 is `2 tan(theta / 2)`, which **is** stereographic: the
**tiny planet**, what Insta360 calls it and what the owner means by the blue
ball, arrived at rather than special-cased. Below that the
sphere closes into a disc of finite radius and the frame reaches past it.
`shrink` is `110 degrees / fov`, held at 1 until the view is wider than that,
so past the threshold `shrink * fov / 2` is constant: **the frame keeps the
half angle of the widest flat view and the world is shrunk into it**, which is
what keeps zooming out zooming out. `the_picture_only_ever_shrinks` is that
claim over the whole range and every point of the frame, and it is the one a
different-looking schedule fails: the widening and the bend pull opposite ways
and an unbalanced pair hands back a scroll that reverses in the middle.

The field of view keeps meaning what it meant, past a full turn included: 360
degrees is the frame's own edges half a turn out, so the sphere is exactly as
wide as the frame, and wider still is the frame reaching past the sphere. That
is where the room around the ball comes from, and it is why the far end
(`fov_ceiling`) depends on the **window shape**: the ball is round and fills
0.8 of the frame's shorter side there, which is 406 degrees on a square window
and 605 on a 16:9 one. Outside the ball there is no ray at all, and since
issue #100 the pass writes nothing there: transparent black, through a
premultiplied blend, so **what fills the room is whatever the shell put behind
the video** (`app::backdrop`). In a window that is libcosmic's own pane, which
is a translucent copy of the background colour over the compositor's blur
while the theme is frosted and the same colour opaque when it is not; in
fullscreen it is black; and a still, which runs the same pass into a texture
cleared black, comes out black with no alpha left in it.

The drag needed no new mathematics, which is the finding rather than luck:
`Camera::look` and `Camera::aim` were already written against `view_ray`
rather than against a `tan`, so they invert whatever map the view is in. Two
things did change. A press on the room around the ball takes hold of nothing,
because there is nothing there (`look` answers `Option`), and a cursor exactly
a quarter turn out along the frame's own horizontal axis has a ray no pitch
can move, which is zero over zero in the height solve and would have left a
NaN camera that never comes back.

Measured on the X4 Air at 2560x1440 (`kjerag-spike --bin ball`):

- **No pop.** One scroll from 20 degrees to the ball, a notch at a time,
  rendered: the largest single step is 64.3 codes at fov 402, and the largest
  a step grows against the step before it is 1.32x at fov 173. Inside the flat
  range, which this change did not touch, that same number is 1.15x. Nothing
  stands out at the threshold or at stereographic; at a quarter of a notch the
  largest growth anywhere is 1.12x. The geometric statement is in `cargo test`
  (`the_bend_starts_without_a_step`): halving the scroll across the threshold
  halves the angle the picture moves, four halvings running, and a jump would
  not shrink at all.
- **It costs a tenth of a millisecond at the far end and nothing at all in
  the range that already existed.** Interleaved across the range, three runs
  back to back agreeing within 0.01 ms a cell: 0.71 ms/redraw at the default
  view, 0.63 at the threshold, 0.67 at 150 degrees, 0.71 at stereographic,
  0.83 at 320 and **0.81 at the ball**, against a 33 ms frame. The trig is not
  what the far end costs, and the table says so itself: 320 degrees runs the
  same bent map over a frame that is all picture and costs the same 0.83. (All
  six rows move together with the box's clock state -- an earlier session sat
  near 2.2 ms a cell with the same shape -- so this table is read across its
  own rows rather than against another day's.) The flat range is unchanged,
  measured the way it was first measured: `--bin zoom`'s cost table off this
  branch against the same binary built off main, alternated on one box, agrees
  cell for cell within 0.04 ms in both directions (fov 90: 0.56 / 0.73 / 0.98
  main against 0.58 / 0.71 / 1.00). It runs the two multiplies it always ran,
  and the `length`, the `atan` and the `sin_cos` the bend needs are all behind
  the one uniform test that says the frame is not flat.
- **Playback holds at the ball.** 20 s of real footage at the far end of the
  zoom, 2560x1440, with three 3840 px captures taken during it: 600 redraws,
  29.97 fps presented, **0 dropped and 0 starved**, 2.65 ms per redraw in the
  pass under live decode. The captures are the ball: a 3840 px still through
  `Scene::capture` at the ball is byte for byte the same picture as the same
  view drawn straight into a 3840 px target, which is issue #15's own check
  run at a field of view it was never written for.
- **Minification, not magnification.** Out wide an output pixel covers 7.6
  delivered texels at the middle of the ball and 5.3 at its rim, so issue
  #11's Catmull-Rom kernel reads 0.00 engagement everywhere past 110 degrees
  and the pass is plain bilinear, which is what it should be. What that costs
  is aliasing rather than softness: against the same view supersampled 4x4 and
  box averaged, the ball is 4.1 codes out over the pixels that have picture
  and 107 at worst, against 1.1 codes and 11 at the default view. A moving
  picture will shimmer on high-contrast edges out there. The fix is a
  prefilter, not a sharper kernel, and it is not built: the imported dmabuf
  textures have one mip level and no room to generate more in place, so it
  would be a downsample pass per frame per lens for a view the player is in
  for a few seconds at a time. Issue #47's comment thread has the numbers.

## Sampling a magnified picture (issue #11)

Zoomed in far enough, an output pixel sits inside one source texel and the
hardware's bilinear tent is what the eye sees rather than the picture. Where
that is true the pass reads a **Catmull-Rom kernel** instead, engaged
smoothly as magnification passes 1:1 and exactly off at or below it
(`crates/render/src/sampling.rs`). Sixteen texels, taken as **nine bilinear
fetches**: each axis's middle pair of weights is positive, so it is one fetch
placed between its two texels; the outer two are negative, which is where the
resolving comes from, and a sampler cannot weigh a fetch by less than
nothing. Measured against the same kernel as sixteen `textureLoad`s on the
highest-contrast view in this footage, the nine agree to 0.14 codes RMS and
one code at worst, which is the sampler's own filter-weight precision.

**How magnified is decided per fragment, off the map's Jacobian**, because
nothing about it is uniform. The fisheye's angular density varies across its
own picture (1106 texels per radian down the X4 Air's axis, 948 radially at
the rim) and a rectilinear output's rises towards its corners, so at the
widest view the player offers a 2560 px window is past 1:1 in the middle
(1.23 texels to the pixel) and two thirds inside it at the corners (0.74).
The shader reads it as `max(length(dpdx(landing)), length(dpdy(landing)))`,
the hardware's own quad derivative of the landing the model just computed,
which is the Jacobian by finite difference with the distortion, the mounting
and the readout already in it. It is taken in the entry point rather than in
`blend`, because a derivative needs uniform control flow and the blend is
branches; `Reframe::texels_per_pixel` is the Rust mirror, checked in
`cargo test` against the paraxial focal length `fx / (1 + xi)` over the whole
zoom range.

Reading the step off the quad rather than off a resolution in the uniform
block is also what makes a **still** right without being told: the capture
draws this same pipeline into a target of its own size (issue #15), and a
quad of that target steps a smaller share of the picture by itself. A 3840 px
still off a 2560 px window is byte for byte a 3840 px render of the same
view, which `kjerag-spike --bin zoom` checks rather than assumes.

**The chroma plane is not upgraded**, and that is the measurement rather than
an omission. NV12's two planes are two grids: chroma is half the size, so one
output pixel covers half as many of its texels and it reaches 1:1 an octave
of zoom before luma does. On this camera at a 2560 px window that means
chroma is magnified at **every** field of view the player offers, so
upgrading it is not a cost paid at high zoom but a cost paid always, and it
is the larger half of the bill. What it buys on 8-bit 4:2:0 chroma that HEVC
has already smoothed is 0.41 codes on 40% of pixels and no measurable change
in detail at all. `Sampling::Sharp` keeps it renderable, one line from
shipping, for the footage that would change the answer.

## Rolling shutter (issue #9): fused, measured, and on

A frame does not leave the sensor at an instant. `rolling_shutter_time` is
15.883 ms on the X4 Air, so the row a ray lands on was read up to 8 ms from
the frame's nominal time, and the orientation that ray should be carried
through is the one at **that row's** instant. That is circular, because the
row decides the instant and the instant moves the row, so the landing is
solved for rather than computed: `Reframe::solve`, from the frame's own
instant outwards. **One round**, which at the hardest instant of a 30-minute
capture (551 deg/s) leaves 4.5 px of a 112 px correction and at the median
rate leaves a hundredth of a pixel; a second round would cost another pass
through the model per lens per pixel for a quarter of a pixel.

It is one rotation per frame and a multiplication per pixel, not a lookup per
row: `OrientationTrack::turn` over the readout window is a rotation vector,
and a row's share of the readout scales it. Measured against the track's own
orientation looked up per row, that straight line is 0.019 to 0.068 degrees
out at the median and 0.64 at the worst of two captures, and what is left is
vibration inside one readout that a 200 Hz track does not resolve either.

The correction is **not under the horizon toggle**: it is the camera's own
motion during the frame, not the display's, so a view that rides the body has
the same skew in it as one that does not. With no IMU record it disables
itself and the pass is what it was before, down to the bits.

**Which way the sensor reads is not in the file**, and it decides everything:
applied backwards, a correction does not fail to remove the skew, it doubles
it. So it is measured per camera in `readout_sweep`, and an X4 reads **down
the delivered frame**, 1.00 +-0.12 of a whole frame in the trailer's own
15.883 ms, against 0.02 +-0.07 across it. Both lenses read down their own
pictures, which is the same world direction, so it cancels at the seam and a
seam measurement is blind to it: that is why issue #42 shipped
`Sweep::Unknown`, and it is also why switching this on cannot put any of the
1.9 degrees of misalignment that an across-frame sweep would have put into
the band #7 blends.

A camera nobody has measured keeps `Sweep::Unknown`, which is a zero axis and
therefore no correction at all rather than a guess. docs/research/insv-format.md
6.7 has the three instruments, the injected controls on each axis of the fit,
and why a still capture cannot answer this question.

## Horizon lock (issue #8)

The trailer's IMU record is read at open, integrated once, and the result is
a `world_from_body` quaternion every 5 ms of the file
(`kjerag_meta::OrientationTrack`). The pass composes its inverse between the
lens mounting and the camera:

```
view_to_lens = lens_from_body * body_from_world * camera_rotation
```

Identity in the middle is the toggle off, and then the pass is bit for bit
what it was. The drag needed no change at all, which is the finding rather
than luck: `Camera::look` answers in whatever frame `camera_rotation` lands
in, so with the lock on that frame is the world, the anchor a press stores is
a world direction, and the solve puts a world direction back under the
cursor while the picture turns underneath it.

The filter is complementary and about sixty lines: integrate the gyroscope,
turn the estimate towards the accelerometer with a 20 s time constant, and
believe the accelerometer only while its magnitude is near 1 g, because a
banked turn is not gravity. Roll and pitch are then locked completely; yaw is
**high passed** with a 3 s constant instead, so a swing is cancelled and a
deliberate turn is not. Every constant is measured, and the tables are in
docs/research/insv-format.md 8.5.

Verification without a Studio export: physics in the footage itself.
`kjerag-spike --bin horizon` renders runs of frames through the app's own
pass and measures the angle of the horizon in each. Residual sway is 0.23
degrees peak to peak over 120 frames of calm flight and 2.86 through a
61 deg/s roll, against a picture whose horizon leaves the frame entirely with
the lock off. The same instrument's 24-way sweep of axis conventions is the
negative control: the string telemetry-parser falls through to for this
camera reads 54 to 65 degrees of standard deviation against 0.04 to 0.68 for
the right one.

The orientation track is also **issue #9's input**, and it needed one method
rather than one call per row: `OrientationTrack::turn` reads the two ends of a
frame's readout window and answers the rotation between them as a vector, so a
row's share of the readout is a multiplication in the shader instead of a
lookup. The section above is what came of it.

## Clock domains (the correctness minefield)

Video PTS, `first_frame_timestamp`, gyro timestamps, `gyro_timestamp`
offset when `is_has_gyro_timestamp`, per-lens exposure timestamps, and
rolling-shutter row time (15.9 ms on the X4 Air). Failure mode is a
swimming horizon, not a crash. Two of them are now nailed down and measured
(below and in issue #8's entry above); the harness that measures them is
`kjerag-spike --bin horizon`, and a Studio export drops into it as one more
row.

One of those is now nailed down. **The trailer's tick is not always a
microsecond**: `is_raw_gyro` selects it, and `first_frame_timestamp` is in
whichever tick the file uses. The X4 Air sets the flag and writes
microseconds; the ONE X2 does not and writes milliseconds, including in
`first_frame_timestamp`. That is the "divide by 1000 twice" of the format
study read as what it is, and it is measured against both cameras'
exposure tracks in `kjerag_meta::ExposureTrack`. The gyro track reads on the
same `Clock`, and then takes `gyro_timestamp` off it as milliseconds (1.6 ms
on the X4 Air).

## Open questions

- Vignetting coefficients are not in the metadata; the seam band may show
  rolloff. The weight field down-weights the rim it lands on, which may be
  enough; flat-field calibration if it is not.
- The **order** of `yaw`, `pitch` and `roll` within the lens pose. Their
  composition is settled (above); the order is not, and no known camera can
  distinguish it, because every one of them records sub-degree yaw and
  pitch (docs/research/insv-format.md 4.8).
- What is left of the seam. Settled and shipped for the geometry: the 2.4
  degrees **across** the seam and the along-seam one cycle under it were both
  calibration, and both come out of a five-knob fit stored per camera
  (above). On the far-field control the whole thing is down to 0.02 along and
  0.11 across, which is under two view pixels. What is left on **flights** is
  0.15 to 0.22 along and 0.49 to 0.92 across, and the second of those two
  numbers is parallax rather than calibration: it is the axis a baseline can
  reach, it moves with what the camera was looking at, and no correction
  applied to the whole sphere can take it out. That is the next thing on this
  seam and it wants depth, not knobs.
  docs/research/insv-format.md 6.8 has the numbers and the transfer table.
- **Exposure across the seam is still not corrected** (6.3), and with the
  crossover now 2 degrees wide instead of 10 there is less band to hide a
  brightness step in. Measured on the flattest, brightest content in this
  footage, it did not become one: the luma profile across the seam is the same
  ramp either way, 147.8 to 153.8 codes over 70 px before and 147.1 to 154.4
  after, because what differs between the two lenses there is vignetting
  inside each lens's own picture rather than a step at the handover. One view;
  it is still the thing to look at first if a seam shows on flat sky.
