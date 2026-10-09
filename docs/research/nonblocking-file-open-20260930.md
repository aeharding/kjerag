# Nonblocking file opening, September 30

## Report and reproduction

The owner reports that the single-reader installed build plays "better", but
opening it from a file takes too long. He confirms the delay is **before the
window appears**, not only before the first picture. This is separate from
the unresolved presentation tails and 240 fps rendering-capacity requirement.

An unchanged installed `78075a42` run opens the reported September 20 NAS file
normally, with no copied-view seek or processing override. Its first native
surface render attempt is 4.811 seconds after `/app/bin/kjerag` exec. The UI
thread spends 2.756 seconds in one 35,927,360-byte motion-track read, followed
by a roughly 2.6 MB ISO read taking 0.173 seconds. Decode/read-ahead is already
reading the same container while this metadata work blocks `App::init`.

The large synchronous read is a confirmed pre-window contribution. Concurrent
reads alone do not prove postponing video read-ahead will shorten total opening.
This is a normal buffered network run, not authenticated cold-cache latency or
the owner's exact click-to-window time. Trace overhead is not a throughput
measurement. Read syscall arguments remain raw, with no metadata/video payload
dumps. Private evidence: `scratch/playback-independent-20260927/` and its
`runtime/installed-startup-nas-trace-01` receipt.

## Ownership change

`Scene::prepare_with` retains an inspected Reader and the same complete
calibration, integrated orientation, camera profile and temporal settings in
`PreparedScene`. It starts no decode/read-ahead or sound device. The shell's
single preparation worker owns this synchronous file work, with one replaceable
queued request. `App::init` returns without waiting for it. The existing stock
welcome view says "Opening video...".

Only the UI consumes the current result, attaches sound/Player, constructs
Scene and applies preferences. The inspected files are not reopened. CLI and
pasted views belong to the same request and apply only on successful opening.
Failure leaves the prior video, view and settings intact. Close or a new choice
revokes publication; stale futures contain IDs, not prepared Readers. A canceled
in-flight read can finish in the worker without reopening the closed picture.
There is no UI join, worker-per-click pool, metadata cache, reduced motion track,
stitching/color arithmetic change, or skipped source input.

The filesystem call itself is not forcibly interrupted. A newer queued choice
waits for the sole active preparation to return. This bounds resource use rather
than accumulating blocked network opens. The old code blocked the entire UI
through that wait, preventing a new choice or Close in the first place.

## Verification checkpoint

CPU regressions exercise initialization, real shell messages and real foreign
format refusal, failed pasted-view preservation, stale results, latest-queued
coalescing, nonjoining shutdown, worker exit and prompt stale-owner release.
They do not open a GPU or sound device.

The UI harness adds `blocked-open`, a real CLI open on a synthetic FIFO whose
format-sniff `File::open` cannot return until a writer arrives. It proves the
actual window paints and responds to Close before IO completes, then releases
the reader and checks that neither an alert nor a pasted view appears. It uses
no test hook or decoder fixture. The installed baseline fails pre-window paint;
its first negative-run cleanup raced window creation after release and aborted
the private compositor. There are no new kernel entries or scoped limit/OOM
events. Negative-path cleanup now waits for the released window before quitting.

Native `startup-blocked-native-02` passes all five real-window checks and quits
normally, with no new kernel or scoped limit/OOM events. An earlier native
receipt had an invalid Close capture assertion and is not that check's evidence.
A CPU harness regression now refuses missing, unchanged, chrome-only or failed
Close captures instead of accepting comparison errors as a change.

Native NAS opening draws its first surface 0.758 seconds after exec, before
the bulk motion read starts. That same read takes 1.378 seconds on `kjerag-open`,
not the UI thread. It later maintains recorded source cadence with zero audio
underruns, 54.9 ms worst reported picture lateness, normal exit and no new kernel
entries. Both real pictures are inspected. Postflight memory pressure is nonzero
(avg10 0.23), so this is not a pressure-free result. Private receipt:
`runtime/native-startup-nas-trace-01`; native executable SHA256:
`8e3afdfdc8805d42ed80c558df510327cbd6f38dbe9b95efa886a70a5de60bb7`.

Native and installed traces use different runtimes. These timings establish
the UI/IO ordering change, not a matched package-speed comparison, instant
first-picture preparation, hitch-free playback, Studio parity or capacity.
Clean Flatpak qualification, both camera suites and owner retest remain pending.
The installed package is still `78075a42` at this checkpoint. The two parked
owner color edits are unchanged and remain outside the committed candidate.

## Clean package qualification and installed delivery

Implementation source `5ecc99466442cc5c61e577709f60c7ffc2e9fd80` passes all eight
CI jobs, including x86_64 and ARM workspace gates. The bounded device-hidden
local workspace passes 1,656 tests, 53 ignored, plus Clippy/format/source/name
and CPU harness checks. That local tree includes the two preserved parked color
edits and unavailable-device returns; it is not clean-source GPU coverage.

The offline clean SDK archive excludes those parked edits. Separate app-path
UI suites pass 48 X4 and 49 ONE X2 checks, zero failures, including all five
blocked-open checks through the packaged shell. Existing isolated sound/portal,
explicit CLI-view, sandbox import-fault and cross-mount fixture skips remain.
Each launched candidate executable is authenticated. Both motion captures per
camera are inspected. The shader/Rust-twin helper remains native, not a clean
SDK shader test. No new kernel entries or scoped memory-limit/OOM events occur;
postflight pressure averages are zero. The actual exported bundle passes a
separate device-hidden private import and executable/metadata/license audit.
It is an unsigned local test bundle, not release-signature qualification.

In packaged NAS trace `runtime/sdk-startup-nas-trace-02`, first surface
preparation occurs 0.776549 seconds after app exec, before the bulk motion read
starts. That 35,927,360-byte read takes 1.433059 seconds on the opening worker,
not the UI thread. Later playback holds recorded cadence with zero audio
underruns and 46.8 ms worst reported picture lateness. Both pictures are
inspected. It exits normally, with no new kernel entries and zero postflight
pressure averages. This uses the same Flatpak runtime as the preceding installed
trace, but remains buffered trace evidence, not authenticated cold-cache or
click-to-window latency, first-picture readiness or a general throughput result.
The first packaged attempt stopped before app launch because its preflight
expected a superseded installed identity; its failed receipt is retained.

The qualified package is now installed from that exact source:

- OSTree: `4a39f4ddae092d45290cdad09b6ef0a1b69a39897e69546c5c46dc165787c7c1`.
- Executable SHA256: `db649245cd11824bdcdc5ac1d6b8a6339980bdb6fc4c504eb905ad4901eefc79`.
- Bundle SHA256: `a6f4d7e837b4871c30e753411b9c6214cf20d5865346648fc7f5932d9973de1e`.

Installation disables dependency/related-ref/pull updates. Origin, permissions
and all recorded shared runtime identities are unchanged. The authenticated
preceding `78075a42` bundle is retained for rollback. The final actual-installed
`runtime/installed-startup-reported-view-01` run uses no app-path substitution.
Its launched executable and OSTree identity match above, and its `goto:` receipt
restores the exact reported NAS view: time 1281.413, yaw -136.14, pitch -19.43,
fov 166.23, lock 1. Subsequent intervals report 30 source advances/s, zero audio
underruns and no growing delay; worst reported picture lateness is 131.0 ms.
It exits normally, with no new kernel entries or postflight pressure averages.
Both moving pictures are inspected. That playback does not establish hitch-free
presentation or the 240 fps capacity target.

Private package: `scratch/flatpak-delivery-5ecc9946/`; camera UI receipts:
`scratch/playback-independent-20260927/packaging/qualification-ui-5ecc9946/`.
The owner's close-and-reopen startup retest remains the merge gate for draft
[PR #242](https://github.com/aeharding/kjerag/pull/242), stacked on PR #241.
No merge or release is claimed.

## Owner-requested blank player follow-up

The owner rejects the "Opening video..." welcome screen and explicitly permits
a blank player while the file loads. The existing real FIFO capture reproduces
that unwanted screen. The shell now renders the normal header and a plain black
video area while its first open request is pending, with no logo, status text
or open button. When replacing a video, the old picture remains until successful
replacement. No-file launch and Close still show the ordinary welcome screen.
Preparation, calibration, source admission and first-picture readiness are
unchanged. This follow-up makes no new window-latency claim.

The real blocked-open regression now requires valid black picture pixels rather
than the app icon, then verifies responsive Close and canceled-result behavior.
Its CPU check rejects missing/truncated captures and nonblank loading content;
all 22 harness tests and five startup parser tests pass. The earlier captured
opening screen fails that new blank-picture assertion as expected. The bounded
device-hidden workspace again passes 1,656 tests, 53 ignored, plus full Clippy,
formatting, vendor-warning, source-list and naming checks. Unavailable-device
returns are not GPU coverage; the two parked color edits remain excluded from
the committed change.

All eight CI jobs pass on exact implementation source
`5aaf97a7e921ff02cf2dd51b5bddda8b9c260070`. The clean offline SDK archive excludes
the parked edits. Separate app-path suites pass 48 X4 and 49 ONE X2 checks, zero
failures, with the existing fixture/service skips described above. The launched
executables are authenticated; both motion captures per camera and the blank
startup capture are inspected. The shader/Rust-twin helper remains native, not
a clean SDK shader test. The exported bundle passes the separate device-hidden
private import and executable/metadata/license audit. This is an unsigned test
bundle, not release-signature qualification.

Native `runtime/blank-player-native-01` and actual-installed
`runtime/blank-player-installed-01` each pass all five real CLI/FIFO checks,
including blank pixels, responsive Close and canceled-result behavior. The
installed blank capture is inspected. Both camera suites and these startup
checks exit normally, with no new kernel entries or scoped memory-limit/OOM
events, and zero postflight pressure averages.

That qualified `5aaf97a7` package was installed and is now retained. Origin, permissions and
recorded shared runtime identities are unchanged; the authenticated preceding
`5ecc9946` bundle is retained for rollback. Exact identities are recorded in
[MERGE_READINESS.md](../MERGE_READINESS.md#preceding-blank-player-private-test-package-september-30).
Package: `scratch/flatpak-delivery-5aaf97a7/`; camera receipts:
`scratch/playback-independent-20260927/packaging/qualification-ui-5aaf97a7/`.
The owner's close-and-reopen startup retest remains the merge gate. No merge,
release, first-picture timing, general hitch-free or capacity pass is claimed.

## Transparent pane clarification, October 1

The owner reports black, then the transparent/blurred pane, then video, and
clarifies that the desired initial background is the transparent/blurred pane,
not black held longer. The actual-installed `5aaf97a7` blocked-opening capture
shows the black first stage; its packaged `open-backdrop.ppm` shows the existing
normal pane before the first picture. This is the reported styling mismatch,
not a new source/stitching latency diagnosis.

The pending-open container now uses the same existing `backdrop(fullscreen)`
as the Scene container. Window loading stays on the normal COSMIC pane, allowing
the compositor's ordinary blur; fullscreen's established black surround remains
unchanged. No first-picture shortcut, new state flag, timing rule or engine
arithmetic is added. The real CLI/FIFO check rejects black/invalid/textured
opening captures and compares the opening pane with the closed player's pane.
The earlier actual-installed black capture fails the new assertion as expected.
All 23 harness CPU tests and five parser startup tests pass; bounded device-hidden
workspace/lint/format/vendor/source/name gates pass. Native real-window receipt
`runtime/transparent-player-native-01` passes all six checks, including matching
opening/closed panes. The actual screenshot is inspected. Normal exit, no new
kernel entries or scoped memory-limit/OOM events, zero postflight pressure
averages.

All eight CI jobs pass on exact implementation
`6dbbf16bd4b3f971d0f6b8361b6e194f87752c2c`. The clean offline SDK archive excludes
the parked color edits. Separate authenticated app-path suites pass 49 X4 and
50 ONE X2 checks, zero failures, with the existing fixture/service skips and
native rather than clean-SDK shader-twin helper. Both motion captures per camera
and opening pixels are inspected. Both suites exit normally, with no new kernel
entries or scoped memory-limit/OOM events. X4 postflight pressure avg10 is zero;
avg60/avg300 are 0.05/0.03. X2 postflight averages are zero. A private device-hidden
import authenticates the exported bundle's commit, executable, metadata and
license. This is an unsigned local test package, not signature qualification.

The package is installed with unchanged origin, permissions and recorded shared
runtimes, using no dependency/related-ref/pull updates. The authenticated previous
`5aaf97a7` bundle is retained for rollback. Actual-installed
`runtime/transparent-player-installed-01` passes all six CLI/FIFO checks; its
opening capture is inspected. Normal exit, no new kernel entries or scoped
memory-limit/OOM events, zero postflight pressure averages. Headless cage checks
the normal pane and does not measure desktop blur. Exact package identities are
in [MERGE_READINESS.md](../MERGE_READINESS.md#current-transparent-pane-private-test-package-october-1).
Package: `scratch/flatpak-delivery-6dbbf16b/`; camera receipts:
`packaging/qualification-ui-6dbbf16b/` in the same workflow. Owner startup retest
remains the merge gate. The subsequently reported persistent pause/resume/seek
stuttering is separate and unlocalized. No latency, hitch-free or capacity
verdict, merge or release is claimed.
