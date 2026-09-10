# Studio 6.0.2 denoiser configuration inputs

Bounded read of the saved worker and application resources, 2026-09-10.
This recovers table data and selector conditions, not a completed automatic
settings provider. No extracted vendor source or resource is production code.

The worker SHA256 is
`0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452`.
The private receipt, disassemblies, resource reader and hashes are under
`scratch/studio-seam-flicker-612-20260909-01/denoise-auto-config-01`.
The independent selector geometry/rate receipt is
`scratch/studio-seam-flicker-612-20260909-01/denoise-selector-source-01/README.md`,
SHA256 `3144e9db4d79fd0ed9398b2766aa3f6209c2d432f2dc9d14227ef17e7fe54190`.
The resource JSON version is `1-3.3`, SHA256
`d65f45de8222e72da5c0a8e4052a0cc547e78d03bfcd52729d9083c64a901a33`.

## Producer and selection

`GenerateDenoiseConfig` (`0x68ddd8`) takes the selected key's `content`, or
the `common` fallback, from `denoise_param_table`. `PrepareInternal` passes
the resulting `{version, info, content}` to `BlockDenoise::Open`, whose
initialization calls `DenoiseFile::ParseParam`. `SetExtConfig` is a separate
update entry, not this initial route.

The actual movie caller is `ProjectExporter::GetBlockDenoiseData`
(`0x16e6f8`), calling the video overload of `GetDenoiseMapKey` at
`0x16e7f8..0x16e804`. That overload (`0xe46ac8`) supplies camera type,
metadata width/height, source fps, export scene 0 and `getGroupType()`.

ONE X2's native camera type 10 (`OneXS`) takes the unmatched-camera `common`
fallback. X4 Air instead dispatches to `GetDenoiseMapKeyFromX4Air`
(`0xe47628`), then export selection (`0xe47f84`). Its jump table at
`0x3e417f7` sends **both group type 0 and group type 8** to the same handler at
`0xe47fc4`. In that handler, maximum dimension 7680 and source fps below 30.1
select `x4a_sp`. That is not the geometry arm used by this source.

`getGroupType()` loads metadata `+0xac`. The video constructor populates it
through `InstaHelper::GetSourceFileType(asset, true, source_fps)`; source-group
initialization can copy it from `SourceGroup+0x58`. For INS video,
`GetSourceFileType` reads Asset metadata key `sub_media_type` at
`0xd5488c..0xd5496c`. `extra_info::SetupItems` lambda 78 (`0x1822e94`) maps
that key from `ExtraMetadata::FileGroupInfo.type`; binary field-number
constants identify top-level `FileGroupInfo` as protobuf tag 26 and nested
`type` as tag 1.

A private value-only read of the April X4 Air metadata record, retaining no
raw metadata or PII, proves one tag-26 message with nested type **0**. With
`VideoAsset::IsVideo()==true`, INS format, and the normal non-bullet branch,
`GetSourceFileType` therefore returns 0. Kjerag's current `ExtraMetadata`
declaration does not expose tag 26; the smallest parser addition is this
nested type, not Studio's full asset classifier. The denoiser's captured
backend **mode 8 is a different enum** and was never evidence for source group
type 8.

The finished export's dimensions/fps do not by themselves authenticate the
metadata inputs to this selector. Matching the captured ISO100 values to a
table row is likewise insufficient evidence.

The independent source audit closes the actual inputs. `VideoMetaData` stores
`ResolveResolution()` at `+0x180` (`0xb29178..0xb29184`). That resolver obtains
the AssetInfo `capture_dimension`, whose registered getter
`SetupItems::$_98` (`0x182513c`) reads protobuf `resolution_size`. The April
trailer value and both video streams are 3840x3840. Because source type is 0,
the type-2-only square-to-double-width rewrite at `0xb29f34..0xb29f84` does
not run; flow-state correction is inactive and the common-resolution path
preserves the square. The selector therefore receives **3840x3840**, not the
finished panorama's 7680x3840.

`VideoMetaData::GetSourceFps` (`0xb3015c`) calls
`VideoAsset::VideoFramerate`; on this non-timelapse input its ordinary path
reads the MediaInfo rational (`0x17143f4..0x1714424`) and narrows the result to
f32 at `0xb301b4`. Both source streams carry 30000/1001, so the selector value
is **29.970029830932617** (`f32` bits `0x41efc29f`).

For group 0, the X4 Air handler first rejects its 7680 and 6016/6144 geometry
arms, then accepts maximum dimension 3840 at `0xe4817c..0xe48184`. Its source
FPS test is below 51.6 at `0xe48188..0xe481a0`, which this value satisfies;
control reaches `0xe48240`, selecting **`x4a_sp`**. Thus the April X4 Air key
is proved by selector inputs and control flow, independently of the observed
700/3/10 values.

## Table law and effective fields

Rows supply ISO, one still-unnamed scalar, noise, temporal radius, limit and
current weight, followed by four count-prefixed float vectors and optional
guided/detail fields. The radius field was initially labelled `fast_level`;
its actual consumer controls the number of past/future references, not the
independent half-resolution search-domain shift.

| Key | ISO knots | Noise at knots | Radius at knots | Limit at knots |
| --- | --- | --- | --- | --- |
| `x4a_sp` | 100, 200, 400, 800, 1600, 2200, 5000 | 700, 900, 900, 1200, 1500, 1700, 2000 | 3 throughout | 10, 12, 14, 16, 18, 22, 26 |
| `common` | 100, 200, 400, 401, 800, 1600, 2200, 5000 | 100, 120, 150, 200, 400, 600, 800, 1200 | 0, 0, 0, 1, 2, 2, 3, 3 | 15 throughout |

`GetParameter` interpolates between ISO knots and clamps to endpoints. Integer
fields use truncation after adding 0.5. Noise supplies both the normalized fuse
noise and motion-confidence `scale_extra`; limit supplies the normalized fuse
limit. The captured noise700/limit10 values remain the only selected runtime
parameter authority used by the existing offline filter.

For both tables, all four vectors are constant: confidence Y/UV is 1/2 and
limit ratio Y/UV is 1/0.5. `GetTable` expands a non-256 count linearly over
endpoint-aligned positions `(count-1)*i/255`. All listed rows have current
weight 256 and their guided gate is zero, including the high ISO knots.
Later nonzero integers are tail fields, not that boolean. The unnamed scalar
and optional tail processing are not used by Kjerag's current normalized
fusion primitive and must not be assigned invented meanings.

The different radii matter for integration: ONE X2's example ISO365 falls
inside `common`'s radius-zero region. It does not request six temporal
neighbors merely because the X4 Air capture did. A zero-radius route must
not call the current fusion primitive, which requires at least one reference.
This observation alone does not implement Studio's zero-reference handling.

This closes the April X4 Air key, not every camera, group type or geometry
branch. Kjerag still needs protobuf tag 26 exposed before it can reproduce the
group input directly. Automatic table consumption, player lookahead and
short-input handling remain unfinished. No installed change or visible-quality
verdict follows from these recovered inputs.
