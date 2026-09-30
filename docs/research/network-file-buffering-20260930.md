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
late. Sharing bytes does not restore that coupling. Custom AVIO is installed
before opening and inspecting the container, never substituted after inspection.
Format ownership closes before custom AVIO buffer/cursor ownership. Reader and
Walk share this capture input; Walk retains synchronous packet delivery.

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

## First clean candidate rejected during UI qualification

The clean `2e4466a8` package holds source cadence in the reported NAS region
after startup, with zero audio underruns and no growing delay. However, its X4
UI suite crashes in libavformat on a later pasted reopen. It was not installed.
The first constructor substituted AVIO after inspection. FFmpeg's MOV demuxer
retains the previous pointer in each stream (`mov.c`, `mov_read_trak`), making
that substitution unsafe even though the original small MPEG-4 fixtures pass.
The constructor is removed, rather than relying on allocation address reuse.
All capture inspection now begins on its final custom IO. Requalification and
an allocation-churn reopen regression remain required.
Source: [FFmpeg 7.1 MOV demuxer](https://github.com/FFmpeg/FFmpeg/blob/n7.1.3/libavformat/mov.c).

Corrected ownership passes all 1,645 workspace tests, 53 ignored, and Clippy,
formatting, vendor warnings, naming and source-list gates. A new regression
repeats open/seek/close 32 times while other AVIO allocations stay alive. The
native reported-view NAS run still reaches approximately 30 sources/s without
audio gaps or new kernel entries after startup. Startup worst lateness is
1844.2 ms; the catch-up interval reaches 40.8 sources/s, so its whole-run cadence
parser fails. This is not a hitch-free pass. Clean SDK qualification remains due.

## Corrected package qualification and installation

Exact source `591cf695` passes all eight CI jobs, the clean SDK build and both
app-path camera suites: 43 X4 and 44 ONE X2 checks, zero failures. The real
pasted-reopen path that crashed the first candidate now passes. Existing
isolated-service/fixture skips remain. Both camera motion captures were inspected.
The real exported bundle passes a private import and binary/metadata/license
audit; this is an unsigned local test, not release-signature qualification.

Normal-audio 2256x1504 60 Hz NAS pan at the reported cue settles at 30 sources/s,
zero audio underruns and no growing delay after startup. Startup worst lateness
is 1238.9 ms. Another region, 1481.413 seconds, maintains approximately 30
sources/s with worst 36.9 ms lateness and zero underruns. The qualified package
is installed; origin, permissions and shared runtimes are unchanged, with the
authenticated `dd908324` package retained for rollback.

The actual installed NAS run still has startup spikes, reaching 2284.5 ms picture
lateness before settling at 30 sources/s with zero audio gaps. Its whole-run
cadence parser fails. Several early draw-completion receipts span 100 to 235 ms;
these alone do not distinguish execution, submission, callback or scheduling
delays. The installed ONE X2 riser run reaches 30 sources/s, worst 39.8 ms and
zero underruns. No new kernel entries or postflight pressure in these corrected
runs. Startup is unresolved, not an accepted delay. Owner retest and 240 fps
capacity remain open; no merge or release.

Package identities and qualification limits are in
[MERGE_READINESS](../MERGE_READINESS.md). Initial native receipts from the unsafe
constructor are historical diagnostics, not qualification of the corrected code.

## Owner rejection and request-sized shared reads

The owner reports that seeking takes too long and playback still hitches.
Short steady-state cohorts above do not override that report. The installed
`591cf695` player is not an accepted network fix.

An installed-player follow-up observes three real pasted-view seeks, each
acknowledged by the application's `goto` handler. Request-to-destination-draw
times are approximately 4.1 seconds at 1681.413, 3.5 seconds at 600.413, and
0.6 seconds back at 1281.413. These are application draw receipts, not scanout
times. The process exits normally, but the outer compositor times out with a
leftover clipboard provider; this is diagnostic evidence, not a passed runtime
qualification. Two earlier attempts have invalid seek injection and are not
seek measurements. All three postflight checks have no new kernel entries.

A diagnostic `pread64` wrapper records unchanged actual file reads. In the
longer first run it observes approximately 2.2 GiB read for 1.1 GiB of unique
1 MiB blocks. In the valid seek run the audio reader fetches approximately
599 MiB and video approximately 734 MiB. Typical full-block reads take tens of
milliseconds, with individual reads over 300 ms. Some stderr records interleave
with production probes; reducers exclude malformed records. The wrapper and
UI probes add overhead, so these are causal diagnostics, not uninstrumented
performance qualifications or sole-cause claims.

The cache fetch size was wrong for sparse audio: its 32 KiB AVIO request forced
a complete 1 MiB read, bringing in video bytes that may be evicted before the
video cursor reaches them. Raising the memory limit does not remove this read
amplification. The follow-up retains the same 16 MiB byte limit but divides it
into 256 pages of 64 KiB. A small request fetches one page; large requests batch
adjacent missing pages up to 1 MiB, stopping before a cached interval. Both
cursors still share the same file, and no timeline, packet budget, source cadence,
decode/GPU retention or stitching/color arithmetic changes.

The sparse-read CPU regression fails before the change, retaining 16 MiB after
only seventeen 32 KiB requests spread across the file, rather than at most
1.1 MiB. It also proves that both cursors' later use of those intervals comes
from the retained bytes. Additional cases exercise batched reads across cached
overlaps, the read-size cap and a short backend tail. Existing independent
real-demux/packet-worker tests continue to compare bytes and metadata through
seeks. Native/SDK runtime qualification and owner retest remain due.

The device-hidden workspace passes 1,648 tests, 53 ignored, plus full formatting,
workspace Clippy, vendored warning, naming, source-list and whitespace gates.
Unavailable GPU/media returns are included, not hardware qualification. This
does not yet establish real NAS seeking or smoothness.

Private receipts: `scratch/playback-independent-20260927/runtime/installed-nas-seek-hitches-01`
through `-03`, with separate health directories. The first failing CPU case was
run before changing production cache code.

## Longer controls and the owner's processing-bypass test

Clean SDK source `8123893c` completes the three valid pasted seeks in about
1.78, 2.03 and 0.95 seconds. The first two are shorter than the installed
diagnostic above, but the runs are not paired cold-cache conditions. Audio-side
backend bytes fall substantially with smaller requests. A subsequent long
2256x1504, 60 Hz NAS pan still falls behind: later source rates are 17.8 to
28.4/s, with worst lateness growing to 4.45 seconds. The identical SDK and view
against the authenticated local counterpart maintain approximately 30 sources/s
through the same long pan, with worst lateness staying at 38.8 ms. These runs
have no new kernel entries or postflight memory pressure. `8123893c` is not
installed or a completed network fix; the short seek/idle success is insufficient.

At the owner's request, a temporary native diagnostic removes the resident
profile and therefore the seam solver, photometric matching and temporal filter.
It also bypasses generic band measurement and optical flow, and replaces the
projection shader with direct luma/chroma sampling of lens 0. Both actual camera
streams still read and hardware-decode, normal silent-output audio remains active,
and sequential presentation prevents dropping source pictures to conceal slow
input. Actual output inspection confirms a raw fisheye image, not a corrected
panorama. The same reported NAS cue and long pan run at about 25 to 27 fps after
startup, with repeated empty decoded-delivery waits and worst lateness growing
to 6.53 seconds. Audio has zero underruns. The player exits normally and the
postflight has no new kernel entries or memory pressure.

This establishes that the network defect can occur without stitching/color/
temporal processing. It does not establish the precise backend or decode cause,
exclude additional processing bottlenecks, compare matched native/SDK speed,
or qualify 240 fps. The diagnostic retains the two parked owner color edits,
but constructs no color-processing owner. The resident-only capacity parser
cannot qualify its deliberately generic draw route and fails; this is a causal
control, not a passed playback qualification. Its private source patch and
binary are retained; the bypass is removed from the worktree and the ordinary
native renderer rebuilt. The installed Flatpak remains unchanged.

Private receipts: `runtime/sdk-nas-sparse-read-seeks-01`,
`runtime/sdk-nas-sparse-read-pan-01`, `runtime/sdk-local-sparse-read-pan-01`,
and `runtime/native-nas-unprocessed-pan-01` under
`scratch/playback-independent-20260927/`, with separate health directories.
`unprocessed-control.patch` records the temporary source change; its binary SHA
is `5d3d17b7b53f5a222071be9a2c193c8b83a216305aa5d4a86f3ff7f6bbb26645`.
