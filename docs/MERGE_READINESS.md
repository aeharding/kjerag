# GPU stitching delivery: merge and release review

Published-release checkpoint: 2026-09-12. PR #183 is merged and 0.3.0 is published.
Packaging-only PR #188 is also merged; 0.3.1 is published.
Release qualification remains open because high-rate X4 playback falls behind
in both the release and a restored accepted-build control. This page
records the cumulative delivery without requiring reviewers to reconstruct
the chronological experiment log in ROADMAP.

The active owner goal includes cleanup, merge and release. The living roadmap
is now separated from its verbatim [historical record](ROADMAP-HISTORY-20260912.md),
with a [research navigation index](research/README.md). The shared playback
slowdown, native/Flatpak differences and frame-time spikes are tracked in
[issue #186](https://github.com/aeharding/kjerag/issues/186).

## Accepted native preview, September 30

Draft PR [#240](https://github.com/aeharding/kjerag/pull/240) adds source-specific
GPU completion proofs, bounded worker-owned source admission and output
completion, and a finite curved native-mesh renderer. The source/seam/color
cadence is unchanged. The renderer keeps full-resolution source planes but
approximates subpixel projection and sampling; the original ray renderer remains
the diagnostic reference and ball fallback.

The frozen preview uses committed source
`dd908324f0e23039a6037d3f4ecb74d6b110264b` plus the two preserved parked
periodic-color edits. Its executable SHA256 is
`668ed968def53d23319fe4f023062e0e2c1f4b6d0f008261524c80f6eae4456c`.
After running it at the reported September 20 view, the owner reports
"Performance looks much improved" and "Good enough" in response to the requested
moving-picture/seam and audio-sync check. This accepts that preview for the
next delivery step. It does not qualify a clean committed-source package,
arbitrary footage, network playback or the 240 fps capacity target.

All eight CI jobs pass on `dd908324`. Native X4 UI passes 55 checks, zero failures.
The bounded device-hidden workspace passes 1,638 tests with 53 ignored; unavailable
hardware returns are not GPU coverage. The real seven-source no-shell regression
and both-camera moving coverage checks pass separately. Local 2256x1504 playback
maintains 30 source advances/s, including 60 Hz pan. Uncapped pan reaches 155.06
completed redraws/s with full source cadence, still below 240; callback
p99/max 27.33/32.78 ms does not establish the 4.17 ms budget. Network pan still fails
badly despite a realtime decoder-only control. No general playback fix is claimed.

The clean committed-source SDK build passes separate app-path UI suites,
43 X4 and 44 ONE X2 checks with zero failures. Existing sound-device, portal,
exact-view, sandbox import-fault and cross-mount paired-fixture skips remain.
Both motion captures for each camera were inspected; the shader/Rust-twin
helper remains native rather than a clean SDK shader test. Each launched
package executable is authenticated. Both suites have no new kernel entries
or scoped memory-limit/OOM events, and postflight pressure averages are zero.
These are functional checks, not capacity, all-camera or freeze-cause proof.

The actual exported bundle passes a separate device-hidden private import:
its commit, executable, metadata and license match the archived source/build.
The local test bundle is unsigned, not a release-signature qualification.

## Current blank-player private test package, September 30

Source `5aaf97a7e921ff02cf2dd51b5bddda8b9c260070`, draft
[PR #242](https://github.com/aeharding/kjerag/pull/242), is installed. The owner
requested a blank player instead of the "Opening video..." welcome screen.
The normal header and black video area now paint while initial preparation is
pending, without a logo, loading text or open button. No-file launch and Close
retain the welcome view; replacement failure retains the old video. Complete
preparation and stitching/color/source behavior are unchanged.

- Installed OSTree: `729f579ca04705465c67b3c2f29ada6480e65cd146d32bc3c35a45426febf265`.
- Executable SHA256: `2feb7837af2cf3c1ef2785c6558dd6fb0946985c5046a2f608ef912bd84e27b9`.
- Bundle SHA256: `506acd934ca8471acfa3ee3ec108ed1ce79c4ba7b7d93fe836899f554e1fe2c2`.

All eight CI jobs pass on this exact implementation. Bounded device-hidden local
gates pass 1,656 workspace tests, 53 ignored, plus full lint/format/vendor/source/
name checks, 22 harness CPU tests and five startup parser tests. The local tree
includes the two preserved parked color edits and unavailable-device returns,
not clean-source GPU coverage. The offline clean SDK archive excludes those
edits. Separate app-path UI suites pass 48 X4 and 49 ONE X2 checks, zero failures,
with the existing isolated sound/portal, exact-view, import-fault and cross-mount
fixture skips. Launched executables are authenticated. Both motion captures per
camera and the startup capture are inspected. The shader/Rust-twin helper is
native, not a clean SDK shader test. The exported bundle passes a separate
device-hidden private import and executable/metadata/license audit. This local
test bundle is unsigned, not release-signature qualification.

The real CLI/FIFO check passes all five checks in native and actual-installed
modes, including blank pixels, responsive Close and canceled-result preservation.
Those runs and both camera suites exit normally, with no new kernel entries or
scoped memory-limit/OOM events, and zero postflight pressure averages. Installation
uses no dependency/related-ref/pull updates. Origin, permissions and recorded
shared runtimes remain unchanged; the authenticated preceding `5ecc9946` bundle
below is retained for rollback. The owner's reopen retest remains the merge gate.
No new first-picture/window timing, general hitch-free or capacity verdict is
claimed. No merge or release.

Package: `scratch/flatpak-delivery-5aaf97a7/`. UI receipts:
`scratch/playback-independent-20260927/packaging/qualification-ui-5aaf97a7/`.
Actual-installed startup: `runtime/blank-player-installed-01` in that workflow.
Details are in the [startup record](research/nonblocking-file-open-20260930.md).

## Preceding nonblocking-open private test package, September 30

Source `5ecc99466442cc5c61e577709f60c7ffc2e9fd80`, draft
[PR #242](https://github.com/aeharding/kjerag/pull/242), was installed and is now
retained for rollback. It is stacked on
PR #241. Complete capture inspection, calibration and orientation preparation
move off the UI thread. One worker and one replaceable queued request retain
the inspected Reader. Decode/audio start only after preparation. Close or a
new choice revokes stale publication without joining a blocked filesystem call.
CLI/pasted views follow their exact request; failed opens retain the old picture
and view. No stitching/color arithmetic, source cadence or horizon shortcut is
included. A blocked active filesystem call can still delay the next queued
file, but does not block the window.

- Installed OSTree: `4a39f4ddae092d45290cdad09b6ef0a1b69a39897e69546c5c46dc165787c7c1`.
- Executable SHA256: `db649245cd11824bdcdc5ac1d6b8a6339980bdb6fc4c504eb905ad4901eefc79`.
- Bundle SHA256: `a6f4d7e837b4871c30e753411b9c6214cf20d5865346648fc7f5932d9973de1e`.

All eight CI jobs pass on this implementation source. The device-hidden local
workspace passes 1,656 tests, 53 ignored, plus lint/format/source/name and CPU
harness checks. It includes unavailable-device returns and the two preserved
parked color files, not clean-source or GPU qualification. The clean SDK archive
excludes those files. Separate app-path suites pass 48 X4 and 49 ONE X2 checks,
zero failures, including five real CLI/FIFO blocked-open checks. Existing
isolated sound/portal, explicit-view, sandbox import-fault and cross-mount fixture
skips remain. Launched executables are authenticated; both motion captures per
camera are inspected. The shader/Rust-twin helper is native, not a clean SDK
shader test. Both suites have no new kernel entries or scoped memory-limit/OOM
events, and zero postflight pressure averages. The exported bundle passes a
device-hidden private import and executable/metadata/license audit. This local
test bundle is unsigned, not a release-signature qualification.

The packaged NAS trace attempts its first surface preparation 0.777 seconds
after app exec, before the 35,927,360-byte motion read. That read now takes place
on the opening worker. The preceding installed trace took 4.811 seconds to first
surface preparation while metadata blocked the UI thread. These are buffered
trace observations, not authenticated cold-cache or click-to-window latency,
and do not time the first video picture. Later packaged playback holds recorded
cadence, zero audio underruns and 46.8 ms worst reported picture lateness.
Both pictures are inspected; normal exit, no new kernel entries or postflight
pressure averages.

Installation uses no dependency, related-ref or pull updates. Origin,
permissions and recorded shared runtime identities remain unchanged. The
authenticated `78075a42` bundle below is retained for rollback. A final
actual-installed run, with no app-path substitution, restores the reported NAS
CLI view at 1281.413 seconds, yaw -136.14, pitch -19.43, fov 166.23, lock 1.
Subsequent intervals report 30 source advances/s, zero underruns and no growing
delay; worst reported picture lateness is 131.0 ms. The process exits normally
with no new kernel entries or postflight pressure averages. Both pictures are
inspected. These are functional opening/playback checks, not an all-file,
hitch-free, display-tail or 240 fps capacity pass. The owner's startup retest
remains the merge gate. No merge or release is claimed.

Package: `scratch/flatpak-delivery-5ecc9946/`. UI receipts:
`scratch/playback-independent-20260927/packaging/qualification-ui-5ecc9946/`.
Packaged/installed NAS receipts: `sdk-startup-nas-trace-02` and
`installed-startup-reported-view-01` under that workflow's `runtime/`, each with
a separate health directory. Full reproduction and scope are in the
[startup record](research/nonblocking-file-open-20260930.md).

## Preceding single-reader private test package, September 30

Source `78075a42d159aeb90c6864e299b5936fcefd4236`, draft
[PR #241](https://github.com/aeharding/kjerag/pull/241), was installed and is now
retained for rollback beneath the startup change above. One normal
FFmpeg input per container routes bounded compressed queues to video and audio;
the competing live file cursors, custom AVIO callbacks and byte cache are gone.
Video retention is 128 MiB/512 packets to pass the measured 67 MiB camera
interleave; audio retains its 256 KiB/128-packet bound. Either full queue
backpressures the reader, so audio independence is finite. Source processing,
stitching/color arithmetic, PCM and clock policy are unchanged.

- Installed OSTree: `854dc120fcb3a7a564ced1fa10b4ef77fbc523ad8a0e86d0431e4382d97d6ed6`.
- Executable SHA256: `96a5004ab350245c53c812ebe1616fedbb22008adb586dafe7e745916bc843d5`.
- Bundle SHA256: `8adc3c2671f85834cd6e36b39c0c4e64c007a7c215a9cf10ed89c319fe2dc57e`.

All eight CI jobs pass on this source. The local device-hidden workspace gate
passes 1,648 tests with 53 ignored, plus full lint/format/vendor/source/name
checks; it includes the two parked owner color files and unavailable-device
returns, not clean-source or hardware qualification. Those files are preserved
and excluded from the committed source and package. Separate clean SDK app-path
suites pass 43 X4 and 44 ONE X2 checks, zero failures, with existing isolated
sound/portal, explicit-view, import-fault and cross-mount fixture skips. Each
launched candidate executable is authenticated. Both motion captures per camera
are inspected. The shader/Rust-twin helper is native, not a clean SDK shader
test. Both suites have no new kernel entries or scoped memory-limit/OOM events,
and zero postflight pressure averages. The exported bundle passes a separate
device-hidden private import and executable/metadata/license audit. It is a
local unsigned test bundle, not a release-signature qualification.

The packaged reported NAS clip completes forward/backward seeks in approximately
1.36, 1.36 and 0.75 seconds, then holds source cadence through the previously
failed region, zero underruns and no growing delay; worst lateness remains
42.4 ms. There are no new kernel entries; postflight pressure avg60 is 0.02,
not entirely pressure-free. A separate local 1 Hz/60 Hz redraw-recovery check
retains approximately 30 source advances/s and zero audio underruns, returns to
a current displayed source after restoration, and exits normally with no new
kernel entries or postflight pressure averages. This verifies the selected X4
path, not indefinite audio progress with stalled video or generic-camera coverage.

Installation disables dependency/related-ref/pull updates. Origin, permissions
and all recorded shared runtime identities remain unchanged. Authenticated
`591cf695` and `dd908324` bundles are retained for rollback. Actual installed
playback, with no app-path substitution, holds 29.95 consecutive sources/s
during the reported 40-second, 2256x1504 requested-60-Hz NAS pan. It records zero
audio underruns and 41.2 ms worst lateness without growing delay, and exits
normally with no new kernel entries or postflight pressure averages. Pictures
are inspected. It completes 62.40 redraws/s within the pan window; completion
spacing p99/max is 29.76/30.78 ms. Steady sourced-picture dwell p99/max is
48.12/51.00 ms, so display timing remains uneven. The whole-run cadence reducer
is false, including startup commits without sourced draws; the separate pan
capacity reducer is true for its defined 60 Hz cohort, not 240 fps capacity.
Owner retest remains the merge gate. No merge, release, instant seeks, general
hitch-free fix or 240 fps capacity pass is claimed.
Package: `scratch/flatpak-delivery-78075a42/`; private runtime receipts:
`sdk-single-demux-nas-seeks-01` and `sdk-single-demux-low-redraw-01` below
`scratch/playback-independent-20260927/runtime/`; actual installed evidence is
`installed-single-demux-nas-pan-01`, with a separate health directory.
Full input/rejected-control
details are in the [network input record](research/network-file-buffering-20260930.md).

## Rejected byte-cache private test package, September 30

Source `591cf695c5a1142732d4b60fd8e9cbeec1d2b0c8`, draft
[PR #241](https://github.com/aeharding/kjerag/pull/241), was installed. Audio and
video keep independent demux timelines over one bounded shared file-byte cache.
Custom IO exists before capture inspection. The first `2e4466a8` candidate's
post-inspection AVIO replacement was unsafe; its X4 pasted-reopen crash rejected
it before installation. Do not install that superseded package or treat its
native measurements as qualification. The corrected candidate passes that exact
reopen path and the repeated-open allocation-churn regression.

- Installed OSTree: `82a418bb0c1688b1a9f1ead5c76e4e3421c786773cc01819d6e9e6056a1aa063`.
- Executable SHA256: `5028b0917d86ea92f443685812a1399d1e3ca7ed9e6e4f28713d623dd9b7452e`.
- Bundle SHA256: `6ac6bb68008ada1cdde16c61956a213b8795aada27afa8aa6602c02d5f58818f`.

Device-hidden CPU gates pass 1,645 workspace tests, 53 ignored, plus Clippy,
formatting, vendor warnings, naming and source-list checks. All eight CI jobs
pass on that source. The clean SDK archive excludes the two preserved parked
color edits. Its app-path suites pass 43 X4 and 44 ONE X2 checks, zero failures,
with the existing isolated-service/fixture skips. Both camera motion captures
were inspected. The shader/Rust-twin helper is native, not a clean SDK shader
test. The real exported bundle passes a private, device-hidden import and
executable/metadata/license audit. This local test bundle is unsigned.

Normal-audio 2256x1504 60 Hz NAS pan at 1281.413 seconds settles at 30 sources/s,
zero underruns and no growing delay after startup, but worst startup lateness
reaches 1238.9 ms. A different NAS section, 1481.413 seconds, holds recorded
cadence with worst 36.9 ms lateness and zero underruns. Neither result establishes
hitch-free playback, all-file coverage or 240 fps capacity.

The actual installed app, with no app-path override, also settles at 30 sources/s
on the reported NAS view, zero audio underruns. Startup still reaches 2284.5 ms
picture lateness and catches up during subsequent intervals; the whole-run
cadence parser fails. Existing trace receipts show several 100 to 235 ms draw
completion spikes early in that run, but do not isolate GPU execution from
submission, callback delivery or scheduling. Startup remains a defect, not an
accepted delay. Installed ONE X2 riser pan settles at 30 sources/s, worst 39.8 ms
lateness and zero audio underruns. No new kernel entries or scoped OOM events;
postflight pressure averages are zero in the corrected playback/UI runs.

Installation disables dependency/related-ref/pull updates. Origin, permissions
and all recorded shared runtime identities remain unchanged. The authenticated
`dd908324` package is retained for rollback. Owner network retesting remains the
merge gate. No merge, release, general performance fix or 240 fps pass is claimed.
Package: `scratch/flatpak-delivery-591cf695/`; evidence and the dated input
controls: `scratch/playback-independent-20260927/` and
[network buffering record](research/network-file-buffering-20260930.md).

## Previous source-completion private test package, September 30

The clean `dd908324` package described above was installed. Its archived
source excludes the two parked periodic-color edits present in the accepted
native preview; both dirty files remain preserved. Exact package identities:

- Installed OSTree: `ecaf7ca759ed4576b93ef73518ec89d1118d543a2b2760eac77a8c4aa15b6354`.
- Executable SHA256: `26dff50a64214df96e803bc078423f8629267a6fe7235d719184d41b3ac78fc9`.
- Bundle SHA256: `19b70da525c14a2e4373c7f632413c475d4f377636fdee70cecb94afa87c59d6`.

Before installation, the actual runtime's local 2256x1504 60 Hz pan maintains
29.96 consecutive source advances/s, with no audio underruns or growing delay.
Network-backed pan still fails, roughly 18.6-22.4 sources/s and 6.05 seconds of
worst accumulated lateness. The separate local 300 Hz pan fails capacity:
129.31 completed redraws/s, 29.44 consecutive sources/s, callback p99/max
33.73/38.53 ms and 236.9 ms worst lateness. Host conditions differ from the
earlier native cohorts; these runs are not an isolated performance comparison.
No new kernel entries or postflight pressure in these checks.

After replacement, the actual installed app, without an app-path override,
maintains 29.94 consecutive sources/s and 62.37 completed redraws/s during a
short local full-window 60 Hz pan at the reported September 20 view. Worst
reported lateness stays at 45.1 ms, with zero audio underruns and no new kernel
entries. Callback p99/max is 21.19/27.84 ms, not the 4.17 ms budget. This is a
smoke check, not a repeated full installed UI suite or hitch-free verdict.

The separate actual-installed ONE X2 riser check maintains 29.92 consecutive
sources/s and 62.33 completed redraws/s in its short 60 Hz pan. Worst lateness
stays at 44.9 ms, with zero underruns and no new kernel entries. Callback
p99/max is 17.07/22.34 ms. Both installed checks' initial pictures were inspected;
neither establishes a 240 fps result or physical scanout timing.

Installation uses `--no-deps --no-related --no-pull`. Origin, permissions and
all recorded shared runtime identities are unchanged. The source `635e9b04`
bundle is authenticated and retained for rollback. This is an interim test
delivery, not a complete network fix, 240 fps pass, merge or release.
Immutable package: `scratch/flatpak-delivery-dd908324/`; runtime/UI and frozen
preview evidence: `scratch/playback-independent-20260927/`.

## Previous private test package, September 29

The interim source-actor package is installed from exact source
`635e9b04e81c4601915a77b953165fb46d4ad732`. It retains the prior independent
audio/source scheduling, adds bounded compressed read-ahead, prepares native
map endpoints once per source, and drains a capture-owned ordered source queue
without a per-source shell handoff. It changes no stitching/color arithmetic,
source cadence or clock-hold policy. The failed triangle prototype, unaccepted
buffering prototype and two parked color edits are excluded.

- Installed OSTree: `122c2d2dce79c2131bbde0e6cad8f9022d7f153c1a1a40930414812119a36278`.
- Executable SHA256: `06ae1bccddfb2996f6e70903102934b1c1a14d3eadfa960426db11fd60ebf63e`.
- Bundle SHA256: `14eefdc4019bf513af5b2a5b1683149487929afc2d72b5054426b6fcf39710b4`.

All eight CI jobs pass. Full device-hidden workspace gates pass1,630tests with
53ignored, including unavailable-device returns and parked color variation,
not hardware qualification. Real GPU no-shell regressions pass on both cameras;
31-source sequences on the reported X4 wide view and ONE X2 riser view are
byte-identical to retained parent captures. Separate actual app-path suites
pass43X4 and44ONE X2 checks, zero failures, with the existing sandbox-service,
exact-view, import-fault and pair-fixture skips. Motion captures inspected;
the shader/Rust-twin helper is native, not the clean SDK shader. X4 postflight
records dock-disconnect/USB-C events, not a GPU reset; ONE X2 has no new kernel
entries. Global postflight memory-pressure averages settle at zero.

Packaged full-size60Hz local pan keeps29.95source advances/s without growing
delay, worst25.2ms. Completion gaps still reach34.1ms. A NAS idle-view repeat
also keeps up, but is a previously exercised range. The300Hz pan FAILS:
213.9redraws/s,19.4source advances/s and14.1s growing delay. More importantly,
the final actual-installed NAS60Hz pan also FAILS, roughly20source advances/s
and10.91s growing delay, no audio underruns. Its capacity parser separately
rejects a skipped screen source; this does not erase the genuine cadence
failure. No new kernel entries in that run. These are callback/wall-time
measurements, not physical scanout or a complete performance qualification.

Installation uses `--no-deps --no-related --no-pull`: origin and permissions
are unchanged and shared runtimes are not updated. The prior68591c10bundle is
retained for rollback. This package is not a reliable playback fix, accepted
branch, merge or release. Owner retest and remaining architecture/performance
work remain due. Immutable build: `scratch/flatpak-delivery-635e9b04/`;
runtime/UI receipts: `scratch/playback-independent-20260927/`.

## Earlier September 29 private test package

The interim playback redesign is installed from source
`68591c10df7c7771b8e735866d2f0eadaf45622a`. Audio refill no longer waits behind
bounded video delivery, and filtered source progression no longer depends on
compositor redraws. The approved presentation policy may omit stale completed
screen updates, while every stitching/color input remains ordered. Shown-frame
ownership is retained separately. Stitching and color arithmetic are unchanged;
two parked working-tree color changes were excluded by building a clean archive.

- Installed OSTree: `075b9f58b288b83233e75fba9c409cd2d6ea4cf226cd17169440c094578280bd`.
- Executable SHA256: `9510194b623b6e3ae3d67a64052420d667b69c961bfc24e21d3a7be008399cc6`.
- Bundle SHA256: `4422659a01af887466b9cba6f35a59e6831e510558d62e04ea630e46d972cdde`.

Separate actual app-path UI suites pass 43 X4 and 44 ONE X2 checks, zero failures,
with the existing sound-device, portal, exact-view, sandbox import-fault and
paired-fixture skips. Four motion captures were inspected. A separate X4
60-to-1-to-60-Hz compositor experiment keeps audio supplied and source completion
ordered, eliminating the old package's accumulating draw lag in that scenario.
It is not proof of physical scanout, all-camera coverage or 240 fps capacity.
The native shader-twin helper uses the working tree, not the packaged SDK shader.
After installation and runtime restoration, a normal installed launch (no
app-path override) also sustains 30 source advances/s in its short X4 check,
with no audio underruns, a normal exit and no new kernel entries. Its executable
is authenticated and its final picture inspected. This is a smoke check, not
a second full installed camera suite or capacity qualification.

The slow-input defect remains: a delayed-packet experiment reduces delivery to
about 23.7 fps and accumulates 3.332 seconds of video lateness before catching up.
The package is not a complete A/V-sync fix. Coordinated rebuffering is not yet
implemented or owner-approved. New branch acceptance and merge gates remain due.

The old installer unexpectedly updated five shared runtime refs. Their exact
pre-installation commits were recovered from the transaction journal, restored
and verified; this was not an intentional dependency upgrade. Future installation
commands must use `--no-deps --no-related --no-pull`. Origin remains
`kjerag-origin`, and the verified September16 `f557ee59` package below is retained
for rollback. No release was published. Private receipts and restoration log:
`scratch/playback-independent-20260927/`; immutable build:
`scratch/flatpak-delivery-68591c10/`.

## Previous private test package, September 16

The installed private test package was built from exact source
`f557ee5970e37e18c1e1561ba76081ff914f3494`, tree
`7adeaf1dece60233215fa2d6f9381352d5c4ad3a`. That tree matches merged main
`748ea008`; the artifact retains its actual build-source identity, not the
later merge wrapper. This is not a new tag, public release or signed-channel
update. The historical capacity failures below remain unresolved.

- Installed OSTree: `e583efed628be69343a4b6be71289d6adfc2942dd131bdfd5a9ba57db039b331`.
- Executable SHA256: `b717806413367dbc4dc1d49ccbaff7a32073d7bafa5e2295368597ee8d717dbb`.
- Bundle SHA256: `2f341b9b0667b57ae718d8499105866e215981edb78b38bc1d91ca74320db44c`.

The device-hidden offline SDK build and separate actual-bundle import audit
passed executable, metadata, FFmpeg 7 and license checks. Separate app-path
runtime suites passed 43 X4 Air, 44 ONE X2 and six focused X3 endpoint checks
before installation; actual installed suites independently passed the same
counts. Every running player executable was authenticated. Both motion captures
from all four full suites and both X3 endpoint captures were inspected.
Drag-release and failed-paste regressions passed on X4 and ONE X2. X3 reached
its final source, navigated backward and returned to a stable endpoint; this
does not qualify X3 horizon or seam quality. The usual sound-device, portal,
exact-view, sandbox import-fault and cross-mount pair-fixture skips remain.
The native shader/Rust-twin check is separate from SDK-shader provenance.
These are 1280x720 functional checks, not capacity or hitch-free qualification.

No new kernel entries or scoped memory-limit/OOM events appeared during the
runtime suites, and each returned to its pre-test GPU-memory counters. Host
pressure counters varied. An intermediate CPU-only build snapshot reached
its 6 GiB limit with limit events but no OOM or scoped swap; this is not final
whole-build resource accounting. These checks do not prove GPU containment
or a desktop-freeze fix.

Only the app deployment was replaced, preserving data, runtimes, related refs
and remote settings. Origin remains `kjerag-origin`. The immediate rollback
bundle is source `1d5e9e46179dd5f445d85bc1c19eb00b3b01cc0f`, OSTree
`19b12aecff2fea225650b648481138b723b845e1f4f34c16fd3fb7dda161bd97`,
executable `3a779b020814c2b83026181136692576fcb94a56ba8569539b2967beb59fda74`
and bundle `4774863f06652d157e33ed5493b38bcc86aedbb13d58ecb7b85179940ecaa35f`.
Its receipts remain in `scratch/main-package-20260916/`. The earlier accepted
`90721189` package and signed 0.3.1 recovery artifacts also remain available.
The owner's "lgtm" applies to `90721189`; the "Yes, fixed" drag verdict applies
to frozen native `5b87ed90`. Neither is reassigned to this combined SDK binary.

Relative to `1d5e9e46`, only PR #228's generic endpoint seek and terminal-wait
correction and PR #230's NaN-volume guard change production behavior. Selected
stitching/color arithmetic, cadence, dependencies and permissions are unchanged.
The unqualified X3 horizon candidate in PR #212 is excluded. Normal-channel
restoration remains due after review.

Private receipts: `scratch/endpoint-audio-package-20260916/`. No new visible
tradeoff, performance acceptance or closure of #186/#187 is implied.

## Accepted tradeoffs

- For the September29 playback redesign, the owner approved omitting outdated
  completed screen updates to recover synchronization. Every camera source still
  receives ordered stitching and color processing. This does not authorize
  reduced source/seam cadence or accept sustained A/V lag.
- The owner accepts the installed branch player: "looks good. not perfect but
  pretty damn good" (2026-09-12). This is Studio-like output on reviewed footage,
  not exact Studio parity or universal camera/mode coverage.
- Reduced-resolution temporal correction and source-rate prefiltering can
  change noise, softness and moving-edge detail. The owner accepted the moving
  comparisons and subsequently the installed player. Full-resolution source
  detail remains the base picture; readable full-resolution references remain.
- Opening/seeking prepares the first picture before starting playback. The
  owner explicitly accepted roughly 0.1 seconds of additional preparation to
  avoid the measured filter-activation pause.
- Seeking restarts stitching history. Initial pictures can differ slightly
  from uninterrupted playback; drag previews are keyframes, release is exact.
- Independent 16-patch-row alignment regions, hardware lens sampling and GPU
  interpolation/threshold rounding can differ from the readable global/CPU
  references. These were disclosed before branch testing; acceptance of the
  installed result does not establish identical internal arithmetic.

Frame-time spikes and sustained source lag are limitations, not accepted
performance tradeoffs. No reduced seam-refresh cadence or invented gradual
color-update policy is selected.

## What was merged

- One automatic GPU-resident stitcher for the qualified ONE X2 and X4 Air
  calibration routes, with source-owned alignment maps and photometric ratios.
- Bounded source preparation, exact source/map/color pairing, asynchronous
  readiness notification and completion-based decoder/GPU resource retirement.
  A busy draw retains the prior complete picture and controls together; that
  fail-safe is not permission for arbitrary control lag or playback stalls.
- GPU temporal correction at source cadence, horizontally periodic sampling,
  and rotation-stabilized world coordinates independent of mouse direction,
  window size and the display's horizon setting. Redraws reuse completed work.
- Responsive direct seeks and first-picture preparation; the local iced
  named-child reconciliation correction preserves Scene across controls
  appearing/disappearing. The known separate COSMIC issue #149 is not claimed
  fixed by this work.
- Local, pinned iced device-limit/presentation patches with their original MIT
  licenses and provenance. Readable reference implementations, regression tests
  and research records are retained; personal video evidence stays untracked.

Before merge, the delivery branch was based on main
`67c7fea25ec57c0eb6e8d6d63940e9d63303dd67`, verified against the remote on
2026-09-12, with no main-only commits to integrate. The tested implementation
commits were retained in order. Preparation changed documentation, store
description and ignores for local build/Python caches, not player code,
shaders or dependency versions.

The subsequent cleanup removes unreachable resident vertex-cache bridges and
restricts that cache's module, pipeline state and shader builders to tests.
Its full-field reference and comparison consumers remain. No selected shader,
filter weight, source cadence, correction chart or scheduling rule changes;
this reduces unused production surface, not a claimed speedup. Obsolete module
comments are corrected to describe the selected GPU/temporal path.

## Exact owner-tested package

- Runtime source: `48e1d741f1bf7e7d8882624dff87754227bb8c7d`.
- Installed OSTree: `e91cb1142e034257e56d72714b34af79fe0e0f484a572d6ff31848dd08fdca04`.
- Executable SHA256: `6e5f5c2e410b2a604f543fae50ca0247ce2a1a62a9259c0c5ce498d65451b44d`.
- Bundle SHA256: `655a0aa11e8485af3268616bdcb6eb9fb131907b59b19b14a5eb9717c0524956`.

Both-camera installed UI qualification passes: 40 X4 and 44 ONE X2 checks,
including exact reported views, pause, backward seek and the actual scrubber.
Package identities are unchanged after qualification. Isolated sandbox sound
device/volume and preload/import-fault cases remain skipped; the shader check
uses the native Rust twin. Separate capacity runs have active null-sink audio.
Permissions, runtime and origin are unchanged. The prior `dad5d709` verified
bundle is retained for rollback.

Historical accepted-build cohorts at 2256x1504, over 40 seconds of installed
playback with a moving view, measured:

| Camera | Completed redraws/s | Source advances/s | Completion spacing p99 / max |
| --- | ---: | ---: | ---: |
| X4 Air | 280.499 | 29.975 | 13.813 / 23.463 ms |
| ONE X2 | 318.024 | 29.975 | 9.729 / 23.788 ms |

Both have 1,199 consecutive source advances and no reported drops, starvation
or audio underruns. These are completed rendering-capacity measurements, not
240 unique decoded frames or physical scanout on the 60 Hz panel. Native X4
remains below target (217.949 redraws/s, 28.575 source advances/s); the native/SDK
gap remains unexplained. Do not substitute the native result for the tested
Flatpak, or use the Flatpak result to promise every native build's performance.
Subsequent release qualification reproduces sustained source lag even in the
exact accepted Flatpak. The historical successes above are retained evidence,
not a current unconditional capacity pass; see the release results below.

Local evidence: `scratch/flatpak-delivery-48e1d741/`,
`scratch/installed-capacity/world1147-installed-candidate-01`,
`scratch/installed-capacity/world-x2-installed-candidate-01`, and
`scratch/chromatic1147/`. Captures and receipts are not release assets.

## Merge gates

Prior exact-runtime qualification passes 1,496 release-workspace tests with 52
explicitly ignored, release workspace/all-target Clippy, formatting, source-lock
and rename checks. Source-policy, actual-shader and real-Scene tests cover
source pose ownership, camera/aspect/horizon changes, history wrap and seeking.
All 31 temporal-off controls and 62 full/half reference pictures remain exact.

The standard non-release local gates pass on Rust 1.97.1: formatting,
workspace/all-target Clippy, **1,496 workspace tests with 52 ignored**, source
consistency, rename, five harness-startup checks, 14 controls-log tests,
AppStream validation and the metadata tests with libav discovery disabled.
Logs are in `scratch/merge-readiness-20260912/radeon-01/`.

The final cleanup tree passes the same complete gate set: 1,496 workspace tests,
52 explicitly ignored, and every ancillary check above. Its separate logs are
`scratch/merge-readiness-20260912/cleanup-final-01/`. This includes the test-only
vertex-cache restriction and removal of its unreachable production bridges.

The first restricted-sandbox test run selected llvmpipe software rendering and
failed 38 GPU checks. Its log is retained, not erased or called a pass. The
successful rerun requires Radeon Vulkan explicitly (`KJERAG_REQUIRE_GPU=1`,
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/radeon_icd.json`, `WGPU_BACKEND=vulkan`)
with serial tests. No runtime code or assertions were changed. Existing
vendored dependency warnings remain; workspace Clippy exits successfully.
Both-architecture GitHub CI passes on preparation head `fc0e6775` in run
`34676341245`, and on final cleanup head `2afef25c` in run `34677590261`:
all six jobs green on each. The rebuilt final cleanup also passes 50 native
X4 UI checks at the reported1147 view before merge.
Release uses the existing hardware-independent CI policy; software-GPU
arithmetic compatibility is not established by the Radeon qualification.

## Remaining scope and release plan

- Camera modes, higher-bit-depth snapshots and GPUs beyond the qualified
  footage/hardware are not newly certified. X4 model-6 projection still uses
  the older calibration's image-circle/alpha support at the coverage boundary.
- Existing DJI Osmo 360 playback is unchanged and not requalified here. Its
  `.osv` command-line/drop path is distinct from the `.insv` chooser and shared
  Insta360 engine; the store/README now acknowledge that existing support.
- Automatic lens color matching is implemented and visually reviewed. Exact
  isolated Studio photometric parity is not claimed; issue #185 remains open
  rather than treating all of its stronger criteria as closed by this delivery.
- aarch64 is compiled/unit-tested in CI, not playback-tested on hardware.
- Current tests bound the reported defects; they do not prove absence of every
  hitch, seam artifact, noisy patch or moving-detail difference.

The active owner goal now explicitly includes cleanup, merge and release.
Feature release **0.3.0** is tagged at `c21afcd05ee8ac984ced905a0a1bf822dca1ddcf`,
after PR merge `53ecc92957518032ecfdc3abbb9aafb814267022`. Both the cargo-release
dry run and execution passed47 native real-footage UI checks. The combined
hook regenerated and checked Flatpak sources before the existing player hook;
sources were unchanged because only workspace versions moved. The release
commit changes Cargo.toml, the five workspace Cargo.lock versions and the dated
metainfo entry, not player semantics. It is authored `noreply@harding.dev`.
The GitHub PR merge used the account's primary address because the coordinator
omitted `--author-email`; published history was not rewritten. Subsequent merges
must specify the mandated address explicitly.

All jobs pass in workflow `34678413630`: six tag CI gates, both architecture
bundles and both signed-channel builds/publications. Package/channel
qualification remains tracked in [issue #187](https://github.com/aeharding/kjerag/issues/187).
See [RELEASING.md](RELEASING.md) for the existing release mechanism.

### Published GitHub bundle

Both SHA256 sidecars match their downloads and the GitHub asset digests:

- x86_64 bundle: `2a02af7a3fec06965a387677ba11c3cd6cd36bb6fd68a8f3c60853457fdf9fbd`.
- aarch64 bundle: `ffc1346f341a5632260350827f56d798f80dd274344c843406b8b5318f4ade2e`.
- Installed x86_64 OSTree: `48524a657862949680937854b24f1b48a4928182b309c4e8f004eba63c47d68e`.
- x86_64 executable: `9a9241d35078cab3e47ad96c7edf30162e6d96cc06495e58641a69e4db48c9b2`.

The actual installed bundle passes 40 X4 Air and 44 ONE X2 UI checks. Its exact
reported-view PPM captures are byte-for-byte equal to the owner's accepted
package on both cameras. Permissions remain unchanged; the sandbox sound-device,
preload/import-fault and native shader-twin qualifications above still apply.
Logs: `scratch/merge-readiness-20260912/release-bundle-ui-01/`.

### High-rate playback does not currently qualify

The existing capacity harness retains executable/OSTree identity guards,
completed source-associated GPU presentations, consecutive source indices and
active null-sink audio. These 2256x1504, 40-second, 300 Hz requested-view cohorts
have no reported drops, starvation or audio underruns, but the X4 source backlog
is real:

| Installed package / camera | Completed redraws/s | Source advances/s | Completion p99 / max |
| --- | ---: | ---: | ---: |
| GitHub 0.3.0 / X4 | 255.174 | 27.175 | 15.158 / 21.301 ms |
| GitHub 0.3.0 / X4 repeat | 250.199 | 26.675 | 15.046 / 19.502 ms |
| GitHub 0.3.0 / ONE X2 | 312.399 | 29.950 | 10.996 / 22.847 ms |
| Restored accepted 48e1d741 / X4 | 254.899 | 27.225 | 14.990 / 23.337 ms |

The X4 runs accumulate about 3.6 to 4.4 seconds of lag. A 250 Hz requested-view
control on the accepted build still measures only 232.049 redraws/s and 27.150
source advances/s. Lowering that requested rate is not a demonstrated fix.
Earlier in the same qualification session, the identical accepted executable
measured 228.174 redraws/s on the host versus 272.850 inside Flatpak, with
29.925 and 29.975 source advances/s respectively. Environment matters, but no
unique cause is established. Source timing is aligned before the high-rate pan;
the sustained deficit develops during it. Do not label this a new-release-only
regression, an accepted performance tradeoff, or a full-rate 240-capacity pass.
Receipts: `scratch/installed-capacity/release-030-*` and
`scratch/merge-readiness-20260912/sdk-cross-runtime-result.md`.

### Signed public channel

The signed public remote is `https://kjerag.harding.dev/`, with both
`gpg-verify` and `gpg-verify-summary` enabled. Its authenticated app commits are:

- x86_64: `60d3f8f271cb26e9a1412f4c6d45bf2f7d13bcbbec44bcee353bf841105129e0`.
- aarch64: `3438a6b776c4c8ba9aef471d174aa35cba4d9495f253a85a0ddb2cedfa3ec372`.

Both architecture-specific summaries advertise the app and both AppStream refs.
An aarch64 AppStream update attempted on this host fails with missing refs, but
this Flatpak client's supported architectures are only x86_64 and i386; its
updater documents that restriction. This is not evidence of missing published
ARM metadata, nor a successful ARM client test. aarch64 install/playback remains
unperformed on supported hardware.

The signed x86_64 executable is independently rebuilt, with SHA256
`ef1429ac32e0b115404335acc7bcdd59de47bbf890255f4b7e6632eeed784163`.
Do not transfer the GitHub bundle's runtime qualification by identity. The
installed app has now been moved from the local, unsigned test origin to the
signed public `kjerag` remote. This exact signed package separately passes
40 X4 Air and 44 ONE X2 UI checks, with unchanged before/after identities and
reported-view PPMs byte-for-byte equal to the accepted package on both cameras.
Its logs are `scratch/merge-readiness-20260912/release-channel-ui-01/`.
Its separate 40-second, 2256x1504 capacity cohorts measure:

| Camera | Completed redraws/s | Source advances/s | Completion p99 / max |
| --- | ---: | ---: | ---: |
| X4 Air | 253.949 | 27.600 | 15.198 / 25.173 ms |
| ONE X2 | 314.724 | 29.975 | 10.886 / 24.094 ms |

The X4 source cadence still fails, with reported lag reaching 3.14 seconds;
ONE X2 advances 1,199 consecutive sources at full cadence. Both report no
drops, starvation or audio underruns. Neither average removes the frame-time
spikes. Receipts: `scratch/installed-capacity/release-030-channel-{x4,x2}-01/`.
The accepted test bundle remains preserved for rollback. Channel manifests and
receipts are under `scratch/release-0.3.0-20260912/channel-audit/`.

The 0.3.0 channel payload omits the license text: it has the AGPL identifier in
metainfo, but no `LICENSE` file. The GitHub bundle's newer builder automatically
records the source license; the separate signed-channel builder does not. The
shared manifest now explicitly installs the repository's `LICENSE` into
`/app/share/licenses/dev.harding.Kjerag/kjerag/LICENSE`. This packaging-only fix
landed in PR #188 and shipped in 0.3.1; it does not retroactively change 0.3.0.
Both actual 0.3.1 payload checks are recorded below. Before publication, the
local Builder successfully resolved the updated manifest; staging its
actual license-install command produces a byte-identical copy of `LICENSE`
(`0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0`).
That staging receipt is a pre-publication manifest/command check, distinct from
the subsequent actual-package verification.

The follow-up tree's unchanged Rust code passes the complete local gates in
`scratch/merge-readiness-20260912/release-record-gates-01/`: 1,496 workspace
tests with 52 ignored, formatting, workspace/all-target Clippy, source/name
checks and the ancillary gates above. Source/name and whitespace checks pass
again after the documentation and explicit license-install changes.
The permanent release hook regenerates and checks offline sources, but now
stops if the result differs from the reviewed commit. A generator change must
land through a normal PR; it cannot silently enter an automatic release commit.
A failed dry run can leave that generated diff for inspection.

### Packaging-only 0.3.1

PR #188 merged at `4b3bcbf1d4fe618ee31f8e6c87df7cb0f1f3327b` after independent
review and all six CI checks in run `34681640290` on exact head `9f984443`.
This merge explicitly uses the mandated `noreply@harding.dev` author address.
Tag `0.3.1` names `1fb97b124dd33b507621080ad649d0aa86c14080`, also authored and
committed with that address. The ordinary dry run and execution each pass 47
native real-footage UI checks, including the new generated-source guard. No
generated-source difference was accepted; offline sources remained identical.
Release receipts are in `scratch/release-0.3.1-20260912/`.
All ten jobs in release workflow `34682255167` pass, including both bundles
and both signed-channel publications. Signed metadata authenticates both app
refs and advertises both architectures' AppStream refs. The x86 AppStream
client update succeeds and reports 0.3.1; ARM client playback remains untested.

No Rust runtime code or shader changes between 0.3.0 and 0.3.1. The patch adds
explicit license installation, documents qualification and hardens the release
hook, with the usual workspace version and metainfo date changes.

Both downloaded bundles match their published SHA256 sidecars and GitHub API
asset digests:

- x86_64 bundle: `e0ac8706c6ac1aeda6e26b2aaf3ff5dd8e806201540ee8831765fb4523e332a9`.
- aarch64 bundle: `67d69b43fae5ced22a4bb461b559d5d80831083536e4c12df0fd4170f0bbfecd`.
- Installed x86_64 bundle OSTree: `45a20e8dd939d5d12fe40aee7ca2bb977b9d4952cf6e4617129fc48ee12615b1`.
- Its executable: `dd2c4c819313d8e887e624b3add083d5f714f92ebaa658f85469869ae479dcbd`.

The installed download's explicit license file is byte-identical to the source
`LICENSE`, with the expected SHA256 recorded above. Its unsigned bundle cannot
replace the signed-origin deployment using `--reinstall`: the client correctly
refused it for missing GPG signatures, leaving 0.3.0 intact. Qualification then
removed only the exact app deployment, preserving settings and runtimes, and
installed the bundle under its generated local origin. Signed-channel
verification was never disabled. The actual installed bundle passes 40 X4 and
44 ONE X2 UI checks, with both reported-view PPMs byte-for-byte equal to the
owner-accepted package. Its receipt is
`scratch/merge-readiness-20260912/release-031-bundle-ui-01/`.

The final installed app is back on the signed public `kjerag` origin:

- x86_64 OSTree: `b7ac816e7d8e6f80ea858fe09ab5d316ac5aaf23ea064a3ae6d62156ed8ea983`.
- Executable: `b25ff421de8fe42629002ef08d636abfcd6d133c86516e8571fe84c98c6db54e`.
- Published aarch64 app ref: `284d4adb79402d8e17dfd1e826c39a9b3392e59a1cfcdf82ac2e916c391a03aa`.

Its actual installed license also matches the source LICENSE byte-for-byte.
Its separate installed qualification passes 40 X4 and 44 ONE X2 UI checks,
with both reported-view PPMs byte-for-byte equal to the accepted package.
This result is measured, not inherited from the download or 0.3.0. Receipt:
`scratch/merge-readiness-20260912/release-031-channel-ui-01/`. The documented
isolated sound-device and import-fault skips still apply; the shader check uses
the native twin. Both routes' metadata is byte-identical to the corresponding
0.3.0 route, preserving permissions and runtime branch. The accepted test bundle
remains available for rollback.

Future unification of the two builds is already tracked in issue #146, now with
the measured license and unsigned-reinstall failures. It is not part of this
patch. Issue #186 was inadvertently auto-closed when GitHub parsed a negated
closing phrase in PR #188; it has been reopened and the phrase corrected.
The performance defect was not fixed or accepted.

Final signed 0.3.1 capacity checks use the same authenticated process/source
completion harness, 2256x1504 output, 40-second pan and active null-sink audio:

| Camera / requested view rate | Completed redraws/s | Source advances/s | Completion p99 / max |
| --- | ---: | ---: | ---: |
| X4 Air / 300 Hz | 258.199 | 27.075 | 14.757 / 21.956 ms |
| ONE X2 / 300 Hz | 314.999 | 29.975 | 10.971 / 24.469 ms |
| X4 Air / 60 Hz | 62.375 | 29.975 | 30.465 / 32.403 ms |

All traces are valid, with consecutive source advances and no reported drops,
starvation or audio underruns. X4 high-rate playback accumulates 3.83 seconds
of lag and fails the combined target; ONE X2 advances 1,199 sources at full
cadence. The X4 normal-refresh control also advances 1,199 sources at full
cadence, with worst reported lateness 17.1 ms. Completion spacing in a 60 Hz
test is not a 240-capacity verdict; these results do not establish physical
scanout or hitch-free playback. Receipts are
`scratch/installed-capacity/release-031-channel-{x4,x2}-01/` and
`scratch/installed-capacity/release-031-channel-x4-60hz-01/`.

Functional package qualification is complete. The capacity/handoff decision
remains open in #187, with the actual performance defect in #186. The final
documentation-only handoff changes no runtime, shader or dependency; it does
not require a further tag or transfer the historical capacity successes into
a current unconditional pass.

### Bounded performance diagnosis, not a fix

The signed 0.3.0 X4 lifecycle run reproduces lag at 233.199 completed redraws/s
and 28.350 source advances/s. UI redraw-event age stays below 0.056 ms p99,
while the consecutive stitch-worker start interval averages 35.32 ms. The
composite start-to-panorama span rises from roughly 10 to 12 ms before changing
view to 32.68 ms mean during it; temporal filtering also becomes slower.

A 60 Hz view-mode control with the same 1,000 Hz mouse input maintains 29.950
source advances/s, with worst reported lateness 17.2 ms. Start-to-panorama
falls to 13.39 ms mean, and start-to-complete from 51.66 to 27.82 ms. This
implicates view-work contention rather than raw mouse-event starvation. These
composite timers cannot split GPU/driver waiting from CPU import/encoding, and
the instrumentation and variable host conditions prevent calling differences
between cohorts a controlled speedup. Normal-refresh headless functionality is
not physical scanout, absence of hitches, or completion of the 240-capacity goal.
The saved interpretation is
`scratch/merge-readiness-20260912/release-030-x4-lifecycle-classifier.md`.

Release notes describe:

- Automatic GPU stitching and lens color matching for ONE X2 and X4 Air.
- Reduced flickering seam artifacts, including rotating-camera and panoramic
  wrap-boundary cases.
- Reuse completed stitch results while reframing; prepare the first picture
  before playback and seeking, and retain video state when controls appear.
- The known X4 high-rate panning slowdown and unfinished combined capacity
  qualification, without turning earlier good measurements into a guarantee.
- Tested ONE X2/X4 Air scope and Studio-like rather than identical output.
