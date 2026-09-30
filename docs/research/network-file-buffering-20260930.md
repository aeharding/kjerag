# September 30 network input follow-up

Issue [#186](https://github.com/aeharding/kjerag/issues/186). The owner's September
20 X4 Air report is at 1281.413 seconds, yaw -136.14, pitch -19.43, FOV 166.23,
horizon lock on. Local playback improved in the installed `dd908324` package;
NAS playback still hitches. No new stitching arithmetic is proposed here.

## Causal controls

All controls use that actual installed package, a headless 2256x1504 window,
60 Hz pan and normal silent-output routing unless sound access is deliberately
removed in the private sandbox. No desktop sound/session configuration changes.

| Input/control | Later source advances/s | Reported worst lateness | Audio underruns |
| --- | --- | --- | --- |
| NAS video and audio | 25.6 to 28.2 | Grows to 2645.8 ms | 0 |
| NAS video, private sound access absent | Approximately 30 | Stays at 43.9 ms | Not an audio qualification |
| NAS video and audio restored | 18.2 to 28.2 | Grows to 10053.6 ms | 474 early gaps |
| NAS video, only audio input redirected locally | Approximately 30 | Stays at 131.3 ms | 0 |

The last control preserves the normal audio decoder, producer, output callback
and playback clock. A diagnostic interposer records exactly two matching input
opens and redirects only the second. The local counterpart has matching size
and authenticated first/last MiB, not a full-file SHA qualification. These
controls locate an input contribution; they do not prove a GVFS internal cause,
eliminate network variability or establish 240 fps capacity.

## Implementation

Keep separate demux timelines and stream selection. One shared file handle
backs both AVIO cursors. Read aligned 1 MiB blocks through a serialized,
16 MiB LRU byte cache. Each cursor retains its own position, error and AVIO
buffer. A seek does not reposition the other cursor. The compressed queues,
decoder surface limits, audio ring, timestamps, source cadence and seam/color
history remain unchanged. No full-file local copy, persistent disk cache,
network-only user setting, or proxy requirement.

Preserving independent demuxers is important: issue #97 measured a 67 MiB
interleave gap where one shared audio/video demux timeline delivered sound too
late. Sharing bytes does not restore that coupling. FFmpeg's normal input is
transferred after inspection at its exact AVIO byte position. Its buffered
packets and stream metadata are retained. Format ownership closes before
custom AVIO buffer/cursor ownership. Offline Walk stays on normal FFmpeg input.

## Qualification so far

The first native NAS pan at the reported cue holds approximately 30 sources/s
after startup, with zero audio underruns and no growing delay. Startup still
hitches: worst 701.5 ms. A different section, 1381.413 seconds, also holds
approximately 30 sources/s after startup; worst 1231.0 ms, briefly processing
36 sources/s while catching up. No new kernel entries or postflight memory
pressure in either run. Actual rendered output was inspected. These native
builds include the preserved parked color edits; they are not clean-package
qualification or owner acceptance.

The device-hidden CPU workspace passes 1,644 tests, with 53 ignored. Clippy,
formatting, vendored warnings, naming and lock/source-list checks pass. Six new
regressions cover byte boundaries/eviction/memory bound, independent seeks and
EOF, retained file ownership after pathname replacement, packet bytes and
metadata versus normal FFmpeg input, raw callback errors, and actual shared
audio/video packet workers seeking independently. The last uses a synthetic
MOV/AAC fixture and no hardware or sound device.

Clean committed-source SDK build, both camera runtime suites, actual NAS
playback and installation remain due. Startup hitching and 240 fps capacity
remain defects, not accepted tradeoffs. Owner retesting remains the merge gate.

Private receipts are under
`scratch/playback-independent-20260927/runtime/installed-nas-*-control-01`,
`installed-nas-source-completion-repro-01`, `native-nas-shared-file-01` and
`native-nas-shared-file-later-01`, with separate health captures for each run.
