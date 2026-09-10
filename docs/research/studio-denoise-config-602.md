# Studio 6.0.2 denoiser configuration inputs

Bounded read of the saved worker and application resources, 2026-09-10.
This recovers table data and selector conditions, not a completed automatic
settings provider. No extracted vendor source or resource is production code.

The worker SHA256 is
`0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452`.
The private receipt, disassemblies, resource reader and hashes are under
`scratch/studio-seam-flicker-612-20260909-01/denoise-auto-config-01`.
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
(`0xe47628`), then export selection (`0xe47f84`). The condition
**group type 8, maximum dimension 7680, source fps below 30.1** selects
`x4a_sp`. This is a conditional result, not a proved selector invocation on
the April file.

`getGroupType()` loads metadata `+0xac`. The video constructor populates it
through `InstaHelper::GetSourceFileType(asset, true, source_fps)`; source-group
initialization can copy it from `SourceGroup+0x58`. It is not a field already
available in Kjerag's parsed trailer. The denoiser's captured backend **mode 8
is a different enum** and does not establish source group type 8. Similarly,
the finished export's dimensions/fps do not by themselves authenticate the
metadata inputs to this selector. Those source-classification inputs must be
closed before automatic X4 Air table selection is implemented. Matching the
captured ISO100 values to a table row is not sufficient evidence.

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

Automatic table selection/consumption, player lookahead and short-input
handling remain unfinished. No installed change or visible-quality verdict
follows from these recovered inputs.
