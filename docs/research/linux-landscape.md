# Linux landscape for Insta360 `.insv` (2026-07-30)

What already exists for viewing, reframing and converting Insta360
dual-fisheye `.insv` footage on Linux. This is the baseline Kjerag has to
beat, or the reason Kjerag should not exist. Everything here was gathered
on 2026-07-30 from public sources; every claim carries a URL and a
confidence rating.

Subject format: Insta360 X4-class `.insv`. An MP4 container holding two
3840x3840 HEVC streams (dual fisheye, over 180 degrees each, **not**
pre-stitched) plus a proprietary metadata trailer. Target workflow: play,
zoom, drag to reframe, screenshot, and possibly export clips.

**Confidence key.** *High* = read from primary source (vendor doc,
upstream manual, source code, repo README). *Medium* = consistent
secondary reporting, or a primary source that does not quite cover the
exact case. *Low* = single report, or inference from adjacent evidence.

---

## 0. The one-paragraph verdict

The gap is real but narrow, and it is not where it looks. No single
existing tool covers view + zoom + drag-reframe + screenshot on raw
`.insv` on Linux, but the union of two zero-code paths covers those
requirements today. Path A, no conversion:
`mpv --lavfi-complex='[vid1][vid2]hstack[vo]' file.insv` plus
[mpv360](https://github.com/kasper93/mpv360)'s GPU dual-fisheye shader
gives live playback, mouse-drag reframe, FOV zoom and `screenshot window`
stills. Every piece of that is independently verified (mpv manual, mpv360
source, Insta360 developer docs), but the assembled pipeline is documented
nowhere and carries three untested risks: copy-back hardware decode at
2x3840x3840, square-resolution hardware decode support, and a naive
shared-centre reprojection with a hand-tuned FOV, no per-lens calibration,
no seam blending and **no gyro horizon stabilisation**. For rolling,
tilting flight footage that last one is the defect that actually hurts.
Path B, convert first: Insta360's own MediaSDK runs natively on Ubuntu
x86_64 with real calibration, AI stitching and stabilisation, costing
roughly 40 GB and about an hour of GPU time for a 36 GB library, after
which VLC, mpv or Bino handle viewing and Kdenlive's bigsh0t filters give
genuine keyframed reframe-and-export, the one Insta360 Studio feature
everyone assumes has no open-source equivalent. Insta360 Studio under
Bottles is a viable third leg for photos and is maintained recipe by
recipe through Studio 5.8.2 (January 2026), but the video evidence is
contradictory and the recurring complaint is precisely that it never
reaches the GPU. **The remaining justification for a native player is
calibrated stitching with a stabilised horizon and a keyframed reframe
timeline in one surface, at full resolution, with no conversion pass.**
That is a product decision, not a missing-capability one. If it is built,
do not derive fisheye maths from scratch: wrap or port a solved model, and
copy the decode-to-GPU-texture pipeline from the Rust references in
section 5.

---

## 1. Insta360 Studio on Linux (Wine, Proton, Bottles)

### Native support

**There is none, and none is announced.** Insta360's community forum has a
dedicated Linux Support section carrying 1,461 posts with no official
commitment found anywhere in it.

- https://forums.insta360.com/section/17/

Worth noting for context: Insta360 does ship Linux binaries, just not the
GUI. Their MediaSDK has an Ubuntu build (section 6). So this is a product
decision, not a technical barrier on their side.

*Confidence: high.*

### Wine and Bottles: it works, and one person is keeping it working

The load-bearing source is a Spanish-language running log maintained since
April 2024, with dated entries per Studio release:

- https://www.modlearth.com/blog/insta360-linux/

Entries: Studio 5.1 (2024-04-22), 5.4.3 (2024-11-10), 5.4.7 (2024-12-18),
5.5.2 (2025-03-10), 5.6.1 (2025-04-27), plus a 5.6.1 Bottles-versus-VMware
benchmark (2025-05-03) in which Bottles wins by a widening margin. The
recipe, restated nearly verbatim at each version:

1. Create a **fresh bottle per Studio version**. In-app update always
   breaks (black windows on import and export).
2. Runner must be **`sys-wine-9.0`**, not soda. Soda reproduces the black
   windows.
3. Windows version set to **Windows 11**.
4. Change nothing else.

The same author's 2026-01-15 post confirms **Studio 5.8.2 running in
Bottles on Ubuntu 24.04 with an X5**, a camera newer than X4:

- https://www.modlearth.com/blog/insta360-darktable/

That is the freshest positive evidence available, six months old at time
of writing. **Caveat that matters: that author's workflow is
photo-centric** (they say outright that this suits you best if you are
mainly doing 360 stills). It is not evidence about 8K video playback or
reframing.

*Confidence: high on the recipe and that Studio launches and runs; medium
that video editing works well.*

### Weaker or stale sources that still rank first in search

- https://github.com/NiklasVoigt/Insta360-Studio-Linux: 41 stars and the
  top search hit, but **stale**: 6 commits total, last one 2024-04-03,
  README says "tested up to v5.0.0" and its install example is Studio
  4.6.6. Do not treat as current.
- https://askubuntu.com/questions/1466232/install-insta360-studio-with-wine
  covers Studio 4.7.6 on Ubuntu 23.04 via Bottles with the sys-wine-8
  runner: "all functions work fine".
- https://plug-world.com/posts/insta360-studio-on-linux/: 2023-02-12,
  Lutris with Proton GE as the runner. Notes "if the app freezes at
  startup, simply kill it and run it again".
- WineHQ AppDB's only entry is Studio 4.2.0 from 2022, and the site now
  sits behind bot-blocking that defeats automated reading. Treat AppDB as
  dead for this question.

*Confidence: high (fetched directly, dates from the GitHub API).*

### Hardware acceleration: reports directly contradict each other

This is the fault line, and it is unresolved.

- https://www.reddit.com/r/Insta360/comments/1mefpjb/insta360_studio_on_linux/
  (mid-2025). One user: running Studio via Bottles on Manjaro Gnome and
  "it is butter smooth and exports surprisingly quickly." Another user in
  the same thread: "Studio in Wine doesn't reach the GPU" despite Intel
  media drivers being installed, leaving it barely functional without
  720p proxy files.
- https://www.reddit.com/r/Insta360/comments/1ksv6br/ (May 2025). Intel
  Core Ultra 7 155H with an RTX 4060: Studio "brings it to its knees",
  all CPU cores maxed, GPU only around 50 percent used, despite Wine
  configuration attempts.
- https://onlinemanual.insta360.com/studio/en-us/troubleshooting/:
  Insta360's own advice in failure scenarios is to **turn off** the
  hardware decoder and encoder in preferences. Their support line
  elsewhere is that export speed is dominated by the GPU driver's encode
  and decode capability.

No X4-specific Wine failure was found, but neither was any confirmed
report of smooth 8K X4 **video** reframing under Wine.

*Confidence: medium-high that GPU decode and encode passthrough under Wine
is unreliable. Note these are search-engine snippets of the threads:
Reddit blocked direct fetching from this environment, so the quotes are
second-hand and should be re-verified before being load-bearing.*

---

## 2. mpv

### Raw `.insv` shows exactly one fisheye circle

X4-class cameras at 5.7K and above store the two lenses as **two separate
video tracks inside one MP4**. From Insta360's own developer
documentation:

> X5, X4, and X4 Air save information for two video tracks in the same
> main code stream (insv) file. [...] To convert an insv file to MP4, you
> can simply change the file extension. This will give you the unstitched
> dual-fisheye video stream. [...] FFmpeg can be used to separate the
> video tracks.

- https://onlinemanual.insta360.com/developer/en-us/resource/integration

So mpv (and VLC, and anything else) selects track 0 and plays a single
flat circular fisheye. This is the reason the naive "just rename it to
.mp4" advice disappoints.

*Confidence: high (vendor primary source).*

### The finding: mpv can merge the two tracks live, with no conversion

mpv's `--lavfi-complex` takes input from multiple source tracks in one
graph. From the mpv manual:

> Set a "complex" libavfilter filter, which means a single filter graph
> can take input from multiple source audio and video tracks. [...] A
> label of the form `vidN` selects video track N as input. [...] A label
> named `vo` will be connected to the video output.

with a worked example that is one filter away from what is needed here:

> `--lavfi-complex='[vid1] [vid2] vstack [vo]'` Stack video track 1 and 2
> and play them at the same time. Note that both tracks need to have the
> same width, or filter initialization will fail.

- https://mpv.io/manual/master/

Therefore:

```sh
mpv --lavfi-complex='[vid1][vid2]hstack[vo]' VID_xxx.insv
```

yields a live 7680x3840 dual-fisheye frame from the original file, no
pre-conversion pass, which is exactly the input layout the dual-fisheye
shaders below expect. **This combination is documented nowhere for
`.insv`.** It was assembled here from separately verified pieces and is
the single most useful result in this survey.

*Confidence: high on each component (all quoted from the upstream manual);
the assembled pipeline is untested end to end.*

### mpv360: the interactive reprojector

- https://github.com/kasper93/mpv360

70 stars, created 2025-06-30, last commit **2026-01-30**, authored by an
mpv core contributor. Lua script plus a **GLSL user shader**, so
reprojection runs on the GPU in the render pipeline rather than through
the CPU `v360` filter.

- Input projections: **`dual_fisheye`** (native, `sample_dual_fisheye()`
  in the shader), equirectangular, dual half-equirectangular,
  half-equirectangular, cylindrical, equi-angular cubemap.
- Controls: `Ctrl+LClick` toggles mouse-look drag, `Ctrl+<arrows>` for
  yaw, pitch and roll, scroll wheel for FOV, `Ctrl+e` to toggle 360 mode,
  `Ctrl+t` for help. **No keys are bound by default**; you wire them in
  `input.conf` or `mpv360.conf`.
- `fisheye_fov` defaults to 180 degrees and is adjustable at runtime via
  the `fisheye-fov-increase` and `fisheye-fov-decrease` script messages,
  which is how you would dial in the real lens FOV by eye.
- Six-degrees-of-freedom movement, three interpolation filters (linear,
  Mitchell-Netravali, Lanczos), stereo eye selection and SBS output.
- Known issues: #2 "360 view breaks with `--vo=gpu`" (moot in practice,
  gpu-next has been the default since mpv 0.41.0), #3 unbounded yaw and
  pitch, #4 Windows path handling.
- **Zero occurrences of "insta360" anywhere in the repository**, code or
  issues. There is no lens preset; you tune FOV manually. Community
  values for Insta360 dual fisheye cluster in the **189 to 204 degree**
  range, not the marketed lens figure.

*Confidence: high (repo and GitHub API read directly).*

### Screenshots need the right flag

mpv's `screenshot` and `screenshot-to-file` commands accept
`subtitles`, `video` or `window`. Only **`window`** captures the rendered
window contents, so a plain `s` keypress saves the *unreprojected* source
frame when reprojection is done by a user shader. Bind
`screenshot window` explicitly.

- https://mpv.io/manual/master/

*Confidence: high.*

### `vf=v360` as an alternative is strictly worse

From `libavfilter/vf_v360.c`: `yaw`, `pitch`, `roll`, `h_fov`, `v_fov`,
`ih_fov`, `iv_fov` and friends all carry `AV_OPT_FLAG_RUNTIME_PARAM`, so
they **are** live-tunable through mpv's `vf-command` (which the mpv manual
notes works only with lavfi filters). But:

- `input=` and `output=` projection types are **not** runtime-changeable.
- `v360` is CPU-only. It is slice-threaded, with no GPU path.
- At 2x3840x3840 in and 7680x3840 out this is a per-frame CPU remap over
  roughly 29.5 megapixels of output.

No benchmark of this exact combination exists anywhere.

*Confidence: high on the source facts; medium on the performance
conclusion, which is inferred rather than measured.*

### Untested risks in the mpv path

1. Nobody has published `.insv` to hstack to mpv360 end to end.
2. `--lavfi-complex` requires software frames, so hardware decode must
   copy back: roughly 2 x 22 MB per frame at 30 fps, about 1.3 GB/s
   across PCIe before the hstack even runs.
3. Square 3840x3840 HEVC hardware decode is unverified on any GPU
   (see section 4).
4. No calibration, no blending, no stabilisation. A visible seam and a
   swimming horizon are expected.

---

## 3. VLC

**Equirectangular only, explicitly, and gated on metadata.** VideoLAN's
own documentation states that VLC supports 360 videos that use the
equirectangular projection type, and that fisheye or other layouts "may
appear as a warped circle".

- https://prime-5.videolan.me/vlc-user/vlm_files/en/advanced/player/360_video.html

Its 360 mode engages off Google spherical-video metadata, so even a
genuinely equirectangular file plays flat until that metadata is injected
(for example with the `spatialmedia` tool). On a raw `.insv` it plays one
flat fisheye track.

Once engaged, VLC does give drag to pan, scroll wheel plus PgUp/PgDn to
zoom, and Video > Take Snapshot for stills, plus little-planet style
viewing modes. So for **already-stitched equirectangular** output it meets
the view + zoom + screenshot bar with zero setup.

**VLC 4.0 is still unreleased on desktop as of July 2026.** Stable is
3.0.23 (2026-01-08); nightlies are tagged 4.0.0.20260729; the 4.0
milestone that actually shipped was the iOS and tvOS beta (2026-06-24).
CES 2026 coverage headlined AV2 decode and said nothing about 360 or
fisheye.

*Confidence: high on 3.x behaviour and on 4.0 release status; medium on
"4.0 adds nothing here", which is absence of evidence.*

---

## 4. ffmpeg-only workflows

### Stream layout, resolved

Because the two lenses are **two tracks in one file** (section 2), an
`hstack` through `filter_complex` **is** required before `v360=dfisheye`
can see both hemispheres. The widely copied two-file `hstack` recipes date
from the ONE X2 era, when the camera wrote two separate `.insv` files. The
`.lrv` is a separate low-resolution proxy **file**, not a track, and may
not exist at all depending on capture settings.

*Confidence: high.*

### Command lines people actually use

The only X4-attributed invocation found, from a corrupted-timelapse
recovery tool that operates on the `.lrv`:

```sh
-vf v360=dfisheye:e:yaw=-90:ih_fov=189.1:roll=180 \
  -c:v libx265 -b:v 40000k -preset ultrafast
```

- https://github.com/jjtt/insta-ffmpeg

The best FOV-tuning writeup is an ONE X2 stills study that tested 200,
202, 204, 206 and 208 and settled on **204**, while noting that the bottom
of frame stays badly stitched regardless and recommending Hugin for
serious work:

- https://www.arj.no/2025/12/19/insta360-to-equirectangular/

Generic dual-fisheye guidance uses 190 to 200; GoPro Fusion tutorials use
190. **No authoritative X4 or X4 Air constant has been published by
anyone.** Deriving it from the on-file calibration is the correct answer,
not guessing.

Other reference command lines and batch scripts:

- https://gist.github.com/nickkraakman/e351f3c917ab1991b7c9339e10578049
  (360 video ffmpeg cheat sheet)
- https://github.com/peterbraden/insv-to-yt (Shell, last pushed
  2022-07-22: joins two `.insv` side by side, remaps, injects YouTube
  spatial metadata)
- https://github.com/bitopsy/Insta-360-INSV-to-MP4-3D-Converter (Python,
  pushed 2026-04-13, extracts per-eye streams, HEVC via GPU by default)
- https://github.com/rekliner/insta360x2toMP4 (C, 2021)
- https://github.com/careyer/Insta360-Air-remap (C, 2020, generates
  explicit remap tables rather than using `v360`)

*Confidence: high that these exist and are what people use; medium on any
specific FOV value being correct for X4-class hardware.*

### Quality against Insta360 Studio: measurably worse, and now quantified

The standout reference, and the most useful single repository found in
this survey:

- https://github.com/BenjaminHenriksson/insv-stitch: Python, 8 stars,
  last pushed **2026-04-20**. "Linux stitching pipeline for raw Insta360
  X5 `.insv` footage. Fisheye dewarp, IMU stabilization, rolling-shutter
  correction, and blending into equirectangular output. No Insta360
  Studio required."

What it establishes, from its README and companion `PIPELINE.md`:

- It parses the **protobuf `.pb` sidecar** for genuine **MEI camera model**
  calibration: `xi = 2.0`, **13 distortion coefficients per lens**, plus
  per-lens extrinsics.
- It solves the IMU-to-camera rotation via Wahba's method against
  ground-truth gravity, derives per-frame stabilisation from IMU gravity,
  and applies per-scanline rolling-shutter correction using 32 SLERP
  keyframes across a **21 ms readout**.
- Everything fuses into **one backward remap per output pixel**, following
  the pattern of the Insta360 SDK and a Qualcomm stabilisation patent.
  Blending is on longitude preference times coverage depth, with symmetric
  per-channel gain across the seam.
- Self-reported **PSNR 22.5 to 22.9 dB against Studio's own render at
  7680x3840**.
- Stated limitations: about **18 px of ghosting at the stitch line for
  objects under 3 m** (a consequence of the 30 mm inter-lens baseline);
  DIS optical flow helps but does not match Insta360's learned
  `ai_stitch_model_v2.ins` on repetitive patterns like fence mesh or
  foliage; IMU calibration is **per-unit**, solved against one X5, and
  unit-to-unit PCB mounting variation degrades it.
- **"Each frame spawns its own ffmpeg process, about 2 s of overhead."**
  So it is a stills tool in practice. A 24-minute 8K clip would incur
  roughly 24 hours of pure process overhead before any real work.

That last point is the reason this is a reference implementation to learn
from rather than a tool to adopt.

*Confidence: high that the README says this; the PSNR figure is
self-reported and unreviewed.*

### The metadata trailer is reverse-engineered, the calibration blob is not

Byte-level writeup:

- https://subethasoftware.com/2022/06/08/insta360-one-x2-insv-file-format/

Magic `8db42d694ccc418790edff439fe026bf` at EOF minus 32, index at EOF
minus 78, then typed segments: 0x0101 maker notes, 0x0300 accelerometer,
0x0400 exposure, 0x0600 timestamps, 0x0700 GPS. Inside the maker notes,
**a type 0x2a field holds 113 bytes of "calibration parameters" that this
public writeup does not decode**. ExifTool reads the trailer today
(implemented in `QuickTimeStream.pl`):

```sh
exiftool -ee -G -s -b -j -a -T file.insv
```

**Gyroflow does not help here.** Its own documentation states that 360
cameras are not supported; it is a single-lens action-camera stabiliser
with no lens or stitch calibration for dual-fisheye rigs.

- https://docs.gyroflow.xyz/app/getting-started/supported-cameras/insta360

*Confidence: high.*

### ffmpeg has no `.insv` demuxer, and hardware decode is an open risk

No Insta360-specific demuxer exists in ffmpeg (Trac, GitHub and changelogs
all searched, nothing found). The trailer is simply ignored, which is why
every workflow above throws the calibration away.

On hardware decode: 3840x3840 is 14.7 megapixels, comfortably inside HEVC
Level 6.1's 35.65 megasample ceiling, so it should be legal. But there is
a concrete NVDEC report of failure on *square* 4096x4096, and **zero
VAAPI-on-Insta360 reports either way**:

- https://forums.developer.nvidia.com/t/nvdecoder-fails-to-decode-4096x4096-yuv/52187

Square resolutions are an under-tested path in every vendor's decoder.
Verify empirically before designing around it.

*Confidence: high on the level maths and on the absence of a demuxer;
medium on the practical hardware-decode risk.*

---

## 5. Open-source players and reframing tools worth stealing from

### Players

| Project | Stack | Status | Dual fisheye? | Linux |
| --- | --- | --- | --- | --- |
| [Bino](https://github.com/marlam/bino) | C++/Qt | active 2026-07-29, on Flathub | No, equirect and cubemap only | Yes |
| [greggman/equirect](https://github.com/greggman/equirect) | **Rust + wgpu + OpenXR**, MIT | active 2026-06-29 | No, equirect only | Yes |
| [gst-plugins-vr](https://github.com/lubosz/gst-plugins-vr) | C/GStreamer | **dead since 2018-01-29** | Partial | Yes |
| [libxcam](https://github.com/intel/libxcam) | C++/Vulkan/OpenCL | **unmaintained** | Yes, multi-fisheye stitch | Yes |
| Kodi | C++ | active | **No spherical support at all** | Yes |
| PotPlayer, DeoVR, Skybox, Whirligig, HereSphere | proprietary | active | varies | No |

`greggman/equirect` is the closest architectural reference for a Rust
build: MIT-licensed, wgpu, and it already solves window plus headset
presentation. `libxcam` is worth reading purely for its fisheye maths and
Vulkan stitching structure even though it is dead.

Also noted: https://github.com/faeton/insta360-quicklook (active
2026-04-26) is a macOS Finder preview that punts entirely and previews the
LRV proxy. It is a good reminder that the proxy exists and that using it
is legitimate for thumbnails.

*Confidence: high (repo metadata read from the GitHub API).*

### Keyframed reframing: the gap is smaller than expected

**Kdenlive ships the bigsh0t frei0r suite, and it does the whole job.**
The effect list under VR360 and 3D:

- Stereoscopic 3D
- VR360 Cap Top and Bottom
- **VR360 Equirectangular to Rectilinear**
- VR360 Equirectangular to Stereo
- VR360 Equirectangular Mask
- **VR360 Hemispherical to Equirectangular**
- VR360 Rectilinear to Equirectangular
- VR360 Stabilize
- VR360 Transform
- VR360 Wrap

Two of these matter enormously:

**VR360 Hemispherical to Equirectangular** (the stitcher). Its
documentation says "the plugin assumes that both hemispheres are in the
frame", so separate streams must be hstacked first, exactly the same
constraint as mpv360. What it exposes is far richer than
`v360=dfisheye`: Yaw, Pitch, Roll, **Lens FOV, Lens Radius, Front X/Y and
Back X/Y hemisphere centres, lens distortion A/B/C, vignetting A/B/C/D,
nadir radius and start, EMoR sensor response**, and interpolation choice.
That is a full manual calibration surface, which is precisely what the
plain ffmpeg route lacks.

- https://docs.kdenlive.org/en/effects_and_filters/video_effects/vr360_and_3d/vr360_hemi2equi.html

**VR360 Equirectangular to Rectilinear** (the reframer). Parameters:
interpolation (nearest-neighbour or bilinear), Yaw, Pitch, Roll, FOV, and
a Fisheye mix percentage. The documentation header states **"Keyframes:
Yes"**.

- https://docs.kdenlive.org/en/effects_and_filters/video_effects/vr360_and_3d/vr360_equi2rect.html
- Index: https://docs.kdenlive.org/en/effects_and_filters/video_effects/vr360_and_3d.html

So a working open-source, Linux-native, **keyframed** 360-to-flat
reframe-and-export pipeline exists today: hstack the tracks, apply
Hemispherical to Equirectangular with hand-entered calibration, then
keyframe Equirectangular to Rectilinear over time and export. This is the
Insta360 Studio reframe workflow, minus the AI stitching and minus gyro
stabilisation (though VR360 Stabilize exists as an image-analysis
substitute).

Shotcut ships the same bigsh0t plugins, with a reported keyframe-button
bug (January 2025).

Other reframing references:

- **reframe360XL** (https://github.com/Sn0wy0wl/reframe360XL): OFX
  plugin, CUDA/OpenCL/Metal, active 2025-11-19. Equirectangular input,
  keyframed virtual camera. Good source for the reframe UX vocabulary.
- **DaVinci Resolve Fusion Spherical Camera** node plus KartaVR for
  dual-fisheye ingest, on Linux Resolve. The canonical architecture:
  video-mapped sphere with a keyframed virtual camera inside it.
- Blender: nothing maintained for keyframed reframing of existing
  footage. The 360 tooling there is for rendering, not ingest.

*Confidence: high (Kdenlive docs read directly).*

### Rust building blocks

- **Gyroflow** (https://github.com/gyroflow/gyroflow): Rust, wgpu, QML.
  Proves the shape of a Rust GPU video tool with real lens maths. **Its
  Insta360 WGSL is GPL-3.0**, usable in this AGPL project with attribution
  and an SPDX header. Does not do 360 stitching.
- **AdrianEddy/gpu-video**: the Gyroflow author's extraction of the
  decode layer, README states "NOT READY YET".
- **iced_video_player** (https://github.com/jazzfool/iced_video_player):
  gstreamer-rs into a **custom wgpu render pipeline** with GPU-side YUV
  conversion. The closest reference for decode-to-texture on Linux in
  Rust, and directly relevant to an iced/COSMIC UI.
- **ffmpeg-next** and **gstreamer-rs** for demux and decode.

**No Rust crate does dual-fisheye dewarp or stitching, and no Rust binding
to Insta360's MediaSDK exists.**

*Confidence: high.*

---

## 6. The convert-first baseline

### Insta360's own MediaSDK runs natively on Linux

This is the headline of the section and the strongest existing path.

- https://github.com/Insta360Develop/Desktop-MediaSDK-Cpp: repo active,
  last pushed **2026-07-13**, 118 stars.

From the README, verbatim in substance:

- **Supported platforms: Windows, and "Ubuntu 22.04 (x86_64), other
  distributions need to be tested".**
- Supported cameras: ONE X, ONE R Twin, ONE X2, ONE RS 1-Inch 360, X3,
  **X4**, X5.
- File support: video `insv` in, `mp4` out; images `insp`/`jpeg` in, `jpg`
  out. Output size must be **2:1**.
- `SetStitchType(TEMPLATE | DYNAMICSTITCH | OPTFLOW | AIFLOW)`. Quality
  order AI > optical flow > dynamic > template; speed order is the exact
  reverse. AI stitching requires `SetAiStitchModelFile` with
  `ai_stitch_model_v1.ins` for pre-X4 material or
  `ai_stitch_model_v2.ins` for X5 material.
- `EnableStitchFusion` performs **chromatic calibration** across the seam,
  addressing the exposure and brightness mismatch between the two lenses.
- Stabilisation parameters, lens-guard accessory type, `EnableH265Encoder`
  with hardware acceleration (recommended above 4K output).
- It documents the dual-track case explicitly: "For X4 cameras, dual video
  track storage is currently used. Regardless of resolution, there is only
  one original video file."
- There is also a **real-time preview stitcher**
  (`ins_realtime_stitcher.h`, `RealTimeStitcher`) intended for live camera
  preview, producing 2:1 panoramic frames from a stream.

Access is gated behind a free application form at
https://www.insta360.com/sdk/apply. Version 3.x requires an NVIDIA GPU;
older sub-3.0.0 SDKs run CPU-only.

**Risk worth flagging for this project specifically: the supported-camera
table lists X4 but does *not* list X4 Air**, whereas Studio's own
documentation does list X4 Air among supported models. Either the SDK
table is stale or X4 Air is genuinely unsupported. Verify before relying
on this path for X4 Air material. *Confidence: high that the table omits
it; unknown whether that reflects reality.*

A working community wrapper exists, which removes most of the setup pain:

- https://github.com/syncom/insta360-cli-utils: 30 stars, last pushed
  **2026-06-06**. Docker image built around
  `libMediaSDK-dev_2.0-6_amd64_ubuntu18.04.deb` extracted from
  `LinuxSDK20241128.zip`, with an NVIDIA Container Toolkit path tested on
  an RTX 4060 Ti, and a CPU fallback path for machines without an NVIDIA
  GPU. Its README states plainly that as of early 2025 Insta360 Studio has
  not shipped a Linux version.

No Wine anywhere in this path.

*Confidence: high (both READMEs read directly).*

### Studio's batch export, if going the Wine or other-OS route

Insta360 Studio has an export queue: batch export to local storage has **no
quantity limit** and processes files one by one in the order added (cloud
export caps at 10). Hardware acceleration is toggled under Settings > User
Preference > Codecs > Exporter Codec. Supported models listed include X4
and **X4 Air**.

- https://onlinemanual.insta360.com/studio/en-us/operation-guide/file-management/file-export-instructions

*Confidence: high.*

### Cost of converting a 36 GB library

Source side, from Insta360's own figures and community measurement:

- X4 8K30 records at **200 Mbps**, up 67 percent from X3's 120 Mbps
  (https://www.insta360.com/blog/tips/insta360-x4-8k-5nm-ai-chip.html),
  which works out to roughly **1.5 GB per minute**.
- 5.7K is roughly **1 GB per minute**.
- Therefore **36 GB is about 24 minutes of 8K, or about 36 minutes of
  5.7K**.

Output side:

- Equirectangular 7680x3840 H.265 at a comparable bitrate roughly
  **doubles** storage: plus 18 to 36 GB. Budget around **75 GB** to keep
  sources and stitched output together. An edit-friendly higher bitrate
  pushes that toward 3x.

Time:

- Studio or MediaSDK with GPU encode is reported at roughly **0.5x to 2x
  realtime** on decent hardware: one RTX 3080 user reports "at minimum 2x
  export speed"; another reports a 30-minute 360 video exporting in 15
  minutes on a well-specified desktop.
- Weak hardware is catastrophic: a **60:1** report (one-minute clip takes
  about an hour), and a 2026 report of 1 h 12 m for a 900 MB file with
  only camera-angle changes applied.
- The ffmpeg `v360` route's only measured datapoint anywhere is a
  Gear360-class (4K) source at **2x realtime on an RTX 3060** with
  `h264_nvenc` doing the encode while `v360` remaps on CPU. Scaled to
  7680x3840 output (29.5 megapixels per frame) plus two 3840x3840 decodes,
  expect sub-realtime.
- `insv-stitch` is hours per clip by construction (2 s of process overhead
  per frame).

**Net: converting the whole 36 GB is a one-time job of roughly 30 to 90
minutes on a GPU, or overnight on CPU, for about 40 GB of extra disk.**
That is entirely viable. It is not a reason to avoid building a player,
but it is a reason the player is not *urgent*.

*Confidence: medium-high. The bitrate and file-size figures are vendor
and community consistent; the export-speed figures are second-hand
community reports spanning very different hardware, and none of them is
specifically X4 Air at 8K on Linux.*

---

## Summary table: what each path actually delivers

| | Play raw `.insv` | Zoom + drag reframe | Screenshot | Keyframed reframe export | Correct stitch | Stabilised horizon |
| --- | --- | --- | --- | --- | --- | --- |
| mpv + hstack + mpv360 | **Yes** (untested) | Yes | Yes (`screenshot window`) | No | No | No |
| VLC | No (one fisheye) | Equirect only | Yes | No | n/a | n/a |
| ffmpeg `v360` | Convert only | No | n/a | No | Approximate | No |
| insv-stitch | Stills only | No | n/a | No | **Yes** (22.5 dB) | Yes (per-unit) |
| Kdenlive bigsh0t | Convert or hstack first | Scrub only | Frame export | **Yes** | Manual calibration | VR360 Stabilize |
| Insta360 Studio (Wine) | Yes (GPU contested) | Yes | Yes | **Yes** | **Yes** | **Yes** |
| MediaSDK (native Linux) | Convert only | No | n/a | No | **Yes** | **Yes** |
| **Kjerag (proposed)** | **Yes** | **Yes** | **Yes** | later | **Yes** | **Yes** |

The bottom row is the case for the project: every existing row has a hole
in it, and no two rows combine without a conversion pass or a Windows
compatibility layer.
