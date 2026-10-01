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
