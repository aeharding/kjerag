# The Insta360 `.insv` format, and what it takes to reframe it

Feasibility study, 2026-07-30. This is the format reference for Kjerag:
what an `.insv` actually contains, what the calibration numbers mean, what
the seam and the gyro cost you, and what was ruled out. Quote it to settle
disputes.

Everything here was derived from public sources (linked inline) plus one
real 30-minute Insta360 X4 Air capture. The parsed calibration from that
capture is checked in at `docs/research/x4air-calibration.json` with
serial number, GPS and capture times stripped; it is the fixture, and the
worked examples below use it.

## License context

Kjerag is **AGPL-3.0**. GPL-3.0 is one-way compatible with AGPL-3.0, so
Gyroflow's `distortion_models/insta360.wgsl` (GPL-3.0-or-later) is usable
as a **direct reference implementation with attribution**, not merely as
clean-room input. Files that derive from it carry their own SPDX header.
This matters for section 5: the projection math does not have to be
re-derived from the Mei paper, though the paper and OpenCV's `omnidir`
remain the better-documented description of the same model.

`telemetry-parser` is MIT OR Apache-2.0 and imposes nothing. Kjerag reads
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
its playback source). Kjerag does **not** depend on it (see the decisions
log in ROADMAP.md): the proxy may be absent, so full-resolution decode
has to stand alone.

### 1.1 The two-file layout, first-hand (issue #79, 2026-07-31)

**Confidence: HIGH**, measured on all three ONE X2 pairs on this box
(firmware `v1.0.62_build2`). The second layout above is not a rumour, and
it is not symmetric, which is the part no published source says.

Each file is **one video stream** of 2880x2880 HEVC plus **its own AAC
stream**. Both files of a pair agree exactly on the frame grid:
`time_base` 1/30000, `start_time` 0, and the identical PTS series 0,
1001, 2002, ... So there is no drift between two clocks to correct and
nothing to resample, and a player needs no policy for two files
disagreeing about when a frame is.

They do **not** agree on length. The `_00_` file runs exactly one frame
longer in all three pairs, so the last frame of lens 0 has no partner. A
capture is the shorter of the two.

| capture | lens 0 frames | lens 1 frames | duration |
| --- | ---: | ---: | ---: |
| 20251018_184419 | 2516 | 2515 | 83.95 s |
| 20251018_191318 | 8204 | 8203 | 273.74 s |
| 20251018_193615 | 7810 | 7809 | 260.59 s |

**Only the `_00_` file carries a trailer.** Every `_10_` file ends in
ordinary mp4 bytes with no magic at EOF, so the trailer reader answers
`NoTrailer` on it. The calibration, the IMU track and the exposure track
therefore exist once per capture and live with lens 0, and a player that
opens the `_10_` file has to reach across to the `_00_` one or it has
nothing to reproject with. That is also the argument for which file is
which lens: the trailer writes record 4 and no record 12, i.e. lens 0's
shutter track and not lens 1's (section 2, and 6.3).

The `LRV_..._11_XXX.insv` proxy beside the pair is a third thing again:
736x368, **both fisheyes in one track**, and it carries a full copy of
the trailer. It is not a lens of anything, and a pairing rule that
matched "the marker field differs" rather than `00` and `10`
specifically would swallow it.

Kjerag pairs at open: `kjerag_meta::sibling` for the name,
`kjerag_media::Reader` for two demuxers in lockstep matched on frame
index, and `CalibrationSet::from_capture` for the trailer reach. Opening
either file of a pair renders the same sphere, byte for byte. A per-lens
file whose partner is not on the card renders exactly what it rendered
before, byte for byte as well: half a sphere, which is all there is.

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
Same bytes, different reading. Entries with a zero size are empty slots.

**"Instead of walking" is not a preference (measured 2026-07-31, issue
#7).** On the X4 Air the chain is not walkable at all past the third
record. Its trailer leaves slack between records, 163 to 250 KB of it on
the three captures measured, so a reader that steps off the front of a
record lands in the gap and reads a length out of nothing. The three
records nearest the footer are packed tight and the walk gets those: 0
(the index), 1 (the metadata) and 2 (the thumbnail), in that order. The
gyro, both exposure records and everything else are reachable only
through the index. Where both have a record they agree byte for byte.

The ONE X2 writes no index record and packs its trailer tight, and there
the walk alone reaches all of 1, 2, 3, 4, 5, 9 and 10. So a reader needs
both: walk from the footer, and if a record 0 turns up in the first few
steps, use it for the rest. The X4 Air also carries ids 11, 22, 27, 28
and 29, none of which is in any published table.

### Record type ids

From `telemetry-parser/src/insta360/record.rs`:

| id | name | contents |
|----|------|----------|
| 0 | Offsets | the index table above |
| 1 | **Metadata** | protobuf `ExtraMetadata`, holds the calibration |
| 2 | Thumbnail | a single H.264 frame |
| 3 | **Gyro** | IMU samples, two encodings (section 8) |
| 4 | **Exposure** | `{u64 ts, f64 shutter}` per sample, lens 0 |
| 5 | ThumbnailExt | a single H.264 frame |
| 6 | TimelapseTimestamp | `u64` timestamps |
| 7 | GPS | 53 bytes per fix |
| 8 | StarNum | unknown, 11 bytes per record |
| 9 | AAAData | 48 bytes; see below, the published description does not fit the X4 Air |
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
it is there. Neither is present on any X4 Air or ONE X2 capture here.

**Record 9 is one track, not one per lens (measured 2026-07-31).** On an
X4 Air capture it is 54024 samples of 48 bytes for 53940 frames, i.e. one
per frame, and it is written once for the file. Read as twelve `u32` LE
each sample is `ts_ms, 0, 0, 0, 0x02000000, a, b, 0, 0, 0, 0, 0`, with
the timestamp in milliseconds whatever the timebase of records 3, 4 and
12, and `a` near 2040 and `b` near 5950 on the frames sampled. The
published reading, "EV target, exposure time, bit-packed ISO and luma
stats", does not fit those bytes. It matters because a per-lens ISO is
the one number that would complete the exposure calculation in 6.3, and
this record does not carry one: there is no second AAAData record for the
second lens.

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

Fields Kjerag cares about, with fixture values:

| field | tag | X4 Air fixture | why it matters |
|---|---|---|---|
| `camera_type` | 2 | `Insta360 X4 Air` | selects the IMU orientation table. See the trap in 4.6. |
| `fw_version` | 3 | `v1.2.7_build1` | calibration grammar has changed across firmware generations |
| `offset` | 5 | 16 floats | v1 calibration, legacy |
| `offset_v2` | 53 | 34 floats | v2 calibration, legacy |
| **`offset_v3`** | 54 | **40 floats** | the model Kjerag uses |
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

A second corroboration, this one first-hand: Kjerag read an
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

Present in the fixture, not used by Kjerag, recorded so a future reader
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

Kjerag should read `offset_v3`, not `original_offset_v3`: if they ever
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

Kjerag reads `offset_v3` directly rather than consuming the synthesized
Gyroflow lens profile, so the first two bugs are not on its path either.
Bug 3 is the one that decided the design: master is unpublished and pulls
two further git forks, so Kjerag walks the trailer itself
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

Gyroflow uses it as roll, and so does Kjerag.

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
it renders the world a quarter turn on its side. Kjerag carries the
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

**Re-checked on the ONE X2 with a number, 2026-07-31 (issue #79).** The
owner reported X2 footage rendering upside down, which reads like an
accusation against this row, and it is not: the datum is right and the
defect was the IMU convention (8.5). Two things say so.

The first is a plumb reference and its wrong-answer control. On a ground
frame of capture 191318 the unlocked view at yaw and pitch 0 shows a pilot
**standing upright** on level ground with his wing laid out beside him,
terrain below and sky above. At yaw and pitch 0 the only other candidate
consistent with the seam is exactly a half turn away, which is that same
picture rotated 180 degrees: the pilot hangs head down with the sky under
his feet.

The second is arithmetic the seam can check. Under a datum of `roll + d`
the two lenses disagree at the seam by `roll_0 + roll_1 + 2d`, so the X2's
own numbers predict:

| datum `d` | predicted along-seam disagreement |
| --- | ---: |
| `-90` (shipped) | **+1.246 deg** |
| `+90` | +1.246 deg |
| `0` or `180` | 181 deg |

`kjerag-spike --bin seam` measures **+1.086** on a still frame of 191318
and **+1.188** on 193615, within 0.16 degrees of the prediction, at an
instrument repeatability of 0.02. The two quarter-turn datums are
indistinguishable there by construction, which is what the plumb reference
above settles; `0` and `180` are ruled out outright, because a correlator
searching plus or minus 2 degrees could not have found 181.

Note what this does **not** claim. A wrong picture datum and a wrong axis
convention are not separately observable in a **locked** view: an error in
one is cancelled exactly by the compensating error in the other, so the
sweep of 8.5 would have absorbed a bad datum silently. It is the unlocked
picture, which has no IMU in it at all, that pins the datum's own half.

Reproduce any row with the headless instrument, which runs the app's own
pass and writes a PNG:

```sh
cargo run --release -p kjerag-spike --bin reframe -- <file.insv> \
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
- ~~**Where the 90 degrees comes from.**~~ Settled in 8.5, by the one thing
  downstream that does care: the IMU is bolted to the sensor rather than to
  the picture, and it wants `Rz(roll)` where the picture wants
  `Rz(roll - 90)`. So the camera delivers the sensor image already rotated a
  quarter turn, and the datum is the delivered frame's.
- ~~**Lens 1.**~~ Settled in 4.9: the nominal opposed arrangement is a half
  turn about the body's vertical, multiplied on the right of the block's
  own angles.

### 4.9 The nominal arrangement of lens 1 (settled 2026-07-31)

**Confidence: HIGH for the choice, MED for what is left over.** Settled
during issue #27, the pass that samples both lenses.

4.3 says the 180-degree flip is not in the extrinsics: lens 1's recorded
yaw is 0.039 degrees. Something has to supply it, and there are three
candidates that all point lens 1 backwards:

```
lens_1 = Rz(roll - 90) * Ry(yaw) * Rx(pitch) * Ry(180)     <- Kjerag
lens_1 = Ry(180) * Rz(roll - 90) * Ry(yaw) * Rx(pitch)
lens_1 = Rz(roll - 90) * Ry(yaw) * Rx(pitch) * Rx(180)
```

The first two differ by **twice lens 1's roll residual** (conjugating a
z-rotation by a half turn about y negates it), which is 1.85 degrees on
the X4 Air fixture: a real difference, at the one place in the picture
where two lenses have to agree. The third differs from the first by 180
degrees of roll, i.e. a rear sensor mounted upside down, which is what
`roll` already records.

### How it was measured

Parallax at the seam is the reason a naive "does it line up" test is
useless, and also the reason this one works. The baseline is along the
lens axis (`tz` dominates), so **every direction on the seam great circle
is perpendicular to the baseline**: parallax there displaces a subject
only *across* the seam, never *along* it, whatever its distance. The
along-seam displacement is therefore a clean measure of the relative roll
of the two lenses, with parallax subtracted by geometry rather than by
assumption.

So: render the same view twice with `kjerag-spike --bin reframe`, once
from each lens (a temporary two-line patch to the pick), on an X4 Air
frame with distant content sitting on the seam, and correlate a patch of
the overlap band between the two. The residual is the shift that
correlates best.

| composition | along-seam residual | correlation |
| --- | --- | --- |
| `... * Ry(180)` (Kjerag) | **0.4 degrees** | 0.97 to 0.99 |
| `Ry(180) * ...` | 1.5 degrees | 0.95 to 0.97 |
| `... * Rx(180)` | no peak at all | 0.02 to 0.18 |

Three far-field patches on a treeline 90 degrees off the front lens's
axis, plus two on the ground under the camera at the other end of the
seam circle, which agree in sign. The two orders measured 1.82 degrees
apart against 1.85 predicted from the file, which is the check that the
instrument is measuring the thing it is named after; and the `Rx(180)`
row is the check that it can tell a wrong answer from a right one, since
that candidate turns the rear picture upside down and nothing correlates.

Confirmed independently by reading the picture: text on the wing
("OZONE", printed along the span) is legible and **not mirrored** in the
back hemisphere, and the horizon runs into the seam from both sides at
the same angle.

### What is left over

**0.4 degrees along the seam, and an across-seam disagreement that is not
characterised here.** Neither candidate order lands on zero, so the
remainder is not an order to choose between; candidates for it are the
reduced calibration model at the extreme edge (5.3: the polynomial
dominates exactly there), the focal scale of 4.3, and the unsettled
**order** of yaw/pitch/roll from 4.8. Across the seam the measured shift
varied between patches by more than parallax explains, and the patches
that disagreed were the low-correlation ones, so no number for it is
claimed.

**Re-measured for issue #7, and still not attributable (2026-07-31).**
The instrument moved off rendered views onto the delivered frames: both
lenses sampled on the *same* angular grid around a direction on the seam
circle, so there is no rotation to undo and the best-correlating shift is
in degrees of world angle, split into along-seam and across-seam by
construction. 36 patches round the circle of one in-flight X4 Air frame,
3.7 degrees across, the 12 busiest by local contrast correlated over
+/-2 degrees in 0.12 degree steps. Of those, five correlate above 0.85.

- **Along the seam**, where parallax cannot reach, every one of the five
  is negative: -0.36, -0.60, -0.96, -1.20, -1.20 degrees. Consistent in
  sign with the 0.4 degrees measured off rendered views, and larger.
- **Across the seam** they run -2.0 to +2.0 with several peaks pinned at
  the search limit, and the largest sit at the part of the seam circle
  that looks at the pilot's own body, half a metre away, where 6.1 puts
  parallax at 3.8 degrees. That part is explained.

What this says is that **an in-flight frame cannot answer the question**.
Two effects that are not calibration were named as candidates: near-field
parallax, and rolling shutter.

**Rolling shutter is now measured out (issue #9, 6.7).** The reasoning was
that 15.883 ms of readout displaces content by the camera's own angular rate
times that time, that the two lenses' rows run in nearly opposite world
directions because their rolls are near +90 degrees in opposed frames, so
that it would not cancel between them, and that near the seam 15 px per
degree turns a small displacement into a large angle. Every step of that is
sound except the one that matters: measured on 214 paired patches of 30
frames rolling up to 123 deg/s, the along-seam residual carries **0.014** of
the displacement that model predicts, on an instrument that reads an applied
displacement of the same size and shape back at **0.985 with r = 0.996**.
Whatever the two sensors do, they do not do it in opposite world directions,
and none of the -0.36 to -1.20 degrees measured here is theirs.

**And now the direction is known, it is ruled out twice (6.7).** The readout
runs down the delivered frame, which is the same world direction in both
lenses, so it cancels between them: the relative displacement it puts into a
seam direction measures 0.000 degrees. The seam residual cannot be the
readout, whatever the camera is doing.

**Measured from a camera that is not moving (2026-07-31).** The capture this
section asks for arrived: an X4 Air standing on a deck, 0.0 deg/s median and
0.1 at the worst, so no readout displacement, no changing parallax and no
lock residual can be in it. 11 patches round the seam circle over 6 frames,
correlated above 0.5:

- **along the seam, -0.78 degrees**, spread 0.409 across patches. That is the
  same size and the same sign as the -0.36 to -1.20 measured in flight, from
  a camera doing nothing at all. A second still capture, indoors with the
  seam looking at a desk half a metre away, reads -0.96 on the one patch that
  correlates there.
- **frame to frame the same patch moves by 0.018 degrees**, against 0.100 in
  flight. So the instrument's own noise on a still camera is a fifth of what
  the flying numbers move by, and four fifths of that movement is the flying.
- across the seam it reads 2.31 degrees, which is the deck 10 cm under the
  camera and is parallax doing exactly what 6.1 says it does.

So the along-seam residual is **calibration**, and issue #48 has one fewer
thing to consider: it is there with the camera still, parallax cannot reach
that axis by construction, and rolling shutter cancels on it.

None of it blocked the blend, which needed the band and not the boundary:
a weight field over a 14-degree overlap does not need a sub-pixel-perfect
crossover, it needs to know where the crossover is. Issue #7 shipped
without an answer here, and the question stays open.

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

**Only the forward direction is on Kjerag's path.** Reframing asks "for
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

For the footage Kjerag targets, almost everything is past 3 m and this is
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

### 6.3 Exposure mismatch: smaller than expected, and NOT what the shutter says

Insta360's own SDK documentation says it plainly: *"since the two lenses
operate independently, their respective video exposures may not align
perfectly... noticeable brightness differences can occur."*

**The file carries per-lens, per-frame shutter speed**: record 4 for one
lens and record 12 for the other, identical shape
`{u64 ts, f64 shutter}`, one sample per frame. Nothing in the
open-source landscape uses this. The timestamps are in the trailer's own
tick, which `is_raw_gyro` selects: microseconds on the X4 Air, and
milliseconds on the ONE X2, whose `first_frame_timestamp` is in
milliseconds with them.

Gotcha: `telemetry-parser` merges records 4 and 12 into the same
`GroupId::Exposure` key, so record 12 overwrites record 4 in its output.
Parse them separately.

The plan that made was a **symmetric split**, `front *= 1/sqrt(g)` and
`back *= sqrt(g)` for shutter ratio `g`, so neither hemisphere gets a
visible step. **It is wrong, and it was measured wrong before it shipped
(2026-07-31, issue #7).**

**Method.** For each sampled instant, decode both lens frames and take
the mean luma of the overlap annulus of each: `r` from 1680 to 1913 px
about lens 0's principal point and 1670 to 1905 about lens 1's, which are
the radii where both lenses have the ray. The two annuli hold the *same*
set of world directions, permuted in azimuth by the relative roll, so the
ratio of the means measures brightness and not content, whatever the
roll; and vignetting is radial and identical in both, so it cancels. Read
the shutter records at the same instant for `g`.

**Result**, over two 30-minute X4 Air captures, 30 and 15 instants:

| | capture A (2026-06-23) | capture B (2026-05-01) |
|---|---|---|
| mean measured brightness step | **3.5 %** | **0.9 %** |
| worst measured step | 7.3 % | 2.7 % |
| shutter ratio `g`, range | 0.54 to 1.81 | 0.62 to 0.74 |
| mean step after the symmetric split | **14.4 %** | **19.8 %** |

The correction is four to twenty times worse than the artifact, and the
two are uncorrelated: on capture A the instants with `g < 1` and the
instants with `g > 1` had the same mean measured ratio, 0.969 against
0.971.

**Why.** The two lenses run independent auto-exposure loops that trade
shutter against sensor gain to reach the same picture brightness. The
shutter ratio therefore measures how differently the two hemispheres are
*lit*, which on a paraglider is sun against ground and is genuinely a
factor of 1.8, not how differently they came *out*, which is a percent or
three. Completing the calculation would need the matching per-lens gain,
and the trailer does not carry one: record 9 is a single track for the
file (section 2).

**What to do instead.** Nothing analytic. A percent or three of step,
laid across the 14-degree blend band of 6.6, is a gradient of 0.015
percent per pixel at a 1920-wide 90-degree view, which is far under the
roughly 1 percent step at a sharp edge that the eye picks up. Measured on
a rendered seam view at 1024 px: the sky either side of the seam differs
by 2.0 percent and no two neighbouring columns differ by more than 1.9
codes of 255, which is the sensor noise. A measured luma ratio, adapted
slowly from the overlap band, is the fallback if a capture ever shows a
step the blend cannot hide, and it costs a GPU readback per frame; do not
build it before that capture exists.

Records 4 and 12 are still worth parsing, and Kjerag parses them: they
are the camera's own frame clock if `pts_type = 2` means what it says
(section 8.3), and they are what this measurement was made against.

### 6.4 The rest of the artifact budget

Ranked for a player that samples one lens for most of the frame. The
first two rows were re-ranked by the 2026-07-31 measurements above, which
put both of them under the blend rather than over it:

1. ~~**Exposure and white balance step at the seam.**~~ 0.9 to 3.5
   percent, and not fixable analytically after all: the shutter records
   do not say what they look like they say (6.3).
2. **Vignetting rolloff.** Lands exactly on the blend band. Coefficients
   are **not** in the metadata; needs flat-field calibration if it bites.
   The best ffmpeg-based workaround in the wild carries the same caveat:
   *"will not correct for lens vignetting darkened stitch edge."* The
   weight field of 6.6 down-weights the rim it lands on, which is the
   cheap half of the fix and may be all of it.
3. **Parallax ghosting.** Only under about 3 m, only in seam-containing
   views. Now the top of this list: on a paramotor the wing and the lines
   are the near-field structure that crosses the seam, and a blend turns
   the hard step they used to show into a soft double image.
4. **Geometric error from a wrong lens model.** About 8 percent MAE.
5. **Chromatic aberration.** Measured as **zero** on X-series; applying
   CA correction made PSNR worse. Skip it.

### 6.5 Rolling shutter is not optional

`rolling_shutter_time` on the fixture is **15.883 ms**. At 3840 rows,
typical handheld motion displaces 12 to 18 px during readout, the **same
magnitude as the parallax above**, but unlike parallax it is fully
correctable from the gyro. On a vibrating airframe it dominates.

Correct it in the same backward mapping as everything else; see 8.

**That was the expectation, and 6.7 is what measuring it found.** The
displacement is in the pictures at about the magnitude this section predicts,
but not on the axis it assumes: the sensor reads **down** the delivered
frame, not across it, so the two lenses sweep the same world direction and it
cancels at the seam instead of doubling there.

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

**As built (issue #7, `crates/render/src/projection.rs`).** For a ray
`theta` off a lens's axis landing `r` px from that lens's principal
point, in a lens whose image circle has radius `R`:

```
claim = cos^2(theta / 2) * (R - r)          zero where the ray is not in the picture
w     = claim / sum of claims
```

The first factor is the longitude preference, 1 down the axis and exactly
1/2 on the seam great circle, which is what puts the crossover on the
seam rather than wherever the two image circles happen to end (they end
8 px apart on the X4 Air, so it is not the same place). The second is the
distance transform, in that lens's own pixels, which the two lenses of a
back-to-back pair share a scale for. Neither carries a width: the band
comes out as the overlap itself, 83.4 to 97.4 degrees off the front axis
on the X4 Air fixture, and 179 columns of a 1024-px 90-degree view
centred on the seam. Outside it exactly one lens claims anything, its
weight is exactly 1, and the pass costs the one texture fetch it did
before the blend existed.

### 6.7 The readout direction, and how it was measured (issue #9)

**Confidence: HIGH.** The correction is built the way 6.5 asks for and fused
into the one backward map (`crates/render/src/projection.rs`). Which way the
sensor reads is not in the file, so it was measured off the pictures, and on
an X4 Air the answer is that **the readout runs down the delivered frame**:
`1.00 +-0.12` of a whole frame in the trailer's own 15.883 ms, against
`0.02 +-0.07` across it. `kjerag_meta::Sweep::Down` is what the calibration
answers for an X4 and the correction is on. Every other camera keeps
`Sweep::Unknown`, which is a zero axis and no correction at all.

It took two rounds to get there, and the reason is worth more than the
answer. Issue #42 measured the same thing on the same footage, found nothing,
and shipped `Unknown`. What it was missing is that the fit answers on **two**
axes and only one of them ever had a control: an injected readout across the
frame read back at 0.79 to 0.84, so the across-frame answer was trusted and
reported as null, while the down-frame answer of the same fits was reported
as not repeating. It repeats wherever an injected control works. The stretch
that disagreed is the one where injecting a known displacement reads back at
-0.10, which is an instrument saying it cannot see, read as a measurement
saying there is nothing there.

#### What the correction is

For each output ray, after the lens is picked and the landing row computed,
the orientation the ray is carried through is the one at **that row's**
readout instant rather than the frame's. The row decides the instant and the
instant moves the row, so the landing is solved for rather than computed:
`Reframe::solve`, from the frame's own instant, `READOUT_STEPS` rounds of it.
No extra pass, no intermediate target and no second sample of the picture,
which is the insv-stitch principle this player is built on (9, ARCHITECTURE).

Two things the pass needs, and both are measured rather than assumed.

- **A straight turn across the readout.** One rotation vector per frame,
  `OrientationTrack::turn` over the readout window centred on the frame's
  instant, scaled by a row's share of it. Against the track's own orientation
  looked up at each row separately, over 20000 instants of two 30-minute
  captures: **0.068 and 0.019 degrees at the median**, 0.27 and 0.09 at the
  99th, 0.64 and 0.34 at the worst. What is left over is the airframe's own
  vibration inside one readout, which the stored track (200 Hz) does not
  resolve anyway; a per-row lookup would not recover it either.
- **The turn is the camera's own.** The stabilized track's turn against the
  raw gyroscope over the same window: **0.058 and 0.018 degrees at the
  median**. Read the other way round it is 0.64 and 0.47 at the median and 15.7
  at the worst, an order of magnitude apart, which is the control that pins
  the sign of the rotation the correction applies.

Rounds to convergence, in pixels of the delivered frame, at each capture's
hardest instant, against a solve run eight rounds:

| rounds | capture A, 551 deg/s | capture B, 270 deg/s |
| ---: | ---: | ---: |
| 0 (uncorrected) | 111.8 px | 35.3 px |
| **1** | **4.49 px** | **0.76 px** |
| 2 | 0.24 px | 0.02 px |
| 3 | 0.01 px | 0.00 px |

One round is what ships. Two would cost a second pass through the model per
lens per pixel for a quarter of a pixel at the worst instant of half an hour,
and the median rate on this footage is 20 deg/s, where one round leaves
0.006 px.

#### Instrument 1: the seam, where a readout cannot cancel

The reasoning 4.9 left the question on: the two lenses are mounted a half turn
apart, so if both sensors sweep the same way across their own delivered
pictures, they sweep in **opposite world directions**, and a readout
displacement does not cancel between them at the seam. It doubles there, and
the picture near the seam is 15 px per degree.

Method, and it is 4.9's own instrument with the readout added: both lenses
sampled on the **same angular grid** around 36 directions on the seam circle,
6 degrees across, correlated over +/-3 degrees in 0.12 degree steps, on the
delivered frames rather than on rendered views. 30 consecutive frames of
capture A from 1151.0 s, rolling 4 to 123 deg/s. A patch counts only where
**every** candidate correlated above 0.7, so all five are scored on one set of
patches: 214 readings. Each patch's own mean over the run is taken off before
anything is compared, because a patch's calibration residual is the same in
every frame and a readout displacement is not.

| candidate | along-seam spread, within patch |
| --- | ---: |
| **uncorrected** | **0.100 deg** |
| corrected, sweeping right | 0.314 |
| corrected, sweeping left | 0.312 |
| corrected, sweeping down | 0.097 |
| corrected, sweeping up | 0.104 |

And the same readings regressed against what a readout across the delivered
frame predicts for them, patch by patch:

| | slope | r | predicted spread |
| --- | ---: | ---: | ---: |
| the pictures as they come | **-0.014** | -0.04 | 0.686 deg |
| **control: with the correction applied** | **-0.985** | **-0.996** | 0.686 deg |

**The control is the point.** Applying a readout correction displaces the two
lenses' pictures by exactly what that readout predicts, and the instrument
reads that displacement back at 0.985 of its size with a correlation of 0.996.
So it can see a 0.7 degree displacement of that shape on these pixels, and
what the pictures themselves carry is 0.014 of it.

**Two sensors sweeping in opposite world directions is therefore ruled out**,
and with it the model 6.5 and 4.9 both assume. Applying it anyway would put up
to 1.9 degrees of misalignment into the seam at 120 deg/s, which is about 28
px of double image in a band that issue #7 just finished blending.

What the seam cannot say is anything about the other two possibilities, and
this is the instrument's own blind spot rather than a result: two sensors that
sweep the **same** world direction (mirrored in their own delivered frames,
which is what two identical modules mounted the same way round in one body
would do), and a readout running **down** the delivered frame rather than
across it. Both predict a seam disagreement of 0.002 to 0.003 degrees, which
is nothing to measure.

**That blind spot is where the answer was.** Re-run on the same 30 frames
after the direction was settled, the seam reads the down candidate's own
relative displacement as **0.000 degrees** while it reads the across ones as
0.131 and hands them back at -0.93 and -1.05. So the seam is not merely
uninformative about a down sweep, it is provably untouched by one: the two
lenses' pictures of a seam direction move together. That is why switching the
correction on cannot disturb #7's blend, and it is the same fact from the
other side as "both sensors read down their own delivered pictures", because
lens 1 is mounted a half turn round and down is down in both.

#### Instrument 2: inside one lens, where a horizon has to stay straight

A great circle projects to a straight line in a rectilinear view, so a bend in
a rendered horizon is the picture's and not the world's, and a readout draws
one: the displacement grows along the picture and a displacement that grows
along a line curves it. `kjerag-spike --bin horizon readout=...` renders runs
of frames through the app's own pass under each candidate and measures the
horizon's own fit residual with the same `skyline` issue #8 used
(`Skyline::spread`).

What an uncorrected readout of the trailer's length would leave, computed
through the shipped map, in a 960x540 view at 100 degrees:

| roll rate | worst displacement | **bend** |
| ---: | ---: | ---: |
| 50 deg/s | 2.7 px | 0.8 px |
| 100 | 5.4 | 1.7 |
| 200 | 10.9 | 3.3 |
| 400 | 21.7 | 6.6 |

Measured, as the change in that residual against the correction switched off,
on the frames every candidate found a horizon in, plus or minus the standard
error of the paired difference:

| stretch | frames | roll | right | left | down | up |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A 1415 s | 22 | 26-97 | **-0.22 +-0.10** | **+0.35 +-0.08** | -0.10 +-0.06 | +0.14 +-0.08 |
| A 252 s | 12 | 60-129 | +0.15 +-0.09 | -0.17 +-0.22 | +0.10 +-0.09 | -0.04 +-0.07 |
| B 1053 s | 23 | 35-81 | -0.16 +-0.28 | +0.13 +-0.26 | -0.37 +-0.22 | -0.24 +-0.24 |
| B 1036 s | 13 | 40-81 | -0.14 +-0.31 | **+1.28 +-0.20** | +0.14 +-0.18 | +0.08 +-0.06 |
| B 1046 s | 40 | 33-106 | +0.01 +-0.17 | -0.18 +-0.22 | +0.10 +-0.10 | +0.16 +-0.16 |

Negative is straighter. Three of the five say what a sweep to the right would
say, and say it with the signature the physics predicts: the correction
improves the residual and its reverse costs about twice as much as the
correction gained. Two of the five say the opposite. The bend a readout would
draw at these rates is 0.8 to 2.1 px against a horizon whose own residual is 3
to 7 px, so the instrument is at the edge of resolving it, and **this was not
a verdict**. It is why #42 answered `Unknown` rather than "right".

Read again with the answer in hand it says more than it did. The same stretch
re-run over 40 frames instead of 22, rolling 1 to 71 deg/s, as the bend
against the correction switched off:

| candidate | bend px | against off | angle sd |
| --- | ---: | ---: | ---: |
| off | 0.70 | | 0.23 deg |
| right | 0.91 | +0.21 +-0.04 | 0.22 |
| left | 0.56 | -0.13 +-0.04 | 0.28 |
| **down** | **0.60** | **-0.09 +-0.02** | **0.14** |
| up | 0.79 | +0.10 +-0.02 | 0.34 |

**Down and up reproduce and right and left do not.** Against #42's run of the
same stretch, down was -0.10 and is -0.09, up was +0.14 and is +0.10, while
right and left swapped signs entirely (-0.22 and +0.35 became +0.21 and
-0.13) when the frame set changed. A candidate the camera does not have moves
this measurement around by whatever the frames happen to hold; the one it has
moves it the same way twice. Down also leaves the straightest horizon of any
variant in the table, including the shipped pass, and it is the only readout
candidate that improves the angle's own scatter rather than costing some.

#### Instrument 3: one lens against itself, a few frames apart

**This is the one that answers it.** The same lens at two instants is turning
at two different rates, so far-off content that has not moved in the world is
displaced differently in each, and the difference is what a readout predicts
and nothing else does. Patches are correlated between frames a chosen distance
apart through the stabilized map, and the fit is eight unknowns least squares:
three for the rotation the horizon lock leaves between the frames, three for
the camera's own translation, whose flow field over far content is a dipole
and looks enough like a readout's to be worth taking out, and **two for how
far the readout sweeps across the frame and down it**, fitted together because
separately each reads the other's displacement as its own.

Five stretches of capture A, `pair=1 count=14 gap=2 search=3`, each one of the
file's hardest rolls (`find=n` picks them). Every row is 150 to 400 patch
readings:

| stretch | roll deg/s | across x | **down y** | predictor |
| --- | ---: | ---: | ---: | ---: |
| 66.0 s | up to 195 | +0.05 +-0.08 | **+1.30 +-0.07** | 0.225 deg |
| 878.4 s | up to 158 | -0.26 +-0.17 | **+0.72 +-0.18** | 0.146 |
| 1263.6 s | up to 155 | +0.04 +-0.07 | **+0.76 +-0.09** | 0.207 |
| 1325.4 s | up to 145 | +0.15 +-0.07 | **+0.92 +-0.09** | 0.184 |
| 1414.5 s | up to 71 | +0.12 +-0.30 | **+1.29 +-0.28** | 0.086 |

Mean **1.00 +-0.12 down** and **0.02 +-0.07 across**, the error being the
scatter between stretches, which is wider than any one stretch's own. Both
lenses read down separately, 0.51 to 1.93 on lens 0 and 0.69 to 1.67 on lens
1, which is the thing the seam cannot see.

**Nothing in that fit knows how long a readout takes.** The unit is a sweep
that crosses the whole delivered frame in the trailer's 15.883 ms, and the
fitted size landing on 1.0 is a check on the shape rather than a fit to it: a
displacement of some other origin has no reason to come out at exactly one
frame per 15.883 ms.

**The controls, all four.** `pair=1` now runs the whole pass again with each
direction's own readout taken out of the pictures, which has to move the fit
by exactly one along that direction's axis and leave the other axis alone:

| stretch | + right | + left | + down | + up |
| --- | ---: | ---: | ---: | ---: |
| 66.0 s | 0.98 | 0.85 | 0.99 | 0.89 |
| 878.4 s | 0.85 | 0.79 | 0.71 | 0.18 |
| 1263.6 s | 0.94 | 1.02 | 1.02 | 0.91 |
| 1325.4 s | 0.96 | 0.93 | 0.99 | 0.96 |
| 1414.5 s | 0.98 | 1.19 | 0.93 | 1.27 |

1.00 is exact. The down column is the one #42 never ran, and it is the one
the answer is on.

**Robustness, and the two rows that are not measurements.** At 1325.4 s the
answer is +0.72 +-0.09, +0.92 +-0.09 and +0.87 +-0.15 at gaps of 1, 2 and 6
frames, and at 66.0 s it is +1.30 +-0.07 and +1.15 +-0.13 at gaps of 2 and 4,
so it does not live in the frame spacing. Against that:

- **1150.5 s**, the stretch #42 read -0.47 +-0.21 off: the injected
  displacement there is 0.019 degrees, which is a sixth of the correlator's
  own 0.12 degree step, and the four controls read back at 0.58, 0.67, -0.10
  and 2.60. The instrument is blind at that stretch and says so.
- **1790.0 s**, the camera lying on the ground: predictor 0.004 degrees, and
  the answer is +0.37 +-0.60, consistent with nothing, with an error bar
  seven times the working stretches. That is the null this instrument gives
  when there is no motion to carry a readout.

So the rule the instrument now prints for itself: the injected displacement
has to be worth several correlator steps before any row of it means anything.

#### What it cost to switch on

`playback 60 60 0 90` at 2560x1440, yaw 90 so the seam is down the middle,
five 60 s runs alternating the file's own readout against `off`, which is the
pass as it was before issue #9: **4.00 and 4.23 ms per redraw off, 4.28, 4.82
and 5.00 on**, so about half a millisecond. **0 dropped and 0 starved in all
five**, 29.97 fps presented, 30.0 redraws/s. (The 1.84 ms in ROADMAP's cost
table is the same measurement before issues #10 and #11 changed what a redraw
does; the arms above are from one build.)

#### The settling capture, and why it answered nothing

This section used to ask for ten seconds of a camera turned as fast as a wrist
will turn it, in front of close, still, sharp content. Two captures arrived on
2026-07-31 and **neither of them turns**: one is the camera standing on a desk
indoors while somebody walks through the room, the other is the camera sitting
on a deck outdoors. Their IMU records agree with their pictures at 0.2 and 0.0
deg/s median, 1.5 and 0.1 deg/s at the worst.

At those rates a whole-frame readout displaces the picture by **0.02
degrees**, and what the seam instrument's control could apply on them is
**0.003 degrees** against the 0.686 it applies on flying footage. All five
candidates read identically to three decimals, because they are the same
pictures. **A still capture cannot answer this question in either direction**,
and reporting one as a negative would have been the same mistake as #42's,
one axis further along.

Two things follow, and both are shipped. `--bin rolling` prints a `carries:`
line before it decodes anything, giving the file's own rate distribution and
the displacement a whole-frame readout comes to at it, next to what a hand
twist would give; that is three seconds and it names the wrong file before an
hour of measuring does. And `pair=1` injects all four directions rather than
one, so an axis that cannot be read says so in its own control column.

The still captures are worth keeping for a different question. 4.9 asks for a
capture from a camera that is not moving, and these are it.

#### Would a hand twist still be worth capturing?

Yes, but as a check rather than as the answer. It would put degrees of
predicted displacement where the flying stretches above have tenths, and the
one thing the current answer rests on is a single 30-minute capture: five
stretches of it are five stretches of one camera on one day. What it would
have to be, in one line, is a file whose `carries:` line reads in the
hundreds of deg/s.

### 6.8 The seam is misaligned by degrees, and it is calibration (issue #48)

**Confidence: HIGH for the measurement and the attribution, MED for the
fitted numbers.** Measured 2026-07-31 on two captures from a camera that was
**not moving**, which is the retest 4.9 and 6.7 both asked for: no parallax
worth the name in the far field, no rolling shutter (0.1 deg/s rms, 0.3 deg/s
peak), no motion blur, and an accelerometer that is gravity and nothing else
(100 percent of the samples inside the filter's own trust window). Instrument:
`kjerag-spike --bin seam`.

The headline is not the along-seam residual 4.9 left open. It is the axis 4.9
declined to put a number on:

| | as shipped | fitted correction applied |
| --- | ---: | ---: |
| along the seam, round the circle | -0.30 to -1.25 deg | -0.02 to +0.02 |
| **across the seam** | **-2.36 to +2.65 deg** | -0.06 to +0.23 |

At the rim the picture is 948 px per radian, so 2.6 degrees is **43 px of the
delivered frame** and about 55 px of a 1920-wide 90-degree view (6.1). It is visible
as a doubled tree trunk in a blended view and as a broken horizon in a hard
cut, on this capture and on the owner's flight footage.

#### The structure names the error

The measurement is 4.9's, sharpened: both lenses sampled on the same angular
grid around 72 directions on the seam circle, correlated to a sub-step peak.
What is new is that it is taken **round the whole circle** and decomposed into
harmonics of the azimuth, because each harmonic is a different error. A
relative rotation `w` displaces a direction `d` on the circle by `w x d`,
whose along-seam component is `w.z` for **every** direction on that circle, so
the two axes separate the causes rather than mixing them.

Measured on the outdoor capture, and against what the shipped map itself says
each calibration field is worth (the instrument prints both tables, so the
attribution is read off the model rather than derived by hand):

| term | measured along | measured across | what only this can be |
| --- | ---: | ---: | --- |
| constant | **-0.762** | 0.491 | relative **roll** (along); focal or `xi` (across) |
| one cycle | **0.457** | **2.678** | principal point (both); lens **tilt** (across only) |
| two cycles | 0.037 | 0.289 | focal aspect, `fx` against `fy` |
| left over | **0.012** | **0.055** | |

A constant and one cycle account for all of it: 0.012 degrees of the
along-seam column and 0.055 of the across-seam column survive them, against
readings of 1.2 and 2.6. The knob table says a lens tilt reaches the
across-seam column at 1.0000 degrees per degree and the along-seam column at
0.0000, and that a principal-point shift reaches both, at 0.0318 and 0.0607
degrees per pixel. Only a tilt can put 2.7 degrees of one cycle across the
seam while leaving 0.46 along it.

#### The fitted correction, and what it is worth

Fitted through the shipped map (`kjerag_render::Reframe`) by perturbing one
field of **lens 1** at a time and reading what that does to the same patches,
so the answer is in the units `offset_v3` writes. The seam sees only the two
lenses' disagreement and cannot say which lens is wrong; quoting it all on
lens 1 is a convention.

| knob | correction | +/- | leaves |
| --- | ---: | ---: | --- |
| roll | **+0.801 deg** | 0.054 | |
| yaw | **-2.293 deg** | 0.245 | |
| pitch | **-0.817 deg** | 0.093 | |
| cx | -4.59 px | 4.01 | |
| cy | -14.73 px | 1.04 | |
| | | | along 0.766 -> **0.077**, across 2.333 -> **0.108** |

A pure rotation, three numbers instead of five, gets across to the same 0.108
and along only to 0.298: the principal-point pair is what takes the last of
the along-seam column. Fitting all eight fields including `fx`, `fy` and `xi`
reaches 0.033 and 0.045 with focal corrections of 300 percent, which is a fit
running away, not a calibration.

**The correction is not a fitted parameter until it is applied and the
measurement is taken again**, so it was: re-measured on the same capture with
it in place, every patch reads within 0.02 degrees along and 0.23 across, and
a **hard cut with no blending at all** is continuous through a tree trunk, a
fern bed and a deck board that were visibly broken before.

#### Why this is the camera and not the scene

Four controls, and the first two are the ones that matter.

- **Injected errors, read back off the same pixels** (`control=1`, the lesson
  of #45 applied at the size being reported): roll +0.50 deg reads back at
  **0.990** of what the map predicts and roll -0.25 at 0.993; yaw +0.50 reads
  back across the seam at **0.998 with r = 1.000**; a 20 px principal-point
  shift at 1.021 along and 1.057 across. An instrument that reads a known half
  degree at 0.99 can see the 2.6 degrees it is reporting.
- **A second capture of a different scene.** The camera was picked up and put
  down between them and they share no content; a calibration residual is fixed
  in the camera's frame and a scene's is not. The correction fitted on the
  first capture makes the second capture's hard cut continuous as well, on a
  straight edge that was broken by about 15 px before.
- **Parallax cannot reach the along-seam axis at all** (4.9, by construction),
  and cannot produce the sign change the across-seam column has: the baseline
  is along the lens axis, so a subject's distance displaces it towards the
  front lens at **every** azimuth. A one-cycle term is positive at one azimuth
  and negative at the opposite one. Far-field parallax on this capture is 0.1
  to 0.2 degrees, and the measured across-seam constant is 0.49.
- **The near field behaves exactly as parallax says it must.** The camera
  stands on a deck, so patches looking down at it are 5 to 30 cm away, where
  the disparity is 6 to 38 degrees. The overlap band is 14 degrees wide.
  Every one of those patches fails to pair, because the two lenses' pictures
  of that content are not in the band together at all, which is the prediction
  and not a limitation.

#### Against Insta360's own stitch

The owner exported the same capture from Insta360's app, which makes their
output the parity benchmark. What it is, fitted by rendering our own pass
under a candidate view and correlating: a **square 1440x1440 reframe**, one
frame per source frame, about 95 degrees across, near level, and mildly
compressed rather than strictly rectilinear (0.79 to 0.87 on a family where
1.0 is rectilinear and 0 is equidistant). It is not an equirect crop; an
equirect would be 2:1 and would carry the whole sphere.

A global fit good to about a degree cannot measure a disagreement of a degree,
so the comparison is each stitch **against itself**: mean squared gradient
within 5 degrees of the seam, over the same statistic 9 to 25 degrees off it
in the same picture. A doubled edge is a blurred edge; a tone curve, a
sharpening pass and a lens are in both terms and divide out.

| picture | band over its own surroundings |
| --- | ---: |
| Insta360's export | **0.83 to 0.88** |
| ours, as shipped | **0.573** |
| ours, correction applied | **0.689** |

Read directly: their stitch keeps six sevenths of its own sharpness across the
seam and ours keeps four sevenths. The calibration correction closes about a
third of that gap; the rest is the blend band, which 6.6 left as wide as the
overlap. Independently, the projection fit's own score, which
is our whole rendered picture against theirs, rises from 0.45 to 0.68 with the
correction applied: the same change makes our picture agree with theirs away
from the seam as well as at it.

Confirmed by eye on one feature, which is what started this: a bare trunk that
their export draws once, ours draws twice about 1.3 degrees apart, and ours
draws once again with the correction applied.

#### What a narrower blend is worth, in numbers

`--bin blend` renders the same frame under the shipped weights and under
crossovers of a stated width, and scores each on gradient energy over the
overlap against the front lens alone over the same pixels. Measured on the
owner's flight footage at a view where the wing lines and the harness cross
the seam, where the two lenses disagree by 1.72 degrees as shipped and 0.84
with the correction applied (the second being mostly real parallax at half a
metre):

| crossover | doubled band | sharpness, as shipped | shear | sharpness, corrected |
| --- | ---: | ---: | ---: | ---: |
| shipped weights | 10.55 deg | 0.538 | 0.16 | 0.518 |
| 14 deg | 11.20 | 0.516 | 0.15 | 0.488 |
| 8 deg | 6.40 | 0.630 | 0.27 | 0.596 |
| 4 deg | 3.20 | 0.696 | 0.54 | 0.658 |
| **2 deg** | **1.60** | **0.731** | **1.07** | **0.687** |
| 1 deg | 0.76 | 0.753 | 2.25 | 0.703 |
| 0.5 deg | 0.25 | 0.773 | 6.76 | 0.712 |
| hard cut | 0 | 0.815 | cut | 0.721 |

Shear is the disparity divided by the band: a picture crossed by `d` degrees
of disagreement inside `w` degrees of blend is locally compressed by `d / w`,
and above 1 the crossover is a fold rather than a blend. That is the whole
trade, and it says the two halves of this issue are not independent. **At
today's 1.7 degree disparity a 2 degree band sits exactly on the fold.** With
the calibration corrected the same band sits at 0.52 and takes 80 percent of
the sharpness a hard cut would give.

So the number to ship is **about 2 degrees, and only after the calibration
correction**. Narrowing the band first would trade a soft wide ghost for a
hard visible tear.

#### What is left open

- **Where the 2.4 degrees comes from.** The recorded extrinsics are all
  sub-degree, so no re-composition of them produces it: neither the angle
  order (4.8), nor the choice of nominal arrangement (4.9, which is a roll
  difference and shows in the other column). Either this unit's factory
  extrinsics are wrong by that much, or Insta360's own pipeline refines them
  per clip and the string is a starting point. The measurement does not
  distinguish those, and the fix is the same either way.
- **The split of the tilt between yaw and pitch.** Its magnitude, 2.44
  degrees, is steady; the axis is not, when each file is fitted on its own.
  **Phase 2 answers this rather than working around it** (below): the axis a
  file's own fit lands on moves because each file's across-seam column carries
  that file's parallax, and one answer fitted where there is none transfers to
  all of them.
- **Whether one correction serves every file from one camera.** Open in phase
  1, **settled in phase 2 and shipped**: one five-knob answer fitted on a
  static capture reads the same along-seam number on the flights as their own
  fits do, while the per-file fits disagree with each other by 15 view px
  (below).

#### Phase 2: one fit per camera, from a capture the pilot points at

**Confidence: HIGH.** Measured 2026-07-31 on seven of the owner's X4 Air
captures spanning three and a half months plus a ONE X2 clip, through the
shipped code (`kjerag_render::seam`, which is this section's own fitter moved
out of the instrument so there is one of it). Instrument for the tables below:
`kjerag-spike --bin leftover`, which scores any candidate correction against
the frames the app itself reads.

**The correction belongs to the camera.** The five knobs are fitted once, on a
capture from a camera that is not moving, and stored under a serial-free
camera key. On the owner's static capture, 13 azimuths, principal point held
by a ridge of 0.05:

| knobs | roll | yaw | pitch | cx | cy | along | across |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| rotation | +0.702 | -2.605 | +0.176 | | | 0.805 -> 0.384 | 2.282 -> **0.110** |
| five, free | +0.810 | -2.352 | -0.678 | -4.18 | -13.91 | 0.805 -> 0.030 | 2.282 -> 0.103 |
| **five, ridge 0.05** | **+0.789** | **-2.450** | **-0.668** | **-2.55** | **-13.84** | 0.805 -> **0.022** | 2.282 -> **0.106** |

**That answer, applied unchanged to every other file**, against what each file
asks for on its own. `own` is that file's own five-knob ridged fit, which is
what the fallback path computes; `rotation` is what PR #87 shipped:

| file | azimuths | transfer along | own along | rotation along | transfer across | rotation across |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| static, 07-31 12:07 | 13 | **0.022** | 0.022 | 0.384 | **0.106** | 0.109 |
| flight, 04-10 | 23 | 0.673 | 0.594 | 0.650 | 0.919 | 0.737 |
| flight, 05-01 | 25 | **0.213** | 0.211 | 0.362 | 0.745 | 0.616 |
| flight, 05-26 | 40 | **0.154** | 0.137 | 0.308 | 0.485 | 0.426 |
| flight, 07-14 | 29 | **0.176** | 0.174 | 0.360 | 0.757 | 0.614 |
| flight, 07-25 | 16 | **0.220** | 0.087 | 0.395 | 0.525 | 0.338 |
| static, 07-31 12:12 | 5 | 0.483 | 0.305 | 0.548 | 1.270 | 0.581 |
| ONE X2, 2025-10-18 | 2 | one lens stream per file: no seam, no fit |

Four things are in that table, and the first of them is what the rest have to
be read against.

**A file's own fit cannot be judged by the residual it leaves on that file**,
because that residual is the quantity it minimized. The `own` column is a
floor, not a score. So the case below rests on the two columns that are not
in-sample: what the per-file fits say about each other, and what any of them
does on a capture with no scene in it.

**The per-file fits do not agree with each other**, and every one of them is a
fit of the same glued pair of lenses. Across the five flights the same body's
own five-knob answer runs roll +0.58 to +0.90, yaw **-1.69 to -2.58**, pitch
-0.80 to -1.63, cx -1.3 to -9.5 px and cy **-5.4 to -18.6 px**. The yaw alone
spans 0.89 degrees, which at the seam is 15 px of a 1920-wide 90-degree view,
and cy spans 13 px of the delivered frame. A camera's extrinsics did not move
15 px between April and July. What moved is what the camera was pointed at.

**Along the seam, one camera's answer lands where each flight's own fit
lands**, and well inside the per-file rotation PR #87 shipped: 0.15 to 0.22
degrees against that rotation's 0.31 to 0.40, and within 0.002 degrees of the
`own` column on three flights of five. That column is the one parallax cannot
reach at all (4.9), so it is calibration and nothing else, and a correction
fitted on a capture months away from a flight reading the same number there as
a fit taken on the flight itself is what a per-unit constant looks like. Two
flights do not match: 07-25, whose own fit reaches 0.087, and 04-10, where the
transfer reads 0.673 against that file's own 0.594 and the rotation's 0.650.
04-10 is the outlier in every direction: its scene is 2.6 m away, and 1
azimuth of 23 correlates at all three places in the file.

**Across the seam the transfer reads higher on flights**, 0.49 to 0.92 against
0.34 to 0.74, and that is the same finding from the other side. The across
column carries parallax as well as calibration; a per-file fit pulls its tilt
until the mean of that column is flat, which on a flight means it has absorbed
that flight's own near-field disparity into a number applied to the whole
sphere. The measurement that separates them is the static capture, where the
content is 580 m away and there is no parallax to absorb: the transfer leaves
**0.022 along and 0.106 across**, which at this view's 16.8 px per degree is
**1.8 px typical and 4.6 px worst**, against 6.7 and 9.1 for the rotation fit
PR #87 shipped. Far content is where a per-camera answer is exactly right and
a per-file one is not.

**Applied and re-read on the pixels**, which is the only test of a fitted
parameter and is the one thing above that is not a prediction through the map:
the calibration put into lens 1, the file decoded again, and the seam
correlated again (`--bin seam mode=fit fix=...`, whose `before` column is the
re-read). The correction turns lens 1, so the seam circle moves with it and
the patch set is not the same one: 27 azimuths correlate on 04-10 before and
18 after, 31 and 16 on 07-14. What the two columns compare is the seam this
file has, twice.

| file | factory along | factory across | calibrated along | calibrated across |
| --- | ---: | ---: | ---: | ---: |
| static, 07-31 12:07 | 0.805 | 2.282 | **0.019** | **0.147** |
| flight, 04-10 | 0.743 | 1.689 | 0.139 | 0.609 |
| flight, 05-01 | 0.964 | 2.149 | 0.355 | 0.569 |
| flight, 05-26 | 0.888 | 1.851 | 0.190 | 0.599 |
| flight, 07-14 | 0.888 | 2.178 | 0.259 | 0.835 |
| flight, 07-25 | 0.867 | 1.944 | 0.121 | 0.572 |
| static, 07-31 12:12 (deck, refused a fit) | 1.027 | 1.381 | 1.051 | 1.273 |

The deck capture is the row that does not move, and it is the row that should
not: its seam looks down at decking 5 to 30 cm away, where a 33.4 mm baseline
puts 6 to 38 degrees of disparity, so what those seven patches read is
parallax and no rotation of a lens can take it out. Every other row is the
whole correction landing.

**The thin captures are refused, not fitted.** Counted by the shipped fitter
itself (`--bin seam mode=fit`, the app's own reading plan), the 12:12 deck
capture correlates **7** of 72 azimuths and the X2 clip **3**, both under
`PATCHES_NEEDED`, which is twice the knob count. Left to run, the deck
capture's free five-knob fit asks for a **-55 px** principal point and a yaw
of **+2.28**, the opposite sign to every other capture from that camera; the
ridge pulls the point to -21 px and the yaw is still wrong. The X2's five
knobs are singular on three patches outright. That is what the count is for,
and it is why the ridge is a prior rather than the whole guard.

**What names a camera** is `CalibrationSet::camera_key`: the model name, the
delivered frame size and every number of the factory `offset_v3` string,
FNV-1a hashed. Measured over the owner's captures, it is
`d8a393389b7b8639` for all seven X4 Air files from April to July and
`5381a2bf9e3d39bd` for the ONE X2. It is not the serial and it is not derived
from one; the serial and the GPS track live in the same metadata record and
neither is in the hash. The delivered frame size is in it on purpose: the
principal-point half of a correction is in delivered-frame pixels, so a
capture mode that delivers a different frame is a different key rather than a
correction scaled wrong.

**What it costs.** Nothing at open: the correction is five numbers in the
pilot's config, applied to the calibration the trailer already parsed, before
the first frame is drawn. The fit itself, when the pilot asks for one, is
**1.4 to 2.1 s** and essentially all of it is decode; the least squares is
under a millisecond, three Gauss-Newton rounds included. It runs on a worker
thread, so the window keeps playing while it reads.

**The landing step is gone**, and here is the number for it. The 12:07 static
capture rendered through the app's own path at 0.2 s and at 2.8 s, at a view
across the seam, 1920 px wide:

| pair | PSNR |
| --- | ---: |
| factory at 0.2 s against factory at 2.8 s (the scene alone) | 26.74 dB |
| **calibrated at 0.2 s against calibrated at 2.8 s (this branch)** | **26.61 dB** |
| factory at 0.2 s against calibrated at 2.8 s (what PR #87 played) | 16.92 dB |

The old first play crossed those two rows: the first seconds were the factory
calibration and everything after the fit landed was not, and the picture moved
by the whole correction while the pilot watched. This branch has no such pair,
because there is nothing to land later; what is left between two frames of the
same capture 2.6 s apart is what moved in front of the camera.

**A camera with no stored calibration still gets one**, fitted off the file
being played, off the decode path, landing a second or two in. That is the old
per-file path demoted to a fallback, and the table above is what it is worth:
better than the factory calibration by a long way, better than the rotation on
the along-seam axis, and carrying whatever parallax that file's seam had.

#### Phase 2: what the 2 degree crossover did to the picture

The band ships as measured. On the flight frame above, scored the same way as
the table in "what a narrower blend is worth":

| | doubled band | sharpness |
| --- | ---: | ---: |
| shipped weights, before | 10.60 deg | 0.723 |
| shipped weights, after | **1.50 deg** | **1.074** |

A sharpness of 1.0 is a band no softer than the front lens's own picture over
the same pixels, so the crossover is now doing what a blend is supposed to do:
hiding the join without smearing what crosses it.

**Against Insta360's own stitch**, the benchmark this issue is measured
against, on the 12:07 capture and their export of it:

| picture | band over its own surroundings |
| --- | ---: |
| Insta360's export | 0.92 to 0.97 |
| ours, before | 0.579 |
| ours, correction only (phase 1) | 0.689 |
| **ours, correction and 2 degree band** | **0.871** |

Four fifths of the gap to their stitch, closed. (Their own number moves with
the view the projection fit lands on, 0.965 in the run scored against our
0.579 and 0.924 in the run scored against our 0.871, which is why it is quoted
as a range.) The second static capture cannot be scored this way at all: the
view its projection fit lands on has flatter surroundings than band, and their
own share comes out above 1, which is the metric saying so.

**A file that gets no fit keeps the narrow band, and that is a choice with a
picture behind it.** The guards refuse a fit where the seam has too little to
correlate: on a ONE X2 clip (issue #79's camera, now paired at open) only 2 of
72 azimuths correlate, the rotation those two ask for is 16 degrees, and both
the azimuth count and the runaway bound catch it independently. The file then
plays on the factory calibration with a 2 degree crossover, which is what this
section warned against: at 2 to 3 degrees of disparity the shear is over 1 and
the crossover folds rather than blends. Rendered both ways at a view where a
ridge crosses the seam, the fold is the better picture: the 12 degree band
smears the hillside texture across a third of the frame, and the 2 degree band
leaves the picture sharp with a local step in it. The width is one constant if
that judgement is ever reversed, and the X2's own overlap is 11 degrees, so
there is no hole either way.

**The narrower band did not turn the exposure difference into a line**, which
is the thing a 2 degree crossover could have cost that 6.3 leaves open.
Measured on flat sky over the seam, where a step would show if it showed
anywhere in this footage: the luma runs 147.8 to 153.8 codes over 70 px with
the shipped band before and 147.1 to 154.4 after, the same ramp. What differs
between the two lenses there is vignetting inside each lens's own picture,
which the band width cannot spread and the coverage-depth factor already
down-weights.

**The pass costs what it cost.** Interleaved runs of 20 s of playback at
2560x1440 under live decode: 3.70, 3.77 and 3.83 ms per redraw before, 3.59,
3.59 and 3.83 after. The crossover reads the two axis cosines the coverage
test already needed, so the arithmetic added is a subtract, a divide and a
clamp per pixel.

That last number took three attempts, and the two that failed are worth
recording. Reading the two lenses' angles back out of the `Blend` array after
the loop that fills it -- the obvious way to write this, since the landings are
right there -- costs **5.5 ms per redraw against 3.6**, because a value read
back out of an array cannot stay in registers. `kjerag-spike --bin zoom`,
which renders the same pass with nothing else on the GPU, reads the two
versions as **equal**; only `--bin playback`, under live decode, shows it.

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

For Kjerag this is not a nice-to-have. Without it, a reframed view
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
container PTS. **Confirmed and measured in 8.6**, where the two clocks are
compared frame by frame on two 30-minute captures.

The failure mode for all of this is a **swimming horizon, not a crash**.
8.5 is the harness that was built instead of the Studio diff, and where a
Studio export drops into it.

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

**And the deeper trap, which is why Kjerag transcribes none of it (8.5).**
A three-letter string is only half of a convention; the other half is the
frame it lands in, and that frame is whatever the project it came from
composes next. Kjerag's chain is its own (`Rz(roll) Ry(yaw) Rx(pitch)`
inverted, into the body frame of 4.8), so a string copied from a table
written against a different composition means nothing here. The table in
`kjerag_meta::imu_orientation` is measured against footage instead, and 8.5
is the measurement.

### 8.5 The IMU convention, the filter, and how both were settled

**Confidence: HIGH for the X4 Air, and nothing here is transcribed.**
Settled during issue #8 against real footage, with the same kind of
instrument 4.8 and 4.9 used: render the app's own pass and look at what
comes out, except that this time a program looks rather than a person, so
that the answer is a number.

#### The instruments

`crates/spike/src/skyline.rs` finds the horizon in a rendered frame: the
topmost strong brightness step in each scan line, a straight line through
them weighted by sharpness, refitted twice with the outliers dropped. Three
things make it survive this footage, and each was added because it failed
without it:

- **topmost, not sharpest**: looking down from a wing the ground has harder
  edges than the horizon, and taking the sharpest finds a road in half the
  columns;
- **a step between two regions**, at least 12 codes across a band either
  side, not just a local gradient: noise clears any gradient threshold, and
  taking the topmost place it does so draws a perfectly straight line along
  the top of the search band;
- **both scan directions**: this camera is clamped rolled about a quarter
  turn, so an unstabilized view has its horizon running *down* the picture,
  and a line fitted as `y` against `x` cannot represent that at all.

Its own tests are the positive control: a synthetic horizon tilted by a
known angle, from -88 to +88 degrees, reads back within 0.2 degrees, and it
resolves 0.1 degrees. Its negative control is a picture of noise, which must
read back nothing rather than zero.

`crates/spike/src/bin/horizon.rs` drives it. `sweep=1` answers the axis
convention; with no arguments it measures the horizon across a run of frames
several ways at once, on the same pixels.

#### Which axis convention (settled)

For each of the **24** three-letter conventions that are rotations (48
strings have three different letters; half are reflections, and an IMU's
three axes are right handed by construction), compare the accelerometer's
idea of up against the horizon in an **unlocked** rendered frame. Unlocked,
the view is in the camera body's own frame, so a horizon found in it names a
great circle there and the normal of that circle is the true vertical in
body coordinates. No lock, no filter and no gyro are involved, so what is
being tested is the axis map alone.

| stretch | frames | best | second best |
| --- | ---: | --- | --- |
| capture A, 700 s | 50 | **xZY, 7.66** | yzX, 32.42 |
| capture A, 1100 s | 50 | **xZY, 8.81** | ZyX, 15.14 |
| capture A, 1300 s | 50 | **xZY, 3.56** | yzX, 32.71 |
| capture A, 1500 s | 50 | **xZY, 4.74** | yzX, 28.80 |
| capture A, 1700 s | 50 | **xZY, 3.02** | yzX, 36.36 |
| capture B, 400 s | 50 | **xZY, 12.19** | ZYx, 36.57 |
| capture B, 1400 s | 50 | **xZY, 2.32** | YxZ, 19.30 |
| capture B, 1600 s | 50 | **xZY, 7.74** | Zxy, 37.04 |

Mean degrees between the accelerometer's up and the picture's, over frames
where a horizon was found. `xZY` wins every stretch of both captures and the
runner-up changes from stretch to stretch, which is what a winner and noise
look like. What is left, 2.3 to 12.2 degrees, is the accelerometer's own
disagreement with vertical in flight: it measures specific force, and a
paramotor in air is not in free fall.

Reproduce with
`cargo run --release -p kjerag-spike --bin horizon -- <file.insv> from=1500 sweep=1`.

#### The datum belongs to the picture, not to the sensor (settles a 4.8 leftover)

4.8 recorded two readings of where its quarter turn comes from and said
nothing downstream could tell them apart: either `roll` is measured from the
delivered frame's horizontal axis, or the camera delivers the sensor image
already turned a quarter turn.

**The IMU tells them apart, because it is bolted to the sensor and not to
the picture.** Held level by its accelerometer alone, an X4 Air comes out a
quarter turn on its side when the IMU is taken through `Rz(roll - 90)` and
level through `Rz(roll)`. So the sensor image really is delivered rotated,
and the datum is the picture's. `kjerag_meta::Pose` carries both:
`lens_from_body` with the datum for the reprojection pass, `sensor_from_body`
without it for the IMU.

Note also that the two are not interchangeable by moving the quarter turn
around: `Rz(roll - 90) Ry Rx` and `Rz(roll) Ry Rx Rz(-90)` are different
rotations wherever yaw and pitch are not zero, and on the X4 Air fixture the
difference moves the seam crossover of 6.6 by 3 percent of the blend band.

#### What the filter is, and every constant in it

A complementary filter: integrate the gyroscope, and turn the estimate
towards the accelerometer slowly. `crates/meta/src/orientation.rs`, about
sixty lines. No Kalman filter: it would estimate the same two states with a
covariance nobody can populate from a file that records no noise figures, and
nothing measured here asks for one.

| constant | value | why |
| --- | ---: | --- |
| `accel_seconds` | 1.0 s | the IMU runs at **997 Hz** and a paramotor engine at about 80, so the raw signal is mostly vibration: the raw magnitude runs 0.69 to 1.63 g between the 10th and 90th percentile and the same signal smoothed over a second runs 0.95 to 1.05 |
| `tilt_seconds` | 20 s | it exists to cancel gyroscope bias, and it settles at `tilt_seconds * bias`. The quietest 10 s of capture A reads **0.25 deg/s**, so 20 s is worth 5 degrees at the worst and about 1 at the bias actually seen in flight. Longer rejects turns better and settles further off |
| `yaw_seconds` | 3 s | the one number that is a judgement about flying. Measured on capture A: at 3 s the view's worst heading swing inside a second is 29 degrees against 103 unstabilized, and it still follows 946 degrees of real turning a minute against 986 unstabilized. At 10 s the swing is 16 but the turning followed drops to 697, which is a view that fights a deliberate turn |
| `trust_g` | 0.05 to 0.20 | an accelerometer cannot tell gravity from a turn: in a 45 degree bank the specific force is 1.41 g and points along the aircraft's own vertical. Outside the window it is not believed at all |

The IMU rate is **997 Hz on the X4 Air and 500 on a ONE X2**, which is 1.8
million samples on a 30-minute capture. The solved orientation is decimated
to 200 a second before it is stored, 14 MB instead of 72, still three times
finer than the 15.9 ms rolling-shutter readout it exists to serve (issue #9).

#### Does the horizon stay level

Rendered runs of consecutive frames, 960x540 at 100 degrees, horizon angle
measured in every frame.

| stretch | frames found | mean | sd | peak to peak | worst per frame |
| --- | ---: | ---: | ---: | ---: | ---: |
| capture A, 1500 s, calm | 120/120 | -0.38 | 0.04 | **0.23** | 0.15 |
| capture A, 1043 s, 61 deg/s roll | 111/120 | 5.88 | 0.61 | **2.86** | 0.67 |
| capture B, 1400 s | 78/120 | -1.38 | 0.68 | **2.73** | 0.61 |
| capture A, 1500 s, **lock off** | 71/120 | 22.75 | 0.98 | 3.53 | 0.72 |
| capture A, 1043 s, **lock off** | 0/120 | the horizon is not in the picture in any frame | | | |

All in degrees. The unlocked rows are aimed at the horizon on their first
frame and then left alone, which is the fair comparison and is also why they
stop finding one. The 5.88 degree mean at 1043 s is the coordinated-turn
lean the filter cannot remove and does not claim to: that stretch is a
wingover.

#### The negative control

A deliberately wrong axis convention has to fail loudly, or the numbers above
are not measuring the horizon. `Xyz`, which is what telemetry-parser falls
through to for an `Insta360 X4 Air`, run through the same pass on the same
frames:

| stretch | frames found | mean | sd | peak to peak |
| --- | ---: | ---: | ---: | ---: |
| capture A, 1500 s | 111/120 | -55.23 | 65.35 | 179.63 |
| capture A, 1043 s | 14/120 | -6.81 | 54.14 | 174.62 |
| capture B, 1400 s | 0/120 | no horizon in any frame | | |

Against 0.04 to 0.68 degrees of standard deviation for the right answer.
The instrument can tell a wrong answer from a right one by three orders of
magnitude, and the 24-way sweep above is the same control run 23 times.

#### The ONE X2 is a different mounting, and the sweep alone cannot settle it (issue #79)

**Confidence: HIGH for the answer, and the interesting part is the
method.** Measured 2026-07-31 on three ONE X2 captures, after issue #79's
pairing made the whole sphere available to look at.

The X2 wants **`Zxy`**, not the X4's `xZY`. Held by the X4's string, an X2's
accelerometer points **121 degrees** from where the picture says up is. That
is the owner's "horizon is way wrong", and most of his "upside down" as
well: the player locks the horizon by default, so a wrong vertical arrives
as a picture turned over.

**The 24-way sweep narrows it to two and stops.** Eight stretches of the
three captures, 120 rendered frames each:

| stretch | frames | best | second |
| --- | ---: | --- | --- |
| 191318, 28 s | 120/120 | **zYX, 8.99** | Zxy, 19.66 |
| 191318, 60 s | 52/120 | **Zxy, 10.37** | zYX, 14.48 |
| 191318, 120 s | 91/120 | **zYX, 18.25** | Zxy, 19.70 |
| 191318, 200 s | 120/120 | **Zxy, 17.49** | zYX, 29.38 |
| 193615, 5 s | 120/120 | **Zxy, 19.36** | zYX, 20.68 |
| 193615, 30 s | 120/120 | **zYX, 9.65** | Zxy, 19.45 |
| 184419, 5 s | 120/120 | **Zxy, 5.86** | zyx, 25.92 |
| 184419, 40 s | 64/120 | **zYX, 35.52** | XZy, 57.17 |

`zYX` and `Zxy` take first and second between them in seven of the eight,
and the other 22 are nowhere. Read against 8.5's own criterion that is not
a winner and noise, it is two winners: the pair are a **half turn apart
about `(1, -1, 0)`**, and on this camera's resting attitudes that half turn
moves the accelerometer's up by as little as 13 degrees, so which of them
wins a stretch is decided by whatever else is in the error.

**What else is in the error is the reference.** This footage is a mountain
launch and there is no true horizon in it: the sky-to-ground line
`skyline` locks onto is a **ridge**, and a ridge is not level. That is the
same limitation PR #51 recorded for the X2 clip, and it is why the residual
here is 6 to 35 degrees where an X4's is 2 to 12.

#### So the last step is not a horizon at all

**Aim the view along what the accelerometer calls up, on a frame where the
camera is not moving, and look at what is there.** At rest the
accelerometer is gravity and nothing else, so the right convention points
at the sky by physics, with no line to fit and nothing to be level.

Three instants across two captures, each with the camera under 5 deg/s and
inside 0.015 g of 1 g, chosen because at each of them the two candidates
point a half turn apart rather than 13 degrees apart:

| capture, instant | `zYX` points at | `Zxy` points at |
| --- | --- | --- |
| 184419, 1.0 s | bare dirt | sky, and a helmet from below |
| 184419, 5.0 s | dirt, and a pair of boots | sky, a helmet and the lines |
| 191318, 1.0 s | dirt, and a pair of boots | sky, and a helmet from below |

A pair of boots seen from above is the nadir. Three for three, and the
renders are one command each:

```sh
cargo run --release -p kjerag-spike --bin reframe -- <file.insv> \
  time=5 yaw=15.1 pitch=39.4 fov=90
```

The yaw and pitch come from the candidate itself: `body_from_imu(axes,
lens0) * accel`, aimed with `pitch = -asin(d.y)` and `yaw = atan2(d.x,
d.z)`. Run the same command with the other candidate's angles and the
picture is the ground.

#### The negative control, on the X2

The shipped `xZY` run through the same pass on the same frames, and the
answer is the loudest kind there is:

| variant | frames with a horizon | mean | sd | p-p |
| --- | ---: | ---: | ---: | ---: |
| `Zxy` (the file's own now) | 10/120 | 4.92 | **0.11** | **0.33** |
| `xZY` (what shipped) | **0/120** | | | |
| `Xyz` (telemetry-parser's X4 fall-through) | 0/120 | | | |
| `Xyz`, on a rollier stretch | 12/120 | -72.04 | **48.69** | 179.14 |

A horizon 121 degrees off level is not in a picture aimed at where it ought
to be, which is the same failure #40 saw and is why the count of frames is
read next to every number. The 4.92 degree mean is the ridge's own slope,
not a lock error; the 0.11 and 0.33 are the lock.

**telemetry-parser's own X2 string is `xZy`, and it is not this answer.**
In Kjerag's frame `xZy` has determinant -1, so it is a reflection rather
than a mounting and the sweep does not enumerate it at all; `horizon`
carries it as a standing wrong answer, where it puts the line 22 degrees
from where `Zxy` puts it. That is 8.4's point with a number on it.

#### Where a Studio export drops in

`horizon.rs` compares variants that differ only in how the picture is held.
An Insta360 Studio export of the same clip is one more variant whose frames
come from a second file rather than from this pass; measured with the same
`skyline`, its row is directly comparable to ours, same frames and same
units. That is the only change it needs, and it is written down at the top
of that file. Until then the reference is physics: a horizon is level, and
an accelerometer at rest reads 1 g.

### 8.6 Which clock the frames are on (settled)

**Confidence: HIGH.** `pts_type = 2` means what its name says: the exposure
records are the camera's own frame clock, and Kjerag aligns the gyro to them
(`ExposureTrack::frame_time_us`).

The container's PTS is a nominal 30000/1001 grid. The camera's own
timestamps drift away from it **linearly, at 6.4 parts per million**, which
is a real sensor clock against a nominal rate. Measured on capture A:

| frame | container PTS | exposure record | apart |
| ---: | ---: | ---: | ---: |
| 0 | 0.000 s | 0.000 s | 0.000 ms |
| 5394 | 179.980 s | 179.981 s | 1.156 ms |
| 13485 | 449.950 s | 449.952 s | 2.876 ms |
| 26970 | 899.899 s | 899.905 s | 5.754 ms |
| 40455 | 1349.848 s | 1349.857 s | 8.635 ms |
| 53886 | 1797.996 s | 1798.008 s | **11.497 ms** |

11.5 ms is 0.345 of a frame. Frame 0 is not sample 0: the camera writes
eight exposure samples before it commits the first frame, so the track holds
54017 samples for 53940 frames and reading it from index zero would put
every frame's gyro lookup 267 ms early.

**What the choice is worth**, which is the only form of the question that
matters, because a few milliseconds cost the horizon the camera's own
rotation over those milliseconds: looked up on both clocks and compared as
orientations, over two 30-minute captures, the two put the camera **0.10 and
0.15 degrees apart on average and 0.95 and 1.48 degrees apart at the worst
instant**.

**The loser, and the margin.** Rendered, the two are indistinguishable:
0.23 against 0.22 degrees peak to peak on the calm stretch, 2.86 against
2.69 on the rolly one, 2.73 against 2.96 on capture B, all differences
smaller than the scatter between stretches. So the picture cannot separate
them on this footage, and the case for the camera's clock is the ~1 degree
bound above plus the reason it is the right one anyway: the gyro timestamps
come off the same camera clock as the exposure timestamps, so aligning them
to each other is self-consistent, and it costs nothing.
`FrameClock::Container` is kept so the losing hypothesis stays measurable.

### 8.7 The horizon dip is a bad start, not a bad constant (issue #45)

**Confidence: HIGH for the mechanism and the size, measured 2026-07-31 on
two X4 Air captures through the render path.** This section replaces an
earlier 8.7 that went out with the revert of PR #51. What survives of that
one is the instrument; the rest is withdrawn below.

With the lock on, the horizon starts a file tens of degrees off level and
walks back over tens of seconds. Panning a circle while it does that is
where a pilot meets it, because a tilted estimated vertical puts the
horizon `atan2(sin e sin(yaw - phi), cos e)` off level, which dips once
each way per revolution.

#### The mechanism

`Filter::solve` seeded the estimate from whichever tilt put **the first
tenth of a second** of accelerometer on the world vertical, and it did that
whatever that tenth of a second read. The running filter refuses a reading
further than `trust_g.1` from 1 g outright. Both were true of the same
code, and the first tenth of a second is exactly the part of a paramotor
file most likely to be a launch:

| file | first 0.1 s | horizon at 6 s |
| --- | ---: | ---: |
| April 10 | **1.281 g** | **48.9 deg** |
| June 23 #1 | 0.737 g | 14.7 deg |
| June 23 #2 | 1.162 g | not measured |
| June 23 #3 | 1.063 g | not measured |

Per-file severity follows what that tenth of a second happened to read,
which is why the same code looked fine on some files and not on others.
The estimate then walks back at `tilt_seconds` divided by the share of
samples the trust window lets through, which on this footage is tens of
seconds: the correction that has to undo a bad start is the same slow one
that exists to ignore turns.

#### The fix, and what it is worth

The seed now searches forward for the first window of accelerometer the
running filter would believe **completely**, and carries it back to the
start of the track with the gyroscope. Tilt of the estimated vertical,
measured through the app's own projection pass with
`kjerag-spike --bin dip`, 12 frames x 36 yaws at 100 degrees:

| seconds into the file | April, before | April, after | June #1, before | June #1, after |
| ---: | ---: | ---: | ---: | ---: |
| 6 | 48.93 | **1.88** | 14.70 | 8.11 |
| 10 | no fit | 1.39 | 12.88 | 6.52 |
| 15 | no fit | 1.43 | 10.78 | 5.01 |
| 20 | 36.59 | 0.62 | 8.56 | 3.70 |
| 30 | 29.21 | 3.93 | 5.97 | 2.46 |
| 45 | 18.65 | 3.41 | 4.91 | 4.42 |
| 60 | 8.12 | 2.50 | 4.24 | 3.01 |
| 120 | 2.71 | 1.76 | 4.71 | 4.49 |
| 300 | 2.77 | 2.77 | 3.23 | 3.23 |

Degrees. The 300 second rows agree to three decimal places on both files,
which is the control: nothing outside the opening transient moved. The "no
fit" rows are the defect hiding the evidence for itself, and they are also
a measure of it: at 6 seconds on the April capture only 50 of 432 rendered
views had a findable horizon in them at all, against **403** after, because
a horizon 49 degrees off level is mostly outside the frame.

Three choices inside the seed, each measured rather than assumed.

- **The first fully trusted window, not the first partly trusted one.** The
  running filter applies a fraction of a correction to a reading it half
  believes; a seed is applied whole. Taking anything inside `trust_g.1`
  leaves the April capture at **13.8** degrees at 6 seconds against **1.8**
  for the whole of it, on the same run of 24 frames, because the window it
  settles for is taken during the launch. Demanding the whole of `trust`
  moves the seed from 0.2 s into the file to 3.2 s.
- **A window as long as `accel_seconds`.** The running filter reads its
  trust off the accelerometer smoothed over that constant, so the seed
  applies the same test to the same kind of signal. The raw magnitude on
  this footage runs 0.69 to 1.63 g between the 10th and 90th percentile and
  crosses 1 g constantly, so a shorter window can take a magnitude that is
  only passing through 1 g for stillness.
- **Every sample carried back before it is averaged.** The window sits
  seconds into the file and the body turns to reach it, so each reading is
  rotated into the frame of the track's first sample before it goes into
  the mean, which makes the answer an attitude at the start of the track
  directly. It is also what recovers a reading from a tumbling launch at
  all: over the April capture's first second the plain mean of the
  accelerometer weighs 0.528 g, and the same samples de-rotated weigh
  0.942.

A second helping of the same defect went with it. The smoothed
accelerometer was initialised to 1 g along the estimated vertical, so it
walked from that fiction out to whatever the sensor really said, and
everything it passed through on the way was inside the trust window and
believed. It starts on the accelerometer now.

#### What is left, and what it is

The seed is as good as one second of accelerometer, and an accelerometer in
flight measures specific force. Running the same solve with the correction
effectively off (`tilt_seconds` 1e5) isolates it: at 6 seconds the seed
alone is **2.8 degrees** off on the April capture and **9.8** on June #1,
which is why June #1 still reads 8 degrees at 6 seconds where April reads
1.9. That is the residual issue #57 was written about, now at its real size
and in its real place, which is the seed window rather than the whole first
minute.

#### Withdrawn: the apparent-gravity attribution and the GPS prescription

The 8.7 that PR #51 shipped concluded that the dip was 1.9 to 8.5 degrees
of the filter's residual against apparent gravity, that the shipped
constants were the optimum of 16 tried, and that the fix was to parse the
GPS track and subtract the aircraft's own acceleration (issue #57). **The
size and the conclusion are withdrawn**, and the numbers above replace
them. The instrument, the axis-order result (all six orderings within 0.13
degrees) and the mounting negative controls stand.

The reason is that the instrument had a **20 degree gate**: a found line
more than that off level was taken to be a wing or a field boundary and
dropped. The defect is 40 to 50 degrees at the start of a file, so every
view of the real thing was dropped and the fit ran on what was left near
the zero crossings. The wandering phase that made apparent gravity look
right was that selection and not the flying: refitting the phase against
the body's own heading instead of the view's moves it by a few degrees
(172 against 170, -77 against -77, 84 against 88), so it was never a
coordinate error either.

**The injection control passed anyway, and that is the lesson.** Tilts of
1, 2 and -1 degrees read back as 0.97, 2.01 and -1.02, on a stretch whose
baseline was already small, so the control never left the range the
instrument happened to work in. A control has to span the regime being
measured. The gate is off by default now, what it drops is counted and
printed next to what it kept, and the acceptance run injects **45**:

| injected | tilt read back |
| ---: | ---: |
| 0 (baseline) | 2.648 |
| 1 | 1.689 |
| 2 | 0.833 |
| 45, perpendicular to the baseline | **45.101** |
| 45, the other way round | 44.438 |
| 45, with the 20 degree gate back on | 48 of 60 views dropped, **no fit at all** |

45.101 against the 45.08 that 45 degrees combined in quadrature with a
2.648 degree baseline predicts. The last row is the whole of the
mis-attribution in one line: the instrument that shipped could not measure
a 45 degree defect even when handed one deliberately.

Two things about reading a dip that size. The angle-against-yaw curve is
only a sinusoid for small tilts, so at tens of degrees the **tilt** column
is the one to read and the fitted amplitude is a fundamental rather than a
peak. And the horizon leaves the frame at the yaws where the tilt puts it
furthest off centre, because at 100 degrees of field the picture reaches 34
degrees above its own axis, so the views thin out exactly where the defect
is worst. That is the second reason to read the count of views next to
every number.

## 9. Prior art worth reading

- **`BenjaminHenriksson/insv-stitch`** (MIT, Python, X5) is by far the
  most valuable. ~1200 lines, reaches PSNR 22.5 to 22.9 dB against Studio
  at 7680 x 3840. Its `PIPELINE.md`, `old/FINDINGS.md` and
  `x5_pipeline.md` are the source for the measured numbers in sections 5
  and 6. Its design principle is Kjerag's: *"stitching, stabilization,
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
word "insta360". Kjerag is filling real empty space.

## 10. Unknowns

Ordered by how much they would cost us.

1. ~~**The sign and order convention for `yaw`/`pitch`/`roll`.**~~ Settled
   against rendered frames on two cameras, 2026-07-31: `Rz(roll - 90) *
   Ry(yaw) * Rx(pitch)`, and the quarter-turn datum is the finding (4.8).
   What is left of it is the **order** of the three, which no known camera
   can distinguish because every one of them records sub-degree yaw and
   pitch. Lens 1's nominal arrangement, which the same entry used to leave
   open, is settled in 4.9: a half turn about the body's vertical,
   multiplied on the right of the block's own angles.
2. **Vignetting coefficients are not in the metadata.** May show as
   rolloff in the blend band. Would need flat-field calibration.
3. ~~**`pts_type = VideoPtsEexposureFile` semantics.**~~ Settled in 8.6:
   they are the camera's own clock, they drift from the container's nominal
   grid at 6.4 ppm, and Kjerag aligns the gyro to them.
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

### 6.9 The band is read on every frame, and the reading is a distance (issue #103, stage 2)

**Confidence: HIGH for the cost and the flicker, MED for the residual.**
Measured 2026-08-01 on the shipped pass, on six captures from two cameras.
Instrument: `kjerag-spike --bin band`, which reads the state back out of the
very buffer `ScenePipeline` dispatches into while it draws.

6.8 corrects what belongs to the camera and leaves 0.12 to 0.36 degrees along
the seam and 0.57 to 0.84 across it on flights. The second of those is not
calibration: the baseline is 33 mm, so 3 m of subject distance is 0.64 degrees
of real displacement (6.1) and no rotation of a lens moves content that is at
two distances at once. So it is measured, per direction and per frame.

**Where the cost is, and it is not where it looks.** The pass is one compute
workgroup per direction: fill two grids from the two lenses' luma, correlate
them along the epipolar axis, filter, write four floats. The obvious
optimisation is to score each candidate shift on a fraction of the patch's
samples, which is exactly what `seam::best_shift` does and for exactly the
stated reason. Measured, it makes the pass **slower**: 9.1 ms per redraw at
2560x1440 under live decode against 8.4, because the refine it needs afterwards
is serial. The correlation is not the cost. **The fetch is**: 3733 taps of a
tiled 3840x3840 decoder surface per direction per frame, on an iGPU that has
VA-API decoding into that same memory. Three changes to what is fetched, and
nothing to how it is scored:

| | taps per direction | ms per redraw, added |
| --- | ---: | ---: |
| 0.08 deg step, search to 3.5 deg, whole ring each frame | 3733 | 3.2 |
| 0.10 deg step, search to 2.6 deg, half the ring each frame | 958 | **0.3** |

The step is resolution the parabola between whole steps gives back, and the
seconds of averaging over it give back again. The search stops where the fold
clamp does, because past 1.8 degrees every reading is clamped to the same
number and the window was being spent telling one clamped reading from
another. Half the ring is free: the filter is paced in **seconds of media
time**, so a direction read at 15 Hz and one read at 30 settle in the same wall
time, and only the near field notices.

Interleaved runs of the same binary, `noband` against `band`, 20 s at
2560x1440 under live decode: 5.09, 5.25, 5.31 ms per redraw against 5.43,
5.56, 5.61. Dropped frames are 2 either way on this file and this box.

**Why per-frame beats the per-clip table it was supposed to lose to.** Phase A
measured a naive per-frame table flickering 0.22 to 0.54 degrees rms frame to
frame against a static residual of 0.2 to 0.4 that it was meant to remove, and
called that a bad trade. It is, and the answer is not to pool: it is that far
field and near field want opposite time constants. A direction reading under
0.19 degrees is looking at something past 10 m, which does not move, and can be
smoothed for two seconds for free; a direction reading degrees is looking at
the wing and has to track. The constant is read off the **smoothed state** and
not off this frame's reading, so a noisy far-field reading cannot unlock the
smoothing that is keeping the horizon still.

Measured at 360 directions round the circle, where the bend is APPLIED rather
than where it was read:

| capture | flicker, deg rms | view px | far field only, deg rms | view px at fov 24.1 |
| --- | ---: | ---: | ---: | ---: |
| static 07-31 12:07 | 0.0077 | 0.13 | 0.0010 | 0.08 |
| flight 04-10 | 0.0140 | 0.23 | 0.0095 | 0.75 |
| flight 05-01 | 0.0199 | 0.33 | 0.0136 | 1.09 |
| flight 05-26 | 0.0182 | 0.31 | 0.0106 | 0.85 |
| flight 07-14 | 0.0225 | 0.38 | 0.0176 | 1.40 |
| flight 07-25 | 0.0161 | 0.27 | 0.0218 | 1.74 |

Ten to thirty times smaller than the naive per-frame table. The far-field
column is the one the pixel-perfect horizon claim rests on: those directions
are looking at things that do not move, so what is left over is the
measurement's own repeatability and nothing else.

**The control, and it is not optional.** A known step is put into the state
each frame, alternating sign, and a step of `s` has to come back at `2s` in
quadrature with what the file already had: 0.05 deg reads 0.1015 against 0.1020
expected, and 0.20 reads 0.4000 against 0.4005. A flicker column is a negative
result and means nothing until it is shown able to read a positive one.

**The shear guard exists now.** 6.8 bounds the crossover's width from below by
shear, the disparity divided by the band, and says that above 1 the crossover
folds rather than blends. The bend has the same bound and for the same reason:
its own gradient across the band **is** the shear, so past 1 the mapping prints
the picture back over itself. The applied disparity is clamped to nine tenths
of the crossover. This is the first time that number has been computable at
runtime rather than quoted; what it clamps is content nearer than about 1.9 m,
which is nearer than the camera maker's own manual asks a subject to be, and
widening the band where it bites is stage 4's.

**What did not change.** A file with one lens stream has a front weight of
exactly 1 everywhere, so the bend each lens takes -- the OTHER lens's weight
times the disparity -- is exactly zero everywhere. Rendered against a build of
`main` in its own target directory, a ONE X2 one-lens file is byte-identical at
yaw 0, 45, 90, 180 and 270; a two-stream flight file is byte-identical at yaw 0
and 180 and differs at yaw 90; and the 12:07 static capture is byte-identical
even across its seam, which is its own result -- it is the capture whose
calibration already lands, and the band finds nothing there to correct.

**The benchmark this section does not reach.** 6.8's band-over-surroundings
share cannot score the 0:09 wing dip against the camera maker's export: the
projection fit correlates 0.7262 there against the 0.977 to 0.9996 a locked fit
gives, and at the one instant nearby where it does lock both shares come out
above 1, which is the statistic saying it does not apply. Scored against our
own picture instead, each its own control, the band takes 05-01's wing crossing
the seam from **0.930 to 0.954**. Also recorded because it was wrong in the
record: TEST.mp4 is the first 20.22 s of `VID_20260501_183417_00_003.insv`, not
of `_00_001`, settled by cross-correlating the two files' own audio (r = 1.000
at offset 0.0 s against 0.64 and 0.67 on the other parts).
