# Studio 6.0.2 denoiser ISO input

Read on 2026-09-10 to remove the captured-ISO100 input restriction from the
temporal-filter work. This is a metadata dependency, not a new colour smoother
or a claim that the reported flicker is fixed. The earlier 607-second native
filter capture remains the selected runtime authority; no Studio capture
covers the owner's 612.078-second report.

## Native producer

All addresses below are unslid virtual addresses in the saved ARM64 worker,
SHA256 `0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452`.
The existing hash-checking `mac-color-update-01/read_worker.py` reads it.

`ProjectExporter::ProcessDenoiseISOPairs` calls
`InstaMetaData::GetIsoValue` at `0x170078 -> 0xb13524`. The metadata object
caches the result of `InstaHelper::GetIso` (`0xd52a68`), which, for video,
copies the asset's ThreeA data and calls `Iso::Iso` at `0xd57b1c`.
The asset's `IsVideo`, `CopyThreeAData` and `GetFirstFrameTimeOffsetMs`
virtual slots are `+0x130`, `+0x2c8` and `+0x3b8` respectively.

These are authenticated vtable targets, not names inferred from offsets.
`iso-provenance-01/read_data.py` checks the actual Mach-O chained-fixup
format, page-chain membership and rebase bits before resolving each target.
Its bounded format-6 reader follows the layout documented in Apple's
[fixup-chains.h](https://raw.githubusercontent.com/apple-oss-distributions/dyld/main/include/mach-o/fixup-chains.h).

The video asset forwards the data request to
`ExtraInfoAsset::CopyThreeAData` (`0x16e22c8`). The ordinary file route reads
**binary trailer record 9**, then returns its bytes without a conversion.
The playlist route (`ReadRawThreeAData`, `0x17a10e8`) also requests record 9;
its `ReadFrameRawData<RawThreeAItem>` appends the first buffer unchanged and
removes only duplicate timestamp-prefix items from later buffers. It does not
rewrite the retained items' timestamps. This establishes the record-9 link
that the preceding integration audit explicitly left unresolved.

## Record and constructor law

Each item occupies 48 bytes. Only two words enter this ISO calculation:

| Item offset | Interpretation |
| --- | --- |
| `0x00` | Little-endian unsigned 32-bit timestamp |
| `0x10` | Little-endian packed word; ISO is `((word >> 19) * 100) >> 6` |

The shifted value is at most 8191, so the SIMD loop's narrowing before
multiplication loses no bits. Multiplication and division are integer
operations; an ISO value is not rounded to the nearest whole code.

`Iso::Iso` (`0xd57b1c..0xd57ec4`) has two cases:

- With 1 through 40 items, its summary is the first item's ISO and the
  timestamped vector is empty.
- With more than 40 items, **both** the summary population and the emitted
  vector omit items 0 through 39. The summary is the integer mean of the
  remaining ISO values. Item 40 begins the timestamped vector.

The native constructor assumes an item exists even for a malformed short
buffer. Kjerag must not reproduce an out-of-bounds read. Missing data and a
payload with no complete item provide no observations, not invented ISO100.
The native caller divides byte length by 48; trailing partial items are not
included in its count.

The time law is separate from Kjerag's shutter-track clock. Let `T` be the
signed 64-bit truncation of `GetFirstFrameTimeOffsetMs()` and `t[i]` the
zero-extended item timestamp:

```text
delta = sign_extend_i32(wrapping_u32(t[40] - low_u32(T)))
origin = wrapping_i64(T + delta)
offset_ms[i] = wrapping_i64(i64(t[i]) - origin)
```

The native pair stores the final signed integer as a double. For the ordinary
non-wrapping range, this rebases item 40 to time zero. The exact width/sign
operations matter at wrapping boundaries and are not replaced with saturating
subtraction. `GetFirstFrameTimeOffsetMs` forwards to
`TimestampOffsetBetweenSystemAndMedia` (`0x16e1928`): it obtains metadata
`first_frame_timestamp`, converts to double and divides by 1000 only when
`IsRawGyro` is true. Its time unit is milliseconds on both camera paths.

The exporter retains pairs inside the requested inclusive source interval and
subtracts the interval start from their timestamps (`0x170100..0x170118`).
Its separate `SCREEN_RECORD` ISO100 branch is not an `.insv` default. The
summary ISO is not substituted for this vector by the inspected exporter.

## Downstream lookup, not metadata parsing

The filter brackets its current `FramePosition` time in the supplied pairs.
For nonzero valid bracket values, it linearly interpolates using a fused
multiply-add (`0x798f90..0x798fb4`), or copies an endpoint. Process truncates
the result to a signed integer before passing it to the backend. The bracket
comparison uses the separately read `fcp::greate` tolerance, not a newly
chosen sampling tolerance.

Two additional native behaviours must not be replaced with a universal clamp:

- A zero in the selected bracket invokes `findNextNonZeroISO` (`0x799ce8`).
  It searches forward and retains the last successful result; the constructor
  initializes that cache to -1. The zero test uses native `1e-6` tolerance.
- A resulting value below 100 invokes `correctInvalidISO` (`0x7998ac`).
  If the first configured pair is below 100, it returns 100 immediately.
  Otherwise it searches for the closest pair whose ISO is at least 100,
  preferring an earlier-time pair on a distance tie; if none exists it returns
  100. An empty/invalid bracket has an error path before this correction.

Those are filter-input policies, not evidence that the camera's physical ISO
is always at least 100. Adding the raw metadata observations does not select
these policies or the temporal filter in the player.

The render-side `temporal_fusion::iso::Lookup` now implements this lookup over
one owned snapshot of the raw observations. Binary search replaces the native
forward cursor for strictly increasing input times and nondecreasing queries;
the native `1e-6` comparison boundary and arithmetic are retained. This is a
Kjerag execution choice, not a changed colour-update law. Empty or unordered
input, a nonfinite query and a backward query without `reset` are refused.
Reset clears both query chronology and the forward-zero cache. These explicit
validation/reset boundaries avoid carrying lookup state across a seek; the
player must still invoke them at the correct source/epoch boundary.

The real-file consumer regression looks up the reported times, not just nearby
metadata rows: 607574 and 612078 ms return 100 for X4 Air, and 212512 ms returns
365 for ONE X2. Synthetic tests cover interpolation/truncation, exact tolerance
boundaries, endpoint values, zero search/cache, invalid correction/ties and
reset. All are included in the passing 94-check temporal suite on the AMD GPU.
Ordinary playback still does not select this lookup or the temporal filter.

## Checks on the owner's files

The independent scratch reader locates record 9 through the same index/walk
format but does not use Kjerag's new parser. It reads no image, serial or GPS
payload and retains only relative times, ISO values and aggregates.

| Input | Record-9 items | Native suffix summary | Relevant suffix observations |
| --- | ---: | ---: | --- |
| April X4 Air `...00_004.insv` | 54,025 | 100 | All 195 pairs from 607000 through 613500 ms are ISO100 |
| October ONE X2 `...00_002.insv` | 2,088 | 178 | At 212478 through 212880 ms, the pairs are ISO365 |

April's nearest pairs to the two reported times are `(607577 ms, 100)` and
`(612082 ms, 100)`. The former agrees with the captured native filter ISO100.
The latter establishes only that automatic input for the 612 example, not
Studio output coverage or a flicker pass. The October result demonstrates
why applying the captured ISO100 universally would be wrong.

This is a **single global track for denoising**, not either lens's gain. It
does not reverse the earlier finding that shutter ratios cannot recover
per-lens photometric corrections.

The production parser's 143 ordinary metadata tests and doctest pass. Its
separate ignored `owner_captures_match_the_record_nine_audit` regression also
passes through `CalibrationSet::from_capture` on both named files, checking
suffix lengths, summaries and the observations above. Synthetic tests cover
indexed and walking trailer lookup, absent/partial records, the 40/41-item
boundary, packed integer decoding and non-cancelling timestamp-wrap cases.
Workspace all-target Clippy passes with the new public calibration field and
the updated construction sites. These are metadata checks, not reruns of the
GPU, moving-video or packaged-player gates.

## Evidence and remaining work

Durable, ignored evidence is under
`scratch/studio-seam-flicker-612-20260909-01/iso-provenance-01` and
`iso-file-audit-01`. The former includes reproducible bounded disassemblies,
chain-verified vtable targets and hashes. The latter contains the independent
constructor audit, its file reader and `analysis-windows.json`. No native
executable or private raw trailer payload is committed.

The separately [recovered parameter tables](studio-denoise-config-602.md)
still need authenticated automatic selection and consumption. Source-window/player integration,
playback performance and the owner's moving-video acceptance remain separate
requirements. This metadata result does not complete any of them.
