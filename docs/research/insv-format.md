# The Insta360 `.insv` format, and what it takes to reframe it

Feasibility study, 2026-07-30. This is the format reference for Kyerag:
what an `.insv` actually contains, what the calibration numbers mean, what
the seam and the gyro cost you, and what was ruled out. Quote it to settle
disputes.

Everything here was derived from public sources (linked inline) plus one
real 30-minute Insta360 X4 Air capture. The parsed calibration from that
capture is checked in at `docs/research/x4air-calibration.json` with
serial number, GPS and capture times stripped; it is the fixture, and the
worked examples below use it.

## License context

Kyerag is **AGPL-3.0**. GPL-3.0 is one-way compatible with AGPL-3.0, so
Gyroflow's `distortion_models/insta360.wgsl` (GPL-3.0-or-later) is usable
as a **direct reference implementation with attribution**, not merely as
clean-room input. Files that derive from it carry their own SPDX header.
This matters for section 5: the projection math does not have to be
re-derived from the Mei paper, though the paper and OpenCV's `omnidir`
remain the better-documented description of the same model.

`telemetry-parser` is MIT OR Apache-2.0 and imposes nothing. Kyerag reads
the trailer itself (see 4.6 for why) but transcribes that project's
protobuf field tags, which is exactly what the licence permits.

## Confidence key

- **HIGH**: verified against the real fixture, or against two or more
  independent implementations that agree.
- **MED**: one credible source, or an inference with arithmetic shown.
- **LOW**: plausible reading, untested. Flagged inline.

---

## 1. Container

An X4-class `.insv` is a standard MP4. `ffmpeg` demuxes it without
complaint. Confirmed layout on the fixture:

- **Two video streams**, stream 0 and stream 1, one per lens. Each
  3840x3840 HEVC (`hvc1`), `yuvj420p` (full range), 29.97 fps, roughly
  90 and 78 Mbps.
- **AAC stereo** audio.
- A proprietary **trailer** appended after `moov`/`mdat`, ending at EOF.

Two other layouts exist in the wild and a general reader should detect
rather than assume (HIGH, from `vsezol/insta360-raw-viewer`, which ships
auto-detection for both): a **single video track with the two fisheyes
packed side by side**, and older or lower-tier models that write **two
separate files per capture**, named `VID_..._00_XXX.insv` and
`VID_..._10_XXX.insv`. Long captures may additionally be **chaptered at
the 4 GB SD-card limit**; `gyroflow/mp4-merge` exists specifically to
rejoin those, and its `src/insta360.rs` shows how the trailers are
merged.

The camera also writes an `LRV_*.lrv` proxy alongside each `.insv`:
H.264 instead of HEVC, 5 to 10x smaller, both lenses side by side in one
track (MED, from `faeton/insta360-quicklook`, which uses exactly this as
its playback source). Kyerag does **not** depend on it (see the decisions
log in ROADMAP.md): the proxy may be absent, so full-resolution decode
has to stand alone.

## 2. The trailer

**Confidence: HIGH.** Three independent implementations agree byte for
byte: `telemetry-parser` (Rust), exiftool's `ProcessInsta360` (Perl), and
`insvtools` (Java).

### Footer

The last **72 bytes** of the file:

```
padding[32] | extra_size u32 LE | version u32 LE | magic[32]
```

- `magic` is the ASCII string `8db42d694ccc418790edff439fe026bf`.
- `extra_size` sits at EOF-40 and covers the **whole trailer including
  the footer**, so `extra_start = filesize - extra_size`.
- `version` is `3` on current cameras. `insvtools` throws on anything
  else.

exiftool reads 78 bytes and unpacks the size at offset 38, which lands on
the same byte. `insvtools` names the first 32 bytes `unknownBuf` and
copies them through verbatim when rewriting, which is the safe treatment.

### Records

Records are stored **backwards from the footer**. Each is:

```
payload[size] | format u8 | id u8 | size u32 LE
```

The 6-byte header trails its own payload, so a sequential reader walks
from the end: read 6 bytes, seek back `size`, read the payload, repeat.

`format`: 0 = binary, 1 = protobuf, 2 = JSON.

**Record id 0 is an index table** and should be used instead of walking.
Its payload is a series of 10-byte entries:

```
id u8 | format u8 | size u32 LE | offset u32 LE
```

where `offset` is relative to `extra_start`. exiftool unpacks `format`
and `id` as a single little-endian u16, which is why its documentation
and warnings say `0x300` and `0x700` where everything else says 3 and 7.
Same bytes, different reading.

### Record type ids

From `telemetry-parser/src/insta360/record.rs`:

| id | name | contents |
|----|------|----------|
| 0 | Offsets | the index table above |
| 1 | **Metadata** | protobuf `ExtraMetadata`, holds the calibration |
| 2 | Thumbnail | a single H.264 frame |
| 3 | **Gyro** | IMU samples, two encodings (section 8) |
| 4 | **Exposure** | `{u64 ts_us, f64 shutter}` per sample, lens 0 |
| 5 | ThumbnailExt | a single H.264 frame |
| 6 | TimelapseTimestamp | `u64` timestamps |
| 7 | GPS | 53 bytes per fix |
| 8 | StarNum | unknown, 11 bytes per record |
| 9 | AAAData | 48 bytes: EV target, exposure time, bit-packed ISO and luma stats |
| 10 | Anchors | highlight markers |
| 11 | AAASimulation | unknown |
| 12 | **ExposureSecondary** | same shape as 4, lens 1 |
| 13 | Magnetic | unknown |
| 14 | Euler | unknown |
| 15 | SecGyro | unknown |
| 16 | Speed | unknown |
| 17 | TBox | unknown |
| 18 | Quaternions | unknown |
| 128 | TimeMap | trim/timelapse time remapping |

Which ids are present is model-dependent. exiftool's commented-out
`%insvDataLen` table notes ids seen only on the X3, and others seen only
on the Ace Pro.

**Records 14 and 18 are the interesting gap.** If either is populated,
it is a pre-integrated orientation track, and horizon lock skips gyro
integration and drift entirely. Nobody has decoded them. Cheap to check:
`gyro2bb --dump` prints `Unknown Insta360 record: 18` with a hex dump if
it is there.

## 3. The metadata protobuf

Record 1 is a protobuf message, roughly 65 fields. Two public schemas:

- `telemetry-parser/src/insta360/extra_info.rs` (prost annotations, the
  most complete)
- `ke4ukz/insvdump/extra_metadata.proto` (a plain `.proto`, easier to
  read)
- `iliakonnov/insta360-nas/.../insta360_messages.proto` is a superset
  scraped from the mobile app and exposes fields nothing else mentions,
  including `offset_v6`, `offset_v8`, and a
  `CameraOffsetCalib { offset_v3_cam0, offset_v3_cam1, *_refined,
  offset_v7_* }`. All undocumented.

Fields Kyerag cares about, with fixture values:

| field | tag | X4 Air fixture | why it matters |
|---|---|---|---|
| `camera_type` | 2 | `Insta360 X4 Air` | selects the IMU orientation table. See the trap in 4.6. |
| `fw_version` | 3 | `v1.2.7_build1` | calibration grammar has changed across firmware generations |
| `offset` | 5 | 16 floats | v1 calibration, legacy |
| `offset_v2` | 53 | 34 floats | v2 calibration, legacy |
| **`offset_v3`** | 54 | **40 floats** | the model Kyerag uses |
| `original_offset*` | 17/55/56 | identical to the above | factory vs adjusted (see 4.5) |
| `dimension` | 19 | 3840 x 3840 | per-lens delivered frame size |
| `window_crop_info` | 27 | src 7680x7680, dst 7424x7424 | sensor crop; feeds the focal scale |
| `first_frame_timestamp` | 24 | 3848400 (us) | clock origin |
| `rolling_shutter_time` | 25 | **15.883 ms** | row readout time |
| `gyro_timestamp` | 28 | 1.6 | gyro-to-video offset |
| `is_has_gyro_timestamp` | 29 | true | whether to apply it |
| `is_raw_gyro` | 62 | **true** | selects the gyro encoding AND a timebase divisor |
| `gyro_cfg_info` | 65 | acc 32, gyro 2000 | full-scale ranges (+/-32 g, +/-2000 dps) |
| `gyro_calib` | 31 | 6 doubles | IMU bias, probably 3 gyro + 3 accel (MED) |
| `gyro_type` | 51 | `InsdevImuType40609` | IMU part identifier |
| `is_flowstate_online` | 42 | **false** | confirms frames are NOT stabilized |
| `is_dewarp` | 43 | false | confirms frames are raw fisheye |
| `pts_type` | 64 | **2 = `VideoPtsEexposureFile`** | see 8.3 |
| `media_data_rotate_angel` | 52 | Unknown | frame rotation, if ever nonzero |
| `cam_posture` | 46 | `CameraRotate0` | camera held upright |
| `total_time` | 10 | 1798 (s) | |
| `user_options.offset_convert_states` | 13.10 | null | lens guard / dive case, see 4.5 |

`fov_type`, `fov` and `distance` are capture-UI settings, not
calibration. Do not feed them to the projection.

## 4. `offset_v3`: the lens calibration

**This is the single most valuable thing in the file**, and the reason a
correct reframing player is possible at all without Insta360's SDK.

### 4.1 Grammar

**Confidence: HIGH.** All `offset*` fields are ASCII strings of
underscore-separated decimals. `offset_v3` parses as:

```
<lens_count> , (19 fields) x lens_count , <version_word>
```

where each 19-field lens block is, in order:

```
xi, fx, fy, cx, cy, yaw, pitch, roll, tx, ty, tz,
k1, k2, k3, p1, p2, calib_w, calib_h, lensType
```

The field order comes from a single comment line by AdrianEddy in
`telemetry-parser/src/insta360/mod.rs`, added in commit `9035a83`
(2022-12-14). That comment, plus the code that consumes it, **is** the
public documentation. There is no vendor document, no PTGui or Hugin
thread, and the official Media SDK treats the string as opaque
passthrough.

Note that AdrianEddy's comment reads
`num_xi_fx_..._lensType_flag`, i.e. 21 names, because Gyroflow only ever
builds a profile for lens 0 and slices indices 0 through 20. On a real
two-lens string, index 20 is not a `flag`; it is **lens 1's `xi`**. The
off-by-one is harmless upstream because the value is discarded, but do
not copy the slicing.

### 4.2 Worked example (X4 Air fixture)

40 tokens = 1 + 19 + 19 + 1.

```
lens_count = 2

lens 0:  xi 2.31494   fx 7087.49   fy 7090.35
         cx 3837.88   cy 3854.42
         yaw -0.103   pitch -0.07   roll 90.534
         tx 0.0       ty 0.0        tz 0.0
         k1 0.95820886   k2 -1.80141151   k3 3.57555127
         p1 -0.0007338   p2 -0.00115458
         calib_w 15360   calib_h 7680   lensType 131

lens 1:  xi 2.31494   fx 7099.03   fy 7097.43
         cx 11550.7   cy 3870.18
         yaw 0.039    pitch -0.193   roll 89.076
         tx -0.002063 ty 0.000334   tz -0.033284
         k1 0.97158086  k2 -2.08655882  k3 4.30578518
         p1 -0.0019249  p2 0.00054564
         calib_w 15360  calib_h 7680  lensType 131

version_word = 197632
```

Independent corroboration on a non-Air X4 (firmware v1.4.8), from
`Ivan1248/irap-tools`: same 40-token shape, `lens_count = 2`,
`xi = 1.94817`, `fx ~ 4623.8`, `roll` 90.493 and 89.661,
`tz = -0.03132`, `lensType 71`, `calib_w 16000`, `calib_h 6000`, same
`version_word = 197632`. The grammar holds across both; the numbers do
not, so **never hardcode a calibration, always read it from the file**.

A second corroboration, this one first-hand: Kyerag read an
`Insta360 ONE X2` capture (firmware `v1.0.62_build2`) on 2026-07-30. Same
40-token shape, `lens_count = 2`, `xi = 1.72859`, `tz = -0.021103`
(21.1 mm, a smaller body), `lensType 41`, canvas 6080 x 3040 against a
2880 x 2880 delivered frame, `rolling_shutter_time` 23.52 ms, and
`is_raw_gyro = false` so the older 56-byte gyro encoding. Lens 0's
translation is exactly zero there too, which is the strongest single
check that the field order is being read correctly. Note that the canvas
slot (3040) is **wider than the delivered frame** (2880) on this camera,
where on the X4 Air it is narrower: nothing may assume the ratio's
direction.

### 4.3 What the numbers mean

**`calib_w`/`calib_h` describe the side-by-side PAIR canvas, not one
lens** (HIGH, arithmetic below). On the fixture the canvas is
15360 x 7680, i.e. two 7680 x 7680 lens images side by side. Therefore:

- **`cx` is in pair-canvas coordinates.** Lens 0 occupies x in
  [0, 7680); lens 1 occupies x in [7680, 15360). Lens 1's
  `cx = 11550.7` is `7680 + 3870.7`.
- Scale to the delivered 3840 x 3840 frame with
  `3840 / (calib_w / 2) = 0.5` for x and `3840 / calib_h = 0.5` for y.

Check, lens 0: `3837.88 * 0.5 = 1918.9` and `3854.42 * 0.5 = 1927.2`,
both within ~8 px of the 1920 frame center. Lens 1, after subtracting the
half canvas: `(11550.7 - 7680) * 0.5 = 1935.4`. Both sane.

**The focal scale is not the same ratio**, because the camera crops the
sensor: `window_crop_info` says the 7680-wide calibration canvas is
delivered as a 7424-wide window, so `fx_px = fx * 3840 / 7424 = fx *
0.51724`. On the fixture that is `7087.49 * 0.51724 = 3665.9`. The
principal point could be scaled the same way, with the crop origin
`(7680 - 7424) / 2 = 128` subtracted first:
`(3837.88 - 128) * 0.51724 = 1918.7`. That agrees with the simpler form
to a quarter of a pixel, because `cx` sits near the canvas center. Either
is fine; be consistent.

**`t` is in metres, and `tz` is the inter-lens baseline.** Lens 0 is the
reference at (0,0,0). Lens 1's translation is dominated by z:
`|tz| = 0.033284 m = 33.3 mm`. The non-Air X4 gives 31.3 mm. This is the
number that sets parallax (section 6), and it is in the file, which is
better than every published estimate.

**The 180-degree back-to-back flip is NOT in the extrinsics.** Lens 1 has
`yaw = 0.039`, not 180. The `yaw`/`pitch` values are sub-degree mounting
tolerances and the extrinsics are **residuals against a nominal opposed
arrangement**. Applying them as absolute poses points both lenses the
same way. This is the easiest way to get lens 1 catastrophically wrong.

**`roll` is a deliberate sensor rotation** plus tolerance: about 90
degrees on both X4 variants, about 180 and 0 on a ONE X2 (HIGH, 4.7). It
is not a constant to assume.

**The `version_word` encodes the offset version** (MED, two data points
across two cameras): `132096 = (2 << 16) | 0x400` on `offset_v2`, and
`197632 = (3 << 16) | 0x400` on `offset_v3`. Useful as a sanity check
when sniffing an unknown string.

### 4.4 The older grammars

Present in the fixture, not used by Kyerag, recorded so a future reader
can identify a string by token count (MED, inferred from the fixture):

- **`offset` (v1)**, 16 tokens: `count`, then 6 fields per lens
  (`f, cx, cy, yaw, pitch, roll`), then a shared
  `calib_w, calib_h, 1155`. No distortion terms at all.
- **`offset_v2`**, 34 tokens: `count`, then 16 fields per lens
  (`f, cx, cy, yaw, pitch, roll, tx, ty, tz, <1.0>, k1, k2, k3,
  calib_w, calib_h, lensType`), then the version word. One focal instead
  of `fx`/`fy`, three radial coefficients, **no `xi` and no tangential
  terms**, i.e. a different and weaker camera model.

The progression v1 to v2 to v3 is a progression in model richness, not
just precision. Use `offset_v3`.

### 4.5 `offset` vs `original_offset`

Unresolved upstream. AdrianEddy says outright that he does not know
(`AdrianEddy/telemetry-parser` issue 35). Best available reading (MED):
`original_*` is factory calibration and `offset*` is calibration after
`ExtraUserOptions.offset_convert_states`, an enum of
`WaterProof, DivingWater, DivingAir, StitchOptimization, Protect,
SphereProtect, FpvProtect`. **Lens guards and dive cases change the
optics and therefore the calibration.** On the fixture the two are
byte-identical and `offset_convert_states` is null, which is consistent
with a bare camera and does not distinguish the hypotheses.

Kyerag should read `offset_v3`, not `original_offset_v3`: if they ever
differ, the adjusted one describes the glass that was actually in front
of the sensor.

### 4.6 Three telemetry-parser bugs that bite the X4 Air specifically

**Confidence: HIGH.** The first two derived by reading
`telemetry-parser/src/insta360/mod.rs` against the fixture's
`camera_type` of `Insta360 X4 Air`; the third measured on real footage.

1. **Wrong IMU orientation.** The orientation table matches
   `Some("Insta360 X4")` and `Some("Insta360 X5")` exactly. `"Insta360
   X4 Air"` matches neither and falls through to the default `"Xyz"`,
   where the X4 wants **`"yzX"`**. A wrong orientation string tilts or
   mirrors the horizon.
2. **Principal point off by 2x.** `insert_lens_profile` strips the
   `"Insta360 "` prefix and applies `cx_fix = 2.0` only when the result
   is exactly `"X4"` or `"X5"`. `"X4 Air"` matches neither, so
   `c_ratio.x` becomes `3840 / 15360 = 0.25` and `cx` lands at 959.5
   instead of 1918.9.

3. **The published crate aborts on this footage.** Version 0.2.6, the
   newest on crates.io, serializes `ExtraMetadata`'s enum fields with
   `unsafe { std::mem::transmute }` of the raw `i32`. The X4 Air capture
   carries a 15 in a field whose enum stops below it, and the process
   dies with `trying to construct an enum from an invalid value 0xf`
   (measured 2026-07-30 against `~/Videos/VID_*.insv`). Upstream master
   replaced the transmute with a `try_into` that falls back to the raw
   integer, which is why the checked-in fixture shows `audio_mode` as the
   number 9 rather than a name.

Kyerag reads `offset_v3` directly rather than consuming the synthesized
Gyroflow lens profile, so the first two bugs are not on its path either.
Bug 3 is the one that decided the design: master is unpublished and pulls
two further git forks, so Kyerag walks the trailer itself
(`crates/meta/src/trailer.rs`, section 2 above) and decodes record 1 with
`prost`. All three are worth upstreaming.

A third trap, not a bug because Gyroflow never hits it: the synthesized
profile is **lens 0 only**, and `cx * c_ratio` does not subtract the
half-canvas offset. Feed lens 1's `cx` through it and you get 5775
instead of 1935.

### 4.7 Slot 8 is roll, not half_fov (settled 2026-07-30)

**Settled by a second camera.** The `Insta360 ONE X2` capture in 4.2 puts
**-179.717** in slot 8 for lens 0 and 0.963 for lens 1, in a block whose
other fields all read correctly (lens 0's translation is exactly zero,
the canvas agrees between blocks). A half-FOV cannot be negative, so slot
8 is `roll`. It also shows the value is not "about 90" in general: this
camera's lenses are rolled about 180 degrees apart where the X4 Air's are
90.534 and 89.076.

How to compose it is 4.8, which the first rendered frames settled.

The original reasoning, kept because it is the argument for the X4 Air
specifically: an independent X5 reverse-engineer labels slot 8 `half_fov`
rather than `roll`, and both readings fit a value near 90.

The evidence favours `roll`: a half-FOV of 90.53 degrees means a
181-degree lens, which contradicts the ~200 to 204 degree FOV that
everyone who fits the value empirically arrives at; and the 1.5-degree
difference between the two lenses (90.534 vs 89.076) reads as mounting
tolerance rather than an optical spec. `yaw` and `pitch` being sub-degree
in the same block supports "these are tolerances, and 90 is a deliberate
sensor rotation".

Gyroflow uses it as roll, and so does Kyerag.

### 4.8 Composing yaw, pitch and roll (settled 2026-07-31)

**Confidence: HIGH**, and this is the first thing in this document
verified against pixels rather than against other people's source. It was
settled during the first shader bring-up (issue #3), which is the job
section 10 assigned to it.

The rotation that renders a lens upright is

```
ray_lens = Rz(roll - 90 deg) * Ry(yaw) * Rx(pitch) * ray_body
```

in a right-handed frame whose axes are the delivered frame's own: **x
right, y down, z out along the optical axis**, with the image plane in the
ordinary photographic sense (`px = fx*x' + cx`, `py = fy*y' + cy`).

**The 90 degree datum is the finding.** Applying `roll` as the file writes
it renders the world a quarter turn on its side. Kyerag carries the
correction as `ROLL_DATUM_DEG` in `crates/render/src/projection.rs`.

### The frames that settled it

The three candidates the shader could apply differ by a rotation about the
lens axis, so at yaw and pitch 0 they are rotations of one square output,
and one frame per candidate decides between them. Two cameras were used
because their rolls are nothing alike: an X4 Air at 90.534 and a ONE X2 at
-179.717, so a rule that fits both is a rule about the convention rather
than about one camera.

| rotation applied | X4 Air, roll 90.534 | ONE X2, roll -179.717 |
| --- | --- | --- |
| `roll` | quarter turn, world up to the left | quarter turn, world up to the left |
| `-roll` | quarter turn, world up to the right | 0.57 degrees from the row above |
| `0` | **upright** | quarter turn, world up to the right |
| `roll - 90` | **upright** | **upright** |

Every cell was rendered and looked at except the ONE X2's `-roll`, which
on that camera is 0.57 degrees from `roll` and therefore the same picture.
Note what the table says about doing the easy thing: dropping roll happens
to be right for an X4 Air and is a quarter turn wrong on a ONE X2.

The plumb references, chosen because they are level or vertical by
physics rather than by eye:

- **X4 Air**: a paramotor pilot seated in his harness, 65 degrees off the
  lens axis, with his wing overhead. A pilot hanging under a wing is a
  plumb bob, and the harness cannot be at 90 degrees to gravity in level
  flight. The horizon crossing the same frame agrees.
- **ONE X2**: a river valley. A water surface is level by definition, and
  the windsock mast on the far bank is vertical.

**There is no mirror.** Text on the wing ("OZONE", printed along the
span) reads the right way round in a reframe after a pure rotation, which
rules out the y-up reading of the image plane: that reading differs from
this one by a reflection, and a reflection would have shown up as
reversed lettering.

Reproduce any row with the headless instrument, which runs the app's own
pass and writes a PNG:

```sh
cargo run --release -p kyerag-spike --bin reframe -- <file.insv> \
  yaw=-24 pitch=-63 fov=60
```

PNGs land in `scratch/`, which is gitignored. They are frames of somebody's
real flights and this repo is public: they stay local.

### What 4.8 does not settle

- **The order of the three angles.** Both test cameras have sub-degree yaw
  and pitch (0.103 and 0.07 on the X4 Air), and near the axis the model's
  effective focal length is `fx / (1 + xi)` = 1106 px/rad, so every
  ordering agrees to about 2 px. A camera with a large yaw or pitch would
  tell them apart; none is known to exist.
- **Where the 90 degrees comes from.** Two readings fit the same numbers:
  the roll is measured from the delivered frame's horizontal axis rather
  than its vertical, or the camera delivers the sensor image already
  rotated a quarter turn (`media_data_rotate_angel`, field 52, reads
  `Unknown` on the fixture and may be where that is stated). Nothing in
  the file distinguishes them and nothing downstream cares, so it is
  recorded rather than guessed at.
- **Lens 1.** This was lens 0 only. The nominal opposed arrangement that
  4.3 says is *not* in the extrinsics still has to be composed with these
  angles, and that is the seam's problem (issue #7).

## 5. The projection model

### 5.1 It is the Mei/UCM unified camera model

**Confidence: HIGH.** The presence of `xi` alongside Brown-Conrady
`k1..k3, p1, p2` identifies it as the unified omnidirectional camera
model of C. Mei and P. Rives, *Single view point omnidirectional camera
calibration from planar grids*, ICRA 2007. OpenCV implements it as
`cv::omnidir` in `opencv_contrib/modules/ccalib`.

Forward map, from a 3D ray to a normalized image point:

```
p  = normalize(ray)
x  = p.x / (p.z + xi)
y  = p.y / (p.z + xi)
r2 = x*x + y*y
x' = x*(1 + k1*r2 + k2*r2^2 + k3*r2^3) + 2*p1*x*y + p2*(r2 + 2*x*x)
y' = y*(1 + k1*r2 + k2*r2^2 + k3*r2^3) + 2*p2*x*y + p1*(r2 + 2*y*y)
```

then through the camera matrix to pixels. Gyroflow's
`src/core/stabilization/distortion_models/insta360.wgsl` is this in 20
lines of WGSL, and is a usable reference under our license.

**Only the forward direction is on Kyerag's path.** Reframing asks "for
this output pixel's view ray, which source pixel do I sample", which is
exactly what the forward map answers. Gyroflow's 200-iteration Newton
`undistort_point` solves the other direction and is not needed.

### 5.2 Why `xi` is the whole point

With `x = Xs / (Zs + xi)` and `xi > 0`, rays with **negative** `Zs`, i.e.
past 90 degrees off-axis, still project to finite coordinates as long as
`Zs + xi > 0`. A ray 100 degrees off-axis has `Zs = -0.174`, and with
`xi = 2.31` the denominator is comfortably positive.

An equidistant or equisolid fisheye model **cannot represent the overlap
region at all**, which is precisely the region a stitch depends on. This
is why ffmpeg's `v360=dfisheye` can never match Insta360 (section 7.1),
and why an OpenCV `cv2.fisheye.undistort` on Insta360 parameters produces
a still-circular image: a user demonstrated exactly that failure in
`gyroflow/gyroflow` issue 848.

### 5.3 The distortion is not a small correction

Fixture coefficients: `k1 = 0.958`, `k2 = -1.801`, `k3 = 3.576`. The
non-Air X4: `0.386, 1.303, -3.933`. These are order 1 to 4, and the
polynomial dominates near the frame edge, which is exactly where the
seam is.

Two consequences. Treating the lens as "equidistant plus a nudge" is
wrong where it matters most. And `k1..k3` here are **Brown-Conrady on the
Mei normalized plane**, not OpenCV-fisheye theta-polynomial coefficients;
feeding them to `cv2.fisheye` maps the image edge to about 35 degrees
instead of about 90.

Measured benefit of getting this right: switching an equidistant model to
UCM with `xi` improved MAE against Insta360 Studio ground truth from
18.32 to 16.80, roughly 8 percent, visible as clean rather than doubled
edges on powerlines and fence mesh (`BenjaminHenriksson/insv-stitch`,
`old/FINDINGS.md`).

## 6. The seam

### 6.1 Parallax is bounded, and computable from the file

Baseline `b = 33.3 mm` (fixture `tz`). Angular disparity is
approximately `b / d`. At a 1920-pixel-wide view with a 90 degree
horizontal FOV, that is 21.3 px per degree:

| subject distance | disparity | pixels |
|---|---|---|
| 0.5 m | 3.8 deg | 81 |
| 1 m | 1.9 deg | 41 |
| 3 m | 0.64 deg | 14 |
| 10 m | 0.19 deg | 4 |

Insta360's own X4 manual says: *"Keep subjects at least 3.3ft (1m) away
from the camera to reduce the visibility of stitch lines."* Community
figures put the X3 near 0.6 m, the X4 near 0.8 m, and an X4 with lens
guards past 1 m.

For the footage Kyerag targets, almost everything is past 3 m and this is
close to a non-issue. Thin near-field structures are the exception, and
they are exactly what ghosts worst.

### 6.2 Reframing wins by avoidance, not by blending

A 1920-pixel view at 90 degrees horizontal FOV has the **same angular
pixel density** as a 7680 x 3840 equirect, so a blend band buys nothing
per degree. What reframing buys is that the seam is a single great circle
at longitude +/-90 degrees:

- a 90-degree view centred on a lens axis contains **no seam at all**;
- roughly 50 percent of yaw-uniform 90-degree views contain a seam edge,
  about 33 percent at 60 degrees;
- an equirect export has the seam in every frame, baked into the encode.

The overlap budget is hard-bounded at `FOV - 180`, about 20 degrees at a
200-degree lens, so +/-10 degrees around the seam. Paul Bourke:
*"Fisheye angles of 190 degrees or more are required for a satisfactory
blend zone, 10 degrees."*

**Corollary worth designing in early:** when the view lies inside one
hemisphere, the second stream contributes nothing and does not need to be
decoded. That halves the decode budget for the majority of view
directions. (Tracked as hemisphere-aware decode gating in M2.)

### 6.3 Exposure mismatch is the artifact that will actually bite

Insta360's own SDK documentation states it plainly: *"since the two
lenses operate independently, their respective video exposures may not
align perfectly... noticeable brightness differences can occur."*

**The file carries per-lens, per-frame shutter speed**: record 4 for one
lens and record 12 for the other, identical shape
`{u64 ts_us, f64 shutter}`. Nothing in the open-source landscape uses
this; Kyerag can.

Gotcha: `telemetry-parser` merges records 4 and 12 into the same
`GroupId::Exposure` key, so record 12 overwrites record 4 in its output.
Parse them separately.

Correction should be a **symmetric split**, `front *= 1/sqrt(g)` and
`back *= sqrt(g)` for gain ratio `g`, with a spatially smooth gain field,
so neither hemisphere gets a visible step.

### 6.4 The rest of the artifact budget

Ranked for a player that samples one lens for most of the frame:

1. **Exposure and white balance step at the seam.** Fixable
   analytically from metadata, per above.
2. **Vignetting rolloff.** Lands exactly on the blend band. Coefficients
   are **not** in the metadata; needs flat-field calibration if it bites.
   The best ffmpeg-based workaround in the wild carries the same caveat:
   *"will not correct for lens vignetting darkened stitch edge."*
3. **Parallax ghosting.** Only under about 3 m, only in seam-containing
   views.
4. **Geometric error from a wrong lens model.** About 8 percent MAE.
5. **Chromatic aberration.** Measured as **zero** on X-series; applying
   CA correction made PSNR worse. Skip it.

### 6.5 Rolling shutter is not optional

`rolling_shutter_time` on the fixture is **15.883 ms**. At 3840 rows,
typical handheld motion displaces 12 to 18 px during readout, the **same
magnitude as the parallax above**, but unlike parallax it is fully
correctable from the gyro. On a vibrating airframe it dominates.

Correct it in the same backward mapping as everything else; see 8.

### 6.6 Optical flow does not help

Insta360's `INSOpticalFlowType` is `{DynamicStitch, Disflow, AiFlow}`;
Disflow is literally OpenCV `DISOpticalFlow`, AiFlow adds a learned
model. Measured against Studio ground truth
(`BenjaminHenriksson/insv-stitch`, one frame):

| method | MAE | PSNR |
|---|---|---|
| **distance-transform blend, no flow** | **18.32** | **18.72** |
| flow + hard seam (7 px feather) | 31.18 | 12.44 |
| flow + smooth distance transform | 18.38 | 18.68 |
| multi-band (Laplacian pyramid) | 18.34 | 18.71 |

Sub-pixel parallax at distance is below the flow's resolution, smooth
blending already masks moderate parallax, and flow introduces artifacts
where it is inaccurate. **A static calibrated warp is correct for a
player.**

The leverage is entirely in (a) using the file's real calibration and
(b) a good weight field. The recommended field is
`w = longitude_preference * coverage_depth`, where coverage depth is a
distance transform from each lens's validity boundary. That needs no
hardcoded feather width and automatically down-weights the vignetted
circle edge.

## 7. What was ruled out

### 7.1 ffmpeg `v360=dfisheye`

**Confidence: HIGH**, read from `vf_v360.c`. `xyz_to_dfisheye()` uses
`acosf(fabsf(vec[2])) / M_PI`, i.e. **strictly equidistant**; picks the
lens with a hard branch on the sign of `vec[2]`, i.e. **no blending at
all**; and clamps out-of-range samples rather than masking them, i.e.
edge smear. There is exactly one global rotation, so **no per-lens
yaw/pitch/roll**. No `xi`, no distortion polynomial. `alpha_mask` is a
no-op for dfisheye input and `in_pad` is cubemap-only. `ih_fov`/`iv_fov`
default to 180, where the X-series wants roughly 200 to 204.

`peterbraden/insv-to-yt` documents the result honestly: *"a nasty visible
join between the two 180 videos"*, and ships a `media/bad-join.png`.

The best ffmpeg-only workaround abandons `dfisheye` entirely and runs
`v360=fisheye` twice with per-lens rotations plus a generated blend map
and `maskedmerge`, which is the shader architecture done the hard way.

`v360` remains useful as a **baseline to diff against**, not as a
target.

### 7.2 The official Insta360 Media SDK

**Rejected.** Recorded here so it stays rejected.

It is real: `Insta360Develop/Desktop-MediaSDK-Cpp` (docs and demo source
only, no binaries). Linux ships as a ~238 MB `.deb` built for **Ubuntu
22.04 x86_64 only**, containing `libMediaSDK.so` (317 MB, unstripped,
~68k mangled C++ exports), a `MediaSDKTest` CLI, and
`ins_stitcher.h` / `ins_realtime_stitcher.h`. Its documentation confirms
our container reading: *"For X4 cameras, dual video track storage is
currently used. Regardless of resolution, there is only one original
video file."*

Why it is out:

- **Access.** Application form plus a handwritten-signature NDA plus your
  camera's serial number. **Redistribution prohibited**, so every
  downstream project is bring-your-own-`.deb`. No EULA ships in the
  package and no non-commercial grant exists. Incompatible with a public
  AGPL repo.
- **The architecture is wrong for a player.** `VideoStitcher` is
  fire-and-forget: `SetInputPath` then `StartStitch()` then a file on
  disk. **No seek, no random-access frame API.** Scrubbable playback
  would mean pre-transcoding every 30-minute capture.
- **Hygiene.** It bundles a stale NVIDIA `libcuda.so.1` from 2020 that
  shadows the real driver, and a Tencent codec library whose strings
  include cloud API endpoints and the EC2 instance-metadata address.
  Compiled against CUDA 8.0.61.
- **FFI cost.** Pure Itanium C++ ABI with `std::string`, `std::vector`
  and `std::function` in signatures, and **no exported destructor** for
  the stitcher classes, so a C++ shim translation unit is mandatory;
  bindgen alone cannot do it.

The one experiment that could reopen this: `RealTimeStitcher` exposes
`HandleVideoData` / `HandleGyroData` with an RGBA callback, but its
`SetCameraInfo` wants a `CameraInfo { std::vector<std::string> offset; }`
normally obtained from a physically connected camera. Whether that can be
synthesized from the trailer's offset strings and fed demuxed file data
is undocumented and untried. Odds: LOW to MED. It would be the only path
to Studio-grade stitching inside a player, and it is still gated behind
the NDA.

Note also that the two GitHub projects that advertise official-SDK Linux
support are thin: `GHSLAB/Vision360-Toolbox` is a Tkinter GUI that
subprocesses the official demo binary, and
`umutcantr/Insta360-INSV-Pro-Converter` is AI-generated (two commits 74
seconds apart, a `main.cc` byte-identical to Insta360's official demo,
and its own source mixing two incompatible SDK API generations, so it
cannot compile). The genuinely useful references are
`pdxmusic/insta360sdk` and `mjmaurer/infra`'s Nix derivation.

## 8. Gyro, orientation and clocks

### 8.1 The frames are not stabilized

Confirmed three ways: `is_flowstate_online = false` and
`is_dewarp = false` on the fixture, and Insta360's own support
documentation, which says 360 mode records raw and applies FlowState in
the App or Studio at export time. Horizon lock is part of that
export-time pass.

For Kyerag this is not a nice-to-have. Without it, a reframed view
inherits every roll and yaw of a camera on a moving mount, and the same
gyro pipeline carries rolling-shutter correction (6.5).

### 8.2 Gyro encoding

Record 3, two encodings selected by `is_raw_gyro`:

- **`is_raw_gyro = true`** (the fixture): 20 bytes per sample,
  `u64 ts_us` then **six u16 with a -32768 bias**, accel triple first,
  then gyro. Scale by `32768 / gyro_range` for deg/s and
  `32768 / acc_range` for g. Fixture ranges are **+/-32 g and
  +/-2000 dps**; note that 32 is not the +/-16 g default that
  `telemetry-parser` falls back to, so read `gyro_cfg_info`, never
  assume.
- **`is_raw_gyro = false`**: 56 bytes per sample, `u64` then six f64,
  accel first in g, gyro in rad/s.

### 8.3 The clock chain

Three clocks, and this is the correctness minefield. From
`telemetry-parser/src/insta360/mod.rs`:

```
t  = raw_timestamp_us / 1000
t -= first_frame_timestamp / 1000
if is_raw_gyro:  t /= 1000          // yes, a second division
if is_has_gyro_timestamp:  t -= gyro_timestamp / 1000
```

Exposure records get the same treatment minus the gyro offset.

One more wrinkle from the fixture: **`pts_type = 2 =
VideoPtsEexposureFile`**, which by the enum's own name means the
authoritative frame timestamps come from the exposure records rather than
container PTS (MED, inferred from the enum name; worth confirming by
comparing the two on real footage). If true, the exposure track is not
just a brightness signal, it is the frame clock.

The failure mode for all of this is a **swimming horizon, not a crash**.
Build the diff-against-Studio-export harness before trusting any of it
(M2).

### 8.4 IMU orientation

`telemetry-parser` encodes the axis convention as a three-character
string, lowercase meaning a negated axis, selected by a **two-dimensional
lookup**: camera model **crossed with** whether `offset_v3` is present.
With `offset_v3`, the X4 and X5 want `"yzX"`; without it the table is
entirely different (the ONE X2 wants `"xZy"`, default `"yXZ"`). Get the
two-dimensional part wrong and the horizon tilts the wrong way. See the
X4 Air fall-through bug in 4.6.

The gyro and accel streams are then additionally rotated by the lens
`yaw`/`pitch`/`roll` from `offset_v3`.

**Code-reading trap:** the Euler-to-matrix block in `mod.rs` has
**swapped variable names**, `let (sr, cr) = (yaw.sin(), yaw.cos())` and
`let (sy, cy) = (roll.sin(), roll.cos())`. Read the arithmetic, not the
identifiers.

## 9. Prior art worth reading

- **`BenjaminHenriksson/insv-stitch`** (MIT, Python, X5) is by far the
  most valuable. ~1200 lines, reaches PSNR 22.5 to 22.9 dB against Studio
  at 7680 x 3840. Its `PIPELINE.md`, `old/FINDINGS.md` and
  `x5_pipeline.md` are the source for the measured numbers in sections 5
  and 6. Its design principle is Kyerag's: *"stitching, stabilization,
  rolling shutter, undistortion fused into a single backward-mapping per
  output pixel. No double-resampling."*
- **`AdrianEddy/telemetry-parser`** (MIT OR Apache-2.0). The trailer
  reader. CLI `gyro2bb --dump file.insv` prints everything; also on PyPI.
  Build note: depends on a git fork, `AdrianEddy/mp4parse-rust`.
- **`gyroflow/gyroflow`** (GPL-3.0). `distortion_models/insta360.wgsl`
  is the projection reference. The app itself **does not support 360
  cameras** and will not open this footage; AdrianEddy's three stated
  reasons in issue 848 are stitching, a different distortion model, and
  the inability to read two video streams at once. Issue 1164 is an open
  request for the same capability for the DJI Osmo 360.
- **`paulbourke.net/dome/dualfish2sphere/`** and `/dome/fish2/`: the
  authoritative parameter set for hand-aligned dual fisheye, an actual
  GLSL shader, and a 4th-order radial correction polynomial, i.e.
  Bourke's own answer to "is one FOV parameter enough" is no. Also the
  doctrine line: *"A perfect blend can occur at any one distance but not
  at all distances."*
- **`vsezol/insta360-raw-viewer`**: closest prior art to a player.
  mp4box.js plus WebCodecs, streams multi-GB files via `File.slice`,
  telemetry ported from telemetry-parser and cross-checked against
  exiftool, auto-detects both dual-fisheye layouts. Caveat: despite the
  README, it does **no** undistortion, it crops and blits. Value is the
  demux and telemetry path.
- **`inirin/immich-insta360-viewer`**: worth reading as a negative
  example. It `hstack`s the two tracks and maps the result onto a
  three.js sphere with plain equirect UVs, which is geometrically wrong.
  Its README is honest that quality differs from Studio.
- **`alex-plekhanov/insvtools`** (Java) and **`ke4ukz/insvdump`**
  (Python): independent trailer readers, useful for cross-checking.
- Ricoh Theta is a **worse** template than it looks. Ricoh publishes no
  lens parameters, so every Theta stitcher is a per-unit hand-calibrated
  LUT. Insta360 handing us a full Mei model is the opposite situation.

Ecosystem gap worth stating: a crates.io survey found **no** Rust crate
for dual-fisheye reprojection, Insta360 stitching, or MediaSDK bindings.
`telemetry-parser` and `exiftool-rs` are the only crates that know the
word "insta360". Kyerag is filling real empty space.

## 10. Unknowns

Ordered by how much they would cost us.

1. ~~**The sign and order convention for `yaw`/`pitch`/`roll`.**~~ Settled
   against rendered frames on two cameras, 2026-07-31: `Rz(roll - 90) *
   Ry(yaw) * Rx(pitch)`, and the quarter-turn datum is the finding (4.8).
   What is left of it is the **order** of the three, which no known camera
   can distinguish because every one of them records sub-degree yaw and
   pitch.
2. **Vignetting coefficients are not in the metadata.** May show as
   rolloff in the blend band. Would need flat-field calibration.
3. **`pts_type = VideoPtsEexposureFile` semantics.** If frame PTS really
   come from the exposure records, that changes the clock design.
   Confirm by comparing against container PTS.
4. **Records 14 (Euler) and 18 (Quaternions).** If populated, they are
   free orientation and skip integration and drift entirely. Unknown
   whether the X4 Air writes them.
5. **`offset` vs `original_offset`.** Identical on our bare-camera
   fixture, so the hypothesis is untested. Would matter to anyone using
   lens guards or a dive case.
6. **`lensType`.** 131 on the X4 Air, 71 on the non-Air X4, 41 on a ONE
   X2, 113 in an X5 sidecar. No decoder table exists anywhere.
7. **The 32-byte `padding` at the head of the footer.** Named
   `unknownBuf` by insvtools, copied through verbatim by everything.
8. **An extended calibration sidecar.** On the X5, the SD card also
   carries a `MISC/Camera01/VID_*.insv.pb` base64 protobuf with a
   27-field-per-lens model adding `k4`, thin-prism `s1..s4`, tilted
   sensor `tauX/tauY`, and a Rodrigues rotation vector, of which
   `offset_v3` is a reduced projection. **Unverified whether the X4 Air
   writes one.** If it does, it is a strictly better calibration.
9. **Records 8, 11, 13, 15, 16, 17 payloads.** Nobody has decoded them.
10. **`gyro_calib`'s six doubles.** Probably 3 gyro plus 3 accel bias
    (magnitudes are consistent with bias), but unlabelled.

## 11. Reproducing any of this

```sh
cargo install --git https://github.com/AdrianEddy/telemetry-parser gyro2bb
gyro2bb --dump SOME.insv
```

That one command yields the offset strings, the IMU ranges, the rolling
shutter time, the flowstate and dewarp flags, the clock fields, and a
warning line for every record id the parser does not understand.

**Never commit a raw dump.** It carries the camera serial number and the
GPS track. The checked-in fixture is the stripped form; extend that file
rather than adding new dumps.

---

## Source URLs

Format and parsing:

- https://github.com/AdrianEddy/telemetry-parser (`src/insta360/mod.rs`,
  `record.rs`, `extra_info.rs`)
- https://github.com/AdrianEddy/telemetry-parser/issues/35 (`offset` vs
  `original_offset`)
- https://github.com/exiftool/exiftool/blob/master/lib/Image/ExifTool/QuickTimeStream.pl
  (`sub ProcessInsta360`)
- https://github.com/alex-plekhanov/insvtools
- https://github.com/ke4ukz/insvdump (`extra_metadata.proto`)
- https://github.com/iliakonnov/insta360-nas (superset proto)
- https://github.com/gyroflow/mp4-merge (`src/insta360.rs`)
- https://github.com/Ivan1248/irap-tools (non-Air X4 `offset_v3` dump)

Projection and stitching:

- https://github.com/gyroflow/gyroflow (`src/core/stabilization/distortion_models/insta360.{rs,wgsl,glsl,cl}`)
- https://github.com/gyroflow/gyroflow/issues/848 (no 360 support, and
  the failed `cv2.fisheye` attempt)
- https://github.com/gyroflow/gyroflow/issues/1164 (open dual-fisheye
  request)
- https://docs.opencv.org/4.x/dd/d12/tutorial_omnidir_calib_main.html
- C. Mei and P. Rives, ICRA 2007, *Single view point omnidirectional
  camera calibration from planar grids*
- https://github.com/BenjaminHenriksson/insv-stitch
- https://paulbourke.net/dome/dualfish2sphere/ and
  https://paulbourke.net/dome/fish2/
- https://ffmpeg.org/ffmpeg-filters.html#v360 and ffmpeg `vf_v360.c`
- https://github.com/peterbraden/insv-to-yt

Players and viewers:

- https://github.com/vsezol/insta360-raw-viewer
- https://github.com/inirin/immich-insta360-viewer
- https://github.com/faeton/insta360-quicklook

Official SDK:

- https://github.com/Insta360Develop/Desktop-MediaSDK-Cpp
- https://github.com/Insta360Develop/Desktop-MediaSDK-Cpp/issues/50 (CPU
  fallback flags)
- https://www.insta360.com/sdk/apply
- https://github.com/pdxmusic/insta360sdk
- https://github.com/GHSLAB/Vision360-Toolbox

Vendor documentation:

- https://onlinemanual.insta360.com/x4/en-us/camera/basicuse/stitching
  (the 1 m minimum subject distance)
