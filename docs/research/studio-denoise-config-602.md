# Studio 6.0.2 denoiser configuration inputs

Bounded read of the saved worker and application resources, 2026-09-10.
This recovers table data and selector conditions. Kjerag now has a narrow
automatic settings provider for the two authenticated routes, qualified by the
`temporal-auto-sequence-01` build and GPU/real-source test record. No extracted
vendor source or resource is production code.

The worker SHA256 is
`0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452`.
The private receipt, disassemblies, resource reader and hashes are under
`scratch/studio-seam-flicker-612-20260909-01/denoise-auto-config-01`.
The independent selector geometry/rate receipt is
`scratch/studio-seam-flicker-612-20260909-01/denoise-selector-source-01/README.md`,
SHA256 `91a1b0ddabbeb2aa305e04afd51897581e64c55ccfbb385b5d11f503818f6739`.
Its selected disassembly is SHA256
`a5b3c5b27ed90fefd696de99899079da773c5232fb0ffa0becbbbf3e41b74fd2`.
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
raw metadata or PII, proves one tag-26 message with nested type **0**. Kjerag
now exposes this optional value as `CalibrationSet::source_group_type`;
presence of the parent carrying zero is preserved as `Some(0)`, while an
absent parent is `None`. With
`VideoAsset::IsVideo()==true`, INS format, and the normal non-bullet branch,
`GetSourceFileType` therefore returns 0. This narrow nested value is not
Studio's full asset classifier. The denoiser's captured backend **mode 8 is a
different enum** and was never evidence for source group type 8.

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
FPS test is below 50.1 (immediate bits `0x40490ccccccccccd`) at
`0xe48188..0xe481a0`, which this value satisfies;
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

`GetParameter` interpolates between ISO knots and clamps to endpoints. Between
knots it forms the fraction in binary32, computes `(1-t)*lower`, then uses a
binary32 fused multiply-add for `upper*t + lower_part`. Integer fields add
binary32 `0.5` and truncate toward zero. Noise supplies both the normalized
fuse noise (`noise / (255*64)`) and integer motion-confidence `scale_extra`;
limit supplies the normalized fuse limit (`limit / 255`). The captured
noise700/limit10 values remain the selected runtime authority used by the
existing ISO100 offline oracle.

For both tables, all four vectors are constant: confidence Y/UV is 1/2 and
limit ratio Y/UV is 1/0.5. `GetTable` expands a non-256 count linearly over
endpoint-aligned positions `(count-1)*i/255`. All listed rows have current
weight 256 and their guided gate is zero, including the high ISO knots.
Later nonzero integers are tail fields, not that boolean. The unnamed scalar
and optional tail processing are not used by Kjerag's current normalized
fusion primitive and must not be assigned invented meanings.

The different radii matter for integration: ONE X2's example ISO365 falls
inside `common`'s radius-zero region. It does not request six temporal
neighbors merely because the X4 Air capture did. The bounded audit in
`scratch/studio-seam-flicker-612-20260909-01/denoise-zero-short-01` closes the
native filter behavior: radius zero sets reference count zero and selects the
backend's copy-current encoder. It is not an early scheduling bypass. The
unconditional seven-real-frame gate still runs before an output is selected;
after that gate, each radius-zero output is a copy of its corresponding
retained current frame.

At the same filter boundary, an end-of-input flush with only zero through six
queued real frames produces zero denoised outputs. The receive gate returns a
nonzero status before consulting flush, and the wrapper appends no frame. This
does **not** prove that a user-visible short clip or seek drops frames: whether
the higher scheduler extends the decoded interval enough to satisfy the gate
remains a separate boundary.

## Kjerag provider boundary

`crates/render/src/temporal_fusion/settings.rs` now owns an ISO-table
`Provider`. Construction snapshots the `DenoiseIsoTrack` through the existing
stateful source-time lookup and selects only:

- exact camera model `Insta360 X4 Air`, source group `Some(0)`, source
  dimension 3840x3840, and a finite positive source `f32` rate which, widened
  to binary64, is below the native 50.1 immediate
  (`0x40490ccccccccccd`); or
- exact camera model `Insta360 ONE X2`, whose recovered native dispatch uses
  the `common` table, with a finite positive source rate.

Other cameras, X4 Air groups and X4 Air geometries fail explicitly instead of
falling back to guessed settings. The authenticated April member of the first
predicate is 30000/1001 (`f32` bits `0x41efc29f`); the predicate is the read
selector branch, not a literal clip-name or rate match.

For each monotonic source time, the provider uses the recovered ISO lookup and
returns the interpolated ISO, radius, integer motion noise, confidence tables,
and normalized fusion noise/limit and ratio tables. Reset is explicit for a
seek, loop or source-epoch replacement. It neither owns a history window nor
decides what to present for radius zero or a short interval; those scheduling
and presentation boundaries remain outside this provider. Focused unit and
private-file tests pass, including the April607/612 ISO100 radius3 settings and
ONE X2 ISO365 radius0 settings. The actual-Scene offline consumer also produces
complete source-ordered April sequences using this provider, preserving every
previously captured full-window output. This is bounded implementation
qualification, not Studio-like visible quality or selected-player behavior.

This closes the April X4 Air key and the ONE X2 common fallback, not every
camera, group type or geometry branch. Selected-player lookahead/filter
publication, short-input presentation, installed behavior and visible-quality acceptance remain
unfinished. No installed change or flicker verdict follows from these
recovered inputs.
