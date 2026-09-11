# Roadmap (living doc)

Update this file in any PR that changes project status. Work queue is
GitHub issues; this doc is the map, issues are the tasks.

**Product direction, owner approved 2026-09-05:** deliver a usable zero-config
player with Studio-like stitching, stable horizons, responsive seeking and
240 fps active rendering capacity on the owner's machine. Generalize the GPU
engine through explicit camera calibration/geometry, preserving ONE X2 while
bringing the owner's X4 Air footage onto the shared engine. Existing code may
be replaced; broad optimization RE stays frozen. The coordinator manages
delivery, but visible tradeoffs and the tested branch still need owner
acceptance before main changes.
[Issue #184](https://github.com/aeharding/kjerag/issues/184) owns this
shared-camera engine work.

**Sky680 test Flatpak installed, 2026-09-11:** after the owner requests
"continue I want flatpak", the accepted periodic-horizontal fix is packaged
from source `dad5d7092052266df6eebc82f04c43f1389f3891` and installed as
`dev.harding.Kjerag`. Installed OSTree commit
`4afd8b3072318b27b774d96530c4bcf320c631e727d98b47c056d4135c6ba163`
and executable SHA256
`4804e8c6c8dcc09c22c2e31648cad5cd743932d93f1ab3dccf44b0ef9f766af8`
match the package. Runtime/permissions metadata matches, origin remains
`kjerag-origin`, and the verified preceding09ca503b bundle remains available
for rollback. The offline SDK build takes 2m43s. Installed UI checks pass
40 X4 and 44 ONE X2 checks, including both exact reported views, moving video,
pause, backward seek, real scrubber, fullscreen and the four paired-file
arrival cases. Captured installed views are inspected separately from the
already owner-approved moving comparison.

The exact installed package's 40-second changing-view tests at 2256x1504
measure 272.424 completed redraws/s on X4 and 317.824 on ONE X2, with
1195/1199 consecutive source advances (29.875/29.975 per second). Both exceed
the 240 redraw/s throughput target in these cohorts. Completion-spacing
p99/max are 13.942/22.658ms and 10.847/27.034ms respectively: this is not a
no-hitches claim. X4 maximum reported video lateness reaches 87.8ms; ONE X2
reaches 5.8ms. No reported drops, starvation or audio underruns occur. Source
cadence, draw throughput and callback spacing are distinct measurements.
Installed results supersede neither the earlier native performance failures
nor the need for the owner's live-player review. No merge is authorized by
the prior movie acceptance. Evidence is retained under
`scratch/flatpak-delivery-dad5d709` and
`scratch/installed-capacity/periodic-dad5d709-{x4,x2}-installed-300-01`.
Dedicated audio-enabled controls checks subsequently pass all three pointer
wakes on each camera, with no reported errors. The installed commit, origin
and executable are reverified after testing; all test players exit. The build
is ready for the owner's live review, not merged. These audio-enabled checks
do not convert the ordinary sandbox harness's skipped volume-popup or preload
tests into passes.

**Sky680 moving fix visually accepted, 2026-09-11:** the owner answers
"Yeah looks good" to new Kjerag versus Studio, then "Looks fixed" to the
source-matched old-versus-new movie. This accepts the periodic-horizontal
correction for the reported moving sky boundary. The accepted candidate is
`scratch/sky680/quarter-periodic-01`, with old LEFT/new RIGHT in
`before-after.mp4`. Playback capacity, sandbox qualification and installed
delivery of this change are next; the previous prefilter bundle is still
installed. No broad-footage, live smoothness or merge verdict is inferred.
Scoped release render Clippy and formatting pass; its only initial lint was
the now-reference-only packed encoder constructor, which is marked test-only
without changing selected production arithmetic.

The native player builds in2m38, SHA256
`303f0a75b6135bb120ef0f8953b724b44c951d3601addd59f3f8c1da2d5c6ce7`.
Its initial40-second2256x1504 X4 capacity run fails the throughput target:
67.075 redraws/s and roughly4 source frames/s. The byte-identical preserved
old player also regresses to78.65 redraws/s and4.65 source frames/s under the
same unchanged harness, versus281.475/29.975 in its historical cohort. Both
current runs have much lower GPU voltage/power and additional host activity.
The shared severe slowdown is not attributable solely to the periodic change;
the changed filter also has measurable extra shader work in current timestamp
probes. These runs do not establish a normal-condition candidate performance
pass or isolated regression size. Installation is withheld pending performance
qualification; no power policy, owner process or installed bundle was changed.
Exact current/historical identities and measurements are retained with the
accepted movie in `scratch/sky680/quarter-periodic-01/README.md`.
The full release workspace suite subsequently passes1492 tests, with51 ignored
across48 suites, required GPU access and both named camera fixtures. Its
compile phase takes2m59 under the current machine conditions. This includes
the corrected whole-stream cyclic-shift regression; the earlier invalid
synthetic-size test failure is no longer pending. The two hash-sealed native
motion/pyramid checks were separately enabled and passed as recorded above.

After the owner's restart, loaded GPU clocks recover from roughly800MHz to
roughly2GHz and the observed PROCHOT_CPU/GFX flags do not recur. The unchanged
old native build measures237.875 redraws/s and29.925 consecutive source advances/s
at2256x1504 in the same40-second stress harness. The accepted periodic build
then measures228.100 redraws/s and28.750 source advances/s, with completion
spacing p99/max14.629/23.286ms and accumulated video lateness up to1.602s.
These are not240fps or smooth-playback passes. The owner requests
"continue I want flatpak" after the restart status. Package the visually accepted
change for actual installed-sandbox testing; do not infer a performance waiver
or merge approval. Both native cohorts are retained under
`scratch/installed-capacity/post-restart-{control,periodic}-x4-native-300-01`.
The final read-only temporal diff audit finds no correctness blocker. Full
release workspace/all-target Clippy and renewed formatting, lock/source coverage,
name and diff checks pass. The pre-restart ordinary native UI run had49/50
checks pass, failing the playing-picture movement check; installed UI verification
must prove that path after the restart rather than treating that run as green.

**Preparation timing explicitly accepted, 2026-09-11:** the owner answers
"Yes, prepare before playback" to replacing the roughly0.1-second ONE X2
activation hitch with extra first-picture waiting when opening or seeking,
and "Yes, that startup delay is acceptable" to the next test build's
roughly0.1-second startup preparation. This confirms the preparation tradeoff,
not acceptance of the separately pending sky-boundary candidate or all frame
time spikes. The existing prewarm implementation remains; no new delay,
cadence change or coefficient smoothing is introduced by these replies.

**Sky680 boundary identified; periodic candidate awaiting review, 2026-09-11:**
the owner confirms "Red line follows the defect. Studio doesn't have the defect"
in the actual installed-versus-Studio moving comparison. The red trace is the
computed body-panorama longitude0/360 cut, not the lens join. The branch tests
consistent horizontal wrapping across the selected quarter temporal pipeline:
pyramid reduction, motion reference search, complete-grid predictors, final
motion packing, fusion, and both current/filtered chroma reconstruction.
Partial coarse grids retain endpoint predictors; Y stays clamped. Full/half
and saved-input reference constructors retain their original clamped behavior.
No invented coefficient smoothing, source skipping, history or cadence change.

The new actual Scene capture passes for all31 exact sources20388..20418, and
the new Kjerag-versus-Studio and before/after movies have been sent for owner
review. Eleven periodic-name regression tests pass on the Radeon GPU, including
both-edge signed motion, motion expansion, chroma, fusion and pyramid checks.
All eight ordinary real-camera filtered Scene checks pass with both footage
fixtures, and the separately enabled retained native motion/pyramid GPU
oracles remain exact. The new whole-stream cyclic-shift test initially uses an
invalid six-level synthetic size; correcting only the fixture to a valid
five-level size yields byte-exact shift/unshift output. Production geometry
gates and numeric tolerances remain unchanged. The larger temporal suite
passes140 other tests, with11 ignored; only the corrected test is rerun so far.
Candidate quality and player-performance qualification remain open. No new
installation, commit, push or merge. The installed prefilter build below is
unchanged. Evidence: `scratch/sky680/quarter-periodic-01/README.md`.

**Prefilter test build accepted and installed, 2026-09-11:** the owner answers
"Looks OK I think" to the new 607 moving prefilter comparison. After the
first-picture delay is explained as roughly 0.1 seconds of extra startup
preparation, not slower playback, and possible extra seeking delay is explicitly
disclosed as unmeasured, the owner answers "oh ok yeah thats fine" to test-build
installation. These tradeoffs are accepted for this test build, not all footage,
live smoothness or merge. The prepared source09ca503b package is installed:
OSTree `7f86afaa2babb9063acd3854dbcfa7c8fa88b003a1611ebd5c7365a19c874f05`,
executable SHA256
`fca43cb5c100d57f3ded60fcfa5c44080c29a738a0f94dce96f73a33303bff7c`.
Installed bytes and runtime/permission metadata match the package, origin
remains `kjerag-origin`, and the previous package is retained for recovery.
Both-camera installed qualification passes: 40 X4 and 44 ONE X2 checks,
including exact views, pause, backward seek, real scrubber, fullscreen and
the four ONE X2 paired-file arrival paths. Dedicated audio-enabled checks
also pass three controls wakes per camera with no drops/starvation/audio
underruns. The actual installed package then passes the strict full-resolution
capacity trace checks during 40-second pans at2256x1504: 312.774 completed
redraws/s on X4 and318.399 on ONE X2, with1198/1199 contiguous source advances
(29.950/29.975 per second, the integer-window count at recorded29.97fps).
Both clips keep up with playback, with maximum reported video lateness5.3/3.3ms,
no drops/starvation/audio underruns and changing-view hashes verified. These
are installed-package measurements, separate from the earlier native rates.
Completion-spacing p99/max remain11.964/21.453ms on X4 and10.537/22.727ms on
ONE X2; this meets throughput in these cohorts, not a no-spikes or broad live
smoothness claim. All test players exit. The installed build is ready for owner
live review, not another optimization or export. No push, merge or public
release. The pending/blocker notes below describe earlier checkpoints.

**Prefilter Flatpak prepared, not installed, 2026-09-11:** source
`09ca503b79a4f1709c05c9cc54493507e755c312` builds offline in the 25.08 SDK
in 2:31.78. Source hashes, Cargo.lock/source coverage, FFmpeg7 linkage,
executable/bundle hashes and the read-only runtime `--version` check pass.
The package's runtime/permissions metadata matches the current installation.
The installed quarter build is unchanged. The new sampling quality and the
earlier roughly 0.1-second prewarm first-picture delay still await owner
acceptance; no installation, installed playback qualification, push, merge
or public release occurred. The prepared bundle and exact identities are in
`scratch/flatpak-delivery-09ca503b/README.md`. Next use that exact package for
installation and both-camera sandbox qualification after the owner's decisions,
not another speculative optimization or a fresh package of a docs-only commit.

**Source-rate display prefilter reaches native throughput target, pending
quality review, 2026-09-11:** the branch evaluates the existing native box
filter at full-resolution source texel centres during the existing snapshot
passes, then uses simple bilinear sampling during corrected view redraws.
Raw source inputs still feed body/map/color/temporal preparation. Source
association, seven-source cadence, allocation count and decoder ownership
are unchanged. Prefiltering before storage quantization and interpolation
changes sharpness/noise; the owner's previous quarter-field approval does
NOT cover this change. Original native-box draw and temporal oracles remain.

Two 40-second native X4 pans at 2256x1504 complete 281.475/274.974 redraws/s
with 29.975 contiguous source advances/s. The matched-build control between
them completes 269.524 redraws/s but only 23.700 source advances/s, accumulating
8.328 s video lateness versus candidate 7.2/49.8 ms. ONE X2 completes 306.450
redraws/s with 29.975 source advances/s across its known ISO transition. All
four runs pass the unchanged strict source/draw/present association and
Radeon/quiet audio checks. These candidate cohorts satisfy the throughput
component of the 240 fps active native-player target, not physical 240 Hz
output or a broad smoothness verdict. Completion-spacing p99/max remain
12.165/25.923 ms and 12.326/32.984 ms on X4, 9.008/22.703 ms on ONE X2.
No no-spikes requirement is invented, and those spikes are not silently accepted.

All 29 direct GPU checks now pass, including the new one-code snapshot
CPU/GPU bound. The initial 28/29 run's sole failure was a shader-text extraction
test needing its reference updated after helper extraction, not a changed
numeric tolerance. All 8 real-camera Scene tests pass and save 93 captures
with matching source indices/times. The fresh 607 moving comparison reuses
the existing Studio export and retained index-derived offset; that Studio
offset is not independently authenticated. All movie-body hashes match the
inputs. The new softened-detail/noise tradeoff is explicitly asked of the
owner, with no answer yet. The full release workspace passes 1484 tests,
51 ignored across 48 suites, with both footage fixtures and required GPU
checks. Release workspace/all-target Clippy, formatting, source-lock, rename
and diff checks pass. A binary32-equivalent test-only literal cleanup follows
the full test run; production code and test bounds are unchanged. All 50 native
X4 UI checks pass, including the exact 612.078 view, pause, backward seek,
scrubber, fullscreen and import-failure recovery. Captures and logs are
retained in `scratch/installed-capacity/quarter-prefilter-ui-x4`.
The installed owner-approved quarter build remains unchanged; no merge or
release. Both this sampling tradeoff and the earlier roughly 0.1-second
prewarm first-picture delay still await owner acceptance before delivery.
Evidence: `scratch/installed-capacity/quarter-prefilter-review.md`.

**Bounded decoder-loan trial retired, 2026-09-11:** removing original-plane
copies preserves all93 source-matched pictures and passes9 real-camera Scene
tests, including recorded GPU draw ownership and both-camera seek pressure.
It holds full source cadence at roughly232 native redraw/s, but repeated
higher-load native runs regress to198-203 redraw/s and13-14 source/s versus
control270/23.75. A faster offscreen result does not override that real-player
regression. A fresh restored control built with the same package selection as
the trial confirms269.875/23.925; the result does not rely on the older frozen
binary alone. The trial is removed before delivery; the approved installed
quarter build is unchanged and240fps capacity remains open. Current FFmpeg7.1
VA-API pools are dynamically allocated (reported initial size0), not the
historical fixed20; the corresponding documentation is corrected. Evidence,
rejected patch and binaries: `scratch/installed-capacity/quarter-zero-copy-review.md`.

**Optional transfer-copy trial retired, 2026-09-11:**
capability-checked DMA-BUF transfer-source imports and four direct texture
copies preserve all93 real607/612/ONE X2 captures exactly. Both synthetic copy
routes, external-memory guards and8 real-camera Scene tests pass; the latter
explicitly require transfer admission. However, matching25-source X4 GPU
measurements show copies cost2.044ms median versus1.527ms for the existing
two MRT passes, a33.9% regression. The trial is removed before app build or
delivery. No new capacity or visual approval claim. Evidence and rejected
patch: `scratch/installed-capacity/quarter-transfer-review.md`.

**Submission-specific completion trial retired, 2026-09-11:**
two paired native X4 runs found no dependable benefit from replacing the
temporal output's queue callback with a private four-byte mapping witness.
At2256x1504 during40-second pans, control redraw rates are231.325/231.075fps
and trial rates231.550/232.775fps. Source-rate differences reverse sign
(+0.675 then-0.475fps); all remain below full29.97fps source cadence under
this load. All strict source/draw/present checks pass and93 comparison frames
are byte-identical, but the0.42% mean redraw gain does not justify the extra
mapping lifecycle. The trial was removed before delivery; the simpler existing
completion path and installed reviewed quarter build remain. No240fps or
smoothness pass. Evidence and rejected patch are retained in
`scratch/installed-capacity/quarter-witness-review.md`.

**Reduced RGB vertex-cache trial retired, 2026-09-11:**
the existing timestamp probe now measures the quarter route itself: X4 body
preparation2.272ms, original-plane copies1.538ms and motion/refinement1.530ms
median over25 source frames. These exclude upstream stitching and viewport
rendering. Reusing the old compact route's164832-byte native-vertex cache
reduces body preparation to1.983ms versus bracketing2.272/2.239ms controls.
The gross saving is under0.3ms per source; other stages do not all improve.
All8 real-camera Scene tests pass, but X4 comparison pixels change by up to
2 codes at607 and4 at612. ONE X2's31 riser pictures remain byte-identical.
The trial was removed before any app build or delivery: the small saving
does not justify new output differences and first-use pipeline cost. No new
movie review, installed replacement or capacity claim. Patch, logs, pixel
counts and the noncomparable first control are preserved in
`scratch/installed-capacity/quarter-rgb-cache-review.md` and its named artifacts.

**ONE X2 first-activation hitch isolated, 2026-09-11:**
preparing the two existing coarse-search pipelines on the temporal worker's
first validated source push removes the measured long hold at source6952 in
two native trials. Frozen controls advance6952-to6953 after104.048/120.797ms;
candidates advance after33.698/34.202ms, near the33.367ms source interval.
Both candidates retain29.975 contiguous source advances/s while panning at
2256x1504. No shader, settings, history or seven-source cadence change.
This is a targeted hitch reduction, not a general smoothness or240fps pass:
completed changing-view rates remain234.525/233.925fps, with completion-spacing
p99 of9.781/11.721ms and maxima21.966/35.919ms.

The startup tradeoff is measurable: first-present-call to first sourced draw
is1647.723/1634.213ms in controls and1732.708/1744.111ms in candidates, about
97ms later on average. This includes compositor/decoder/worker timing, not
isolated compilation or process-launch latency. Fresh seek epochs also prepare
pipelines; their extra latency is not measured here. The tradeoff was explicitly
asked of the owner and is not yet accepted. The installed reviewed quarter build
is unchanged. Native qualification passes1484 release workspace tests,51
ignored across48 suites, with both footage fixtures and required GPU checks.
All93 source-matched607/612/ONE X2 frames are byte-identical to the preceding
quarter candidate. All50 native X4 UI checks pass, including the exact612.078
view, pause, backward seek, real scrubber and import failure. Root inspected
the reported-view capture. Release workspace/all-target Clippy, formatting,
source-lock, rename and diff checks pass. No installed replacement or merge.
Evidence: `scratch/installed-capacity/quarter-correction-load-review.md`, with
both controls, both candidates and the pre-pan audio-identity rejection retained.

**Installed full-resolution load comparison, 2026-09-11:**
the unchanged quarter-correction Flatpak was exercised with 40 seconds of
continuous pointer panning at 2256x1504, at nominal 300, 240 and 60 Hz. Exact
installed identity, Radeon execution and quiet audio checks pass. At 300 Hz,
player source reports deteriorate to 23-25 fps with 7.073 seconds worst
lateness. At 240 Hz they remain near 30 fps, with 29.20/30.80 catch-up windows
and 115.8 ms worst lateness. At 60 Hz, steady reports are 29.80-30.00 fps; the
256.4 ms worst lateness is already present in the startup-inclusive report
and does not grow during panning. All runs report zero drops, starvation or
audio underruns. The 300 Hz overload is not itself a failure at the 240 target.

The source-authenticated capacity checker correctly fails all three traces:
the new corrected draw path lacks the existing old resident path's draw marker.
The recorded surface-only rates are 307.12/243.42/62.42 commits per second,
with commit-spacing p99 of 8.666/9.031/16.956 ms and maxima of
21.513/19.516/22.562 ms respectively. These are not source-authenticated
changing-view rates, physical presentation or a 240 fps pass. Missing markers
are not evidence of blank frames. Preserve the strict checker; reuse its
existing draw marker on the corrected path before claiming a native capacity
gate. No app or installed-package change was made for these runs. Evidence:
`scratch/installed-capacity/quarter-correction-load-review.md` and its three
named run directories. Ordinary-display playback and high-refresh overload
are now separated; owner live review and performance headroom remain open.

The working branch now reuses one shared opt-in draw marker in both old
resident and corrected draws. It retains the existing JSON schema, Reframe
hash and completion callback, with no shader, normal GPU command, scheduling
or cadence change. The two marker unit tests and all eight real-camera Scene
tests pass; all 93 source-matched 607/612/ONE X2 pictures are byte-identical
to the preceding quarter captures. Release workspace/all-target Clippy,
formatting, source-lock and rename checks pass.

The first corrected-path native 240 Hz run now passes the unchanged strict
source/draw/present checker: 9,302 sourced commits, all eventually completed,
9,301 inside the 40-second pan (232.525/s). Sources 18549-19710 are contiguous
at 29.025 advances/s. Of 8,140 same-source redraw pairs, 7,704 have a changed
Reframe hash. Begin-spacing p99 is 8.105 ms/max 16.329 ms; completion-spacing
p99 is 10.646 ms/max 20.791 ms. This establishes changing-view rendering
with stitching active, but not the required capacity/full source rate.
Native runtime and marker overhead differ from the installed runs above;
this is not an A/B speed comparison. The installed owner-review package is
unchanged. Evidence: `quarter-x4-612-native-marked-240-01` under the same
installed-capacity directory, plus `quarter-marker-*` test/build logs.

The matching ONE X2 run also passes strict association: 234.300 completed
redraws/s with 29.975 contiguous source advances/s, spanning sources 6571-7770
and the known ISO transition. Begin-spacing p99 is 8.190 ms/max 30.140 ms;
completion-spacing p99 is 10.133 ms/max 36.046 ms. It is not a 240 fps pass.
The source-authenticated commit trace locates a 104.048 ms dwell from source
6952 to 6953 at first temporal activation, not merely a pre-prepare `shown`
report interval. That transition lazily creates two coarse-search pipelines.
Their contribution is a testable cause, not yet established: next isolate
preparing those pipelines during initial buffering, without changing picture
math, source cadence or the installed owner-review build. Evidence:
`quarter-x2-native-marked-240-01` in the same directory.

**Quarter correction installed for live owner testing, 2026-09-11:**
source `5f7fc59d9170e5c46a4574ae83c9efc1aad1ee11` now supplies the installed
`dev.harding.Kjerag` stable test build. Executable SHA256 is
`65dce13b2289284bebb71be7242919327f8fcdaa429012f4ed3bfd82e78cae4c`;
installed OSTree is
`db43978530f93ac3fc52b71f4ec4ed7a507be0376e964710464f28a09d2047d2`.
The archived-source SDK build completes in2:34, and installed executable bytes,
runtime/permission metadata and deployment identity match the package before
and after qualification. Origin remains `kjerag-origin`. The preceding
half-field package is retained. There is no merge or public release.

The actual installed app passes40 X4 UI checks at612.078 and44 ONE X2 checks
at212.512, including their exact views, pause, backward seek, real scrubber,
screenshots, fullscreen and ONE X2 paired-file opening. Root inspected both
reported-view captures. Audio controls and injected import failure skip in the
main sandbox harness; native qualification separately covers those paths.
This verifies test-build delivery, not broad visual acceptance or240fps capacity.
Separate audio-enabled installed controls-wake checks pass for both cameras,
with three pointer wakes each and null-sink48kHz stereo. Steady source reports
are29.99-30.01fps, with zero drops, starvation and audio underruns. Startup-inclusive
reports are16.44/18.94fps; worst video lateness is5.1/15.8ms and startup-inclusive
audio errors59.6/69.1ms. These short1280x720 checks guard controls wakeup, not
full-resolution presentation or the4.17ms capacity budget. Evidence:
`scratch/flatpak-delivery-5f7fc59d` and `scratch/controls-wake/run.sqZkL1gY`,
`run.7xwvlLEH`. The remaining owner review is live blotches, motion and hitches
in the installed build, not another offline movie.

**Quarter correction approved in moving comparison, 2026-09-11:**
the owner answered "Yes, looks acceptable" to the607 moving comparison with
Studio on the left and this faster candidate on the right. The smaller field,
retained full-resolution detail and possible noise/motion-edge differences
were explicitly disclosed. This approval covers that movie, not612, all footage,
live-player smoothness,240fps capacity or a merge.

The selected branch uses a1920x960 X4 correction field and1408x704 ONE X2
field, with five real motion levels. ONE X2's natural quarter raster is rounded
down to complete32-pixel blocks while retaining the entire2:1 sphere. Original
source planes, maps, photometric updates, direct-view detail, exact source
ownership and seven-source temporal cadence are unchanged. No coefficient EMA,
skipped update or interpolation between fields was introduced. The preceding
half and full paths remain test oracles.

At2256x1504 sRGB, two uninstrumented12-second X4 active Scene samples reach
259.65/239.56 completed changing-view redraws/s and29.998/29.997 contiguous
source advances/s. Bracketing half-field controls reach161.85/149.33 redraws/s
and26.99/24.17 source advances/s. Candidate p99 times are8.05/9.42ms and maxima
9.82/17.48ms, with zero drops, starvation or audio underruns. One sample falls
below240 and tails exceed4.17ms: the target is NOT passed. This queue-prefix
completion diagnostic is not native-window presentation or GPU-only timing.

All8 real-camera Scene tests pass, including exact reported views, source
ownership, seek/EOF and the ONE X2 ISO transition. The full release workspace
passes1481 tests with51 ignored across48 suites, with both footage fixtures
and required GPU checks. Release workspace/all-target Clippy, formatting,
source-lock and rename checks pass. Native UI qualification passes50 checks
at the exact612.078 view. The initial type-mismatch build failure and subsequent
successful rebuild are retained, not omitted from the evidence.

The607 comparison uses the existing authenticated Studio export;612 and ONE X2
comparisons use preceding Kjerag candidates, not Studio. All movie panels decode
to their named source frames exactly below the labels. Evidence and frozen
binaries are in
`scratch/studio-seam-flicker-612-20260909-01/quarter-correction-review-01`.
Next deliver and separately qualify this exact branch candidate in the installed
Flatpak for owner testing. No merge or public release is authorized by the movie.

**Reduced-correction capacity budget and rejected motion trial, 2026-09-11:**
the uninstrumented Scene diagnostic still misses the2256x1504 active target:
162.91 completed changing-view redraws/s and27.33 contiguous source fps,
p9915.124ms and maximum22.152ms. Paused482.26fps is not the active result.
This queue-prefix completion diagnostic includes host/polling/backlog costs,
not native presentation or intrinsic shader timing.

A test-only, asynchronous encoder-timestamp probe now separates the correction
stages without an added submission or completion wait. Across25 warm X4 sources,
median body preparation is6.30ms, source-plane copies1.37ms, control conversion
1.16ms, history/pyramid1.07ms, motion/refinement3.65ms, fusion1.77ms and final
colour0.41ms. Upstream stitching and viewport rendering are outside those spans;
summing their medians is not an end-to-end source measurement. Both real-camera
Scene checks pass, and the31 profiled612 captures are byte-identical to the
saved selected-player frames. The first X2 command used the wrong fixture
variable and exercised no footage; only its corrected invocation counts.

One bounded workgroup-tile trial shared reference reads across motion-search
shells, preserving candidate order and sums. Raw-vector/shader checks pass
(14 passed,1 existing ignored), and all31 actual612 frames remain identical.
It nevertheless increases motion/refinement to5.03ms while the other stage
times remain similar. The trial was removed from active code, not delivered.
The complete patch and logs remain in
`scratch/studio-seam-flicker-612-20260909-01/correction-capacity-01`.
There is no new owner acceptance or240fps result. Next evaluate a further
reduced correction field as an explicit quality/performance alternative, not
as equivalent arithmetic or an approved change to the installed test build.

**Reduced correction installed for owner testing, 2026-09-11:**
the clean archived source `0830edcb85bf46231cf0c5b1366027871a2b1229` now
supplies the installed `dev.harding.Kjerag` stable test build. Its SDK-built
executable SHA256 is `3cdfba2ada4f9de1a74e88e1e2851cdcfb12bd7250b66cf91684c448b8e19dd1`;
the installed OSTree commit is `d3ea2acdedfebd9c11b671f2cab1e74937f27bdbf6fa530ccd6f770c09875eb4`.
Runtime/permission metadata and executable bytes match the built package before
and after qualification. Origin remains `kjerag-origin`; the preceding exact
package is retained for recovery. No merge or public release was performed.

The native build passes50 UI checks. The actual installed bundle passes40
applicable X4 checks at612.078 and44 ONE X2 checks at212.512, including both
reported views, paused holds, backward seeks, the real scrubber, screenshots
and paired-file opening on ONE X2. The main sandbox harness skips audio controls
and injected import failures; the native run covers those paths. Separate
audio-enabled installed controls-wake checks pass at X4 time607.574 and ONE X2
time212.512, with48kHz stereo routed to the null sink. Steady reports reach30fps
with zero drops, starvation and audio underruns. Startup-inclusive reports are
15.66/18.51fps; worst reported video lateness is5.3/14.3ms, and worst audio
clock errors are62.4/69.0ms including startup. These1280x720 short runs are not
a full-resolution smoothness or240fps capacity gate.

Root inspected both installed reported-view captures. The X4 native/installed
viewport crops are not byte-identical:159,349 of2,150,400 RGB components differ,
mean absolute difference0.1018, maximum24. No cause or perceptual verdict is
inferred from that diagnostic; live moving-output acceptance remains the owner's.
Only the named8-bit sources are qualified. The current copier's higher-bit-depth
limitation remains open. Evidence is in `scratch/flatpak-delivery-0830edcb`,
`scratch/controls-wake/run.IaRZfBla` and `run.D6qSpzbD`, plus the live-correction
directory below. The player is available for review, not declared finished.

**Reduced correction reaches the live player, 2026-09-11:**
the owner answered "Yes" to the607 moving Studio/correction comparison. The
coordinator explicitly interpreted this as acceptable/no blotches, asking for
correction if the answer instead meant objectionable differences. This is scoped
to that movie, not acceptance of612, all footage, performance or a merge.

The working branch now uses the same half-linear RGB temporal field with
GPU-owned copies of the original lens planes and cloned immutable map/colour
bindings. The final source projection and residual addition share one surface
pass, within the native three-bind-group limit. No full-resolution panorama or
history is needed. All source updates, exact stamp pairing and seven-source
cadence remain. Seeking cancels unpublished old history without blocking the UI;
the last complete picture stays independently drawable. The previous full-field
Stream and its producers remain comparison oracles.

The initial integration passes the exact normalized plane-copy GPU test,
three correction shader checks,131 temporal checks (11 ignored) and all8 real
Scene tests across ONE X2 and X4, including seek/EOF/screenshot paths. All31
607 and31 612 live screenshots differ from the reviewed offline half-correction
by at most2 RGB codes, with mean absolute component differences0.0231/0.0239.
Root inspected the actual607/612 pixels. These are integration diagnostics,
not a new temporal-flicker verdict.

The final complete release-workspace suite passes1478 tests with51 ignored
across48 suites, with GPU checks required and both real source fixtures enabled.
Release workspace/all-target Clippy, formatting, source-lock and rename checks
pass. The initial native player builds needed promotion of RGB-history helpers
previously compiled only in tests; all such build failures remain in the evidence.

Native new/control/new runs at1280x720 now show a meaningful throughput change.
The control sustains17.29 source fps. The first new run averages28.68fps with
startup/catch-up and up to2.03s late; the repeat sustains29.9706fps over551
contiguous shown frames, with0 drops/starvation/audio underruns and12.9ms worst
reported lateness. The retained66.3/65.2ms maxima are `native-pump`'s
pre-prepare `shown` reporting intervals, not established visible pauses. That
probe records the prior installed frame before the same redraw's prepare can
install its newly offered frame. In the repeat, representative reported gaps
of65.159/63.611/61.795ms correspond to consecutive offered/install opportunities
only33.359/33.362/33.350ms apart, with the required temporal output already
complete. The largest supported pacing outlier is instead source18866 to18867:
the latter was ready in advance, a pump requested the next redraw1.745ms away,
and the next observed tick arrived14.689ms later, about12.945ms late. These
events locate delayed tick delivery, but neither this pre-prepare probe nor a
submit proves physical presentation timing or absence of a visible pause. The
run therefore remains unqualified for smooth native presentation. At2256x1504
sRGB, the existing completed-redraw Scene diagnostic measures407fps paused
but171fps active, active p9921.37ms and25.24 source fps. Its per-redraw queue
completion can underfeed processing and include
queue backlog, but it does not establish the required240fps active capacity.
Keep the target; do not present paused capacity as completion.

The standard full native UI harness now passes50 checks at the exact612.078
reported view. This covers pause/resume, backward seek, the real scrubber,
screenshots, fullscreen, both drop transports, import-failure recovery and
refusal surfaces. The scrubber check reports7 seconds for the whole interaction,
including deliberately paced pointer setup, key settling and held-picture
captures (at least5.44 seconds of fixed sleeps). It does not measure a7-second
seek. Passing its10-second guard is not a seek-latency verdict.
The one-file X4 input skips paired-file drop cases. GPU shader/twin checks pass.
Logs and captures are retained in `uitest-native-01.log` and
`ui-x4-native-01` under the evidence directory below. Installation was qualified
separately in the newer checkpoint above; no merge is claimed. The native app
snapshot tested here is
`e6245d4f2f63928ef37d769a4672165a05f46528e59946807eb3328b2ff934cb`.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-correction-live-01`;
native runs `run.fina8OU3`, `run.AkyjQCdT`, `run.wYANkADf` under
`scratch/controls-wake`. Native performance and actual active rendering capacity,
not another isolated shader gain or broader RE, decide subsequent delivery.

**Whole-pipeline alternative reaches a moving quality gate, 2026-09-10:**
an unselected test path keeps the exact high-resolution direct viewport and
adds a half-linear full-sphere temporal RGB residual. The residual is the
low filtered image minus its same-matrix unfiltered NV12 round trip, sampled
periodically in longitude and clamped at the poles. It retains all seven real
sources, automatic settings and source/output association, with no coefficient
EMA or interpolation between updates. Six real motion-pyramid levels replace
seven only in the explicit review constructor; changed angular block support,
motion decisions and retained fine detail/noise are unaccepted differences.

The existing actual-Scene review supplies identical source/map/color inputs to
the full-resolution reference and candidate. With three real look-ahead frames,
all31 reviewed full-reference frames at607 and all31 at612 are byte-identical
to the saved selected-player output. ONE X2's212.512 riser has radius0 throughout
this interval: the candidate's correction is exactly zero and preserves the
direct picture. An additional231.898333 sequence exercises four nonzero-radius
sources across the real ISO transition and returns to zero. All four34-source
reviews pass stamp/cadence/flush checks; this is not visual acceptance.

Two compositor GPU checks and131 temporal tests pass, with11 existing ignored
temporal tests. Release workspace/all-target Clippy passes. Lossless moving
reviews preserve their source pixels; only607's left panel is Studio, while612
and both X2 panels use the current full filter as the left control. The owner
has been asked to judge the607 movie. Root inspected the first607/612 frames,
not a human temporal-flicker verdict. No candidate source-path timing, concurrent
240fps capacity, retained-live-owner qualification, installation or merge is
claimed. The candidate avoids full-resolution body/history/filter work by
design; this quality harness deliberately runs both versions and is not a
performance benchmark. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-correction-field-01`.

**Delivery strategy reset after owner escalation, 2026-09-10:**
the owner rejected the15.1-to17.4fps gain below as an inadequate use of effort.
It remains a measured implementation improvement, not a usable-player milestone.
The coordinator stopped the sequence of isolated pass optimizations. The next
decision must establish an end-to-end path to29.970fps source playback and
4.17ms active view rendering, evaluating elimination or fundamental reorganization
of full-resolution panorama/history/search/filter work. Existing images remain
the oracle; no quality, cadence, or invented color-smoothing compromise is approved.
Do not start another local shader experiment merely because it can produce a
measurable gain. The current full-picture implementation is the reference for
that architectural decision, not a qualified installed deliverable.

**Grouped temporal filtering improves native throughput, 2026-09-10:**
the selected stream now fuses each2x2 Y/UV footprint in one MRT fragment,
sharing motion/luma lookups while preserving ordered per-component arithmetic
and normalized storage. Packed Y converts straight into the same full RGB
panorama without an unpack pass. All source frames, history, motion search,
color coefficients, filter settings, seek/EOF behavior and publication remain
unchanged. The old full-plane fragment filter remains an independent oracle.

Full-resolution X4 and ONE X2 comparisons are byte-exact. In warmed isolated
X4 tests, filter cost drops from roughly14ms to8–9ms; these host-wall intervals
exclude conversion, source preparation and presentation. Native new/old/new
runs at612.078 measure17.32/15.12/17.39 contiguous shown source fps after
startup, about15% faster. The new runs' maximum shown-frame gaps are91.09 and
81.27ms versus101.22ms for the control. This is still below29.970fps and does
not qualify smooth playback or240fps active capacity. The native test window
is1280x720, not2256x1504; its separate coarse controls-wake checks all pass.

The integrated build passes127 temporal tests with11 ignored and all8 Scene
checks, including seven real-input checks across both cameras. All31 frames
in each607/612 sequence match the retained vertex-cache checkpoint exactly.
This preserves those optimized review images, not new owner acceptance of them.
Release workspace/all-target Clippy and the full workspace tests pass
(1466 passed,51 ignored across48 suites). No installed replacement,
new Studio export, broader RE, changed cadence, color smoother or merge.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-packed-fragment-01`;
native runs`run.FIR0pZMC`, `run.JemV28Qw`, `run.LRxkQrKY` under
`scratch/controls-wake`. App SHA256
`d4539c8bd94437d7ddf29995462f85c9a24932e531fe20bd8617e6d08b43e1d3`.

**Native vertex reuse improves throughput, not smoothness, 2026-09-10:**
the compact source producer computes5151 native vertices once per map in a
164,832-byte GPU cache. The prepass shares the panorama encoder and existing
retirement. Full source resolution, refresh cadence, triangle/interpolation
law, colour and temporal settings are unchanged. The uncached paths remain
oracles. Warm isolated X4 preparation falls from24.18–24.42ms to16.42–16.84ms;
ONE X2 likewise improves. These intervals include cache allocation/preparation
but exclude history unpack and are not playback FPS.

Authenticated native new/old/new runs at the same612.078 view measure
16.28/14.57/16.12 contiguous shown source fps after startup, roughly11% faster.
Those cached runs' maximum shown-frame gaps are185.75/182.97ms versus93.84ms
for the control. A later unchanged cached control reaches15.66fps with an88.19ms
maximum gap, so the long hitch cannot yet be attributed to the cache itself.
Better average throughput is not a smoothness pass. These are1280x720 isolated native
window runs, not the2256x1504/4.17ms capacity gate. All three controls-wake
checks pass their separate coarse100ms pump bounds. No installed replacement,
owner acceptance of this new output, or merge.

Three shader/cache/native-binding checks,125 temporal tests and all8 real
X4/ONE X2 Scene checks pass. Workspace all-target release Clippy passes; the
full release-workspace suite passes1464 tests with50 ignored across48 suites.
The explicit byte-identity diagnostics fail with unchanged sparse one-code
differences: X4 Y37/UV18 components and ONE X2 Y7/UV4. No tolerance was relaxed.
Lossless moving reviews preserve current Scene pixels:607 against the existing
Studio export,612 against the preceding uncached compact player, not Studio.
The owner's earlier607 acceptance does not carry to these new movies.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-vertex-panorama-01`;
native runs`run.O7wQ2Ekc`, `run.n29XkKbQ`, `run.kANb6oZZ` under
`scratch/controls-wake`. Current app SHA256
`77671c01457d08a327152fe56ec1051ddabf346a3311047fc63f020e343a9fa7`.

The earlier cached maximum gap repeats at shown18529→18530 after the second scripted
controls transition. The uncached run's same-source gap is80.35ms. Source
preparation and filter/presentation callbacks all back up while pumps continue;
these wall intervals do not isolate GPU execution or identify the underlying
cause. A bounded workspace-reuse follow-up is now retired: new/control/new
native rates15.37/15.66/15.47fps show no gain. All62 saved607/612 frames stay
byte-identical, and the pending-map contamination regression passes, but those
correctness results do not justify retaining an unhelpful performance change.
The retained code is again the per-source cache. The follow-up angular-seed
shortcut is also retired: native candidate/control/candidate runs reach
15.78/15.43/14.91fps, with no dependable gain. Direct latitude initially selected
different rows on both cameras; retaining the old latitude and deriving only
longitude passes exhaustive seed equality and preserves all62 saved607/612
frames exactly. Correct output alone does not justify an unhelpful optimization.
Its code stays in git history, not the active renderer. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-angular-seed-01`.
The next architectural work must address the remaining full-panorama preparation
and filtering cost, not claim another scalar shortcut as a playback solution.
No new RE, accepted quality/cadence tradeoff, installed replacement or merge.

**Compact YUV integrated; native playback still too slow, 2026-09-10:**
the source worker now carries a sealed compact YUV panorama into the same
seven-source history. It no longer materializes the full-size RGB intermediate.
Source/map/color provenance, bounded retirement, source cadence and temporal
settings are unchanged; the previous RGB route remains a test oracle.

The first native attempt failed at startup: the prototype added a fourth bind
group, but the native renderer permits three. Reusing the exact bound source
Reframe for matrix/size removes that redundant group. Four focused checks,
including GPU pipeline creation on a three-group device, pass;125 temporal
tests pass with10 ignored, and all8 real X4/ONE X2 Scene checks pass. All31
607 and31 612 frames remain byte-identical across this binding correction,
not across the preceding RGB-to-compact representation change.

The corrected native controls-wake run passes its three100ms wake guards, but
steady playback is only14.2–15.0 source fps. Contiguous shown-frame transitions
independently measure14.37fps after startup. Its measured wake-window gaps are
55.79–58.06ms. This is neither smooth full-rate playback nor240fps capacity.
The installed app remains unchanged and this candidate is not qualified for
delivery. Evidence: `scratch/controls-wake/run.2mB3RXva`, app SHA256
`a2bd966891ab5e1a7c37cb37395393d8f95b5e57e7565093983d7151d12a580d`.

The preceding offscreen `view-rate` new/old/new runs all reached about13.6–13.7
source fps. That instrument waits for draw completion before pumping/admitting
again and can underfeed the source worker; these numbers cannot establish a
native throughput comparison. Its queue-wide completion registration also can
include worker submissions arriving after the draw. Native traces avoid that
admission gate and independently confirm the current slowdown. No authenticated
native c5 RGB baseline exists, so no compact native speedup is claimed. Existing
filtered traces do not time the final view pass or establish240fps capacity.

**Direct compact YUV preparation, isolated gain only, 2026-09-10:**
a new unselected producer avoids the disposable full-size RGBA body image.
On exact X4 source18344 at612.078133333s, repeated warm isolated preparation
intervals are about28–29ms versus39–44ms for the RGB-then-NV12 reference.
The compact measurement excludes later packed-Y unpack and UV history transfer;
this is not a player-FPS result. Full source sampling count, matrix, temporal
filter and source cadence remain unchanged.

The candidate explicitly preserves an RGB8 quantization boundary but is not
byte-identical. Reusing raster-interpolated coordinates reduces the larger Y
outlier population from2355 to552 of29,491,200 pixels, maximum19. The exact
reported projected still differs by at most6 codes; its framing/structure was
inspected, not accepted as moving-video parity. The GPU Y unpack is byte-exact
against its CPU reorder. Resident retirement/history integration was subsequently
completed and tested above. The installed app is unchanged. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-direct-nv12-01`.

**Block-sharing filter experiment retired, 2026-09-10:**
the unselected compute candidate read each block's motion/luma once, then wrote
packed YUV buffers and copied them to narrow textures. On the same initialized
7680x3840 six-reference input, warm encode/submit/completion intervals were
32.78–36.52ms for 64 lanes versus 18.13–23.01ms for the existing fragment filter.
A single 256-lane, one-pixel-per-lane follow-up also lost: 37.22–38.33ms versus
17.95–20.17ms. These are isolated host wall intervals, not actual playback or
GPU timestamps. No useful speedup was established, so both variants were removed
from active source, with their code and failed tests retained in scratch.

The exact comparison also found one-code output differences; the half-code
regression reproduces the normalized packing/attachment discrepancy. No tolerance
was relaxed and neither variant entered the live path. The player remains the
visually accepted 607 checkpoint below, still around 14fps. Next evaluate direct
body-to-YUV preparation to remove the large disposable RGB intermediate, without
changing source cadence or inventing a colour-update smoother.

**GPU-resident temporal architecture candidate, 2026-09-10:**
the live Stream now retains all seven motion-pyramid levels on the GPU and runs
coarse search, inter-level prediction, finest search and luma-dependent motion
packing without image readback, CPU search or motion/luma re-upload. Preparation
uses queue ordering instead of a CPU completion fence. Exact source history,
filter settings and completed-only publication remain; the final large GPU
submission and per-output completion wait still need scheduling work.

Coarse blocks now use immutable same-level predictors, omitting serially updated
neighbours and adaptive bad-block recovery. This is a disclosed independent-block
candidate, not a claimed Studio-exact algorithm. The owner accepted its new
607-second moving comparison below; other views and live performance remain open.
The GPU coarse-to-finest result matches its new CPU reference on even/odd grids
with one, three and six references. The integrated app compiles and 124 temporal
tests pass (10 ignored). The first motion-packing comparison used out-of-range
test thresholds, rejected by the preceding upload entry; its corrected fixture
passes without changing production limits. All eight real-camera Scene checks
pass, including live 607/612 captures. The paused Scene regression now permits
asynchronous audio-ring filling while requiring unchanged video/audio accounting,
nondecreasing queued audio and a fixed presentation clock.

The first 2256x1504 sRGB actual-Scene run authenticates automatic temporal output
but reaches only 14.06 distinct source frames/s, with 104.80 completed redraws/s,
42.95ms p99 redraw and 6.42s maximum display age. GPU residency has not established
a throughput gain or smooth playback. All 31 live 612 pictures differ from the
preceding candidate, so its acceptance cannot carry forward. New lossless moving
reviews contain 28 current 607 frames against the existing Studio export and 31
current 612 frames against the previous player output (not Studio). Their decoded
picture bodies match the input PPMs exactly. The owner responded "visual check
looks ok" to the new 607 comparison. This does not accept slow playback or the
separate 612 comparison. Full release-workspace verification passes 1456 tests,
47 ignored across 48 suites; workspace all-target Clippy passes. GPU filter work
reduction is next, not a new colour policy. No installation, merge or broader RE.
Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-resident-motion-01`.

**Native import recovery restored; performance unresolved, 2026-09-10:**
checkpoint UI qualification passed43 of44 checks. Its import-fault shim targeted
only the main thread, missing the selected `kjerag-stitch` import path. A corrected
probe with thread-specific injection receipts reproduced permanent file stoppage
after a0.4-second resource shortage. The source importer now retries the same
decoded pair before the stitch transaction, using the existing two-second limit.
Only classified resource exhaustion retries; invalid inputs and later pipeline
errors do not. No seam calculation, source cadence or colour history changes.

The corrected actual-player retest passes all9 fault-path checks, with separate
authenticated worker injections: transient recovery, sustained stop with raw
error, held picture, stopped clock/audio, one alert and successful reopening.
Five matching unit tests pass, including retry deadline oversleep; workspace
all-target Clippy, formatting, source-lock and rename checks pass. The complete
release-workspace rerun passes1440 tests with47 ignored across48 suites.
Native fault-path qualification is not
installed sandbox qualification or a performance result. Playback remains about
12–15fps on the30fps X4 clip. Source staging and shorter GPU jobs remain separate
unmeasured experiments; no installed replacement, owner playback acceptance,
merge, new colour smoother or broader RE. Evidence is in
`scratch/studio-seam-flicker-612-20260909-01/temporal-static-cache-01`, suffix10.

**CPU search cost reduced; GPU stalls remain, 2026-09-10:**
the selected x86-64 SAD validates both full16-row footprints once, then uses
unaligned SSE2 loads and two64-bit accumulators. The existing portable expression
already compiled to SIMD; the savings remove per-row checked multiplications,
bounds branches and horizontal reductions. Scalar/portable oracles remain,
including the non-x86 fallback. No search decision, source cadence or colour
law changes. The saved six-reference adapter oracle passes: coarse preparation
is22.32ms serial and3.65..4.77ms with six workers. During actual playback, mean
coarse preparation falls from27.20ms to13.01ms in the recorded runs.

The complete build passes1017 render tests (38 ignored), seven real-camera Scene
checks, workspace all-target Clippy and the full release-workspace test run
(1436 passed,47 ignored across48 suites, including documentation tests).
Both real-camera seeks now additionally
assert shared executor identity and distinct temporal history. All31 actual612
frames remain byte-identical. The first2256x1504 run reaches14.64 distinct source
fps, but a repeat is12.57fps: an overall playback gain is not yet established.
The respective p99 redraws are27.09/36.43ms, maximum38.65/58.82ms; completed redraw
rates170.10/134.83 per second still include repeated pictures. CPU work is cheaper,
but neither smooth source playback nor240fps active capacity is achieved. These
are internal implementation results, not an accepted visible tradeoff or a
qualified test build. GPU scheduling and the remaining serial dependencies are
next; no gradual colour update or new RE. Suffix07 receipts remain beside06 below.

**Bounded stitch/filter overlap, still not a usable player, 2026-09-10:**
the resident source/map/color worker now overlaps one successor with a separate
serial temporal executor. Admission reserves startup/steady/EOF outputs before
work enters the four-picture ready FIFO. Publication remains completed-only and
exact-source ordered. A shared capacity-one executor across seek restarts avoids
the initial prototype's unbounded per-seek threads/history retention. Its
blocking handoff is on the stitch worker, never the UI; errors remain per epoch.

The corrected build passes 1014 render tests (38 ignored), seven explicit real
X4/ONE X2 Scene checks and the shared-executor queue regression. All31 actual612
frames remain byte-identical to the preceding automatic integration. At
2256x1504 sRGB, the corrected actual-Scene capacity run advances13.08 distinct
source frames/s with307.82 completed redraws/s. The13fps result is still below
the29.970fps source; p95/p99 redraws are24.45/25.92ms, maximum34.68ms, and maximum
display age6.82s. Compared with the restored10.08fps serial run, source throughput
improves but stalls are more frequent (serial p95 was4.01ms). Neither the average
redraw count nor byte equality establishes usable playback or owner acceptance.
Workspace all-target Clippy, formatting, source-lock and rename checks pass
after simplifying the stopped-worker error handoff. The final build's full
release-workspace tests also pass, as recorded above. UI/installed qualification
remain pending. No installation, merge, invented
colour fade or broader RE. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-static-cache-01`, suffix06.

**Cache and loop experiments retired, 2026-09-10:**
the tested automatic integration is checkpointed at`8843aef7`. A full-body
triangle/weight cache used450MiB for X4, failed its packed/alpha/fusion bitwise
regression and changed actual612 output pixels. Its player run remained9.98fps
and source preparation32.02ms, so the cache was removed, not accepted as a
picture-quality tradeoff. A separate reference-count shader specialization
preserved all31 actual612 frames and all104 offline image/map/color artifacts.
It passed1010 render tests and7 real-camera Scene checks, but its player run
was only9.25fps despite a faster short isolated fusion measurement. It too was
removed: neither experiment established improved actual playback. Variable
clocks mean these short runs do not establish the cause of every timing change.

After removing both experiments, rebuilt app/view-rate binaries matched the
preceding checkpoint's SHA256 hashes exactly. Its immediate restored-player
recheck was10.08fps. Those binaries have since been replaced by the bounded
overlap build above, not offered as an installed test build.
Installed Flatpak, main and public releases are unchanged.
Exact rejected patches, test failures, comparisons and binary hashes remain in
`scratch/studio-seam-flicker-612-20260909-01/temporal-static-cache-01`.

The overlap implementation was prepared on`perf/temporal-stage-overlap` and is
now integrated and measured above. Broad RE remains frozen.

**Exact execution savings; playback still too slow, 2026-09-10:** the automatic
filter now keeps its finest image on the GPU, reading only the six smaller
pyramid levels for unchanged CPU coarse search. RGB conversion writes straight
into the resident history layer. This removes a 7.37 MB CPU readback and a
44.24 MB GPU copy per X4 source without changing source cadence. Finest search
binds one reference per dispatch; zero-confidence fusion skips unused neighbors;
the existing box sampler only fetches its fallback when that branch is selected.

The final branch build passes 1009 render tests, seven explicit real X4/ONE X2
Scene checks, the two saved Studio denoiser packets' existing one-code gate,
workspace all-target Clippy, formatting, source-lock and rename checks. All 31
actual 612-second view frames remain byte-identical to the preceding player
output. The first two GPU shortcuts reduced the short full-picture timed stage
mean from 31.67 to 29.10 ms; this excludes source preparation and CPU work.

Actual 2256x1504 sRGB playback capacity rises only from 8.83 to 10.16 distinct
video frames/s, with 581.8 completed redraws/s, 25.02 ms p99 redraw time and
7.97 s maximum display age. This still fails 29.970 fps video and the active
4.17 ms frame budget. Worker traces show roughly 32 ms for panorama completion,
conversion and pyramid preparation, 23 ms CPU coarse search and 25 ms final
GPU submission/completion, plus map preparation and encoding overhead. These
are wall intervals under concurrent drawing, not isolated GPU timestamps.
Next investigate caching static body-panorama mesh geometry and overlapping
independent processing stages. No new RE or invented color fade.

The installed Flatpak is unchanged. The owner was asked whether a temporarily
slow visual-test build is useful; no new accepted tradeoff is assumed. Native UI
and installed qualification, full workspace tests, deployment and owner player
acceptance remain undone. Evidence stays in
`scratch/studio-seam-flicker-612-20260909-01/temporal-usable-window-01`.

**Moving blotch preview accepted; automatic player activation, 2026-09-10:**
the owner watched `temporal-parallel-refine-01/review-607.mkv` and reported
"looks like this fixes the blotches", requesting Kjerag for testing. This is
acceptance of that specific offline moving result, not installed-player
behavior, the exact612 view or performance. No invented colour fade is added.

Supported live opening now constructs the temporal provider once from source
metadata/fps and automatically selects the full-picture filter. Unsupported
selectors explicitly keep the existing spatial path; bad ISO on a supported
source is an error. Stills remain spatial. Near-tail exact seeks prepare the
last seven real sources without showing pre-roll or moving the requested clock.
ONE X2 now uses floor-halved odd pyramid levels; its nonzero-radius source at
frame6953/time231.998433333 passes a real Scene transition/history-wrap test.
Radius-zero sources needed by later centers recover motion inputs once from
their exact retained NV12. Concurrent coarse reference preparation preserves
serial results; the saved fixture measures43.318 ms serial versus9.953..14.187 ms
concurrent, not a complete playback-rate result.

All seven automatic Scene checks pass, including both cameras' last-frame seeks.
The31-frame actual612 sequence remains byte-identical to the preceding player
integration. Before automatic activation,1006 render and84 media tests passed;
the native app and existing `view-rate` diagnostic now compile. The first
2256x1504 sRGB active-playback capacity run advances only8.826 distinct source
frames/s on the29.970 fps April clip. Its535.5 completed redraws/s repeats stale
pictures and does not meet the target: display lateness grows to8.51s and p99
redraw time is26.04ms. The owner was told this result; the installed app remains
unchanged. Existing full-picture GPU timestamps identify steady fusion at
14.73..16.95ms and finest search at10.67..13.38ms before source preparation and
CPU coarse search. Exact execution optimizations are being measured, not a
different color-update policy. No installation, merge or capacity claim yet. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-usable-window-01`.

**Filtered-picture publication reaches real Scene, 2026-09-10:** a test-selected
route now feeds exact decoded sources through the shared stitch worker, full
body panoramas and the seven-source temporal stream. Only a completed filtered
picture acknowledges the current delivery; upstream processing may advance
without moving presentation or audio. Four independent ready pictures bound the
queue. View redraw and screenshots consume the retained full panorama, and a
fresh seek root shares immutable inputs without old filter history.

Real X4 and ONE X2 checks pass for paused startup, ordered steps/history wrap,
view changes, renderer recreation and the reported exact seeks. Both cameras'
screenshots are nonblack and byte-identical across renderer recreation. The
shared panorama helper still matches all 28 original displayed-source pictures.
A one-source EOF landing explicitly reports unavailable output instead of
hanging. A complete seven-source EOF interval presents every output without
spinning a paused window on a full ready FIFO. All four real-source checks and
999 ordinary render tests pass (38 ignored). Workspace all-target Clippy,
formatting, source-lock and name checks also pass. The 31-frame X4 sequence at
612.078133..613.079133 now comes from the
actual filtered Scene/screenshot path, not the separate offline consumer.
Its lossless movie preserves every decoded RGB byte. This uses a fresh history
at source 18344, unlike the older offline sequence starting at 18341.

No invented colour fade is present. This route is still test-selected: CPU
coarse search/readback remains on the bounded worker, nonzero ONE X2 temporal
geometry is unsupported, short-tail seek pre-roll needs a usable policy, and
playback capacity is unqualified. The installed app is unchanged and the owner
has not accepted the moving filter candidate. The earlier 607 Studio comparison
remains available; no authenticated Studio movie covers 612.078. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-live-publication-01`.

**Automatic filter inputs and complete offline sequences, 2026-09-10:** the
source-group metadata field and source-time ISO provider now select the recovered
X4 Air/ONE X2 parameter tables without a fallback ISO. Runtime GPU motion accepts
the actual one-through-six references; history supplies native clipped phases
and a separate radius-zero current-frame copy. All retain exact source identity.
The existing actual-Scene diagnostic now emits startup0..3, steady3 and flush4..6
after seven real sources, rather than dropping its first/last three outputs.
The six-input boundary yields no filtered outputs, as read at the selected
backend boundary; this is not a claim about Studio's higher short-seek scheduler.

Automatic April runs produce all37/35 requested outputs at612/607. Their60
shared full-window images and504 controls are byte-identical to the previous
resident-panorama checkpoint; references, phase bits and copy counts pass.
All105 temporal tests (including native fixtures and both cameras' settings),
990 ordinary render tests and145 ordinary metadata tests pass, plus workspace
all-target Clippy, formatting, source-lock and name checks. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-auto-sequence-01`.

This closes automatic settings consumption and edge scheduling in the offline
consumer, not filtered-frame publication in the player. CPU coarse search and
explicit completion waits remain diagnostic. No colour fade, new Studio export,
installed change, speed qualification or owner flicker verdict. Next connect the
exact filtered-picture readiness to the existing due presentation/audio gate;
do not mistake earlier stitch-map completion for filtered-frame completion.

**Non-presenting panorama preparation verified, 2026-09-10:** explicit
`Player::prepare_ahead` can retain six contiguous successors while startup or
a seek landing stays paused, without advancing source/audio time or statistics.
A separate resident panorama owner uses the established bounded stitch worker
and exact source/map/fusion draw, committing computational history without
publishing a raw display future. Retirement capacity is reserved before work;
completed panorama textures do not retain old decoder surfaces.

The real-Scene regression checks both cameras at startup and after the reported
seeks: X4 sources 18344..18350 and ONE X2 sources 6369..6375, plus 0..6 on each.
All 28 source-stamped 256x128 panoramas match the unchanged displayed-source path
exactly. Current delivery, pause intent, clock and stats stay fixed during
preparation. A weak-owner check proves the first decoder source releases after
display advances while its retained panorama remains unchanged. All 80 ordinary
media tests and 980 ordinary render tests pass (3/37 ignored respectively);
workspace all-target Clippy, formatting, source-lock and name checks pass.

The bounded settings audit also closes April's actual `x4a_sp` selection through
its 3840/4000 low-rate arm: source group 0, 3840x3840, f32(30000/1001). It is not
the separate 7680 panorama-size arm or denoiser backend enum 8. Automatic settings
consumption and filtered-frame publication are still unfinished. This is an
explicit real-source integration test, not ordinary Scene selection, full-size
filter qualification, a smoothness result or an installed flicker fix. No new
Studio capture, gradual colour update, UI/Flatpak qualification or owner verdict.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/panorama-ingest-01`
and [configuration input note](research/studio-denoise-config-602.md).

**Temporal ISO lookup and startup/tail bindings verified, 2026-09-10:** the
render-side lookup now consumes the decoded ISO track with Studio's recovered
interpolation, zero-cache and invalid-value correction. Resident GPU history
can expose every center's clipped reference interval without padding. All 94
temporal checks pass on AMD, including the owner-file ISO lookups and 63
clipped-window GPU comparisons against rebuilt arrays. All 976 ordinary render
tests also pass (37 ignored); workspace all-target Clippy passes. No playback
policy or colour-update change is selected.

The remaining integration needs non-presenting preparation of six successors
at startup, separate from due-only filtered-frame publication; increasing the
current two-slot lookahead alone would stall. Recovered settings also show
ONE X2's ISO365 example has radius zero in `common`, not X4's radius three.
Automatic X4 Air selection still needs authenticated source classification:
the captured denoiser mode 8 must not be mistaken for selector group type 8.
No new Studio capture, moving review, speed claim, installation or flicker pass.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-input-window-01`
and [configuration input note](research/studio-denoise-config-602.md).

**Automatic denoiser ISO metadata verified, 2026-09-10:** normal capture opening
now reads the global ISO observations from binary trailer record 9. The native
producer, packed field, 40-item prefix and independent millisecond clock law are
traced in the pinned Studio worker. The production metadata loader agrees with
an independent reader on both real files: April X4 Air has ISO100 near 607/612,
while the ONE X2 example has ISO365 near 212.512. All 143 ordinary metadata tests
and its doctest pass, as does the opt-in two-file regression and workspace
all-target Clippy. This is not per-lens gain, a colour smoother or a selected
player filter. The temporal consumer's lookup/settings and source-window
integration remain separate work, and the measured filter is still too slow.
No new Studio capture, installed change or owner flicker verdict.
Evidence: [ISO input note](research/studio-denoise-iso-602.md).

**Resident GPU input for temporal integration verified, 2026-09-10:** the
body-image producer now consumes the player's exact resident source/map/fusion
carrier with existing draw-retirement ownership, without map readback/reupload.
Full panoramas match the old input exactly on X4 Air and ONE X2. Both X4 temporal
sequences preserve all 60 filtered frames and 504 controls; the ONE X2 control
preserves 49 artifacts. All 966 ordinary render tests and 83 temporal checks
pass, including abandoned-panorama source retention. Ordinary playback does
not yet select this input or the filter. The next correctness dependencies are
automatic ISO selection and source lookahead/window integration; the player
currently supplies two successors, but the filter needs three. No new capture,
gradual colour update, installed change or owner flicker verdict.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-resident-panorama-01`.

**Exact packed-gray motion reads verified, 2026-09-10:** one GPU packing pass
per arriving source replaces retained scalar gray texels with four-byte groups.
Search decisions and colour updates are unchanged. All 83 temporal tests pass;
both full Scene sequences preserve all 60 filtered pictures and 504 controls.
The short 612 refinement mean falls from 32.097 to 20.799 ms. Combined measured
motion/filter means are 66.672/63.785 ms at 612/607, versus 79.419/78.540 ms;
these exclude earlier panorama preparation and remain outside playback budgets.
Whole diagnostic runs take 8.59/8.29 seconds, including the added source packing.
No new visual result, owner acceptance, gradual update or installed change.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-packed-gray-01`.

**Duplicate-SAD reuse rejected, 2026-09-10:** a workgroup-local raw-cost cache
preserves all79 temporal tests and both98-artifact short comparisons, but makes
the actual612 finest kernel slower:47.632 ms mean versus33.250 ms for the
retained64-lane baseline. The experiment is archived at `c9f35ad6`; its shader
bookkeeping is removed, restoring the exact previous shader. All79 checks pass
again on the restored path. Two new full-plane
GPU regressions keep duplicate-candidate and ring-penalty handling explicit.
No new visual result, colour fade or installed change. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-sad-reuse-01`.

**Exact GPU workgroup optimisation verified, 2026-09-10:** a short actual-Scene
timestamp run locates the remaining dominant kernel in finest motion search,
not fusion:51.567 ms mean versus about4 ms for fusion. Eight8-lane teams replace
eight32-lane teams while retaining complete integer SADs and ordered decisions.
The selected64-lane layout averages33.245/33.888 ms in two short runs; a128-lane
candidate averages35.210/36.422 ms and is not retained as another active route.
All77 temporal tests pass, including complete CPU-oracle vector equality.
Uninstrumented612/607 Scene sequences preserve all60 filtered pictures and504
controls. GPU/upload/filter/project/completion means fall from90.236/88.817 to
68.443/66.819 ms. Combined measured motion/filter means79.419/78.540 ms still
exclude panorama preparation and remain outside playback budgets. The moving
reviews stay unchanged and unaccepted; no colour fade, installed change or
flicker-fix claim. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-gpu-cost-01`.

**Resident temporal history verified, 2026-09-10:** one source-stamped
seven-layer NV12 owner replaces per-output allocation and recopying of the
entire window. Steady outputs copy one arriving pair instead of seven, while
preserving center/reference identities, filter parameters and phase order.
All 77 temporal tests pass on AMD, including three ring wraps without host
waits and rejected-input/no-write checks. Both actual Scene sequences preserve
all 60 filtered frames and 504 controls exactly, so existing moving reviews
remain current and unaccepted. GPU/upload/filter/project/completion averages
90.236/88.817 ms at 612/607, versus 117.807/116.951 ms; the combined measured
motion/filter portion is 101.151/99.320 ms, still excluding panorama preparation
and far outside playback budgets. No gradual color update, new Studio export,
installed change or flicker verdict. The remaining GPU work, not source-window
copying or CPU coarse search, now dominates this diagnostic's measured portion.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-resident-history-01`.

**Exact CPU coarse-search optimization verified, 2026-09-10:** a portable
row-slice byte-SAD reduction preserves every search decision and avoids explicit
SIMD/unsafe code. Saved-input six-worker coarse preparation falls from a
63.8–96.2 ms range to 6.9–12.7 ms across repeated runs; all six predictor hashes
and sealed complete-search outputs remain exact. All 75 temporal tests pass.
On the actual 612/607 Scene sequences, coarse search averages 11.318/11.030 ms
instead of 72.736/79.017 ms. All 60 filtered pictures and 504 controls remain
byte-exact, so existing moving reviews are unchanged and still await acceptance.
The combined measured motion/filter portion is now 129.125/127.980 ms, still
excluding panorama preparation and far outside playback budgets. Its next
execution target was recurring seven-frame NV12 array allocation/copies;
the resident-history checkpoint above now removes that repetition without a
filter or gradual color-update change.
Evidence: `scratch/studio-seam-flicker-612-20260909-01/temporal-cpu-sad-01`.

**View-scissored temporal candidate verified, 2026-09-10:** full-sized targets
now permit conservative per-view fusion/conversion execution without changing
their coordinates, shader arithmetic, quantization or final projector. All
70 temporal tests pass on AMD, including full-versus-scissored projected pixels
at seam/pole/portrait views and a missing-chroma-halo negative control. Actual
612/607 Scene runs preserve all 60 filtered frames and 504 controls exactly,
including source associations and hashes. Existing review movies remain current;
no new movie or owner acceptance is inferred.

The diagnostic GPU/upload/filter/projection/completion region averages
120.124/117.782 ms versus the prior full-path 134.222/132.368 ms. This includes
host work and logging, not GPU-only time. CPU coarse preparation is unchanged;
the combined measured portion still averages 192.859/196.799 ms, excluding
earlier panorama preparation. This is a modest execution saving, not playback
readiness or a flicker fix. Full-size allocation/history cost remains, and the
partial filtered panorama is valid only for its exact prepared view. No gradual
color update, installed change, new Studio export or broader RE. Evidence:
`scratch/studio-seam-flicker-612-20260909-01/temporal-view-scissors-01`.

**Independent GPU refinement candidate rendered, 2026-09-10:** the changed
finest-level search matches its new readable CPU oracle on all 172,800 saved
vectors in repeated runs. Warm submit/completion/readback takes 31.1 to 42.9 ms,
versus about 159 ms for the retired serial-decision prototype. This changes
45,789 vectors relative to that serial reference and is not Studio-exact or an
accepted quality tradeoff. All 60 temporal tests pass on AMD, including direct
GPU refinement-to-motion packing with no intervening readback.

Actual Scene runs produce 31 filtered pictures at the exact 612.078 view and
29 at the earlier 607 comparison. All 504 unfiltered/map/alpha/color controls
and center/reference source associations remain exact. New lossless moving
reviews compare the 612 candidate with the earlier filter and the 607 candidate
with cached Studio footage; no new export or registration. Owner moving verdicts
remain absent. The measured motion/filter portion averages about 210 ms per
center, excluding panorama preparation and other diagnostic work; whole runs
take 13.20/12.59 seconds. This is still an offline candidate, not an installable
player implementation. Color updates and recovered fusion arithmetic are
unchanged. Next work must address CPU coarse search and full-panorama processing
cost without treating numerical agreement as a flicker fix. Evidence lives in
`scratch/studio-seam-flicker-612-20260909-01/temporal-parallel-refine-01`.

**Exact row-ordered GPU search rejected on speed, 2026-09-10:** a bounded
finest-level prototype matches all 172,800 sealed reference vectors in three
runs, including separate synthetic dependency and adaptive-search tests. It
takes about 159 ms for the finest level alone on the AMD GPU, before coarser
search and filtering. This is not a viable playback implementation. Do not
extend the serial-decision port or mistake numerical identity for delivery.
The rejected prototype is archived in branch history, not selected by the
player. Next evaluate independently parallel motion refinement against real
filtered pictures, explicitly disclosing changed spatial prediction. The
captured filter and color-update policy remain unchanged; owner moving
acceptance and an installable flicker fix are still absent.

**GPU temporal motion packing verified, 2026-09-10:** the next optional GPU
stage reproduces all2,764,800 signed lanes in the six native motion fields.
The six independent CPU searches can also run concurrently without changing
their vectors or reference order. All49 temporal checks pass on AMD. With
both routes and GPU pyramids enabled, the exact612.078 diagnostic preserves
all31 filtered pictures and259 controls byte-for-byte. Search averages177ms
per output and GPU/upload/fuse/readback89ms in this offline run, still far
outside playback budgets. No complete GPU search, automatic ISO/history
policy, installed change or flicker acceptance is claimed. Both delivered
moving comparisons remain unchanged; no gradual color update or new export.

**GPU temporal brightness preparation verified, 2026-09-10:** the half-size
brightness image and all subsequent pyramid levels can now be prepared by
GPU render passes, with no submission, wait or readback inside the primitive.
All 68,808,600 logical pixels across seven captured Studio pyramids match
exactly. The optional actual-Scene route also reproduces all 31 filtered
612-second pictures and 259 unfiltered/map/color controls byte-for-byte.
All 40 temporal tests pass on AMD, including native fixtures. Workspace
all-target Clippy and formatting pass. The diagnostic still reads these levels
for the unchanged CPU motion search; this is not a fast player implementation.
Both moving owner verdicts remain pending, and the delivered movies and
installed app are unchanged. No gradual color-update policy or new export.

**Existing-Studio temporal-filter comparison, 2026-09-10:** the unchanged
offline candidate now has a direct moving comparison with the retained 607-second
Studio export, without another export or registration fit. Thirty-five Scene
inputs 18208..18242 yield 29 complete centered outputs 18211..18239. Separate
Studio-versus-filtered and Studio-versus-unfiltered movies preserve every
picture pixel below their labels through lossless encoding; both decode above
source cadence. The existing Studio source labels remain index-derived, not
independently authenticated. This supplies the missing direct visual comparison,
not a flicker pass, 612-second Studio coverage or a player implementation. Both moving
owner verdicts remain pending; no gradual color update or installed change.
Receipts are in `scratch/studio-seam-flicker-612-20260909-01/temporal-studio-607-01`.

**First actual-Kjerag temporal-filter sequence, 2026-09-10:** an offline
candidate now runs the recovered seven-source filter over the owner's exact
612.078 view. Thirty-seven contiguous Scene sources produce31 centered outputs
18344..18374, with three real past and three real future inputs, not padded
history. The reference uses the captured ISO100 parameter regime and unchanged
per-source stitching/color corrections. A repeat without filtering reproduces
all259 ordinary/map/color and panorama-control artifacts byte-for-byte.
The earlier31-source control also reproduces155 saved baseline artifacts.

This is moving output for review, not a flicker fix or installed player path.
CPU search/packing and explicit GPU waits are intentionally offline. Native
RGB-to-NV12 details remain unclosed; the disclosed Kjerag round trip has its
own unfiltered comparison. The half-luma bridge has selected/static OpenCV
authority, not a same-input native pixel receipt. Rust motion search matches
all six combined-adapter outputs but retains the known native vector gap.
All34 temporal checks and921 render tests pass, with32 opt-in tests ignored in
the latter; the seven-source ONE X2 panorama control passes too. Workspace
all-target Clippy passes. Full workspace tests/UI harness were not run for
this offline candidate, and it is not being pushed or installed. No gradual
color update, new Studio export,612 Studio oracle or owner acceptance.

**Standalone motion search runs, 2026-09-10:** saved Studio inputs now drive
a pinned public-family CPU reference. Two native-read differences, per-block
global-predictor refresh and inclusive seed clipping, reduce differing vectors
from 22,169 to 4,167 of 172,800 without parameter tuning. Independent repeats
reproduce complete outputs; all output costs are exact SADs at in-bounds
landings. This is not a perceptual pass or a selected player path. The next
product gate is moving output from Kjerag's own stitched pixels, not eliminating
every numerical motion difference. Body-panorama raster and current RGB
sampling are identified; selected native RGB-to-NV12 conversion remains
unclosed and must not be filled from an unselected shader. Panorama preparation,
conversion control and source-stamped history integration remain. No new Studio
export, invented gradual color update, installed build change or 612 flicker fix.

**Motion producer inputs captured, 2026-09-10:** one completed, detached
CPU-only Studio capture now supplies all six raw/packed motion pairs, their
actual gray pyramids, confidence tables and luma grid. It confirms the selected
Analyse path, seven pyramid levels, zero overlap and half-resolution search;
the alternate Recalculate settings are not applicable. Raw motion is240x120
and is resampled into480x240 packed motion. Independent luma reconstruction
matches all115,200 bytes; confidence/resampling matches all2,764,800 packed
lanes; recursive pyramid reduction matches every coarser logical level in all
seven images. Readable Rust pyramid and motion-packing references now pass
the native comparisons too; all19 temporal tests pass with the existing GPU
fusion checks on AMD. These references are not selected in playback. Motion
search and player integration remain unfinished. The unchanged earlier607 project
exported63 frames; invocation6 is not a source-frame identity. No invented
gradual color update, installed-player change or612 flicker fix.

**GPU temporal-fusion primitive, 2026-09-10:** the recovered normalized
pixel fusion now runs on the AMD GPU with explicit image history, motion,
luma grids and parameters. Both saved native input packets pass a maximum
one-code output difference: 396,604 of 405,756 components are exact and
9,152 differ by one code. This is less numerically exact than the CPU
reference, not a perceptual acceptance or a player-performance result.
Synthetic checks cover reference ordering, separate Y/UV confidence, signed
chroma displacement, rejection and limits. Encoding adds no CPU wait.
The primitive is not selected by playback: motion production, prepared
panorama history and source-stamped scheduling remain unfinished. No invented
gradual color update, installed build change, new export or 612-second fix.

**Denoiser inputs captured, 2026-09-10:** the unchanged earlier607 project
now has same-call input/output receipts for sources18214/18215. Both selected
frames use six references around current, ISO100, effective noise/limit700/10,
full-range output and guided-UV off. The actual Y/UV limit tables are1/0.5.
These are observed values for this run, not constructor defaults or a policy
for every clip. Current/reference pixels, motion and luma grids are saved;
all four current-plane patches match independent pre-denoise CV reads exactly.
Managed texture snapshots are explicitly CPU-shadow receipts. The independent
Rust fuse reproduces405,723 of405,756 output component codes exactly, with
33 one-code differences across these two patches. This verifies a bounded
same-input calculation, not a player implementation or visible flicker pass.
The63-frame export finished, debugger detached and project
remained unchanged. No invented gradual color update, new612 oracle,
installation or merge. See the temporal research note for corrections and limits.

**Selected denoiser shader identified, 2026-09-10:** static tracing from the
captured Metal mode 8 selects the normalized two-plane `nap_fuse_y_N` /
`nap_fuse_uv_N` kernels. The previously found packed-C4 source is a different
variant: its weight-256/integer-round description must not become this path's
implementation. The selected kernels use floating motion-weighted pixel
fusion, independent U/V rejection and current-relative output limits. Neither
variant implies gradual lens-color coefficient updates. Native parameters,
motion inputs and same-input output verification remain required; there is
no new player build, export, 612-second oracle or owner-visible fix from this read.

**Selected pulse isolated to Studio's BlockDenois stage, 2026-09-09:** a
same-source intermediate capture separates the two post-stitch filters in the
unchanged earlier607 comparison. Defringe leaves both selected luma patches
byte-identical; green residual changes1.273177 to1.287654, not attenuation.
After BlockDenois it is0.392611, with direct luma1.222739 to0.423762. The
state1 callback path skips end-of-chain materialization. Runtime selects the
Metal denoiser and its seven-frame queue threshold; recovered kernel source
describes motion-aligned current/reference pixel fusion, not gradual lens-color
coefficients. Exact reference selection, ISO-derived parameters and optional
filter branches remain unclosed. This identifies a relevant output mechanism,
not a complete implementation or an owner-visible flicker pass.

All six final/middle/post ROI snapshots are associated and safely unlocked;
the63-frame diagnostic export finished and the debugger detached with the
project unchanged. New post pixels differ from the preceding capture, so the
analysis uses only this run's three stages; no cause for that repeat variation
is assigned. No player arithmetic, installation or merge changes. The owner's
"no gradual unless it matches studio" rule remains binding. The612.078 view
still lacks Studio coverage and a tested fix. Receipts and next unknowns are
in `studio-image-fusion-temporal-602.md`.

**Selected pulse attenuates in Studio's post-filter stage, 2026-09-09:** the
follow-up now authenticates both selected sources from final stitch output
through the image-chain input to AlgoFrameEnd and captures all four post-filter
Y/UV patches. At the unchanged earlier607 event, the shared-motion green
residual decreases from1.273177 to0.400292 codes before encoding; direct luma
decreases from1.222739 to0.420283. This locates attenuation inside the observed
Defringe/BlockDenois image chain for that event, not specifically in one filter
or in all owner-reported flicker. Encoder identity remains unclosed despite
observing through the finished63-frame export. The post-filter result does not
depend on that missing link. New612 Studio coverage remains absent.

A new opt-in GPU/reference regression covers31 saved X4 observations, four
admissions and27 bit-exact held GPU outputs; maximum reference discrepancy is
0.0000667572. All70 image-fusion checks pass with both camera fixtures; five
opt-in checks remain ignored in that filtered suite, with the new warm test run
separately and passing. This is implementation consistency, not flicker
acceptance. No production arithmetic or installed app changes. The owner has
rejected the proposed Kjerag-specific gradual color-correction update:
"no gradual unless it matches studio". No such policy is implemented. The
next relevant RE question is which observed post-filter behavior attenuates
the event and how it works, not choosing an invented color catch-up constant.

**Post-stitch filter chain observed, 2026-09-09:** the existing follow-up
export selects `MediaRender -> ImageAlgoNode`, whose instantiated internal
chain is `Defringe -> BlockDenois -> AlgoFrameEnd`. This is a concrete reason
not to attribute the earlier output/movie difference solely to compression;
it does not prove either filter changes the reported seam patch. Both selected
final-output Y/UV patches reproduce the prior capture byte-for-byte.

The corrected F-input/preencoder hooks now receive calls, but do not match the
selected final AVFrames. A supplemental observation authenticates source18215
at the image-chain input, then fails to match its post-filter output. A later
sample-address reuse is rejected by the AVFrame identity guard. No post-filter
pixels were captured, so this is an incomplete diagnostic, not a causal finding
or a fix. The debugger detached with balanced CoreVideo transactions, the
existing export finished63 frames, and the project hash remains unchanged.
The installed `f5be77cd` still fails the owner's612.078 test. No new production
policy, build, installation or merge follows from this run. The temporal note
records the exact receipts and the unresolved boundary.

**Studio output versus export boundary, 2026-09-09:** a bounded follow-up
captures two CPU-visible final-output patches at the earlier confirmed607
view, before and after native color update18215. Independent luma/chroma
matching supports correspondence to frames6/7 of the same new export, not a
timestamp-authenticated handoff: both presumed downstream writer hooks had
zero hits and final AVFrame PTS was unset. The observer detached safely;
the accepted project hash stayed unchanged.

At the previously selected event patch, a shared-motion green residual is
1.273 codes in Studio's captured output and0.405 in the exported movie using
identical NV12-to-RGB conversion, versus1.385 in the exact-native-coefficient
Kjerag diagnostic. Direct luma independently decreases1.223 to0.417. This
locates attenuation between CPU-visible output and the movie for one event;
it does not yet identify compression versus other export processing, establish
overall flicker parity, or cover the new612 report. The compressed oracle
remains the owner's visual target, but its lower residual is no longer valid
evidence by itself for a missing stitch-time interpolation rule. Do not add
another solver equation or invented smoother on that premise. The installed
`f5be77cd` build remains rejected and unchanged. No new candidate or merge is
selected. Receipts and remaining gaps are in the temporal research note.

**Owner rejects the installed coordinate candidate, 2026-09-09:** the verdict
on `f5be77cd` is "Yes still flicker", with a new exact April004 example at
612.078/yaw-80.71/pitch-48.46/fov95.45/lock1. The two arithmetic corrections
and passing functional checks do not resolve the visible defect. The retained
qualified renderer now captures sources18344..18374 through actual Scene at
this exact view, including no-color/no-flow/lens-only controls. This is a fresh
seek root, not yet a reproduction of the owner's uninterrupted history. A
current-versus-no-color moving diagnostic is retained for owner review.
No existing authenticated Studio movie covers612.078: the accepted earlier
April oracle ends at the index-derived source18270/time609.609. Do not reuse
its trace/view or imply it verifies this new interval. No new smoother,
Studio export, merge or further candidate is selected yet.

**Color-coordinate candidate installed for review, 2026-09-09:** source
`f5be77cd` is now the installed `dev.harding.Kjerag` test candidate. The exact
native release passes50 UI checks at the reported April607.574 view. The
installed Flatpak passes40 applicable X4 checks and44 ONE X2 checks, including
the established1153.452/212.512 views, backward seek, scrubber and paired-file
opening. Sandbox audio-control and preload-failure checks explicitly skip;
the native run covers those paths. Steady reports show30 fps and zero dropped
or starved frames on both cameras. This is functional qualification, not a
240 fps capacity pass, a perceptual smoothness verdict or flicker acceptance.

Installed executable SHA-256 starts `f89d46a252658578`; OSTree commit starts
`e0b140d99e9598ac`. The source/archive and installed executable/permissions
are checked before and after qualification. The later owner rejection is
recorded above; this checkpoint establishes functional qualification only.
No merge or release is selected.
The earlier statements below that the installed app is unchanged describe
their individual diagnostic checkpoints, not the current installation.

**Two color corrections implemented on the branch, 2026-09-09:** X4 output
coordinates now preserve the exact native ratio-texture centers instead of
recomputing the endpoint lattice. The shared GPU producer also now performs
the already-recovered periodic-edge join. Both real-camera GPU fixtures pass
without loosening tolerances; the previous sealed binary failed the ONE X2
fixture because the readable reference had the join and the GPU did not.
The X4 GPU test now requires a bit-exact opposite-lens permutation at every
node, including the previously excluded poles/meridians.

Separate coordinate-only and coordinate-plus-join actual-Scene captures cover
15 April sources. Packed geometry, alpha and the exact-native diagnostic
images/maps remain byte-identical. Ordinary color updates still occur at
18215/18222. The join changes coefficients outside this view but none of its
15 rendered pictures; the visible change here comes from texture centering.
The coordinate correction lowers the selected green residual at18215 from
0.543 to0.230 codes, but18222 retains a larger residual than Studio. These
are localized diagnostics, not a flicker pass. Installed build, owner verdict,
broader camera-view acceptance and rendering-capacity status are unchanged.
The full workspace gate passes with both cameras and required GPU fixtures:
1,282 passed,34 ignored; all-target workspace Clippy, formatting, name and
crate-source checks pass. A focused GPU test rejects missing joining and
round-to-even at the105.5-code boundary. A short candidate-left/Studio-right
movie is available in `april-color-coordinate-edge-01/review/` under the
comparison directory; it has no owner verdict yet.

An intervening apparent camera-basis mismatch was an analysis error: the
wrong trace row's center was used, and native panorama V was interpreted
upside down. The corrected pixel is(685,300); actual Kjerag view-matrix and
native packed-UV checks support the existing physical bridge. Native's own
coefficient update also predicts about one green code there, so a zero
same-source color delta is not a Studio-parity requirement. See the temporal
note for the invalidated receipt and the exact-run VS limitation.

**Color-texture coordinate discrepancy, 2026-09-09:** a test-only exact rebase
of Studio's published X4 textures identifies a texel-centering difference in
the camera-output conversion. The packed-map comparison supports the existing
physical camera rotation/lens ordering; recomputing the producer's endpoint
lattice is not equivalent to reflecting the consumed texture. The exact ratio permutation is opposite
lens `[99-r,(99-c)%200]`, whereas centered packed-map/alpha conversion uses
`[99-r,(100-c)%200]` (and complements alpha). At the previously localized
source18215 green pulse, the moving diagnostic decreases from 2.158 to 1.385
codes versus Studio's 0.420. This is partial output evidence, not a visible
fix, and other residuals remain. At that diagnostic stage no production
coordinate change had been selected; the branch implementation is recorded above.

The bound type11 Metal color, packed-UV and alpha textures at source18215
have also been read back and match their CPU uploads exactly. This closes
those four CPU-visible payloads, not GPU coherence or final output provenance;
the separate fisheye texture was not read. Substituting the captured native
alpha as a frozen diagnostic does not eliminate the first pulse. A new
same-source counterfactual holds pixels/geometry fixed while changing only
old/new ratios: that update adds 1.071 green codes with Kjerag alpha and 1.088
with native alpha at the selected bin. The native alpha is one source's map,
not an observed temporal history. The installed player and owner's latest
negative verdict remain unchanged. Details, failed attempts and receipts are
in `docs/research/studio-image-fusion-temporal-602.md`.

**Owner rejects the full-overlap flicker candidate, 2026-09-09:** on the new
candidate-left/Studio-right movie for source `365cedf6`, the owner reports
"Yep still flickers". Correcting the missing equations does not resolve the
visible defect. The native prepared-image match remains a calculation finding,
not flicker acceptance. Keep the installed candidate and evidence intact while
isolating the remaining changing color-map discrepancy against saved Studio
output. No new export, invented smoother, merge or rollback is selected.

The subsequent native read identifies the missing periodic-edge join between
the prepared images and ratio construction. Adding it to the readable
reference reduces the maximum same-native-input ratio discrepancy below
0.0000068 on all six saved maps. This is reference-only; the ordinary GPU
producer and installed app remain unchanged, and no flicker fix is claimed.
A 15-source actual-Scene diagnostic now replaces only draw coefficients with
the recorded native-band history, validated against every saved native map
before rebasing to the camera chart. The moving control, covering the 14
sources shared with the existing Studio movie, is preserved under
`april-native-input-control-01/review/` in the comparison directory. Its
owner verdict is negative: "both side have it. why cant you see it?" Both
ordinary and recorded-native-input Kjerag arms retain the flicker. It tests the native-like coefficient sequence,
including both the different input history and recovered edge join, rather
than declaring either one the cause from numeric agreement.
The owner approved first developing a temporal output check against the
existing human-labeled sequences: flickering Kjerag variants versus stable
Studio, color-off and fixed-first-color controls. Until that check actually
distinguishes the labels, neither closer native numbers nor still inspection
will be used to present another presumed improvement. No new export or
production smoothing policy is selected.

The saved-output check now locates coherent color pulses at every changing-map
event in the full-overlap and both short controls, with the fixed-color and
no-color sequences as same-frame controls. The ordering persists with
independent texture motion, unreliable-motion masking and lossless projection
of the existing Studio export. A single aggregate score is explicitly rejected
as a quality gate: moving-ground residuals can dominate it. The useful result
is event-localized output evidence, including native-input updates18215/18221
and ordinary updates18215/18222, not a fix or owner acceptance. Details and
limitations are in the temporal research note's owner-labeled output section.
The installed app remains the rejected full-overlap build. Next isolate why
those correction updates become visible in Kjerag but are lower in the
accepted Studio output; do not substitute more type2 solver-number agreement
for that result.

**Drawing-path control rejected, 2026-09-09:** changing only the test draw to
per-fragment map lookup, the captured type11 1x1 source footprint and explicit
float-map interpolation leaves coherent color-update pulses at every tested
event. The ordinary and recorded-native coefficient histories and all geometry
are unchanged; the disabled-control pictures reproduce the previous capture
byte-for-byte. At a previously localized native brightening, the signed green
residual is +2.158 codes before and +2.200 afterward. This is a negative output
diagnostic, not a new owner verdict or a complete type11 reproduction. The
test-only draw experiment is retired; no new build is installed or presented
as a fix. Evidence and limitations are in the temporal note's consumer-control
section. The remaining important evidence gap is explicit: the accepted April
panorama export has no captured per-frame type11 color-map history; the replayed
15-source history belongs to a later type2 diagnostic export. Next observe the
actual panorama consumer's source identities and bound color-map changes.

**Panorama color history observed, 2026-09-09:** the bounded follow-up now
captures both actual type11 CPU color-map uploads on every source 18208..18222
through Studio's original 360 export path. The active project matches the
accepted project's SHA-256 before and after; source-size/rate, Original
bitrate, codec and extra-processing switches are checked in the UI. All 30
RGB map payloads exactly match the previous type2 diagnostic's published maps
after BGR/RGB conversion, including updates at 18215/18221 and every hold.
Thus a different panorama coefficient history is not the explanation in this
prefix. The new output retains lower measured pulses at the previously
localized events, but has no new owner acceptance and is not byte-identical
to the earlier export. Actual GPU texture-content identity and the complete
mapping/blending boundary remain unproven; do not infer them from CPU uploads.
Next isolate the application of these same corrections to source pixels and
the lens blend. No new player build or temporal smoothing policy is selected.

**Missing color equations identified, 2026-09-09:** Studio measures robust
color bounds from rows 49/50 and columns 18..193, then admits equations over
the entire four-row, 212-column overlap. Kjerag incorrectly used the narrow
measurement domain for both. The corrected readable reference now reproduces
five of six saved native prepared images byte-for-byte; the sixth differs in
one byte by one code. The outer-stage result matches the native-prepared
substitution on all three observations. This confirms the cause of the large
inner calculation mismatch, not yet the owner's moving flicker. The GPU port
passes real X4/ONE X2 input checks and the actual Scene capture completes all
31 reported April sources with unchanged geometric maps and alpha. A new
candidate-left/Studio-right loop is ready in `april-full-overlap-01/review/`
under the existing comparison directory. No new Studio export was needed;
the installed candidate's rejection below still stands until a new owner
verdict. The full-overlap change is now installed for testing, not accepted
for merge.
The full workspace gate passes with both cameras and required GPU fixtures:
1,274 passed, 34 ignored; all-target workspace Clippy and formatting also pass.

**Full-overlap player delivery, 2026-09-09:** source `365cedf6` now backs the
usual installed `dev.harding.Kjerag`. The native release harness passes 50
checks at the exact April flicker view; installed-ID harnesses pass 40 X4 and
44 ONE X2 checks, all with zero failures. Playback, reported views, backward
and late scrubber seeks, and ONE X2's four paired-file arrival paths pass.
The installed tests retain the prior audio-unavailable and import-injection
skips; X4's paired-file checks do not apply. These are functional checks,
not audio, performance, or flicker acceptance. The package/source/executable
identities verify before and after testing, and the previous `9e193956`
bundle remains available for rollback. Evidence is in
`scratch/flatpak-delivery-365cedf6/` and the comparison directory's
`april-full-overlap-01/native-qualification/`. The owner subsequently rejected
this moving A/B as a flicker fix, as recorded above. No merge or release is selected.

**Owner rejects the color-boundary candidate, 2026-09-09:** on the candidate's
moving candidate-left/Studio-right comparison, the owner reports "The left
still flickers same way". The input/admission improvements do not fix the
reported defect. Preserve this negative verdict alongside the authenticated
lens/chart finding; neither full test passes nor closer update counts
substitute for the owner's moving result. The installed `9e193956` build is
not accepted for merge. Next compare the already captured native bands and
coefficient sequence to distinguish remaining calculation/output differences,
without another Studio export or an invented temporal smoother.
The first split replay substitutes Studio's saved inner-solver output into
our unchanged outer ratio stages: maximum coefficient differences fall from
0.024/0.014 to about 0.0012/0.0016 on the first observation. Most of this
measured discrepancy therefore precedes ratio construction, although its
relevance to visible flicker is not yet proven. A test-only native node-order
permutation produces identical prepared bytes and is not a fix. No further
product change, install or export was made; the new tests and native boundary
checks are recorded in `studio-image-fusion-temporal-602.md`.
The next saved-input controls also leave every prepared output byte unchanged:
explicit normal-matrix multiplication, native binary32 centering, native
dot-product reduction order, and their combination. These measured arithmetic
differences are not a useful fix for this discrepancy. Native tolerance,
initialization and stopping conditions match the reference; the remaining
check is the actual input/evidence construction. Playback remains unchanged.

**Owner feedback and seam reports, 2026-09-08:** chromatic calibration
"looks really good"; pausing "seems ok on initial glance". Preserve that color
result and treat pause feedback as preliminary, not blanket smoothness or
branch acceptance. The owner confirms that the April X4 Air t607.574 Scene
clip reproduces the reported moving seam, described as "almost like blotting".
The 61-source capture covers sources 18209..18269. All 61 alpha maps are
identical. No-color, no-flow and solo-lens diagnostic controls complete for the
first 31 sources without changing the original source maps or screenshots.
They are discriminatory views, not fixes or a cause finding. The owner requires
a direct Studio/Kjerag A/B at this exact view before assuming the products
differ or choosing a fix. Actual Studio 6.0.2 exports now cover all three
reported intervals; the direct visual-review set is described below.

**Owner confirms the April Studio difference, 2026-09-08:** after reviewing
the direct side-by-side, the owner reports that Kjerag looks worse and has
blotting along the seam that Studio does not. He clarifies "flickering blotting",
endorses this moving A/B as the acceptance oracle, and doubts stills could
distinguish the products. This closes the human A/B confirmation for April,
not its cause or the July/August reports. Preserve the accepted-looking color
correction; still-image similarity cannot close this temporal defect.
The owner also closed Kjerag and handed control back, removing the running-
player obstruction to the pending isolated playback tests. No fix acceptance
or permission to merge is inferred.

A separate broader-smoothing report is registered on the July X4 Air footage
at t1616.348, yaw109.01, pitch-1.50, fov66.70, horizon locked. Its initial
31-source Scene capture completes for sources 48442..48472; owner confirmation
of that new capture is pending. Do not assume the April and July reports share
a cause or that either differs from Studio before direct matched A/B evidence.
A third exact report is registered on August X4 Air footage at t19.686,
yaw-142.64, pitch-19.92, fov114.41, horizon locked; capture and owner
confirmation were pending initially. Its 31-source Scene capture now completes
for sources 590..620; owner confirmation remains pending. None of these
fresh-seek sequences establishes uninterrupted-history behavior. No production
stitch/color/cadence change or new installed build is selected. Scene evidence
is `scratch/x4-ground-seam-20260908-01/` and the August capture below.

**Direct three-view Studio A/B, 2026-09-08:** actual source-size/source-rate
Studio exports and muted side-by-side movies now cover April, July and August.
Kjerag is left, Studio right. Initial Direction Lock ON exports align their
anchors but drift to different viewing directions through July/August; those
movies are not valid moving-seam evidence. Corrected exports use Direction
Lock OFF with stabilization, AI stitching and Image Fusion still ON. Complete
project diffs change only Direction Lock plus source-identical hard-link aliases,
generated IDs/cache paths and timestamps. No camera/stitch parameter was fitted.

Each presentation uses one fixed rigid rotation of the entire Studio panorama,
unchanged requested FOV, and Kjerag's screen projection, including August's
wide-angle projection. Root inspected April samples 0/15/30/45/60 and July/
August 0/15/30. The large viewing drift is absent in these corrected samples;
residual framing and texture differences remain. This is visual-review evidence,
not a seam cause finding or parity verdict. Saved-trim arithmetic supplies the
source/output association, which is not independently authenticated; neighboring
candidates do not prove a one-frame uncertainty bound. Sampling/encoding
differences are not isolated, and fitting bands are not proven seam-free.
Private movies, hashes, complete project diffs and provenance are retained in
`scratch/studio-seam-ab-20260908-01/`, using its `*-direction-off-ab/` set.
No production behavior, installed app or native player changed.

**April localization and sampling control, 2026-09-08:** the saved-alpha
0.5 trace crosses the reported mottled ground corridor and moves across the
locked view. Original Scene pixels are preserved outside the diagnostic red
marks; ray-replay RGB was rejected as an exact image replacement. In the
inspected controls, the corridor remains with public flow or color disabled.
One higher-resolution Scene capture at each of sources 18209/18224/18239,
reduced by a fixed 2x2 area average, reduces fine speckling but does not remove
the broader mottled pattern. The 31-source sampling test passes and all 155
original image/map/alpha/ratio artifacts remain byte-identical. Sampling
contributes to texture appearance; these three stills do not establish the
moving defect's cause, a Studio difference isolated to stitching, or a fix.
The new `sampling` mode is test-only. No production filter or temporal change
is selected. Artifacts are the comparison directory's `april-local-review-02/`,
`april-sampling-01/` and `april-sampling-review-01/`.

The follow-up `sampling-sequence` control covers all 61 sources. Each exact
installed Scene is also captured at 5120x2880 and reduced to 1280x720 by a
fixed 4x4 coded-RGBA area mean. All 305 original artifacts remain byte-identical;
one retained high-resolution anchor independently reproduces the reduction and
raw RGBA hash. This is a test-only sampling control, not a Studio-derived filter
or a proposed live 16x rendering cost. Muted current/control and control/Studio
movies are in `april-sampling-sequence-02/` under the comparison directory.
The owner watched current/control and reports "Those look the same". This
control therefore provides no useful visible improvement and is not selected
as a fix. That negative result does not rule out every sampling mechanism or
prove a temporal-solver cause. The original accepted moving A/B remains the
oracle. No production filter, stitch history or app changed.

**April residual alignment and drawing candidate, 2026-09-08:** source-owned
geometry isolation preserves all 155 baseline artifacts and shows that the
public correction improves a larger parent mismatch but leaves a local
roughly one-output-pixel residual. Best-assessed descent, unquantized temporal
median and fresh-only warm blending were tested separately. None supplied a
convincing flicker fix; their test hooks were removed and their exact source,
executables and captures retained. Removing histogram rounding improved some
alignment without removing the localized temporal signatures. Bypassing the
retained blend worsened the sampled alignment. These are bounded controls,
not evidence that all temporal mechanisms are excluded.

A follow-up 31-source capture preserves all baseline artifacts while exposing
the first source's existing blurred-belt and L1 probes. Its public corrections
substantially align the actual solver images. Separately, reconstructing the
displayed triangle from saved UVs identifies downstream compact-map/mesh
resampling error: at two trace locations, a first-order prediction is about
(+0.74,+0.67) and (+0.83,+0.74) output pixels. This combines compact-map/base
interpolation with mesh interpolation, not a complete or mesh-only cause
finding. No clip-fitted output correction is used.

The new **test-only** drawing candidate connects the existing periodic
200-column, 100-pole-inclusive-row map with 200x99 cells, instead of resampling
it through 100x50 cells. Both hardware-mesh and curved-ray shader topology and
draw counts change together; producer, source cadence and history do not.
Its April 31-source Scene run passes with all 124 packed-map/alpha/fusion
artifacts byte-identical. Three lens-isolation anchors preserve all 155
candidate artifacts. Local zero-offset lens correlation improves at eight of
nine existing trace patches, but residual offsets remain. Root inspected
actual candidate/current and candidate/Studio pixels. Shader validation and
full workspace/all-target Clippy passed. Subsequent CPU and GPU topology checks
passed, including poles, periodic wrap, the August curved view and a ball view;
the real Scene also completed 61 ONE X2 and 31 August sources. These checks did
not establish temporal quality or drawing capacity.

**Owner rejects the drawing candidate, 2026-09-08:** after watching the new
candidate/Studio movie, the owner reports "left has same issue". The local
alignment gain is therefore not a fix for the reported flickering blotting.
The test-only mesh hook and its generalized CPU topology were removed; exact
candidate source, executable and captures remain in scratch. The original
native-grid regression retains its added curved/ball and coverage-hole checks.
Further candidate performance and July qualification were stopped. No installed
app or production policy changed. Private evidence is under
`scratch/studio-seam-ab-20260908-01/`, especially `april-solver-inputs-01/`,
`april-map-curvature.json`, `april-map-node-mesh-01/`, and
`april-map-node-mesh-02/` and `mesh-qualification-01/`.

The next discriminator uses the existing source-matched moving component
controls, not another alignment score or new Studio export. The current/no-color
movie in `april-color-motion-control-01/` changes only final photometric
correction at the same 31 sources. On 2026-09-09 the owner reports "no flicker
just line" and explicitly confirms this means the right-hand no-color view is
stable while current Kjerag on the left still flickers. This establishes a
dependence on photometric correction for the reported flickering, not the cause
of the remaining line or a reason to remove the accepted-looking calibration.
It does not yet isolate coefficient timing from a static spatial-field effect.
Earlier static
inspection did not exclude either color or public flow as a temporal cause.
The existing public-field removal is also encoded in
`april-flow-motion-control-01/`, ready if the color verdict calls for it; it has
not yet been requested as another owner test. A provenance recheck confirms
the solo-lens controls retain both public flow and color, so they cannot be
described as raw-source/sampling-only evidence. No further candidate is selected.

The saved April ratio maps have exact multi-frame holds followed by broad-field
changes at the previously located color pulses. The producer applies admitted
updates immediately and has no temporal output transition; GPU/CPU review finds
no new arithmetic or ownership mismatch. This is a concrete suspect mechanism,
not a proven complete cause. `april-color-ratio-audit-01/` records the raw-map
audit. A new **test-only** `fixed-color` Scene review retains the first source's
ratio pair for diagnostic draws while every source, geometry, alpha and ordinary
color producer continues unchanged. Its 31-source capture passes, all 155
ordinary artifacts match the original baseline, and its first fixed draw is an
exact same-consumer null. The moving comparison is in `april-fixed-color-01/`;
the owner confirms "yeah right free of flicker" on 2026-09-09. Holding only the
coefficients removes the reported flicker in this comparison. This localizes
the dependency to changing color correction, not all possible defects or
Studio's own update mechanism. A permanent freeze is not selected playback
behavior.

The coordinator proposed a Kjerag-specific 100 ms source-time publication
low-pass, then the owner requested actual Studio behavior before considering
that workaround. The unbuilt/unqualified draft was removed from the active
code and archived in `paused-color-smoother-01/`; it was never installed or
accepted. The replacement investigation is bounded to the existing Studio
color-update boundary. Initial static inspection of the hash-pinned Mac 6.0.2
worker finds a separate optional `GetOutputColorMaps` recurrence using 0.64
retained plus 0.36 current maps on admitted updates, with immediate first
initialization and holds on skipped updates. This is distinct from the older
Windows reference's inner metric recurrence. The subsequent caller/default
check **rules this out as an automatic X4 Air behavior**: constructor and reset
clear the smoothing flag, Init enables it only for camera type 26 (Antigravity
A1), and the same native name table maps X4 Air to 23 and ONE X2 to 10. The
ordinary StitchFusion setup does not call the exported override setter, which
has no direct calls in this worker. An unrelated external API client's override
is outside that bounded static conclusion. Do not port this optional recurrence
as the missing X4 Air fix. Evidence is `mac-color-update-01/` under the same
comparison directory. No new Studio export has been requested.

The bounded Mac inner-state check finds the same metric/budget/warm-solve
structure, plus a fused metric-rounding difference at seven integer input
pairs; occurrence or relevance in the April clip is unmeasured. The inspected
export handoff queues or reuses one correction pair without blending successive
pairs. No correction policy is selected from these findings. The next necessary
observation is the reported interval's native color-input/admission/metric/map
sequence, followed by replay of those inputs through the readable reference.
This is not a request for another video oracle export or renewed output-frame
alignment work. `docs/research/studio-image-fusion-temporal-602.md` records the
exact boundaries. The Mac availability check was read-only; no new capture,
production change or installed build was made.

**First native color-input prefix, 2026-09-09:** the pinned Mac worker now
provides 15 complete observations at trim sources 18208..18222. Native color
updates occur only at ordinals 0/7/13, with exact holds between them and the
optional smoother confirmed off. Kjerag's readable CPU reference, given those
same native bands, reproduces all 15 admission decisions and raw/retained
metrics. Ratio values still differ, so this is not a parity or fix claim.
The evidence points the excess update frequency toward differing sampled
inputs. Next is a real-Scene band/coordinate comparison, controlling the
earlier Kjerag capture's one-source-later cold start.
The diagnostic used 1080p and an observed type-2 consumer, not the existing
7680x3840/type-11 movie oracle; that path distinction remains explicit. The
requested 63-observation trace was manually closed at 15 fully published rows
after long export-progress gaps, with debugger hooks removed and detached.
`studio-image-fusion-temporal-602.md` records the initial timestamp-guard
recovery, native states and CPU-replay differences. No production policy or
installed app changed; three replay-validation tests and the prefix replay pass.

**X4 color-boundary mismatch localized, 2026-09-09:** the same-cold-start Scene
capture supplies 31 sources. The first 15 native observations update 3 times;
Kjerag's update 14 times, and the readable CPU gate reproduces both from their
respective bands. The missing conversion is the X4 native lens exchange and
fixed sphere `Ry(pi)` datum already present in `x4_model6_static`, but absent
from photometric input/output. Native source pixels and a bounded Mac static
check confirm the association; a guessed half-turn alone was rejected.
A working candidate carries that camera conversion through both color
boundaries, preserving ONE X2, the solver and the update policy. It adds no
GPU pass or history smoother. It is not yet visually qualified or installed.
Details, including the first failed diagnostic and the earlier-cold-start
geometry comparison failure, are in `studio-image-fusion-temporal-602.md`.

**Candidate ready for moving review:** both 31-source April Scene passes keep
their matching baseline's packed maps/alpha byte-identical. New native-start
color admission is 0/7/14, versus Studio 0/7/13; remaining input differences
are not declared visually harmless. All 61 ONE X2 pictures/maps/alpha/ratios
remain byte-identical (305 artifacts). Focused checks pass 58 tests and both
cameras' actual cold/warm Scene checks pass; all-target workspace Clippy and
static gates pass. Full offline workspace tests now pass 1,269 tests, zero
failures, 31 ignored, with both real camera inputs and the GPU required.
Three more 31-source captures (April riser, July and August reported views)
preserve all packed maps/alpha against their same-start baselines. The frozen
native release player passes 47 UI checks, zero failures; this is functional,
not performance or flicker qualification. Bundle qualification remains pending
and packaging preflight found insufficient disk space (1.1 GiB free).
The owner has been sent the muted candidate-left/Studio-right moving A/B from
`april-camera-fusion-01/review/`, using the existing Studio reference. Await
that flicker verdict before declaring success or selecting further changes.
**Installed for owner testing:** source checkpoint `9e193956` now has an
offline SDK-built local Flatpak. Installed-by-ID UI runs pass 40 X4 checks and
44 ONE X2 checks, zero failures, including both exact starting views, backward
and late scrubber seeking, real playback and the ONE X2 paired-file paths.
The isolated sessions report no sound device, so their volume-popup checks
skip; the injected stuck-import check cannot run inside the sandbox. These
are functional checks, not performance/audio or flicker acceptance. Package
and complete receipts are in `scratch/flatpak-delivery-9e193956/`; installed
executable SHA starts `68dba116`. An old 11 GiB compiler dependency cache was
removed to make room, with source, saved binaries, footage and evidence kept.
The previous verified Flatpak bundle is retained for rollback. No merge or
release occurred; await the owner's moving-video verdict before more changes.

**April coverage and blend-weight controls, 2026-09-08:** dense CPU reference
rasters at sources 18209/18224/18239 contain no out-of-range lens coordinates
with nonzero blend weight. Invalid regions in the solo-lens pictures therefore
do not establish edge-clamping leakage in these normal blends. This does not
check physical image-circle support or the GPU filter footprint; no coverage
clamp is selected.

A retained Studio X4 alpha upload from t1152.417933 contains 2,004 fractional
nodes, versus 2,392 in Kjerag's current map. A fixed-convention coordinate/lens
conversion was tested as an explicitly cross-time shape proxy, not as Studio's
actual alpha at t607.574 or a proven static producer law. Three same-consumer
baseline/proxy pairs preserve source, geometry and saved color ratios; all
seven alpha-independent image arms remain byte-identical, as do the earlier
baseline replays. Root and independent visual review find the broader mottled
corridor and apparent row bending/merging still present. The proxy does not
provide a convincing fix or isolate the direct Studio/Kjerag difference.
Native chart orientation is convention-derived, not independently landmark-
authenticated; native texture contents and cross-time lifecycle remain unproven.
No new export, production blend rule, filter, temporal policy or installed build
is selected. Evidence: `april-coverage-01/` and `april-alpha-proxy-01/` under
the existing private comparison directory. Owner review of the direct A/Bs
remains the quality gate, not these diagnostic substitutions.

The follow-up performance audit identifies an unqualified view family: August's
114.41-degree view uses the curved fragment-ray consumer, while the installed
240-capacity measurements below use narrower hardware-mesh views. This is a
coverage gap, not an established bottleneck. The first installed reported-view
idle attempt refused while desktop Kjerag was open. After the owner closed it,
the unchanged installed app completed all three checks at 2256x1504/60 Hz.
Observed source cadence was only 14.03/s April, 14.18/s July and 7.89/s August;
all source advances were consecutive. These are not 240-capacity tests. A
separate no-Kjerag host audit found cosmic-comp using 41–57% graphics-engine
time. That is a substantial confounder, not proof of the slowdown's cause or
a code regression. App identity is unchanged; historical runtime deployment
hashes were not retained. No desktop processes/settings were changed and no
COSMIC fix is selected. Receipts: `scratch/reported-view-playback-20260908-02/`.
CPU-only checks before this sampling-sequence addition passed: full
workspace/all-target Clippy, 135 metadata tests including the doc test, and the
frozen render test's fusion-shader validation. Formatting and name/source-list
checks pass; no new full workspace runtime or installed UI qualification is
claimed. First attempt: `scratch/reported-view-playback-20260908-01/`.

**May color-reading localization, 2026-09-08:** exact pixel membership now
locates the previously reported neighboring-bin peaks. At source 18916, bin 77
has 447 eligible pixels confined to x1168..1205/y700..719; bin 78 has 2,115
pixels spread across x1133..1279/y437..719. Root viewed native ON/neutral images,
exact red/cyan overlays and identically amplified crops for sources 18916,
18921, 18933 and 18934. The selection includes separated right-edge patches;
the 10.649% statistic is not a direct adjacent-output-pixel discontinuity or
a localized stripe through the central field. This does not establish color
acceptance, explain every roughness component, or dismiss the owner's gate.
Accounting by exact empty-row gaps explains the mixed averages: source 18916's
bin 78 combines 1,144 pixels in negative-mean-lift upper patches and 971 pixels
in a +2.393-code bottom patch, yielding only +0.071 codes overall. At source
18921 the two bottom patches receive +2.579/+2.546 codes, while one bin also
contains a separate -2.255-code upper patch. The large bin-to-bin statistic
therefore cannot be treated as the correction difference between those nearby
bottom patches. No smoothing change is justified by that reading alone.

The optional observer preserves the original measurement arithmetic. The May
31-source rerun leaves all 281 existing artifacts, including metric TSVs,
byte-identical. Every retained population reconciles exactly; independent
RGB-sum regrouping differs by at most 1.542e-12 neutral luma codes and
3.997e-14 applied codes. Four CPU tests and full workspace gates pass:
1,260 tests, zero failures, 30 ignored, formatting, all-target Clippy and
name/source-list checks. Frozen test executable is
`cc7d62f4057ab4b31d891216012b5e53ee7c43179e4a2b06441c93b2618bf7c2`.
Evidence is `scratch/photometric-interior-20260908-01/selection-01/` and
`localization-01/`. The installed player remains `31ea781d`; no color correction,
stitch cadence, new quality threshold or live carried-correction policy changed.

**Current-GPU field-interior readings, 2026-09-08:** the registered dark-ground
coherence check now consumes the exact GPU ON/neutral pairs and the prepared
Reframe before Scene advances. Its existing arithmetic is shared with `colour`
in a diagnostic-only render module; no legacy estimator, rebuilt view geometry,
new smoothing or acceptance threshold is introduced. A source-level extraction
audit preserves operation order and constants; it is not an executed bitwise
comparison with the old function. Every retained bin, summary and original
null/0.5/2-code control is recorded for all 124 hard-view sources.

All views have sufficient coverage under the existing rule. Automatic roughness
ranges are 0.815..1.734% May dark soil, 0.048..1.013% April sun 1,
0.298..1.843% April sun 2, and 0.541..2.253% August glare. Nulls are exactly zero.
The 0.5-code plant ranges from 1.911..2.017% on May to only 0.013..0.061% on
the narrower April sun-1 view, so sensitivity differs substantially by view.
These are measured concerns, not a quality pass or proof of visible streaks.
May's largest reported neighbour jump is 10.649% at source 18916; its largest
roughness is at 18933. Root inspected those images as well as first/middle/last,
with equally amplified dark-ground pictures and the existing signed-cell view.
The correction appears broadly structured in those samples; metric localization
and owner judgment remain necessary before accepting it.

All 876 previous hard-view artifacts remain byte-identical. Three synthetic
diagnostic tests and the 124-source live review pass; full workspace gates pass
1,259 tests, zero failures, 30 ignored, formatting, all-target Clippy and
name/source-list checks. The frozen test executable is
`99993fce4271d38fedf0106481f37ebca50842b461399a5e36e0efaca1aebe97`.
Evidence is `scratch/photometric-interior-20260908-01/`. Playback calculations,
source cadence and the installed `31ea781d` build remain unchanged. No merge,
new Studio export, parity verdict or owner acceptance.

**Registered hard-view color coverage, 2026-09-08:** the current GPU producer
now has 31-source ON/neutral sequences at each of the four registered dark-soil,
sun-facing and glare-heavy owner views. All 124 sources retain one capture per
sequence, pass exact source/map/ratio ownership checks, and match ordinary Scene
output within one 8-bit code/channel. The first wide-angle attempt correctly
failed that null: the test helper forced a flat-perspective mesh above 110
degrees. The helper now follows the exact prepared Reframe's projection choice,
matching the unchanged player. Both failed and passing receipts are retained in
`scratch/photometric-hard-views-20260908-01/`.

Root inspected all four first/middle/last contact sheets. The correction is
subtle in these views and visible green/dark variation remains in the April
ground pictures; this does not close the reported color-cast or field-interior
streak requirements. The existing owner-built Studio screenshots were also
viewed, but have different framing and are not paired output evidence for
these sequences. No new Studio export, fit, correction rule or live scheduling
change is introduced. The four 31-frame videos show neutral and automatic GPU
color on identical geometry, not a Studio comparison or owner acceptance.

The frozen test executable is `2152dd3f4e77192b454dcbb4dce49331816b06dfbaf1ea71d137a9276c9e0dd4`.
All 460 original X4/X2 regression artifacts remain byte-identical. Full workspace
gates pass 1,256 tests, zero failures, 30 ignored, formatting, all-target Clippy
and name/source-list checks. This is test-only coverage; the installed `31ea781d`
app and its known performance limits below are unchanged. The registered
estimator-free field-interior check still needs to consume these exact pairs
and matching view geometry before the dark-soil gate can be evaluated.

**Automatic-GPU color output comparison, 2026-09-08:** a test-only Scene
sequence now captures sources 34538 through 34639 of the owner's X4 file,
retaining one live GPU producer and its history across all 102 sources. Each
ordinary screenshot is compared with a diagnostic draw of that exact displayed
map and automatic ratio pair, within one 8-bit code per channel. A neutral
diagnostic removes only the ratio binding, retaining identical source, packed
geometry, alpha and PIS backend. The installed map and ratios are rechecked
after each diagnostic. No captured Studio coefficients, host estimator, prefix
warmup or new playback setting is used. Source indices/times and all images,
maps and ratios are retained in `scratch/photometric-live-output-20260908-01/`.

The existing sealed Studio ON/OFF exports are projected with the same previously
recorded display view, without new fitting or export. Root viewed comparison
panels at output candidates 0, 31 and 101 and neighboring difference fields.
Both products show broad, oppositely signed color regions; their magnitude,
shape and the underlying view registration are not identical. This is sampled
visual agreement in the kind of correction, not authenticated source/output
association, coefficient identity or Studio parity. The 102-frame presentation
videos and signed-cell diagnostics are available for owner review. The wider
dark-soil, sun-facing and hard-mode references remain open. The existing
`colour mode=profile` still toggles legacy pooled tone, not this GPU fusion;
running it unchanged would not qualify the current field-interior requirement.

The new test executable is frozen at SHA-256 `d389d074f92a1a80a358f810950769ef0959e2de678c3263d4e3e81e100fe669`.
The opt-in sequence passes all 102 source waits, and the original 31 X4/61 X2
sequences retain all 460 baseline artifacts byte for byte. Full workspace gates
pass 1,255 tests, zero failures, 30 ignored, plus formatting, all-target Clippy,
name/source-list checks, two device-limit and three GPU preflight tests. Only
test code changes; native and installed player binaries remain unchanged.
The installed `31ea781d` test build and its performance limits below still apply.

**Due-result wake candidate, 2026-09-08:** branch playback replaces repeated
redraw polling with one coalesced worker notification only while an exact due
source is admitted and its actor can finish autonomously. Registration and
temporal commit share a lock; future-ready publication, full draw retirement,
backpressure and absent-listener diagnostics keep their existing retries.
Input and controls remain independently drawable. This changes scheduling,
not source cadence, seam arithmetic or correction age. Both real-camera
sequences complete with the renderer asleep for all 31 X4 and 61 X2 source
waits, and all 460 original output artifacts remain byte-identical to the
control. The initial candidate native executable is `c2074e2f` in
`scratch/fusion-live-20260907/ready-wake-02/`. Native UI qualification passes
50 X4 and 54 ONE X2 checks, zero failures, with both reported-view captures
byte-identical to the preceding controls-only build. Full workspace gates pass
1,254 tests, zero failures, 30 ignored, including the lost-wake, exact-identity,
failure and poisoned-owner wake regressions. Formatting, all-target Clippy,
name/source-list checks, two device-limit and three GPU preflight tests pass.
Four authenticated native measurements of that frozen `c2074e2f` executable
complete at 2256x1504 with quiet audio. The 40-second nominal-300-Hz pointer
cohorts pass strict accounting: conservative completed-changing capacity is
273.125 X4 and 280.399 X2 updates/sec, with consecutive 29.975 source advances/sec
on both. Draw completion wall p99 remains 8.712/8.100 ms. Ordinary 60 Hz idle
cohorts have no holds at least 47 ms, but the X4 panning run's post-pointer tail
has a recovered 50.322 ms source hold. These are not paired comparisons with
the old control and do not establish a causal improvement. The detailed limits
are in `ready-wake-02/PERFORMANCE.md`. This is not a demonstrated hitch fix or
owner acceptance. Installed-runtime qualification of the exact committed source
is recorded separately below.

After the initial candidate was frozen, two test-only assertions were added
for a poisoned owner's failure wake and a same-index/different-epoch stamp.
The final native rebuild is `6fc7943f`, separately retained as
`ready-wake-02/final-kjerag`. The `c2074e2f` UI/capacity measurements above must
not be silently relabeled as measurements of that rebuilt executable. The
current source's SDK-built installed application is qualified separately.

**Installed due-result wake build, 2026-09-08:** exact source `31ea781d` passes
CI 34194254914 and is built, exported and installed as `dev.harding.Kjerag`.
An independent audit matches all 449 archived files to that Git tree and
confirms unchanged dependencies, runtime and sandbox permissions. Installed
OSTree is `9a0623d57c6bf80be8e513831468f798fd1750e87b7ccc3c1c6d2faffa344eb7`;
packaged and installed executable SHA-256 is
`40cafdd105d4c2fec7e7ee7d623b7b25e604a8e68a3d4361c3f1b6e0521050e5`.
The full installed UI suites complete with 40 X4 and 44 ONE X2 checks,
zero failures. Before/after deployment and executable guards pass. Both
reported-view PPMs are byte-identical to the preceding installed `eaa304bc`
captures; root viewed and linked the new PNGs. The isolated UI sessions lack
a sound device, so the volume check skips; preload failure injection also
skips in the sandbox. The shader check uses the harness's native Rust twin,
not a second sandbox executable. Evidence is
`scratch/flatpak-delivery-31ea781d/`. The previous verified `eaa304bc` bundle
is retained for rollback. No merge, release or owner acceptance.

Four authenticated measurements of that actual installed executable finish
with exit zero, live null-sink audio and positive exact-process Radeon graphics
work. At 2256x1504, the 40-second nominal-300-Hz pointer cohorts pass strict
source/draw/completion accounting: conservative completed-changing capacity
is 292.449 X4 and 297.624 X2 updates/sec, with 1,199 consecutive source advances
in each run (29.975/sec). Draw completion wall p99 remains 8.652/8.463 ms,
maximum 17.997/17.272 ms. Neither pan nor ordinary 60 Hz idle repeats the
previous X4 early source-hold episode or X2 phase-debt range; no observed
source transition holds at least 47 ms. These are single runs, not paired
comparisons or physical scanout, and do not establish a causal speedup or a
general hitch fix. Whole traces retain source-less startup commits and a
termination-cutoff callback; strict pointer and common idle cohorts verify
their own boundaries. `scratch/flatpak-delivery-31ea781d/PERFORMANCE.md`
records exact cohorts and limits. All six dedicated installed controls wakes
also pass with authenticated before/running/after identities: maximum pump
gaps 34.122/33.767/33.492 ms X4 and 33.729/33.495/33.780 ms X2
(`run.uhQZ7pwQ`, `run.VfvzfLYz`). The candidate is retained for owner testing;
4.17 ms draw tails, desktop smoothness and Studio-output color qualification
remain open. No further export or broad reverse engineering is authorized by
these results.

The existing installed X4 lifecycle repeat does not reproduce the prior
115 ms excursion. Its smaller repeated old-source draws occur before the next
deadline, many aligned with status updates or pointer/window activity. It
does not establish due-source polling as the earlier stall's cause. Exact
identity, matched-cohort timing and driver-observation limits are recorded in
`scratch/installed-capacity/x4-installed-idle-lifecycle-01/RESULT.md`.

**Carried-correction visual experiment, 2026-09-08:** following the owner's
question about reusing stitching while new video advances, a test-only
experiment now renders source N with its fresh parent/preimage/base geometry
and the preceding completed source's chart-relative public flow. It does not
reuse the preceding absolute packed UV map, change the selected player, or
skip any source's full solver transaction. Separate arms retain current
photometric ratios or carry the preceding ratios as well.

All 31 X4 and 61 ONE X2 source frames render. Recomposition with same-frame
flow is byte-exact to every GPU packed map; the diagnostic direct mesh's
current-map rendering matches the actual Scene screenshot within one code per
channel. All 460 original sequence images, maps, alpha and ratios remain
byte-identical to the prior control. A synthetic test proves only the public
flow ranges are substituted, with current geometry/statics and the current snapshot
unchanged. Full workspace gates pass: 1,248 tests, zero failures, 30 ignored,
formatting, all-target Clippy, name/source-list checks and existing GPU preflight.

Root inspected all 61 X2 riser pairs as contact sheets, the existing computed
alpha trace and enlarged high-difference frames. The riser remains connected
in that inspection, but fast edges visibly move with carried correction, for
example frames 6371 and 6396. This is not an exact-seam claim or accepted
tradeoff. A two-second side-by-side was linked to the owner with an explicit
question about a live test; no acceptance is inferred. Evidence, source patch,
control hash guards and videos are in `scratch/fusion-live-20260907/carried-flow-01/`.
The independent X4 inspection finds texture-shaped resampling differences but
no visible new seam opening in this slow, distant 31-frame scene; root also
viewed its central seam pairs. Current ratios are only an isolation arm here,
not an already-available low-latency live input, because they depend on the
current completed final map. No broader camera or motion conclusion is drawn.
The experiment runs only after the current full solve has completed, so it
proves hypothetical pixels, not live latency, deadline availability or safe
shared-source ownership. Live preview would require fresh current geometry,
bounded immutable correction snapshots and separate compute/draw lifetime
proofs. This experiment did not change the installed `eaa304bc` controls
correction; the later `31ea781d` delivery above also retains same-frame maps.

**Actual installed capacity, 2026-09-08:** two authenticated 40-second
2256x1504, nominal-300-Hz pointer runs with live null-sink audio exceed 240
conservative completed-changing updates/sec: 280.524 X4 and 293.924 X2.
Source advancement remains approximately 29.97 fps overall; draw callback
wall p99 is 9.604/9.312 ms, not a 4.17 ms tail-latency pass. X2 temporarily
accumulates about 235 ms of source phase debt while view draws continue, then
catches up. This is gradual debt over 56 source intervals, not one 235 ms
freeze. The same source range in two native controls does not accumulate debt;
the internal cause is not yet located. In ordinary 60 Hz idle playback, the
installed X2 run does not repeat that episode. Installed X4 instead has a
localized early episode about three seconds into playback, reaching 116 ms
of phase debt and recovering; excluding it from a five-second steady cohort
does not make it harmless. Raw evidence is `scratch/installed-capacity/`.
The exact common steady native/installed idle cohorts are complete on both
cameras with no source holds at least 47 ms; source-hold p99 is
34.355/34.821 ms X4 and 34.479/34.742 ms X2. The native X4 arm does not have
the early excursion at the same source range. `idle-01-RESULT.md` separates
that episode from the later cohort and records trace/termination boundaries.
Average capacity is now measured in the installed runtime, but intermittent
source holds, frame-time tails and desktop/owner smoothness remain open.

**Two-chunk scheduling trial rejected, 2026-09-07:** the installed controls
correction below is now qualified on both cameras. A subsequent native trial
kept the six L1 command buffers and their order but permitted two chunks
without completed-prefix credit instead of waiting between every chunk.
It passed 35 lease/facade tests, both real-camera cold/warm checks, both
autonomy and prefetched-seek checks, and all 460 image/map/alpha/color artifacts
remained byte-identical across the 31-frame X4 and 61-frame X2 sequences.

Two control/candidate comparisons in opposite orders then used 40-second
pointer pans at 2256x1504, nominal 300 Hz, with real source playback and verified
null-sink audio. All eight pointer cohorts pass strict source/draw/completion
accounting. Conservative completed-changing rates and draw callback wall p99,
control to candidate:

| Camera / pair | Changing updates/sec | Draw p99, ms |
| --- | ---: | ---: |
| X4 / 1 | 272.624 to 268.949 | 8.276 to 9.286 |
| X4 / 2 | 270.974 to 268.249 | 8.864 to 9.069 |
| X2 / 1 | 282.474 to 280.925 | 8.049 to 8.587 |
| X2 / 2 | 282.449 to 281.974 | 7.598 to 8.172 |

The lower rate and worse draw tail repeat on both cameras in both orders.
Matched source-hold p99 changes by less than 0.32 ms, with no holds at least
47 ms in any matched cohort. Source order stays consecutive, video keeps its
recorded cadence, and reported lateness stays bounded with no audio underruns
or drops. Temperature direction reverses by order; it does not justify
accepting the repeated regression. The trial is removed from selected Rust,
and `target/release/kjerag` is restored to the frozen `6927f39e` native binary.
The installed `eaa304bc` Flatpak is unchanged by this experiment.

The four unchanged native control runs therefore exceed 240 average changing
updates/sec alongside full source playback. This is useful bounded capacity
evidence, not a guarantee of 4.17 ms frame times, physical scanout, or the same
capacity in the Flatpak runtime. Whole traces retain source-less startup
commits and cutoff callbacks at termination; the strict pointer and matched
source cohorts explicitly verify their own boundaries. Evidence and the
rejected source patch are retained in `scratch/fusion-live-20260907/worker-credit-window-01/`
and sibling `*-capacity-credit-window-*-0[12]` directories. No merge or owner
acceptance. Further work should verify sustained installed-app smoothness and
reduce real execution cost, not repeat this queue-window sweep.

**Delivery split after capacity failure, 2026-09-07:** the combined `8c6921cf`
package is built but will not replace the installed app. Its final native
binary's 40-second 2256x1504, nominal-300-Hz ONE X2 panning run passes the
pointer cohort's trace-integrity check, but delivers only 25.875 consecutive
source advances/sec and ends with 5.77 s reported worst lateness. Completed
draws average 254.77/sec, which does not excuse the source deficit; draw wall
time p99 is 13.04 ms. The corresponding X4 trace is refused because stdout's
ordinary `play:` report interleaved into a stderr JSON record. Its counters
cannot substitute for a valid capacity result. No player arithmetic needs to
change to separate those two diagnostic output streams on the next run.
GPU temperature reaches 91 C and CPU Tctl 100.6 C by the X2 run's end; those
readings are context, not proof that throttling caused the failure.

The coordinator is separating the independently verified controls-tree
correction from the unqualified larger reserve: restore b2's autonomous worker,
two decoded successors and one completed future, while retaining the UI fix.
The larger FIFO/pre-roll adds measured memory/startup cost without a qualified
overall smoothness benefit, and its owner tradeoff question is withdrawn from
this delivery. This is not a claim that the FIFO caused the capacity failure
or that the smaller reserve resolves it. The full performance goal remains open;
the intent is to deliver the concrete controls correction without bundling the
unproven reserve. The independent same-epoch forward-step EOF correction is
retained. The split passes full workspace gates: 1,247 tests, zero failures,
30 ignored, formatting, all-target Clippy, name/source-list checks, both
device-limit tests and three real-GPU preflight tests. Its rebuilt native
executable is `6927f39e4bc9a58d447d1bef837b66a036477e1ea18a729ea7a8234528906d19`.
The full native UI suite passes 50 X4 and 54 ONE X2 checks with zero failures.
Both reported-view captures match the b2 native UI captures byte for byte;
root viewed and linked both. Runtime commit `eaa304bc` is pushed and CI
34185000786 passes. The exact split binary also passes all three dedicated
wakes per camera: maximum pump gaps 33.70/33.61/33.46 ms X4 and
33.52/33.81/33.51 ms X2. The larger FIFO is therefore not needed to remove
the reproduced controls-wake pause.

The exact-source Flatpak build, export and install complete with exit zero.
The installed app is the selected `eaa304bc` split at OSTree
`efeb42af77deb598c8fedc9be247ce3c4c7d9f1301021f092fbd14c244f23756`;
its packaged, installed and running executable SHA-256 is
`10ffb2886adc89bda16d1823fba390250bd1c3490da2b3efb5d745b528478e18`.
The Flatpak artifact SHA-256 is
`ac77dbd5e5a31c07c70af98b5d22de75e78b476ee90457457af946c3ee779910`.
All three installed controls wakes pass on each camera: X4 maximum pump gaps
33.622151/33.571097/33.615027 ms and ONE X2
33.567535/33.903542/33.784812 ms. Both runs authenticate the same OSTree and
the same executable before, during and after playback. The installed UI suite
completes with 40 X4 and 44 ONE X2 checks, zero failures, and passing before and
after executable hash guards. Root viewed and linked both reported-view PNGs;
both PPMs match the preceding installed b2 captures byte for byte. The isolated UI
sessions have no sound device, so their volume check skips, and sandbox preload
injection also skips. Their shader comparison uses the harness's native Rust
twin helper, not a second in-sandbox executable. The separate installed
controls-wake runs above do have the null-sink audio path live; they are a
bounded wake result, not a general audio qualification. No merge, release or
owner acceptance.

The archived combined package has executable SHA-256
`04c836b981a4a973e69d9cedea3e91dfc23b6cda6fbaca8d6cc001b782df86d7`
and OSTree `c8be21c36024a7f872eb72a2e074fa40fd6a891ad38fa825699335f5c996dbde`.
The first build stopped on a 300-second dependency-download timeout; its logs
remain in `scratch/flatpak-delivery-8c6921cf-fetch-failed/`. An offline retry
using the exact cached pins completed all archive/hash/source-list/link checks
in `scratch/flatpak-delivery-8c6921cf/`. It is not an installed-runtime pass.
Capacity evidence is `x4-capacity-controls-fifo-01/` and
`x2-capacity-controls-fifo-01/` under `scratch/fusion-live-20260907/`.

The dedicated controls-wake harness now also tests the actual installed
Flatpak. Its verified b2 X4 baseline reproduces all three pauses at
252.86/253.26/252.81 ms in `scratch/controls-wake/run.SqclrhZI/`. Exact installed
commit, manifest command and running executable are authenticated; Flatpak's
reported child PID is its wrapper, so the app is found only beneath that
authenticated sandbox. Transient proxy/compositor sockets use a unique directory
under the caller's runtime and are removed; all logs/private config/evidence
remain in scratch. Earlier startup failures from an unsuitable proxy-socket
path, abbreviated commit comparison and wrapper-PID hashing are preserved as
failed harness attempts, not player failures. No owner desktop/config is changed.

**Installed test build and seam-preserving redesign, 2026-09-07:** the owner
clarified that preserving the old installed Flatpak is not a requirement,
rejected staggering the two directions' refresh cadence if it affects the
seam, and requested considering rearchitecture. The unbuilt stagger prototype
was removed completely. A sliding two-chunk queue-window alternative remains
an unrun scratch patch, not selected runtime code: it could shorten worker
waits but also put more compute ahead of an interactive draw.

Exact qualified source `b2c133e5` is now built with the Flatpak SDK and installed
as `fdc22560f244cd0810fc052fcbd0ed40f9e9874560f6bdf04007166fad377c79`.
Installed executable SHA-256 is
`fe14f473e25f03190024fc54df7b2a58ae13edc8b54d22c066a79fd1a76692c9`.
The actual installed bundle passes 40 X4 and 44 ONE X2 UI checks with no
failures, including the reported views, backward seeking and late scrubber
landings. Both use dmabuf import. Normal audio is not qualified by these
isolated Flatpak sessions: they lack the Pulse socket and sound controls skip,
as documented by the harness. Sandbox preload checks also skip. Scripted
pause/seek report intervals are not a sustained-performance or hitch test.
Root viewed both installed exact-view captures; their full-window bytes differ
from the native captures, so no cross-runtime byte-identity claim is made.
Source, package, installed executable and harness hashes are recorded in
`scratch/flatpak-delivery-b2c133e5/`. No release, merge or owner acceptance.

The next architectural candidate is a bounded FIFO of three completed future
source/map pairs with one additional source in flight. Decode/stitch pre-roll
must run while the presentation and audio clocks remain held, then start both
once the ready queue is primed. Publish only the exact due FIFO head; preserve
source order, calculations, refresh cadence and fresh seek history. This aims
to absorb isolated work spikes without adding more GPU work ahead of draws;
it does not increase mean compute capacity. Startup/seek-resume latency and
memory use must be measured and surfaced before owner testing. The design is
not implemented or a smoothness result at that installed-build checkpoint.

**Controls-wake pause corrected in native tests, 2026-09-07:** both native X4
arms show a roughly 250 ms gap in Scene pumping immediately after the cursor
wakes the hidden controls. The candidate has three exact future pictures
already ready, and draw/presentation callbacks finish promptly. This episode
is not an empty stitch queue or evidence of slow seam computation. Inspection
then found the pinned iced named-child reconciliation overwrites the retained
content when inserting the named COSMIC header ahead of it. The new app
regression reproduces the failure: the expected two-child tree has only one
child after the transition. No Scene widget means no media deadline request;
the next 250 ms controls Tick rebuilds the missing content. The local dependency
correction now passes that test and all 67 app tests, without changing seam
calculations or UI layout. The new private-compositor regression reproduces
three X4 wake gaps of 253.70/253.24/253.45 ms before the correction. The corrected
build passes all three X4 wakes (maximum pump gaps 33.43/33.55/33.77 ms) and all
three ONE X2 wakes (33.99/33.42/33.52 ms), at 1280x720. These are source-playback
update intervals, not a 240 fps changing-view capacity pass. The parser's 14
synthetic tests cover malformed, incomplete and delayed evidence; CI runs them.
Full workspace gates now pass: 1,258 tests, zero failures, 30 ignored, plus
the two device-limit and three real-GPU presentation-readiness checks. The
first full run exposed an EOF test assuming an immediate redraw even when the
two draw slots are full; its corrected assertion requires the existing bounded
1 ms retry and retains exact final-frame/EOF acknowledgement checks. Runtime
code did not change for that test correction. The broader native UI suite now
passes all 50 X4 and 54 ONE X2 checks, including exact reported views, backward
seeks and real scrubber landings. Both reported-view captures match the preceding
FIFO native build byte for byte; root viewed and linked them. The runtime is
recorded in branch commit `8c6921cf`; the final native UI executable SHA-256 is
`0d0390696daec8eee7f13f58f70f21805984cf65f7544a51ad6d76842304066d`.
Evidence is `scratch/fusion-live-20260907/ui-09-controls-wake/`.
Actual X4 sourced-presentation holds
around the three wakes fall from 286.80/256.30/255.65 ms to 41.73/43.68/46.15 ms;
none of the corrected wake windows contains a presentation without a Scene
draw. These are native commits, not physical scanout measurements.
This is a verified correction for the reproduced controls-wake pause, not all
of the owner's pauses or the separately reported issue #149. Evidence starts at
`scratch/fusion-live-20260907/controls-wake-01/` and the X4 `ready-fifo-idle-01`
logs. No upstream interaction, new installed build or owner acceptance.

**Ready-frame reserve implementation, 2026-09-07, performance not yet qualified:** the
branch now implements that three-completed/one-additional FIFO and explicit
clock-held pre-roll. The first exact target becomes visible before playback
starts; startup, seek and resume prepare successors while the actual media
clock and audio Beat remain paused. A real decoder EOF permits fewer than
three successors, so a short tail need not wait for nonexistent frames. Pause
cancels new pre-roll admission; accepted source owners remain valid. Source
order, seam arithmetic, full-refresh cadence and draw-retirement capacity are
unchanged. Media unit tests pass (79 passed, zero failed, three footage tests
ignored); the production renderer compiles. A new regression first reproduced
and then verified the correction for forward stepping losing an already observed
same-epoch EOF, which otherwise strands resume pre-roll at the tail. Native
real-camera checks now pass: 68 Scene, six root, one worker and 35 facade tests,
plus 50 X4 and 54 ONE X2 UI checks. All 460 frame/map/alpha/color artifacts over
31 X4 and 61 X2 frames match the preceding deadline build byte for byte. Evidence
is in `ready-fifo-02/` and `ui-08-ready-fifo/` under the same scratch parent.
The idle comparison does not establish a smoothness win: the valid X2 common
cohort has essentially unchanged p95/p99 holds and two long holds versus one;
the X4 pair is refused for a source-less presentation inside its common cohort.
The controls-wake gap above falls on different sides of the two arms' steady
selection boundary, so their maximum-hold figures are not a valid comparison.
Final-binary controls-wake repeats pass all three wakes per camera, with maximum
pump gaps of 33.77/33.80/33.93 ms on X4 and 33.83/34.02/33.82 ms on X2.
Same-harness 1280x720 memory snapshots against the b2 native control show
104.24 MiB more driver-accounted memory on X4 and 118.02 MiB more on X2,
deduplicating DRM client IDs before adding VRAM and GTT. These are one steady
snapshot per arm, not peak GPU usage; process RSS is reported separately and
must not be added to those potentially overlapping driver accounts. The time
from the first target's publication to its first successor is 80.43 versus
49.18 ms on X4 and 80.37 versus 48.32 ms on X2. This measures about 31–32 ms
additional initial-picture hold in these runs, not total open latency or all
seek/resume costs. The coordinator explicitly asked the owner to accept this
test-build memory/startup tradeoff; no answer or acceptance is recorded yet.
Raw runs and exact executable hashes are under `scratch/controls-wake/` and
summarized in `controls-wake-01/README.md`. Active-view capacity and the FIFO's
overall smoothness benefit remain unqualified. The combined `8c6921cf` package
is being built from an exact git archive, without installing it or accepting
the buffer tradeoff. The installed Flatpak remains the qualified b2 worker build,
not this unqualified candidate. No main merge or new owner acceptance.

**Autonomous source processing under qualification, 2026-09-07:** the owner
challenged the prolonged micro-hitch investigation and the pipeline's design
for smoothness. Further small scheduling sweeps are paused. The current branch
is separating computational progress from window redraws: one renderer visit
admits up to two decoded sources; the worker owns final validity completion,
unpublished temporal commit and successor startup. Only the renderer can
publish the exact due picture or reserve a draw. A completed second result
parks when the one future slot is occupied, ending worker service until
publication makes space. Shader arithmetic, source order, history, six L1
submissions/five pacing waits and the two-source bound are unchanged.
Paired real X2/X4 regressions now pass source progress with no main-thread
pump, prepare, draw, GPU poll or readback, plus safe seek/recreation and exact
map/alpha/color-ratio comparison. All 34 facade tests and the actor panic check
pass. The 31-frame X4 and 61-frame X2 Scene sequences preserve all 460 image,
packed-map, alpha and color-ratio artifacts byte for byte. Evidence and native
candidate: `scratch/fusion-live-20260907/autonomous-worker-04/`.
The full workspace gates pass (1240 tests, zero failures, 30 ignored), plus
the UI device-limit/preflight checks. The rebuilt native player passes all
50 X4 and 54 X2 UI checks, including both real scrubber/backward-seek paths.
Both exact reported-view window captures are byte-identical to the preceding
deadline build. Root inspected and linked the new captures; owner testing is
still pending. UI evidence: `scratch/fusion-live-20260907/ui-07-autonomous/`.
One isolated 60 Hz idle comparison, using identical source ranges with complete
source/commit/completion records, reduces holds of at least 47 ms from 64 to 1
on X2 (874 transitions) and 52 to 15 on X4 (877). Both p95 holds fall from about
48 ms to 34 ms. X4's worst hold worsens from 66.4 to 91.8 ms; its target spends
85.7 ms inside the worker body while autonomous predecessor handoff takes only
0.011 ms. The specific internal operation is not identified by that log.
Active-view A/B results are inconsistent, with refused traces and concurrent
host load recorded; successful candidate runs cross 240 average changing FPS,
not every-frame 4.17 ms timing. Whole-log integrity and physical scanout are
not established by the selected cohorts. This is not a qualified smoothness
fix, a new review package or owner acceptance. All existing owner packages and
the installed application remain unchanged.

**Smoothness candidate under qualification, 2026-09-07:** ordinary idle
playback now waits for exact video deadlines instead of refreshing merely to
poll speculative stitching. The measured previous refresh often presented the
old picture just before its successor was due, arming the Wayland callback and
delaying publication of an already-ready successor. Input redraws, source order,
two draw-retirement slots and stitching/color arithmetic are unchanged. A
post-prepare follow-up handles an exact due picture that is not ready yet.
In matched 25-second native COSMIC runs, X4 source-hold p95 fell from 49.04 to
34.22 ms, with holds at least 47 ms falling from 47 to zero. ONE X2 p95 fell
from 48.05 to 37.71 ms and long holds from 36 to 13, but its maximum rose
from 64.99 to 68.19 ms. Remaining X2 holds coincide with unavailable due
results; reduced idle lookahead headroom is not an accepted compromise.
Both traces retain six source-less startup commits and fail whole-run strict
accounting. These are native commit timings, not scanout measurements. Three
real-GPU renderer preflight checks and 62 Scene tests pass. Fresh full gates
pass 1,240 workspace tests, zero failures and 30 ignored; native UI passes
50 X4 and 54 ONE X2 checks. Both exact reported-view captures and all 460
artifacts across 31 X4/61 X2 consecutive frames remain byte-identical to the
preceding color build. Two 40-second native panning runs at 2256x1504 reach
257.87/255.50 changing commits/s on X4, with 29.975/29.950 source fps and
bounded lateness. ONE X2 reaches 272.47/257.60 changing commits/s, but its
second run drops to 29.425 source fps and accumulates 733 ms worst lateness;
that repeat fails the full-source-cadence requirement. All four pointer
cohorts pass strict trace accounting, while whole-run traces retain failures
outside that cohort. Commit p99 is 10.21 to 12.06 ms, not an all-frame
4.17 ms pass. This is a measured X4 improvement, not a qualified two-camera
smoothness fix. The candidate is held for investigation of the X2 failure.
No owner review package, installed application or main branch has changed.

**Package verification follow-up, 2026-09-07:** an additional, uninstalled
Flatpak is built from exact source `ad1d9d62`; the earlier owner review builds
remain frozen. Its first isolated UI run passes 44 ONE X2 checks and 39 of
40 X4 checks. The X4 failure is a harness ordering race: opening prints
`media:` before the command-line view's `goto:` acknowledgement. The completed
log contains the exact requested view, and the separate real-frame assertion
passes. The harness now waits boundedly for the first acknowledgement, still
rejecting a wrong first view. Five no-GPU regression scenarios exercise the
actual harness decision, including delayed output, timeout and process death;
CI runs them before installing dependencies. This changes no player code.
The same unmodified package then passes all 40 X4 and 44 ONE X2 UI checks;
both exact reported-view captures repeat byte-identically across package runs.
Root viewed and sent both captures. The sandbox skips sound-device and injected
import-failure checks, and the shader/Rust-twin helper is native; the native
suite's corresponding coverage is recorded above.
Fresh full gates after the harness correction again pass 1,240 workspace
tests, zero failures and 30 ignored, both device-policy checks, all three
real-GPU renderer preflight checks, and all five startup regression cases.

Later actual-desktop tests fall to exactly one redraw per second in both this
package and the unchanged earlier Flatpak/native controls. A desktop capture
shows the test player completely covered by another window. Worker execution
and GPU completion remain prompt while renderer collection waits for redraws;
audio callbacks continue but starve behind bounded video decoding. These
covered-window runs do not qualify visible playback and are not evidence of a
new-package-only regression. A targeted activation request did not establish
sustained visibility or recovery; the follow-up capture still shows the player
covered. The owner confirmed that the desktop is in use, so automated work now
stays isolated. A visible-window retest remains pending. This is not a fix for
the separate known COSMIC pause/hidden-controls issue #149, and no desktop
settings have been changed. The additional package is available for experimental
owner review, not a completed two-camera smoothness or capacity fix.

**Latest qualified follow-up, 2026-09-07:** live GPU photometric matching now
has one immutable texture publication instead of duplicate buffers. Full gates
pass 1,238 tests and native UI passes 50 X4/54 ONE X2 checks, now including the
real scrubber and backward seeks on both fixtures. Review sequences remain
byte-identical. Micro-hitches and X4's saturated-playback deficit remain open;
the indexed-mesh trial was removed because it did not give a useful X4 gain.
The owner-facing color Flatpak stays frozen at `6207b377`, with no install,
merge or release. Owner visual acceptance of the color increment is pending.

**Chromatic implementation started, 2026-09-07:** the smoothness review
package stays frozen. One bounded Studio capture now supplies both 200x100
RGB ratio uploads and their named fragment-slot metadata. Optional Metal
readback failed and the attempted export did not finish; neither GPU-content
identity nor a new usable video oracle is claimed. The previously isolated
ON/OFF video pair remains the visual reference. The sanitized capture
contract is `docs/research/studio-image-fusion-maps-602.json`.

**Color integration checkpoint:** the GPU photometric producer and live
capture integration are implemented. Each valid source observation samples
its own final packed map, computes correction on the GPU and carries immutable
ratios into its own draw. View redraws reuse them; seek restarts color history.
No new host readback/solve/wait or source submission is added. Standalone
captured X4/X2 comparisons reached maximum ratio error `9.54e-7` against the
CPU reference, with identity/lens-swap/channel-swap decoys rejected. Integration
corrected the global-validity convention to native `u32::MAX` success. The
two real-source Scene comparisons now pass, including installed ratios against
the CPU reference and final mesh-rendered RGBA within one code per channel.
The initial pixel test compared the diagnostic fullscreen path with the live
mesh path and failed; using the same mesh corrected the test without widening
its tolerance. Full integrated gates and the two short rendered sequences now
pass; the new Flatpak is qualified below, while owner acceptance remains
pending. The integer content
gate's binary64-edge difference and GPU reduction order are disclosed in the
architecture and reference contract. The frozen smoothness test package and
installed application are unchanged. Subsequent paragraphs record earlier
implementation checkpoints, not the current live-feature status.

The first real-window run failed because iced requested only two bind groups;
the fused consumer requires three. That device request and its pre-construction
guard are corrected. Native window retests pass 48 X4 and 54 ONE X2 checks with
zero failures, and root viewed/sent both reported-view captures. The first color
candidate failed active-playback performance on both cameras (roughly 22–28
source fps). A same-harness archived pre-color X4 control retained 30 fps.
GPU timestamps located expensive scalar admission/measurement, then the three
serial channel solves. Parallel integer admission/histograms and independent
channel workgroups reduce the sampled changed-warm color update from about
24.7 ms to 2.6 ms, with unchanged CPU-reference error and 51 focused checks
passing. These are isolated producer timings, not actual-player capacity.

**The buffer-backed color candidate fails on X4:** in the third 40-second
2256x1504 nominal-300-Hz panning run, ONE X2 sustains roughly 30 source fps, but
X4 remains around 26–28 fps and accumulates about five seconds of delay. The
X4 trace also contains malformed/interleaved JSON, so it cannot establish a
validated rendering-capacity result. This is not an accepted compromise or a
shipping result. Reducing native draw retirement from two slots to one restored
X4 source cadence but reduced changing-view capacity to about 178 fps (208 fps
on X2), so that trial was rejected and two slots restored.

A diagnostic bypass of only the final color consumer, with the GPU producer
still running, restored X4 source cadence and about 240 changing commits/s.
That bypass is archived, not selected by normal source. The current candidate
therefore writes the same final f32 ratios into immutable RGBA32F textures in
the existing remap pass. Explicit texture loads retained X4's deficit: the
direct-coordinate trial reached 27.825 source fps and 233.85 changing commits/s;
X2 reached 30.050 and 241.92, respectively. Both strict traces also fail on a
source-less commit. That trial does not qualify the candidate.

The next candidate uses optional full-f32 hardware bilinear filtering, retaining
explicit interpolation on devices without that feature. No extra pass or
half-precision conversion is introduced, but interpolation rounding can differ.
The 51 color tests, three consumer tests and two real-source Scene comparisons
pass, including bitwise texture/buffer publication equality and the unchanged
one-code-per-channel final-pixel bound. Adding arbitrary off-grid UVs then
exposed the anticipated hardware interpolation rounding: the manual consumer
still passes `2e-6`, while the filtered test fails that bound. All-sample
measurement finds a maximum `0.00054196` (0.138 of an 8-bit code) at the
synthetic map's discontinuous periodic join. Hardware now has a separately
named half-code test, while the manual bound and real-frame one-code gate
remain unchanged. This is disclosed interpolation rounding, not exact parity.

The filtered native run reaches 240.42/261.25 changing commits/s on X4/X2 at
2256x1504 with nominal 300 Hz panning. Source cadence is 29.375/29.975 fps:
X4 still accumulates about 1.4 seconds of delay and is not accepted. Commit
p99 is 8.74/8.76 ms, maxima 15.72/31.25 ms. The X2 pointer cohort passes strict
accounting; X4 fails on one source-less commit, and both whole-run checks
retain source-less records. These are not all-frame 4.17 ms passes. A bounded
warm-playback admission experiment prioritized an already-submitted exact due
source over further old-picture redraws, keeping both retirement slots. It
restored X4 to 29.975 source fps but cut changing capacity to 140.80 fps and
raised commit p99 to 19.73 ms. That policy and its tests were removed; ordinary
two-slot redraw admission is restored. The full-gate attempt stopped at one
test-helper `let_and_return` Clippy error, subsequently corrected. The workspace
test and UI gates did not run in that attempt.

The next trial uses RGBA16F only for terminal live ratio-map storage and
filtering, keeping all solver history, arithmetic and diagnostic buffers f32.
CPU saved-map replay remains RGBA32F, so the real Scene pixel comparison can
independently check the production storage conversion. This precision trial
passes 52 color, four consumer and two real-source Scene checks. The original
texture test incorrectly assumed nearest rounding; the observed downward
conversion is allowed by WGSL. Its corrected representation test requires
exact storage for representable values and at most one half-float spacing
otherwise. Both captured fixtures reach maximum storage error `0.00097644`;
the f32 producer/reference error remains `9.54e-7`, and the actual Scene
one-code-per-channel pixel gate is unchanged and passes. However, native X4
reaches only 29.275 source fps and 239.47 changing commits/s, and X2 reaches
29.975/260.75. Neither improves meaningfully over full f32. The half-storage
trial is therefore removed, with no precision compromise retained.

**Current qualification boundary:** retain the full-f32 hardware-filtered
implementation, complete the integrated gates, rendered sequences, native UI
and actual-display checks, and keep the unresolved X4 saturated-capacity
deficit explicit. This does not lower the 240 fps goal or qualify chromatic
playback for release. It prevents further speculative performance trials from
postponing the implementation's complete correctness and video review.
The restored implementation passes all-target Clippy and static gates,
1,238 workspace tests with zero failures and 30 ignored, two device-policy
tests and both opt-in real-Radeon window-readiness tests. Source hashes remain
unchanged across those gates. The rebuilt native UI passes all 48 X4 and 54 ONE
X2 checks, including both reported views and ONE X2 mouse-scrubber and backward
seeks. Those two seek checks were restricted to the X2 fixture; the later
texture-only follow-up extends them unchanged to X4. The
actual resident Scene captures 31 consecutive X4 frames and 61 consecutive X2
frames with their exact packed maps, alpha and ratio pairs. Root inspected
sampled pixels and computed seam overlays and sent both videos and overlays to
the owner. The overlays use the diagnostic ray consumer, not the live mesh;
the sequences use the actual resident screenshot path. Neither establishes
whole-video parity or new owner acceptance.

Both 25-second native checks on the actual 60 Hz COSMIC desktop retain roughly
30 source fps after startup, with zero drops, starvation or audio underruns;
worst reported lateness is 29.2/30.0 ms for X4/X2. The X4 strict cadence parser
refuses a draw cut off before commit at timed termination. X2's complete log
retains six source-less startup commits, so its strict result also fails.
Its closed steady-source subset has dwell p95/p99/max 46.37/50.99/62.85 ms,
not uniformly spaced source changes. These counters are
not proof that micro-hitches are resolved or that the uncapped target passes.
That baseline native executable is archived as `ui-03/kjerag`, SHA-256
`e1b8a4292243464aad2130fe2e8a3a7489e8f091ee651fb99eddb4a5f7387e92`.
An uninstalled Flatpak from clean archived `6207b377` now passes all 38 X4
and 44 ONE X2 sandbox UI checks. The real mouse-scrubber and backward-seek
checks in that harness run are ONE X2-only, not X4 coverage. Root inspected
both package captures; runtime
hash, archived source, unchanged sandbox permissions and installed-app checks
pass. The sandbox harness has no audio device and skips import-failure injection;
its shader-twin helper remains native. Actual-COSMIC package runs on both clips
hold 30 source fps after startup without drops, starvation or audio underruns.
Their complete traces both fail six source-less startup commits. Closed steady
source-dwell p95/p99/max are 46.75/50.22/64.26 ms for X4 and
47.27/52.07/67.06 ms for X2, so timing tails remain open. No strict smoothness,
240-capacity, Studio-video or owner acceptance is claimed. X4's running audio
stream was independently observed on the null sink; the X2 sink inspection
arrived after exit. Source CI `34128706598` passes. The package executable SHA is
`77c7b56856edf526949cac0133ef211395e52e5ac2e681af306d0fb4c5b13166`.
The owner has `scratch/review-color-20260907/run.sh x4` (`x2` for the riser).
It substitutes the package for one run without installing. The frozen smoothness
package and installed stable remain unchanged; owner review is still required.
The frozen smoothness package remains unchanged. Evidence:
`scratch/fusion-live-20260907/`.

The retained follow-up removes duplicate photometric output
buffers: the draw and explicit diagnostics now share the same immutable f32
textures. This removes two 320,000-byte allocations and duplicate stores per
source observation without changing the remap values, shader consumer or
temporal law. All 51 color, three consumer and two real-Scene comparisons pass.
The 31-frame X4 and 61-frame X2 sequences are byte-identical to the preceding
implementation across every PPM, packed map, alpha and both ratio maps. Native
capacity shows no useful speedup: X4 reaches 29.175 source fps and 239.20
changing commits/s, X2 29.950/261.12. X4 still accumulates delay. Commit
p99/max are 8.78/18.07 ms and 8.69/20.80 ms, respectively. X4's strict pointer
cohort fails one source-less commit; X2's passes. Both whole-run traces retain
source-less commits. This is duplicated-publication cleanup, not a performance
fix. Full gates stopped at a test-only `needless_range_loop` lint, subsequently
corrected; fresh complete gates and native UI now pass as recorded below. The packaged
`6207b377` review build stays frozen.

A bounded indexed-mesh trial preserves all 460 saved frame artifacts and passes
twelve direct-consumer, 51 color and two real-Scene tests. Its complete gates
also pass 1,239 workspace tests, zero failures and 30 ignored. Nevertheless,
X4 reaches only 29.475 source fps and 238.95 changing commits/s, with
8.79/23.00 ms commit p99/max and growing lateness. X2 reaches 29.975/262.82,
with 8.47/19.17 ms p99/max. Both strict pointer cohorts fail one source-less
commit. The additional index-buffer code is rejected for lack of a useful X4
gain; the original triangle draw is retained. The resulting texture-only
implementation passes all-target Clippy, static checks, 1,238 workspace tests
with zero failures and 30 ignored, both device-policy tests and both Radeon
preflights. Native UI passes 50 X4 and 54 X2 checks, now exercising the same
backward-seek and real mouse-scrubber tests on both named fixtures. Both reach
the late scrubber destination within the unchanged ten-second limit and hold
the exact backward target. Root inspected and sent both reported-view captures.
The archived `ui-05-texture-only/kjerag` executable is byte-identical to the
capacity11 candidate, SHA-256
`f2aa8b92f032105e55dcf4cb035c35ee117251f5a1fc61a3f8d2c201072a668e`.
These are correctness and interaction results, not a new performance pass.

An explicit captured-map replay consumer and a separate readable
Windows selected-X4 inner MGP reference are implemented. The latter
reuses the existing normal-equation/CG primitives but replaces the legacy
floating unaligned evidence with the recovered BGR-byte support, admission,
retained metric, live iteration budget and same-ordinal byte correction.
It does not yet implement source sampling, the outer content gate, or a
GPU-resident automatic producer. Reused sparse
traversal/reduction order is not claimed bit-identical. Mac producer and
other-camera equivalence are unverified. CPU reference and actual GPU
consumer tests pass, as do the required-Radeon workspace tests, all-target
Clippy and static gates. Both reported views render byte-identical saved-map
diagnostic PNGs with correction disabled, before and after this change.
Captured ratios also run through the real decoded X4 source and fragment
consumer. That replay uses the native map chart, not the owner's stabilized
view; source conversion and uncaptured fisheye alpha remain replay limits.
It is not a Studio-output parity comparison, an enabled automatic playback
feature, or a replacement for the frozen smoothness test package.

The source-color prerequisite is implemented and branch-verified: the shared
direct consumer previously hardcoded the ONE X2 BT.601 matrix, whereas both X4 container streams
and the captured Studio X4 uniforms specify BT.709. The ONE X2 container
specifies SMPTE 170M (BT.601). This independent file metadata now reaches the
source sampler without camera-specific guesses and gives the later estimator
and renderer the same RGB interpretation. The inner MGP BGR/YCC transform is
a separate algorithm and is not changed by the source color-space metadata.
Ordinary drawing shares the matrix helper; depth/range and box filtering are
unchanged. Untagged and unhandled matrices retain the historical ordinary
709 fallback, not a claim of supporting arbitrary color spaces. Required-Radeon
workspace tests pass 1,197/0 failed/30 ignored, with all-target Clippy and
static gates. The actual source-sampler GPU test covers neutral and colored
NV12 samples on both lenses under each matrix. Both reported saved-map views
render successfully; the ONE X2 full PNG is byte-identical to the previous
consumer. Root viewed both. These are diagnostic frames, not new owner or
whole-video acceptance. The native UI harness passes 48 X4 and 54 ONE X2
checks with zero failures, including the reported views, backward seeking
and the real scrubber. The full ONE X2 reported-view app screenshot is also
byte-identical to the frozen smoothness native build. Root viewed both native
captures. The existing smoothness Flatpak review build remains byte-identical
and uninstalled; this color change does not replace the owner's pending retest
or establish a new rendering-capacity result.

The next reference increment implements the selected Windows spatial stages
from aligned 200x100 BGR8 working images through RGB ratio maps: current-row
replication, periodic extension, inner solve, center crop, own-ordinal ratio
construction, local box filtering and chart remap. Direct caller/literal
review corrected older notes: neutral fills are one, the selected ratio
margin is 40 rather than model id 23, X blur wraps, and the remap reads full
100-row maps rather than 20-row blur ROIs. The actual ten-row ratio dispatch
leaves left row 40 retained from the preceding blur; the reference preserves
that recurrence. Give-back and endpoint normalization touch original rows
which cannot enter these fixed-size outputs, so their unconsumed state is
omitted. See `docs/research/studio-image-fusion-spatial.md`. Automatic source
sampling/admission and GPU production remain unfinished; this is not a newly
enabled player feature or a Mac-equivalence claim. All 27 photometric reference
and consumer CPU tests pass, including an admitted non-gray end-to-end pair,
neutral repeated observations, the retained boundary recurrence, exact ROI
edges and poisoned unconsumed state. Required-Radeon workspace gates pass
1,212 tests with zero failures and 30 ignored; all-target Clippy and static
checks pass. Native and frozen review executables remain byte-identical to
their pre-increment versions. No new UI or video-parity result is claimed for
this unused reference module.

An explicit GPU source diagnostic now reaches the existing spatial reference
and fragment correction on both actual reported frames via `stitch-layers`
trailing `estimate-fusion`. It retains exact decoded-source/map stamps and
bindings, uses the same source matrix/filter as drawing, and retains camera
profiles for stepped input without starting a live stitch transaction. Root
viewed both input pairs, corrected views and computed seam overlays. Colors
change modestly; this does not establish improved whole-video output or Studio
parity. Disabled final PNGs are byte-identical to the preceding source-color
diagnostic. Full required-Radeon gates pass 1,217/0 failed/30 ignored, including
the actual GPU shader with a half-turn negative control and source-owner/stamp
rejection. The frozen Flatpak review package remains unchanged. A direct
native binding review identified a remaining prerequisite: Studio gates full
800x16 source bands and area-resizes them into working rows 48..51, whereas
this first diagnostic takes full-chart point samples. That source reduction,
the exact source-band rays, content admission and automatic GPU production
remain separate work; the point sampler is not enabled in playback.
The rebuilt native player's unchanged UI harness passes 48 X4 and 54 ONE X2
checks, zero failures, including exact reported views, backward seek, real
scrubber and two-file loading. Both full reported-view screenshots are
byte-identical to the preceding source-color native build and were viewed.
This is a regression check, not an additional smoothness or chromatic-video
acceptance result. Evidence is ignored `scratch/fusion-sampling-20260907/`.

The source-band replacement now implements the bounded native construction:
positive-quarter-turn four-row packed-map composition, endpoint-aligned
800x16 expansion, independent lens sampling, the retained three-strip content
gate, 4x4 area reduction and sticky coordinate invalidity. The previous point
sampler and its image-support substitution are removed. The CPU reference
preserves binary64 mean comparison, including its exact-threshold rounding
case. The sampler uses Kjerag's qualified packed maps and container-driven
float color conversion; neither the native upstream map producer nor its
runtime integer conversion branch is claimed equivalent.

All 45 focused photometric CPU/GPU checks pass. The actual GPU fixture tests
every composed map node against the scalar reference, distinguishes opposite
rotation and repeated rather than endpoint-interpolated UVs, and checks both
textured source lenses at byte precision. Both reported real frames render
through band admission and the existing correction consumer. Root viewed the
800x16 inputs, final corrected views and computed seam overlays, and sent the
corrected views to the owner. Disabled final PNGs remain byte-identical to the
preceding source-color diagnostic. This is still-frame implementation evidence,
not Studio-output/video acceptance or a live calibration feature. No new export
was needed. The automatic GPU producer is next; the native player and frozen
smoothness review package remain unchanged. Evidence:
`scratch/fusion-sampling-20260907/attempt-08/`.
Required-Radeon full workspace gates pass 1,231 tests, zero failures and 30
ignored, with all-target Clippy, formatting and static checks. Source hashes
are unchanged across those gates. Evidence:
`scratch/fusion-sampling-20260907/gates-03/`. The initial gate attempts retain
the corrected test-typing, compute-binding, fixture and lint failures; no failed
run is reported as passed. No new UI or playback-capacity result is claimed
for this diagnostic/reference-only increment.

**Owner test: seam accepted visually, micro-hitches next, 2026-09-07:** the
owner says the frozen test build's seam looks good, but playback feels uneven
despite its 30 fps counter. They request smoothness first, then photometric
lens matching. This is not approval to merge a smoothness fix without retest.
The exact X4 command handed over is the initial reproduction assumption; the
owner did not explicitly name a camera in the hitch report.

The unchanged packaged candidate reproduces uneven source delivery on actual
COSMIC: a 25-second detailed trace sustains roughly 30 source fps but has
steady source-commit dwell p95 49.96 ms, p99 51.00 ms, maximum 64.48 ms, against
33.37 ms source spacing. Sources are consecutive; many native window commits
continue at roughly one display refresh while the same source is repeated.
An earlier desktop run additionally fell to 1 Hz after about six seconds;
that abnormal phase is retained separately, not assigned to COSMIC #149.
Frame commits are not physical scanout evidence. Startup UI-only commits and
the detailed run's final callback cut off by timed termination remain explicit
trace-integrity failures, not waived by these cadence statistics.

A diagnostic-only extension of the existing Scene lifecycle log prints the
already borrowed due and lookahead stamps. In a native sibling run, 68 staged
draws still use the previous source after its successor is due, affecting
66 distinct due frames; 60 of those share one modulo-ten phase. This narrows
the investigation to resident readiness and preparation lead time, with the
ten-frame full-work cadence initially a hypothesis, not a proven cost center.

Worker timing subsequently confirms that the exact full-work phase crosses
the source interval: an isolated run records 55 of 56 such warm jobs above
33.37 ms, median 37.52 ms, versus 16.55 ms for ordinary jobs. Admission delay
is below 0.1 ms. Worker time includes host submission and callback pacing, not
isolated GPU execution. A two-chunk pacing trial lowers worker time, but a
60 Hz native comparison does not improve source-dwell p95 (48.15/48.20 ms)
or median full-work readiness (47.41/47.42 ms); that trial is removed.

A bounded-buffer candidate is now implemented and under runtime verification: one completed
unpublished source/map pair may advance the temporal prior while the next
source is computed. Exact due publication, sequential source processing and
the two-draw retirement cap remain separate gates. Media exposes two decoded
successors; pause, seek, EOF and renderer-recreation coverage is being extended.
The first native 2256x1504 comparison shows fewer periodic holds on both cameras.
For source-aligned steady cohorts, source holds above 40 ms associated with the
full-work phase fall from 57/59 to 6/56 on X4 and 39/57 to 3/57 on X2. All steady
sources are consecutive. Full-log instances of staging an older map after
the next source is already due fall from 115 to 8 on X4 and 113 to 1 on X2; all
remaining candidate instances are in the initial 11/1 successors respectively.
Both candidates sustain about 30 source fps, no audio underruns, and no growing
lag in these 25-second samples. Null-sink routing and live executable identity
are authenticated; root viewed both native captures.

These are isolated native API commits on a nominal 60 Hz wlroots output whose
observed grid is about 16 ms, not physical 60 Hz scanout. Steady p95 remains about
48 ms, consistent with some three-tick holds on that grid; the comparisons do
not establish hitch-free COSMIC playback. X4 full-run reported worst lateness
increases from 35.4 to 44.3 ms in this pair; the candidate maximum is already in
its first interval. Do not erase startup behind the steady cohort.
Strict trace inventory retains startup UI-only commits and final callbacks
cut off by exit. Required-Radeon full workspace passes 1,178 tests, zero
failures, 30 ignored; all-target Clippy, formatting and static checks pass.
Real-camera overlap tests exercise pause, renderer recreation, exact due
publication and bit-identical packed maps/alpha against serial processing
through frame two on both cameras. This is a bounded arithmetic regression,
not whole-video identity. The unchanged native UI harness passes 48 X4 and
54 X2 checks, including the exact reported views, pause/resume and the real
X2 scrubber/backward-seek path. Root viewed both reported-view captures.
Compared with the earlier Flatpak, these native screenshots have sparse,
predominantly one-code differences, not byte identity across runtimes.

Renewed 40-second active-pan runs at 2256x1504 on a nominal 300 Hz headless
output record 274.92/297.67 sourced draws per second for X4/X2 and 29.975
consecutive source advances per second on both. Changed-source-or-view rates
are 250.05/272.97 per second. Neither accumulates lag. Commit-spacing p99 is
9.13/6.60 ms, maximum 22.22/17.11 ms; this is not an every-draw 4.17 ms result.
Each strict trace still fails one UI-only committed frame, so those rates
are observational sourced subsets, not strict passes or physical scanout.
Full-run reported worst lateness is 253.7/260.3 ms, retained rather than hidden
by the steady rates. Evidence: `scratch/microhitch-20260907/buffer-candidate/`.

A subsequent 25-second actual-COSMIC X4 run does not enter the earlier 1 Hz
phase. Only the first successor is staged late; no later already-due source
uses an older map. Steady source dwell p95 is 43.76 ms, p99 48.32 ms, maximum
50.99 ms, versus 49.96/51.00/64.48 ms in the earlier packaged reproduction.
This is not a controlled same-runtime A/B or physical scanout measurement.
Sources remain consecutive with bounded phase error; full-run reported worst
lateness is 29.7 ms and no audio underruns occur. Its strict inventory retains
five startup UI-only commits and the final present/draw callbacks cut off by
intentional termination. Shader and
temporal arithmetic are unchanged. The reviewed packaged candidate and
installed app stay frozen. Evidence and current diagnostic identities:
`scratch/microhitch-20260907/`.

**Smoothness package ready for retest, 2026-09-07:** clean source af85908a is
packaged without installation. Candidate executable `01b4b80b`, bundle
`32c6a925`, passes the unchanged Flatpak UI harness: 38 X4 checks and 44 X2,
zero failures. Both entire reported-view captures are byte-identical to the
previously reviewed package; root viewed both. This is a two-frame regression,
not whole-video identity. The isolated sandbox harness has no audio device
and skips import-failure injection; its shader-twin helper remains native.

A separate actual-COSMIC package run reports 30 source fps after startup,
no drops, starvation or audio underruns, and worst lateness 30.1 ms. Its full
cadence parser refuses one interleaved native/Wayland JSON record, so no
package cadence percentiles or strict pass are claimed. The native-COSMIC
statistics above remain the measured cadence result. This run used quiet
routing flags, but an independent sink probe arrived after exit.

CI 34094150584 at af85908a passes. The new hash-checked launcher is
`scratch/review-smoothness-20260907/run.sh`, with `x4` and `x2` arguments.
It runs via app-path, not installation. Owner smoothness retest and branch
acceptance remain required. Installed stable and the previous candidate are
unchanged. Packaging's temporary repository uses a 10 GB free-space reserve
instead of its default percentage; this changes no application permissions.
Evidence: `scratch/flatpak-candidate-af85908a/` and
`scratch/flatpak-buffer-review-20260907/`. Chromatic work proceeds separately;
this review candidate stays frozen.

**Candidate reported-view and runtime qualification, 2026-09-07:** the
uninstalled candidate now passes the unchanged Flatpak UI harness at both
exact owner views: ONE X2 44 checks, X4 Air 38, zero failures. Both actual
reported-view video regions are byte-identical to the archived earlier branch
Flatpak captures. This is two-frame regression evidence, not a new Studio
or whole-video verdict. Earlier X4 from-zero blank-baseline and warm-to-cold
copy/return failures remain recorded below; the exact-view pass does not waive
them. Evidence: `scratch/flatpak-app-path-20260907/`.

Separate audio-capable candidate observations at 2256x1504 on isolated 60 Hz
Weston outputs sustain 29.8–30.0 source fps on X4 and 29.8–30.2 on X2 after
startup, through source times 53.50/53.53 seconds. No drops, starvation, audio
underruns or audio drops are reported. Reported worst lateness is 34.4/34.3 ms
during these runs; first intervals are slower at 19.40/20.66 fps. Live binary
hashes, Radeon driver maps, 2256x1504 window buffers and null-sink routing are
authenticated.
Root viewed both final captures. These are runtime playback observations,
not physical A/V-sync, clean-exit or new 240-capacity evidence.

Installed stable and candidate binary/bundle hashes remain unchanged. The owner approved
trying the test build after the control-hold and seek-history behavior was
explained in plain language. The exact-view launcher has been handed over;
this is permission to test, not acceptance of the branch or shipping tradeoffs.
Owner results and required photometric fusion remain open; no install,
merge or release. Native source and executable are unchanged, so existing full
gates apply. CI 34085222013 at ed3b2966 passes all six jobs. Runtime receipts:
`scratch/candidate-desktop-load-20260907/`.

**CI toolchain drift, 2026-09-07:** the pushed native-readiness candidate
passes local Rust 1.97.1 gates, but CI's moving stable installs 1.98.1 and
fails on the new `chunks_exact_to_as_chunks` lint in existing metadata parsers.
The workflow now pins compiler, formatter and Clippy to the qualified 1.97.1
toolchain for both architectures and the dependency-free metadata check.
This makes verification reproducible without changing parser or player code;
it is not a claim that newer Clippy passes. CI run 34082652007 confirms both
architectures pass Clippy on that pin, then exposes seven L2 GPU tests that
unconditionally require an adapter on GPU-less runners. Their test-local setup
now skips only absent adapters when neither `KJERAG_REQUIRE_GPU` nor
`KJERAG_REQUIRE_RADV` is set. Device creation and shader/arithmetic failures
remain fatal. All seven skip explicitly without a GPU and fail as required
under either strict flag alone. On the real Radeon all eleven L2-module tests
execute and pass. Full strict-Radeon workspace with both clips passes 1,171
tests, zero failures, 30 ignored; full-target Clippy and static gates pass.
The native executable hash is unchanged. Evidence and test limits:
`scratch/gpu-ci-policy-20260907/`. No playback code changes in this repair.
CI run 34084410699 at 1c698c25 passes all six jobs, including x86 and ARM gates.

**Uninstalled candidate bundle, 2026-09-07:** clean archived source 95850005
builds and exports the native-readiness candidate into
`scratch/flatpak-candidate-95850005/kjerag-candidate.flatpak`. Candidate metadata
matches the installed app, and its version command runs in the Flatpak runtime.
Initial version/metadata checks are packaging smoke only. Subsequent actual
candidate playback uses `flatpak run --app-path` with identical installed
permissions and the unchanged UI harness, without replacing stable 80bbad8a.
An in-runtime hash proves which candidate executable runs. ONE X2 at the exact
reported view passes 44 checks, including backward seek, real scrubber movement
to unseen content and all paired-file arrival orders. X4 from-zero passes 34
of 36 checks: the toast baseline is blank below the header, and copy/return
reaches the same reported view but does not reproduce warm-history pixels.
Both failures and actual images remain recorded, not waived.

A bounded unchanged-assertion control compares two fresh exact X4 seeks:
their entire video areas are byte-identical, whereas the uninterrupted warm
picture differs from the first cold return. Together with the explicit facade
restart, this supports temporal-history reset as the cause in that sample,
not a universal source-identity or acceptable-quality verdict. No harness or
runtime fix is introduced. Audio is unavailable in these isolated sandbox
sessions; the final shader twin helper is native. This is not an installed
transaction, A/V-sync, new capacity or owner-acceptance gate. Full evidence:
`scratch/flatpak-app-path-20260907/`. Owner tradeoff approval and branch testing
remain outstanding; no install, main change or release.

**Timed draw-backpressure retry retained for qualification, 2026-09-06:**
the next candidate replaces immediate retry spinning specifically when both
draw-retirement slots are full with a 1 ms Scene deadline. The renderer sends
one initial wakeup, then honors the Scene's scheduled retry; primitives without
their own pending retry keep the immediate fallback. Shader-first preparation,
pre-acquire readiness, exact source ownership and the two-slot limit remain.
This is Kjerag scheduling, not new Studio arithmetic or reverse engineering.

Contemporary battery-powered native runs at 2256x1504, under a 12-second
changing-view pan on a 1000 Hz virtual output, improve X4 Air from 204 to
305 sourced draws/s and ONE X2 from 204 to 321. Both candidates sustain about
30 source advances/s versus about 23 in their controls. Candidate commit-period
p99 is 7.43/7.73 ms and maximum 15.32/15.79 ms respectively, so the averages
do not imply every draw meets the 4.17 ms budget. Every arm retains one UI-only
commit, consistent with the excluded COSMIC controls-relayout issue; strict
trace validation still fails that frame. No physical 240 Hz scanout is claimed.

Longer qualification distinguishes capacity from overload. X2 sustains about
30 source fps and 335 sourced draws/s for 40 seconds under the 1000 Hz stress
output. X4's 60-second run delivers 294 draws/s but only 27.87 source fps, with
4.27 seconds of growing lag; a tracing-disabled repeat also accumulates lag.
At 300 Hz virtual output, unchanged X4 instead sustains 262.07 actual sourced
draws/s and 29.925 source advances/s for 40 seconds at 2256x1504, with no growing
lag. Source-or-view changes account for 242.6 transitions/s, so duplicate poses
do not alone establish the 240 capacity result. Strict trace validation passes
at that operating point. Commit-period mean 3.816 ms, p99 10.090 ms, maximum
21.443 ms; 2649/10482 intervals exceed 4.167 ms. These are native rendering
capacity results, not physical scanout or an all-frame-latency guarantee.
No additional scheduler trial is justified by the 1000 Hz overload behavior alone.

The candidate passes both real-camera Full/recovery tests, two real-Radeon
renderer readiness/retry regressions, and the native ONE X2 UI harness
(51 checks, zero failures). Required-Radeon full workspace: 1171 passed,
zero failures, 30 ignored; both camera inputs and quiet audio. Full-target
clippy, formatting, name, Cargo-source and diff checks pass. One initial X2
scripted quit failed; unchanged repeats and the UI harness exit normally.
Owner branch acceptance remains pending, including retaining controls with
the previous complete window during backpressure. The installed Flatpak is
unchanged. Full results, limits and raw runs:
`scratch/native-retry-timer-20260906/`. This supersedes the preflight-only
assessment below, which remains historical evidence.

**Native draw-backpressure fix in progress, 2026-09-06:** an actual-window
high-refresh trace reproduced hundreds of cleared video frames when both
draw-retirement slots were occupied. The branch candidate checks video
readiness before acquiring a window buffer, retaining the previous window
on retry without changing source ownership, the two-slot limit or shaders.
An initial acquire-then-discard prototype stalled and is replaced. The revised
candidate completes native runs on both cameras, and 60 Hz runs recover
29.8–30 source fps after startup with no audio underruns. It is **not ready to
ship**: X4 still delivers only 25–27 source fps under high-refresh load, retry
traffic is excessive, and full candidate gates/owner acceptance remain open.
One empty controls-relayout frame remains, consistent with the separately
documented COSMIC issue the owner excluded. Installed Flatpak stays frozen.
Evidence and test limits: `scratch/native-capacity-20260906/`.

The next candidate prepares custom primitives before built-in UI batches, so
refused window attempts do not stage or submit UI work. Both real-camera Scene
readiness tests and an actual-Radeon renderer regression pass. The latter
checks zero UI preparations/submissions on refusal, once-only preparation,
recovery, clipping and ungated offscreen rendering. Native trials do **not**
establish a retained speedup: the contemporary X4 pair advances 25.42 versus
26.00 sources/s but loses sourced redraws and worsens completion tails. Both
arms are on battery; comparisons against earlier AC runs are confounded, and
X2 shows substantial unexplained variability. Tracing-disabled X4 controls
remain below 30 source fps. No 240 fps acceptance or installed update follows.
The callback-association review does not justify a wgpu extension as a speed
fix; that proposal is held. Details: `scratch/exact-draw-retirement-20260906/`.

**View-cache and preencoding trials declined, 2026-09-06:** a lazy source-owned
cache removed four texture-view creations per repeated-source draw but did not
produce a repeatable shared-camera gain. Seven direct-render tests and both
actual-camera Scene routes passed; X4 redraw p99 worsened in both alternating
pairs, with mixed ONE X2 results. Full evidence:
`scratch/source-view-cache-20260906/`.

A separate warm-only experiment prepared stages 2–13 before dispatch, retaining
the source-owning initial submission, each subsequent exact submission index,
command order and the existing five L1 callback pacing boundaries. Independent
source review found no intermediate CPU readback dependency or host-upload
overwrite hazard. Two new required-Radeon ownership/order tests and both
actual-camera cold/warm Scene routes passed. All eight capacity arms retained
360 sources with zero reported accounting failures. X4 throughput rose slightly
and over-budget redraws fell, but p99 worsened from 9.62/10.35 to 10.71/10.93 ms.
ONE X2 throughput fell and over-budget counts rose in both pairs, with mixed
tails. No shared-camera optimization is retained. Full evidence and review/test
limits: `scratch/preencoded-worker-20260906/` and
`scratch/deferred-worker-feasibility-20260906/`.

Both prototypes and their tests are fully removed; restored source and normal
binaries match the prior verified checkpoint. Formatting, name, Cargo-source
and diff checks pass; the prior identical-source full suite remains applicable.
No candidate full suite, pixel sequence, native run or installed update is
claimed. Next, verify the smallest actual-native changing-view capacity test:
the owner requires 240 fps active rendering capacity with spikes reported,
not an independently added no-spikes diagnostic-p99 shipping gate. Existing
offscreen averages above 240 do not alone establish complete native-app capacity.

**Demand-aware submission trial declined, 2026-09-06:** an admission-only
candidate gave an announced view request priority before the next worker
submission, with one owed worker turn to prevent starvation between adjacent
view scopes. A small vendored renderer hook held scopes through the actual
host submit, not GPU completion. Session-owned gates covered renderer reuse;
no shader, source ownership or completion-proof semantics changed.

Four gate tests, one vendor scope test, both actual-camera cold/warm Scene
routes, renderer recreation and worker-context checks passed after two test
compile corrections. Two alternating capacity pairs per camera retained all
360 source changes and zero reported drops, starvation or audio underruns.
X4 redraw p99 worsened from 9.29/10.15 to 10.63/10.93 ms, with lower throughput
and worse source arrival in both pairs. X2 was mixed, with worse arrival p99
in both. The candidate and temporary instrument are fully removed; restored
source and normal binaries match the prior verified checkpoint. No native
candidate run, candidate full suite, pixel gate or installed update is claimed.
The existing source-identical full gates still apply; formatting, name,
Cargo-source and diff checks pass after restoration. Evidence and limitations:
`scratch/demand-admission-20260906/`. This is a rejected policy, not a speedup
or proof that every priority-scheduling design must fail.

**Installed desktop-sized playback observed, 2026-09-06:** the unchanged
`80bbad8a...` repair now has an actual-window observation at 2256x1504 rather
than only the earlier 1280x720 harness size. Isolated Weston 13 reports a
60.000 Hz output and a full-size Kjerag dmabuf buffer. Weston uses AMD radeonsi;
the installed player is restricted to Radeon Vulkan and its live driver maps
confirm it. From-zero X4 playback reports 29.8–30.2 fps after startup through
58.49 s, with cumulative worst lateness 97.1 ms. X2 holds the same range
through 63.54 s, worst 100.1 ms. Both report zero drops, starvation and audio
underruns, with real audio routed to the verified null sink.

Root inspected the full-size captured output. Screenshots/scene inspection
can perturb adjacent intervals, and these runs intentionally end with SIGTERM.
They are not clean-exit or interactive UI gates, physical A/V-sync proof,
ordinary COSMIC desktop acceptance, or 240 fps capacity evidence. Failed nested
Cage setup and an initial Flatpak proxy-socket failure are preserved separately;
neither reached playback. No app, renderer, installed or native binary changed.
The prior source-identical full gates remain the code verification; no new
source suite was needed for these observations. Evidence:
`scratch/desktop-load-20260906/`.

The existing Image Fusion ON/OFF pair also received a bounded offline frame
comparison, with no new Studio export. Exact captured source-frame-34538
geometry, rather than an image-fitted rotation, favors output frame 0 over
frames 1 and 2 in both arms. This is candidate agreement, not proof of the
export consumer's source identity or a constant frame offset. The initial
fitted-rotation comparison was circular, and the OFF encode shares ON geometry;
neither is promoted to independent authentication. No correction coefficients,
field, temporal rule or renderer implementation follows from this result.
Full raw receipt, script and limits: `scratch/x4-fusion-association-20260906-01/`.

**Exact-completion scheduling trials declined, 2026-09-06:** an instrument-only
comparison now qualifies zero-GPU-timeout polling of the draw's exact returned
submission index. An earlier completed index is distinguishable from later
pending queue work, and concurrent submission progresses. The same 2560x1440
two-pair-per-camera comparison still has substantial redraw tails; conservative
queue-prefix callbacks do not by themselves explain the remaining delays.
Zero GPU timeout does not bound CPU lock, maintenance or wakeup time. Evidence:
`scratch/exact-completion-20260906/`.

Using that same exact-draw instrument on both arms, pacing every worker stage
against its own predecessor improves X4 redraw p99 from 10.70/9.90 to
5.67/5.84 ms, but delivers only 355/347 sources instead of 360 and raises
arrival p99 to 198/451 ms. X2 retains 360 sources but loses throughput and
worsens redraw/arrival tails. A second trial changes only the five existing
L1 pacing checks from queue-prefix callbacks to exact-index polls, with no new
pacing points. It retains 360 sources in every arm but worsens X4 redraw p99
in both pairs, from 10.64/8.51 to 11.03/10.88 ms, with mixed X2 results.
Neither policy is retained. Required-Radeon focused completion and actual
cold/warm Scene-route tests passed for both trials. The first all-stage compile
failed after a used provenance helper was mistakenly removed; it was restored
unchanged before any measured run. No candidate real-image, native-window or
full-workspace acceptance is claimed. Evidence:
`scratch/exact-predecessor-worker-20260906/` and
`scratch/exact-l1-completion-20260906/`.

All trial runtime, test and temporary instrument changes are removed; archived
patches and executables preserve the experiments. Native and normal capacity
binaries match the prior verified checkpoint. The installed repair remains
unchanged for the owner's ordinary-desktop retest. This checkpoint adds no
runtime speedup and does not satisfy the sustained 4.17 ms target. Further
blanket pacing or polling substitutions are not justified by these results.

After removal, the restored required-Radeon workspace passes 1,165 tests with
zero failures and 30 ignored, using both real camera inputs and quiet audio.
Formatting, full-target clippy, name, Cargo-source and diff checks pass.
No native-window rerun or new owner acceptance is implied by these restored
source gates.

**Current worker/view timing, 2026-09-06:** a temporary probe now covers the
actual resident worker's thirteen warm submissions and the exact staged source
consumed by each view draw. One 2560x1440 cohort per camera authenticates 360
displayed successors plus one explicitly drained successor. X4 view-pass
timestamp p99 is 1.77 ms versus 10.13 ms completed-redraw wall time; ONE X2 is
1.73 versus 7.40 ms. All measured view intervals stay below 3.3 ms, including
the paused phases. Every over-budget playing redraw (612 X4, 362 X2) has a
sub-budget view interval. This points to queue/host scheduling outside those
view boundaries, not a need to change the image filter. The Vulkan timestamps
are bottom-of-pipe intervals, not exclusive shader busy time or native-display
latency. Repeated-source redraws also retain substantial tails.

The first attempt stopped before measurement because an older diagnostic
described the inactive legacy draw path. The corrected probe reads the actual
resident draw capability; ordinal, source, count and timestamp checks pass.
All temporary probe code is removed and the normal measured capacity binary
is restored. No installed/native player, image arithmetic or broad RE changed.
Evidence: `scratch/current-worker-timing-20260906/`.

One warm L2-entry queue-prefix wait was then tested, distinct from the previously
declined every-stage waits and five-chunk L2 pacing. Both actual-camera Scene
route checks pass. Two alternating capacity pairs per camera improve redraw
p95/p99 in all four pairs, but X4 over-budget redraws increase from 570/584 to
679/655 and picture-arrival p99 worsens from 11.90/13.97 to 27.96/23.06 ms.
ONE X2 over-budget counts improve from 387/387 to 226/210, with mixed arrival
statistics. All eight arms retain 360 source changes and zero accounting
failures. The candidate is declined and fully removed, including its trial-only
counters/assertions. These prove successful route admission, not mutation-tested
waiting at that call site. No candidate actual-image, native-window or full-suite
gate is claimed. Evidence: `scratch/warm-l2-admission-20260906/`.
The next decision concerns coordination of worker and view submissions; neither
another shader micro-optimization nor another blanket wait policy is justified
by this checkpoint. The installed repair remains frozen for owner desktop retest.

**Parent square-root certificate declined, 2026-09-06:** a native estimate
accepted only by an exact integer rounding certificate, with the original
restoring fallback, passes eight required-Radeon parent tests. Two alternating
capacity pairs per camera do not justify retention: both X4 pairs worsen
throughput and redraw tails; ONE X2 is mixed with p99 worse in both pairs.
All eight arms retain 360 source changes and zero drops, starvation or audio
underruns. Candidate code and tests are removed; no candidate real-image,
native-window or full-workspace gate is claimed. Evidence:
`scratch/parent-sqrt-certificate-20260906/`.

After both trial removals, all production/test source matches the prior
checkpoint. The restored required-Radeon workspace passes 1,165 tests with zero
failures and 30 ignored, using both real camera inputs and quiet audio.
Formatting, full-target clippy, name, Cargo-source and diff checks pass. Native
and normal capacity binary hashes are unchanged. This is evidence and a narrowed
next decision, not a retained runtime speedup or a new native-window acceptance.

**Bounded persistent PIS ranges declined, 2026-09-06:** a separately compiled
128-lane L1 entry coalesced 47 diagonal dispatches into seven while preserving
the six existing worker command buffers, five callback waits, exact shared
arithmetic and stripe boundaries. Nine required-Radeon PIS tests pass, including
range-start/count and stripe-boundary mutations. Two actual Scene tests prove
the new dispatch-encoding route together with successful cold/warm worker
submissions on both reported cameras; paused redraw adds no stitch work.

Two alternating 2560x1440 baseline/candidate pairs per camera do not justify
retention: X4 redraw p99 worsens from 9.86/10.52 to 10.54/10.77 ms despite
fewer over-budget redraws, and X2 results are mixed (second p99 6.89 to 7.70 ms,
over-budget count 378 to 405). All eight runs advance 360 source frames with
zero reported drops, starvation and audio underruns. The candidate and its
trial-only counters/tests are removed. The existing successful worker-submission
regression is now also run on X4 Air, sharing its assertions with ONE X2.
No candidate real-image sequence, native UI or full-workspace gate is claimed.
Native and installed players remain unchanged. Evidence, source patch and
candidate executable: `scratch/bounded-pis-ranges-20260906/`.
After removal, the restored workspace passes 1,165 tests with zero failures
and 30 ignored, including both real camera inputs and quiet audio. Full-target
clippy, formatting, name and Cargo-source checks pass. No native-window rerun
was needed for the retained test-only change; no new visual claim is made.

**Bounded draw-box loops declined, 2026-09-06:** the unchanged 1.788-source-unit
box filter was tested with a maximum of two loop iterations per axis, retaining
the original early breaks, sampling and accumulation order. An independent
frozen old-loop shader agrees bit-for-bit across the atlas grid, and named
one/two-tap threshold phases discriminate a one-tap mutation on Radeon.
Two alternating 2560x1440 pairs per camera show no dependable gain: X4 redraw
p99 worsens from 9.38/10.09 to 10.50/10.17 ms; X2 throughput falls slightly in
both pairs with mixed p99 and over-budget counts. All eight runs advance
360 source frames with no reported drops, starvation or audio underruns.
Production changes and trial-only tests are removed; normal capacity and
native executables again match the verified separable-mask checkpoint. No new
actual-sequence, native-window or full-workspace gate ran for this rejected
candidate. Installed Flatpak remains unchanged. Evidence:
`scratch/bounded-draw-box-20260906/`.

A separate literal native-window high-refresh feasibility check did not reach
the player. A locally unpacked `wlr-randr` requested a 2560x1440, 240 Hz mode
from an isolated headless Cage session; Cage aborted in
`wlr_scene_output_layout_add_output` while applying the mode. No 240 Hz output,
player capacity, desktop-mode change or system-package installation is claimed.
The compositor is not being patched as part of this checkpoint. Evidence:
`scratch/native-high-refresh-20260906/`.

**Ordered PIS accumulator lanes declined, 2026-09-06:** four lanes executed
the four independent descent accumulations with their original row-major
operation order, followed by an extra barrier and unchanged final correction.
All eight required-Radeon PIS tests pass, including deliberate component,
survivor-count and recurrence-order mutations. Two alternating 2560x1440
pairs per camera do not justify retaining it: X4 redraw p99 improves from
10.75/10.84 to 9.35/8.66 ms with mixed throughput, but X2 loses throughput and
has more over-budget redraws in both pairs (371/365 to 433/434). Its p99 is
mixed. All eight runs advance 360 consecutive source frames with zero reported
drops, starvation or audio underruns. Both source files and trial-only tests
are restored; native and installed players are unchanged. No actual-sequence,
native-window or full-workspace gate is claimed for the rejected candidate.
Evidence: `scratch/pis-ordered-accumulators-20260906/`.

After both removals, the restored runtime passes the full required-GPU workspace
suite again: 1,164 passed, zero failed, 30 ignored, with both real camera inputs
and quiet audio. Full-target clippy, formatting, name and Cargo-source checks
pass. Native and capacity binary hashes still match the separable-mask checkpoint;
the installed `80bbad8a...` repair remains frozen for the owner's desktop retest.

**Separable bilateral mask verified, 2026-09-06:** the
geometry shader factors its 9-by-9 validity conjunction into a nine-column
horizontal scan and nine-row packed-word AND. Camera support, retained maps,
public A/B mask words and sentinel remain unchanged. One extra compute pass
uses a private 64,800-byte tail on the existing allocation, with no extra
buffer or binding. The downstream guard validates its real enlarged size;
it does not disguise that size or loosen validation. Five required Radeon
geometry tests pass, including exact CPU-oracle output and deliberate mutations
of both radii, bilateral validity, packing, vertical AND and byte lanes.

Two alternating 2560x1440 pairs per camera improve throughput and redraw
p95/p99 in each pair. X4 capacity changes from 310–315 to 316–319 redraws/s,
with p99 from 11.14–13.84 to 10.21–10.84 ms. X2 changes from about 381 to
387 redraws/s, with p99 from 7.73–8.01 to 6.93–7.49 ms. All eight runs retain
360 source changes. Not every measure improves: X4's second over-budget count
rises from 588 to 624, and its arrival statistics worsen in that pair; X2
arrival medians, p95 and maxima worsen in both pairs despite better arrival
p99. This is a modest capacity/tail improvement, not a universal latency win,
ordinary-desktop acceptance or a 4.17 ms pass. Required-GPU workspace tests
with both real camera fixtures pass: 1,164 passed, zero failed, 30 ignored.
Full-target clippy, formatting, name and Cargo-source checks pass. All 276
real Scene X2/X4 image/map/alpha artifacts are byte-identical to the import
cleanup checkpoint. Root inspected both final sequence frames and the native
exact-view X4 capture. The saved native candidate passes 48 UI checks with
zero failures; four paired-file checks are inapplicable to this X4 capture.
Installed Flatpak remains frozen for the owner's retest; no merge, release
or performance compromise is accepted. Evidence: `scratch/separable-mask-20260906/`.

**PIS reciprocal broadcast declined, 2026-09-06:** one shared exact division
per patch-owned workgroup replaced duplicate lane/descent divisions, at the
cost of one new barrier. Required Radeon PIS checks pass, but two alternating
2560x1440 pairs per camera do not justify retaining it. X2 p99 improves from
7.26/7.93 to 6.22/7.06 ms; X4 changes from 9.81/11.01 to 11.24/10.74 ms, with
capacity lower in its first pair and nearly unchanged in its second. All
runs retain 360 source changes. The shader and trial-only tests are removed;
native and installed players are unchanged. No capacity or pixel-identity
claim follows. Evidence: `scratch/pis-reciprocal-broadcast-20260906/`.

**Import-time draw binding removed and verified, 2026-09-06:** the
selected source owner no longer allocates an unused view-uniform buffer,
queues its upload, creates four texture views, or retains a picture bind group
at import. Actual window and screenshot passes already create their own
immutable bindings; that path is unchanged. The duplicate session sampler,
retained layout handle, dead source draw/write methods, and obsolete test
fixture are removed. Exact imported planes-before-decoder-frame drop order
and immutable first/second/screenshot pass coverage remain. Two alternating
capacity pairs per camera do not establish a speedup: X4 p99 is slightly
higher in both pairs, while X2 throughput improves modestly and tails are
mixed. This is removal of proven unused work, not a speedup or 240 fps claim.
Required-GPU workspace tests with both camera fixtures pass: 1,163 passed,
zero failed, 30 ignored. Full-target clippy, formatting, name and Cargo-source
checks pass. All 276 real Scene X2/X4 image, packed-map and alpha artifacts
are byte-identical to the previous callback checkpoint. Root inspected both
final sequence frames and the exact-view native X4 window capture. The saved
native candidate passes 48 UI checks with zero failures; four paired-file
checks are inapplicable to this X4 capture. The installed Flatpak is
deliberately unchanged for the owner's retest, and no new performance or
visible compromise is accepted. Evidence:
`scratch/import-binding-cleanup-20260906/`.

**Worker-wide stage admission declined, 2026-09-06:** a separate experiment
waited for the previous queue prefix before every worker stage/chunk, instead
of only between L1 chunks. It preserved shader arithmetic and used the current
nonblocking 100-microsecond callback polling, with no duplicate L1 waits.
Two alternating pairs per camera reject the policy: X4 redraw p99 improves
from 10.42–10.50 ms to 5.15–5.55 ms, but source cadence falls to 28.66–28.92 fps,
only 344/347 frames arrive versus 360, and arrival lateness reaches
445–547 ms. X2 retains source cadence but redraw capacity falls from about
388 to 358–359/s and p99 rises from 7.08–7.51 to 11.17–11.47 ms. The extra
waits are removed, and the cleanup-only source patch matches its saved
pre-experiment snapshot exactly. No installed build, visible compromise or
performance acceptance changed. Evidence: `scratch/worker-stage-admission-20260906/`.

**Sustained installed playback follow-up, 2026-09-06:** the unchanged
`80bbad8a...` Flatpak also completes a from-zero ONE X2 run through the end
of the 273.7-second clip. Post-start, pre-EOF five-second samples are
29.80–30.21 fps; worst reported source lateness is 54.5 ms, with zero audio
underruns. The player stops its clock at 273.67 seconds and exits normally.
Root inspected the retained final picture. A separate 55-second installed
X2 run reaches 68.1 ms worst lateness; the previous 739.4 ms native maximum
did not recur in these two installed runs. These isolated 1280x720 results
do not prove physical A/V synchronization or ordinary COSMIC presentation.
No X2-specific source change is justified by the earlier single native spike.

The current-source offscreen capacity checkpoint at 2560x1440 still misses
the tail budget: X4 363.24 redraws/s with p99 7.61 ms; X2 399.40 with p99
6.83 ms. Both advance 360 consecutive source frames. Paused redraw p99 is
3.05/3.15 ms. The current diagnostic observes queue-prefix completion, which
can include later concurrent worker submissions; it is not physical display
latency. No 240 fps / 4.17 ms completion is claimed. Evidence:
`scratch/flatpak-playback-defect-20260906/` and
`scratch/capacity-after-playback-repair-20260906/`.

**Owner-reported playback failure, 2026-09-06:** the owner likes the sampled
clip appearance but reports actual installed X4 playback at only 17–19 fps
with seconds of increasing video lateness. This takes priority over photometric
follow-on and further draw-kernel trials. From-zero, real-audio runs of both
the unchanged native and installed players reproduce initial video debt and
continuing lag in an isolated 1280x720 compositor. A normal-desktop run also
slowed severely, but its visibility state was not authenticated; the exact
sustained desktop condition remains under investigation.

Source inspection identifies one definite startup defect: the running clock
anchors on decoded frame zero before the first resident map is installed.
The sequential policy neither reanchors nor skips late source frames, so it
needs surplus processing capacity to repay that initial debt. Stitch-wait time
also bypasses the current drop/starvation counters; the reported sound offset
is against the media clock, not the late displayed picture. Earlier UI and
offscreen capacity passes therefore do not establish usable actual playback.
The startup regression fails before the change on both reported clips and
passes afterward, including pause-before-install and exact acknowledgement.
Reusing the existing landing hold removes the X4 native test's initial underrun
burst (98 to zero) and most initial video debt, but alone still leaves roughly
29.4 fps and increasing lag. A second scheduling-only change shortens the
worker's callback-poll fallback from one millisecond to 100 microseconds;
uncapped diagnostics had driven callbacks more frequently from their own loop.
The bounded continuous native X4 comparison with both changes records
29.80–30.20 fps after startup and worst reported source lateness of 33.2 ms
through 53.55 seconds, with zero audio underruns. ONE X2 improves from
2,960.3 ms worst lateness with the one-millisecond control to 739.4 ms with
100 microseconds, but still has timing spikes. Both X4 native trial sessions
produced their full timing records but hit cage exit 134 during forced teardown;
they are not clean-exit passes. The X2 control and candidate exit normally.

The final installed Flatpak (`80bbad8a...`, executable SHA256 `1d3c1139...`)
also completes the exact reported X4 portal-path run with a normal exit.
At 1280x720 in isolated cage, its post-start samples are 29.80–30.01 fps,
worst reported source lateness stays at 21.9 ms through 48.50 seconds,
and there are zero audio underruns. The previous installed portal run reached
6,365.3 ms worst lateness. This is bounded installed-player evidence, not
ordinary COSMIC-desktop acceptance, displayed A/V acceptance, or the 4.17 ms
capacity gate. More frequent CPU wakeups have not yet been costed.

An immediate step after an already-installed startup landing retires the hold
before selecting the existing warm continuation; its strict capture-identity
regression is retained. Empty clean EOF also retires an unlanded startup hold.
Required-GPU workspace tests with both reported camera fixtures pass: 1,163
passed, zero failed, 30 ignored. Formatting, workspace clippy, name check and
Cargo-source consistency pass. The installed binary matches the build output,
all tracked source hashes match its recorded inputs, and permissions metadata
is unchanged. Full installed UI checks pass: X4 39 and X2 45, zero failures,
including exact views, X2 backward seek and real scrubber drag, paired-file
arrivals, and real quiet audio controls. Both captures were inspected.
The stuck-import injection group skips inside Flatpak; X4 additionally skips
four inapplicable paired-file checks. The final Rust/shader twin helper runs
natively, not inside the installed resident solver. Evidence:
`scratch/flatpak-playback-defect-20260906/`, issue #184. Owner retest remains
required before merge.

**Prior installed-bundle check, before playback repair, 2026-09-06:** the shared-camera source
now has a fresh local Flatpak build and two real-camera UI runs. Every tracked
file copied into the build matched the recorded branch inputs. The manifest
now also excludes `.target`, avoiding an unrelated 1.7 GB cache in that copy;
runtime, dependencies and permissions are unchanged. The old installed app
was preserved as a rollback bundle. The new installed commit is
`5b23a6cea552df831bb0db1a962a02e1b8742957c16f04628d1488a72aa7180a`.
Both exact reported views pass: X2 44 checks and X4 38, zero failures, including
X2's backward seek, real scrubber drag and paired-file arrivals. The isolated
bundle sessions played silently, so the sound-dependent check skipped; the
injected stuck-import group also skips in Flatpak mode. X4 additionally skips
four inapplicable paired-file checks. The harness's final Rust/WGSL helper is
native, not an installed resident-solver gate.

Root inspected both bundle captures and the prior native X4 view. Their
structure looks closely similar in those samples, but the X4 picture is not
byte-identical: most changed channels differ by one code, with 28 picture
pixels exceeding four and a largest difference of 27. No cause, invisibility
or parity is inferred. Mixed UI-run timing logs include slow initial intervals
and later 29.80/30.00 fps source presentation; they are not a capacity pass.
Repository gates pass 1,158 tests, zero failures, 30 ignored, full-target
clippy, formatting, name and Cargo-source checks. Native executables remain
unchanged. Evidence and the rollback bundle are in
`scratch/flatpak-current-20260906-01/`. This test build is installed locally,
not released or merged. Sound synchronization in the bundle, 4.17 ms active
redraw tails, photometric implementation and owner acceptance remain open.

**Image Fusion comparison available, 2026-09-06:** the required photometric
follow-on now has one isolated X4 Studio 6.0.2 OFF export paired with the
existing ON reference. Full project JSON differs only by Image Fusion and
save time; all 102 video packet timestamps, durations and keyframe flags match,
as do the resolution, rate and export color metadata. Original ON settings were
restored and verified. Root inspected both arms at output frames 0, 31 and 101,
plus the whole sphere at 31: the reported view differs subtly in color without
an obvious shape change. Magnified signed differences show smooth regions of
opposite color adjustment, consistent with spatial lens matching. Separate
lossy encodes mean those differences are not the actual per-lens coefficients.
The reference's exact camera-source frame association remains pending.
`docs/research/studio-image-fusion-oracle-602.json` seals the artifacts and limits.
This is evidence for issue #185, not an implemented feature or a new stitching
acceptance. The verified player is unchanged; old unaligned color estimation
will not be silently reused. No additional export or broad optimization RE is
needed to inspect this pair.

**Lazy draw-filter fallback declined, 2026-09-06:** replacing the final eager
selection with conditional fallback sampling passed a frozen-old-shader
bit-exact check across the atlas interiors, join and clamp edges. Manual
interpolation coverage and the shader layout guard also passed. The tiny
shader change did not demonstrate a dependable cross-camera capacity benefit:
both X2 pairs improved modestly, but X4's first candidate run regressed sharply
and its second improved throughput while slightly worsening p95/p99. Paused
X4 throughput was lower in both pairs. Logical removal of a texture-sampling
call was not proof of hardware savings, and no cause is assigned to the large
first-run difference. The shader change and its candidate-only frozen reference
are removed. The normal capacity binary again matches the retained checkpoint;
the native player was never changed. No actual-sequence, native or full-suite
gate is claimed for this rejected candidate. Evidence is preserved in
`scratch/gpu-box-lazy-fallback-20260906-01/`.

**L2 callback pacing declined, 2026-09-06:** the earlier-level solve was tested
with the same callback chunking as L1. The real Scene regression observed 15
cold L2 submissions, none on a cached redraw and five additional warm
submissions, alongside unchanged L1 counts. Two alternating pairs per camera
used the corrected nonblocking completion diagnostic. Both X4 runs and the
first X2 run lost throughput and sharply worsened redraw tails; the second X2
run improved substantially (432.96 redraws/s, p99 4.008 ms versus 417.41 and
5.392 ms). Picture-arrival p99 worsened in all four pairs. This does not
establish a reliable cross-camera benefit, and the cause of the bimodal X2
result is not identified. All 360 source changes and zero accounting failures
were retained in every run. The L2 change and its trial-only counter coverage
are removed; L1 callback pacing remains. The normal capacity binary was rebuilt
and its hash matches the prior checkpoint. The native player was never rebuilt
with this trial and remains unchanged. No full suite, native or pixel gate is
claimed for the rejected candidate. Evidence remains in
`scratch/gpu-callback-l2-20260906-01/`.

**Callback-paced worker candidate, 2026-09-06:** pinned wgpu holds a fence read
lock throughout `Device::poll(Wait)`, while every `Queue::submit` needs the
same fence's write lock. This explains how an off-thread wait can block UI
submission. The first callback-paced trial still regressed under `view-rate`;
the instrument itself used that blocking wait after each draw, preventing
later worker chunks from submitting. Those old measurements remain raw
instrument observations and cannot isolate native scheduling behavior.

The diagnostic now observes queue-prefix completion with nonblocking polling
and 100-microsecond receive timeouts, keeping one redraw in flight and including
polling/wakeup costs. Both comparison arms were rebuilt with this measurement.
The selected resident L1 candidate uses the same sixteen-row solver, shader
arithmetic and dispatch order in six chunks, with five worker-only callback
waits. Each wait retains the exact submission lease and decoded source;
nonblocking fallback polling ensures progress without the UI. There is no
dependency change or new Studio export.

Two alternating 2560x1440 pairs per camera favor candidate throughput and
redraw p95/p99/maximum in all four pairs. X4 baseline/candidate capacity is
335–342 / 349–364 redraws/s and p99 is 9.57–9.78 / 7.85–8.28 ms. ONE X2 is
392–393 / 393–408 redraws/s and p99 is 6.64–7.10 / 6.05–6.27 ms. X2 arrival
p99 worsens in both pairs (7.60 to 8.77 and 6.55 to 7.10 ms), and its second
arrival median worsens (4.23 to 4.44 ms); do not call it a universal latency
improvement. Every arm retains 360 source changes with zero drops, starvation
and audio underruns. These are completed offscreen redraws, not compositor
presentation; the 4.17 ms target remains unmet.

Focused callback ownership/no-UI-progress and real Scene worker tests pass;
the latter observes 18 cold, unchanged cached and six additional warm chunk
submissions. Final gates pass 1,158 workspace tests with 30 ignored, full-target
clippy, formatting, name and Cargo-source checks. All 276 real Scene X2/X4
PPM/map/alpha artifacts are byte-identical to the preceding cleanup checkpoint;
root inspected both final frames. The rebuilt native player passes 48 window
checks with zero failures and four inapplicable paired-file drop checks. Root
inspected its exact-view X4 capture, also byte-identical to that checkpoint.
The prior native and harness remain preserved. This callback candidate is
retained on the working branch, not accepted by the owner or merged.
Evidence is in `scratch/gpu-callback-l1-20260906-01/` (old
blocking instrument) and `scratch/gpu-callback-capacity-20260906-02/` (rebuilt
controls). Owner branch acceptance and main are unchanged.

**Exact map-corner cache trial declined, 2026-09-06:** moving four identical
retained-map loads outside each 3x3 sampling loop passed exhaustive coordinate
invariants, GPU/CPU qualification and cached-corner/FMA mutation checks. It did
not establish a capacity benefit: both X4 pairs lost throughput and worsened
p99/over-budget counts, while X2's two pairs disagreed. The shader change and
its candidate-only tests are removed; logical read counts were not measured
hardware traffic. The exact patch, four passing focused tests and eight
capacity runs remain in `scratch/gpu-belt-corner-cache-20260906-01/`. No actual
Scene pixel sequence, native UI or full-workspace gate is claimed for this
rejected candidate. The native player remains unchanged.

**Eight-row propagation trial declined, 2026-09-06:** the existing parameterized
solver was tested at eight instead of sixteen rows, with GPU/CPU qualification
and cross-boundary mutation checks for both sizes. Two alternating capacity
pairs per camera reduced redraw p95/p99 and over-budget counts, but average
throughput was mixed and worst redraw and arrival p99 worsened in three of
four pairs. All 360 source changes and zero counters were retained in every
run. Both actual Scene cohorts were rendered: all 31 X4 and 61 X2 pictures
and maps changed, while alpha remained exact. Root inspected baseline/candidate
X4 frame 34572 and X2 frame 6374, finding a more pronounced double edge around
the latter's upper left riser connector. The modest, mixed timing gains do not
justify taking that extra visible change. Production remains sixteen rows;
eight-row reference/mutation coverage is retained. No native build or full
workspace gate is claimed for the rejected candidate. Evidence remains in
`scratch/gpu-stripe8-20260906-01/`; the verified native player is unchanged.

**Scheduling verification correction, 2026-09-06:** the earlier L1 pacing
change was on the diagnostic CPU-grid entry, not resident playback's
`GpuL2BridgeOutput::submit_l1_pis`. The worker and due-frame lookahead are
active, but playback did not use the claimed six paced L1 commands. The
L1-only paced/unpaced and batch-size comparisons therefore did not exercise
their intended controls; their causal conclusions are withdrawn. Raw timings
and the worker/lookahead pixel and lifecycle checks remain recorded, without
crediting L1 pacing.

The audit also mistakenly used `objcopy --dump-section` without an output ELF
path, rewriting two archived capacity executables and the build target. The
original worker/lookahead baseline survives intact in the L1-paced-named trial
directory; no intact duplicate of the original batch-four executable was
found. Raw logs, source patches, pixel artifacts and the native player were
unaffected. Original versus rewritten hashes are recorded in
`scratch/gpu-resident-l1-pacing-20260906-01/artifact-audit.md`; archived binary
references below must be read with that correction.

The subsequent actual resident-path trial is also rejected, now with a real
Scene test proving 18 cold and six additional warm paced submissions. Two
2560x1440 baseline/candidate pairs per camera lost throughput and worsened
p95/p99 redraw time: X4 averaged 331–347 / 304–305 redraws/s with p99
6.72–7.03 / 15.77–16.95 ms; X2 averaged 393–406 / 307–308 with p99
5.29–5.44 / 13.97–14.15 ms. Median first-draw arrival lateness rose from
4.0–4.7 ms to 23.3–25.7 ms. X4 had fewer over-budget draws despite worse
tails; X2 had more. All eight arms retained 360 source changes and zero
drop/starvation/audio-underrun counts. No pixel, native-window or full-workspace
gate is claimed for this rejected scheduling candidate. Its patch, binaries,
test and raw measurements remain in
`scratch/gpu-resident-l1-pacing-20260906-01/`.

Pacing-only code is removed. The worker and lookahead remain, with an
actual Scene regression for ordinary cold/warm worker submissions and no
extra work on cached redraws. Final cleanup gates pass 1,157 workspace tests
with 30 ignored, full-target clippy, formatting, name and Cargo-source checks.
Seven pacing-only tests were removed with their retired implementation, and
one actual Scene worker-routing regression was added. All 276 X2/X4
PPM/map/alpha artifacts remain byte-identical to the worker/lookahead checkpoint;
root inspected both final frames. The rebuilt native player passes 48 window
checks with zero failures and four inapplicable paired-file checks. Root
inspected its exact-view X4 capture, also byte-identical to the preceding
capture. The prior native binary and harness are preserved. Evidence is in
`scratch/gpu-resident-l1-cleanup-20260906-01/`. This is verified cleanup and
corrected attribution, not a performance improvement or owner acceptance;
4.17 ms frame-time compliance remains unmet.

**Further performance trials, 2026-09-06:** the eight-to-four L1 batch trial
was ineffective on playback, as corrected above. A separate exact
separable-mask prototype passed all five
geometry tests, including new radius/packed-lane mutations, but showed no
repeatable capacity benefit for its extra pass and 64,800-byte buffer; X2
throughput and redraw p95/p99 regressed in both pairs. Both prototypes are
removed. Their eight-run comparisons, source patches and binaries remain in
ignored `scratch/gpu-lookahead-batch4-20260906-01/` and
`scratch/gpu-mask-separable-20260906-01/`. The verified worker/lookahead native
player is unchanged. No new Studio export or RE, or inherited native/pixel
verification for the rejected prototypes, is claimed. The 4.17 ms target
remains unmet.

Read-only counters with all Kjerag jobs stopped also found substantial desktop
GPU work: the compositor's graphics counter advanced about 2.592 seconds over
5.024 wall seconds, while overall GPU-busy readings were 55–56 percent. No
applications or settings were changed. This is shared-load context, not a
diagnosed COSMIC bug, an attribution of benchmark stalls, or grounds to accept
the rejected candidates. The owner does not want known COSMIC issues fixed.

**Bounded worker and due-frame lookahead candidate, 2026-09-06:** the shared
resident stitch chain now runs on one capture-shared worker. The UI never
waits for that worker. This checkpoint's L1 pacing claim was incorrect, as
corrected above. One already-decoded successor can stitch before its presentation time,
but its completed map remains private until Player promotes the exact same
opaque delivery. Pause, renderer recreation and seek replacement retain the
correct source owners; EOF keeps drawing until its final map installs. Shader
arithmetic, solver dispatch order and temporal calculations are unchanged.

All 93 X4 and 183 X2 actual Scene artifacts are byte-identical to the bilateral
mask checkpoint, and root viewed both new final frames. Real GPU tests cover
future Ready withholding across pause/recreation, exact due publication, final
EOF installation, and seeking away from a prefetched Ready through complete
retirement. Final workspace gates pass 1163 tests with 30 ignored, plus full
clippy/fmt/name/Cargo-source checks. The rebuilt branch player passes all 48
native-window checks at the exact X4 view; four paired-file drop checks are
inapplicable. Root viewed its capture, byte-identical to the prior checkpoint.
Evidence is in `scratch/gpu-lookahead-final-20260906-01/`.

The same 2560x1440 diagnostic binary as the trial formerly labeled L1-paced
was reproduced byte for byte at verification time. Its worker/lookahead
baseline/candidate pairs per camera reduced median first-draw
arrival lateness from 13.6–16.2 ms to 4.4–5.4 ms and redraw p99 from
12.7–14.4 ms to 7.7–11.2 ms. **This is not a general capacity pass:** average
redraw throughput fell on both cameras, X4 measured 239.8–247.8 redraws/s, more
draws exceeded 4.17 ms, and X4 arrival p99 worsened to 26.8–30.5 ms. All eight
runs retained 360 consecutive source changes and zero reported drops,
starvation and audio underruns. These are offscreen completed redraws, not
native presentation or physical audio latency. The 240 fps frame-time target
and owner branch acceptance remain open; these defects are not accepted
tradeoffs.

Broader worker pacing added picture delay in its separate trial. The supposed
removal of L1 pacing did not change the selected playback path, so the measured
differences in that comparison cannot support rejecting unpaced L1 submission.
The raw records and limits remain in ignored scratch. No new
Studio export, profiling probe or optimization RE was needed. Main is unchanged.

**Bilateral mask work consolidated, 2026-09-06:** geometry now evaluates each
joint two-lens mask word once and writes both output sections, instead of
repeating the identical 9-by-9 validity and support calculation. Workgroups
fall from 507 to 254; the 32,401-word buffer, arithmetic and sentinel remain
unchanged. Actual Radeon qualification, including deliberately corrupted
second-lens and sentinel writes, passes. All 93 X4 and 183 X2 real Scene
artifacts are byte-identical to the preceding recurrence checkpoint; root
viewed both new final frames. Workspace 1140 passed, 30 ignored, with full
clippy/fmt/name/Cargo-source gates passing. The rebuilt native player passes
all 48 window checks at the exact X4 view; four paired-file drop checks are
inapplicable. Root viewed its capture, byte-identical to the prior checkpoint.

Two alternating capacity pairs per camera at 2560x1440 show mixed results:
X4 baseline/candidate 302–323 / 305–309 redraws/s and p99 12.77–13.47 /
13.12–13.17 ms; X2 361–366 / 367–370 and p99 11.62–11.66 / 11.34–11.44 ms.
All eight runs retained 360 source changes with zero drops, starvation and
audio underruns. This is redundant-work removal, not a demonstrated general
capacity or tail-latency win. The 4.17 ms target remains unmet. No additional
timing probe or Studio export was used. Evidence is in ignored
`scratch/bilateral-mask-once-20260906-01/`; main and owner acceptance are
unchanged.

**Exact PIS input preparation optimized, 2026-09-06:** the GPU front end no
longer reconstructs each patch's entire row/column prefix independently.
Cached horizontal and vertical recurrences preserve every rounded operation,
including intermediate positions between emitted patches. Patch-sum input
reads fall from 46,505,536 to 85,100 per source frame. A 41,040-byte private
scratch tail uses an existing allocation and binding; public outputs, model
calculations and temporal admission are unchanged.

A bounded timestamp experiment over 360 actual source changes per camera
measured the front-end GPU interval falling from 2.066 to 0.188 ms on X4 and
1.972 to 0.195 ms on X2. Paired per-frame totals fell from 7.767 to 5.692 ms
and 6.846 to 5.245 ms respectively. These are instrumented submission intervals,
not isolated busy times or native presentation. The temporary timestamp code
is removed; its patch, binaries, raw records and interpretation limits remain
in ignored `scratch/warm-stage-timing-20260906-01/`.

Two uninstrumented baseline/candidate pairs per camera at 2560x1440 show lower
p95 and p99 redraw times in each pair. X4 baseline/candidate capacity ranges
are 287–307 / 301–307 redraws/s, with p99 14.20–15.69 / 13.26–13.56 ms. X2
ranges are 330–338 / 363–365, with p99 13.11–13.89 / 11.44–11.80 ms. All eight
runs retained 360 consecutive source changes with zero reported drops,
starvation or audio underruns. Not every distribution improved: X4's count
over 4.17 ms increased in both pairs, substantially in one, despite its lower
p95/p99. The active
240 fps frame-time target remains unmet; no universal throughput claim is made.

All 93 X4 and 183 X2 real Scene PPM/map/alpha artifacts remain byte-identical;
root inspected the final frame from each new sequence. GPU qualification and
late-row/late-column/scratch-boundary mutation checks pass, as do all 1140
workspace tests (30 ignored), full-target clippy, formatting, name-check and
Cargo-source consistency. The refreshed branch native executable passes all
48 window checks at the exact X4 view. Root inspected its capture, also
byte-identical to the prior corrected native capture; four paired-file drop
checks are inapplicable to the single-file X4 capture. The previous binary and
harness directory are preserved. Main is unchanged and owner acceptance
remains required.

**Shared exact arithmetic and native cleanup gate, 2026-09-06:** parent mapping
and PIS now compile the same binary32 division helper. PIS arithmetic is
unchanged; parent mapping replaces its duplicate restoring divider with PIS's
existing hardware estimate plus exact integer correction and restoring
fallback. This is consolidation, not a demonstrated whole-player speedup.
The direct GPU test passes all 67,300 input pairs both normally and with the
fallback forced, and all seven parent GPU tests pass. All 93 X4 and 183 X2
actual Scene artifacts remain byte-identical. Workspace tests pass 1140/0
with 30 ignored; full-target clippy, formatting, name-check and Cargo-source
consistency pass. The refreshed branch player, including the camera-profile
cleanup below, passes 48 native UI checks with zero failures at the exact X4
view. Root inspected its native-window capture, which is byte-identical to
the previous corrected capture. No owner acceptance or main merge is implied.
Evidence: `scratch/shared-exact-division-20260906-01/`.

Alternating 2560x1440 X4 capacity runs do not establish a repeatable improvement:
baseline/candidate/baseline/candidate averaged 297/275/265/278 redraws/s, with
p99 redraw times of 14.695/15.463/15.942/15.966 ms. Each retained all 360 source
changes with zero drops, starvation or audio underruns. The 4.17 ms tail
target remains unmet. A separate final-map input-buffer reuse experiment also
showed no repeatable preparation-time improvement and was removed; its patch,
separate binaries and logs remain in `scratch/final-map-input-reuse-20260906-01/`.
No native-arithmetic or scratch-buffer-pooling prototype remains selected.

**Camera setup consolidated, 2026-09-06:** one immutable resident profile now
resolves live admission, parent inputs, static maps and image support while
opening the capture. GPU attachment consumes that profile instead of rebuilding
camera inputs, and restarts share it while creating a fresh temporal lineage.
Scene's duplicate optional calibration and redundant parent validation are
removed. Production retains only the prepared inputs, not the full raw
calibration; the frozen CPU diagnostic retains a test-only copy. This is a
structural correction, not another stitching law or a claim that the remaining
v3-derived X4 support has been corrected.

All 93 X4 and 183 X2 actual Scene artifacts (92 frames plus their packed/alpha
maps) are byte-identical to the corrected pre-refactor baseline. Workspace
tests pass 1140/0 with 30 ignored; full-target clippy, formatting, name-check and
Cargo-source consistency pass. Evidence is in
`scratch/camera-profile-20260906-01/`. The subsequent shared-arithmetic
checkpoint above includes the fresh native-window harness run for this
refactor. The branch is not merged.

The capacity diagnostic now separates host pump/prepare time from draw plus
queue-completion time without adding GPU synchronization. These are independent
wall-time distributions, not GPU timestamps or additive percentiles. One
post-refactor 2560x1440 run per camera retained all 360 source changes and zero
drop/starvation/audio-underrun counts. X4 reached 287 completed redraws/s with
p95 prepare 3.315 ms and completion 8.435 ms; X2 reached 330 with 2.825 ms and
6.956 ms respectively. Both host preparation and queued work need attention;
this refactor is not a measured performance improvement.

**Native PIS arithmetic experiment rejected, 2026-09-06:** replacing only the
five arithmetic wrappers with ordinary GPU operations did not establish a
repeatable whole-Scene capacity improvement. Corrected-X4 baseline runs at
2560x1440 averaged 284 to 308 completed redraws/s; native-arithmetic trials
averaged 277 and 311, with more over-budget redraws than their adjacent exact
runs. All preserved 360 consecutive source changes per twelve-second sample
with no reported drops, starvation or audio underruns. Neither arm meets the
4.17 ms tail target. The production constructor and exact shader are restored;
the owner-review player was not rebuilt. The prototype, separate binaries and
logs remain in ignored `scratch/native-pis-arithmetic-20260906-01/`.

Actual Scene comparison also completed for all 61 X2 and 31 corrected X4
review frames. Alpha stayed bit-identical, while terminal decisions and final
UVs changed. Root inspected the worst-RMS frame from each camera but makes no
owner-quality acceptance claim. No additional X2 performance trial or native
window gate was needed after the X4 performance criterion failed. This narrows
the performance work away from assuming exact rounding is the primary problem;
it does not prove native arithmetic could never be useful.

**Implemented X4 camera correction, 2026-09-06:** the real Scene seek/stitch/
draw/screenshot path now renders frames 34569 through 34599 at the owner's
exact view with a broadly straight horizon instead of the prior visible bend.
Root inspected full-size first/middle/last frames and the complete sequence
contact sheet. This is a branch candidate awaiting owner review, not whole-video
or exact Studio parity. The review video is
`scratch/x4-model6-scene-review-20260906-01/review.mp4`.

The fix is camera geometry, not another solver: parse optional `offset_v6`
without replacing the v3 IMU/camera identity; use its crop-scaled intrinsics,
Template mounting and all thirteen distortion coefficients in the shared
scalar/GPU parent projector. Resolve native calibration record 1 to container
stream 0 and record 0 to stream 1 once in static packing. A fixed BODY Rx(pi),
equivalently producer-SPHERE Ry(pi), preserves the existing view convention.
No per-clip fit, source decoder reordering or new playback configuration is
introduced. The attempted import-order-only patch was rejected after it
rendered the wrong view and has been removed, not carried into playback.

The Radeon 760M model-6 GPU parent pair matches the scalar reference bit for
bit, and all seven existing model-3 qualification cases remain exact. The
61-frame ONE X2 actual Scene regression is byte-identical to the current
pre-change `shared-onex2-review-01` baseline in all 183 PPM/map/alpha files.
The first comparison against the older `240-final-f6339-6399` baseline differed
in RGB rounding only; it was the wrong baseline for this change. Final workspace
tests passed 1139/0 with 30 ignored, and the exact-view native-window harness
passed 48 checks with zero failures. The four two-file drop checks correctly
skip for this single-file X4 capture. Root inspected the native window capture
as well as the Scene sequence. Formatting, workspace all-target clippy,
name-check and Cargo-source consistency checks pass. The branch player is
rebuilt and ready for owner testing; main remains unchanged.

The X4 blend/support still use the existing Kjerag v3-derived image-circle and
blend law, and rolling motion still uses Kjerag's shared pose provider. Those
are not newly authenticated Studio internals. Peripheral support coverage and
the active 240 fps capacity target remain open; this visual improvement does
not establish either. No visible tradeoff or merge is owner-approved yet.

**Source-order finding, 2026-09-06:** the isolated Studio-map replay had its
source lenses reversed. A read-only CoreVideo snapshot of the actual renderer
inputs is byte-identical, across all 22,118,400 logical luma/chroma bytes per
record, to container stream **1 for FTD/shader source 0** and container stream
**0 for FTD/shader source 1**. Both are full-range NV12 at PTS 34572538.
The source-read texture-assignment path preserves this FTD order. The earlier
replay's filename-based stream-0-first assumption was wrong; its camera and
interpolation ablations below must not be treated as correctly paired native
replay evidence.

With the verified source order, the same captured maps and uniforms now render
a broadly straight horizon, without the prior obvious doubled terrain, at the
fixed owner-view comparison. Only one common panorama rotation is fitted and
applied unchanged to the final image, both lens-only images and alpha. The
replay still differs locally and is softer than Studio. This is a corrected
diagnostic, **not a native-player fix or owner acceptance**. The next work is
implementation of the verified camera geometry with explicit source assignment,
preserving ONE X2 and the player's horizon/view convention, not more blind
camera-constant substitutions or a global decoder swap.

Evidence: `scratch/x4-type11-source-pixels-20260906-02/` contains the exact
decoded planes, same-window maps/blocks, balanced successful read-only lock and
unlock receipts, and the byte-comparison script/result. The report hash matches
on both hosts; all ten breakpoints were deleted and Studio detached. The fixed
view and Studio comparison are in
`scratch/x4-type11-direct-replay-20260906-01/registered-source-authenticated-to-studio-on/`.
`docs/research/studio-x4-video-reference-602.json` records their hashes. The
preceding source-pixel attempt never reached a render entry and copied no
pixels; it is terminal and detached. Production source and native binary were
unchanged at that capture checkpoint; the implementation above supersedes it.

The following paragraphs retain the preceding diagnostic sequence. The verified
source assignment above supersedes their unresolved-source conclusions.

**X4 owner rejection and diagnosis, 2026-09-06:** the shared-camera candidate
below is NOT accepted: the owner reports distorted terrain/horizon through the
join. A same-source ablation of frame 34569 at the exact reported locked view
now separates ordinary projection, parent maps, final maps and each lens alone
(`scratch/x4-layers-20260906-01/`). Before alignment, the parent lenses show
offset, broadly straight horizons; final displacement bends them toward each
other. The detached final-map consumer visually reproduces the saved Scene
picture, with small sampling differences (RGB PSNR about 70.34 dB). This
localizes the shape change, not its correctness versus Studio. The neutral X4
parent test agrees with the existing generic projection within 0.0014 source
pixel before the inherited normalization difference; agreement with our own
projection is not a Studio calibration verdict. No production fix is claimed.
The exact April file is now on the Mac with matching source hashes, active in
Studio for a bounded ordinary output comparison. The fresh X4 project has
Optical Flow off, AI Stitching and Image Fusion on; preserve those defaults
for the first reference rather than assume the earlier ONE X2 Flow On setup.
Broader RE stays frozen. An ordinary default-setting X4 360 export is now
captured and copied to durable storage on both hosts, with matching video and
project hashes recorded in `docs/research/studio-x4-video-reference-602.json`.
The saved roughcut is leftTrim 34538, rightTrim 19301, totalFrames 53940;
the actual output has 102 frames at 30000/1001, 7680x3840, lasting 3.4034 s.
The target-candidate panorama is now registered to the owner's field of view
using only a common rotation (Studio v360 yaw 90.90, pitch -2.30, roll -0.37)
at the same 63.63-degree horizontal FOV and 1280x720 output. Actual pixels show
Studio's broadly straight horizon where the current Kjerag final bends through
the central join. The same distinction is visible in neighboring Studio frames.
Independent source/output frame association remains pending: export index 31
is an index-derived candidate, while the fixed-view 30/31/32 comparison favors
30. This permits a near-time shape comparison, not an exact-frame parity claim.
The earlier fractional accessibility-slider readback was NOT proof of a seek
to 1153.452 s. Acquisition was recovered using guarded physical clicks and
the saved project range. No production fix or owner acceptance is claimed.

A second export of the **same short range with Stitching Optimization Off**
now separates AI from the base-mapping issue. Its preserved project differs
from the default reference only in AI true-to-false and modification time;
Image Fusion, stabilization and trim are unchanged. It has the same 102-frame
packet grid. At the identical registered view, actual Studio pixels still
show a broadly straight horizon, while the current parent lens-A discrepancy
persists. The reference manifest records both exports and their hashes.
This redirects the next work to camera/base mapping before optional alignment,
not to reproducing Studio's AI. It does not identify the correct replacement
camera law. Studio's original AI-on setting has been restored and verified.

The next bounded calibration check has a concrete source basis: the April
source's tag-111 v6 record is byte-identical to the archived X4 fixture,
while current Kjerag reads only v3. The recovered Windows Studio 5.9.10
selection uses v6 and its 13-term distortion model; that is not a runtime
selection trace of the current Mac 6.0.2 export. Earlier all-v6 rejection
mixed distortion, crop, validity and unverified mounting changes, so it
does not isolate the model. Compare those camera inputs in lens-only parent
renders before changing the shared solver. No production selection changed.
That five-arm comparison is now complete: current v3, v6 pose only, v6
pose/intrinsics with v3 distortion, full v6 generic mounting, and full v6
with the previously disclosed fixed-datum Template rotation all retain a
visible horizon join in the actual lens-only and blended renders. The current
null reproduces the earlier parent pictures byte-for-byte. Feature-derived
relative lens disagreement improves from about 2.03 to 1.48 degrees with
v6 pose/intrinsics, but the extra distortion terms do not materially improve
that particular statistic. This is not an accepted correction or a proof of
the remaining mounting law. The scalar equivalent-null differs by at most
0.001233 source pixel with identical sentinel validity; changing the full-v6
cap leaves all 28185 jointly valid nodes identical, while changing support
at 4675 others. The isolated f32 diagnostic does not claim Studio's double
arithmetic or X4 rolling-pose parity. Outputs and exact test sources remain
in durable ignored `scratch/x4-model6-*`; the temporary test include is
removed and the native player hash is unchanged. Do not repeat these
calibration substitutions as new fixes or tune the solver to hide the offset.

The current Mac 6.0.2 export now supplies direct camera-input evidence, not
only the older Windows selection trace. Both captured lenses use model 6;
all eight delivered center/focal binary64 values match the v6 crop schedule,
and both lens quaternions match the recovered raw Template law to host
arithmetic precision. The report and observer are hash-matched in durable
storage on both hosts. This closes static model selection and those inputs
for this activation, not the complete parent map or the visible defect.
The observer did not capture the configured angular cap, mirror, common
mapping quaternion or pose batch. Those remain explicitly unverified.

A further exact-frame diagnostic separates the nominal coordinate-frame
turn from rolling motion: it applies the captured lens quaternions and moves
the fixed 180-degree datum to the output grid, then compares against the
previous post-quaternion turn and a zero-motion control. The first comparison
changes at most 0.657 source pixel across 28185 jointly valid nodes with no
validity changes. The current provider turns at most 0.132 degrees from the
frame center across the readout; removing that motion still leaves the visible
horizon offset. Both actual renders and the computed seam trace were inspected.
These are diagnostic controls, not an accepted camera correction. No production
source or native binary changed. The next distinction is Studio's actual
parent output versus a later stitching correction, not another blind mounting
or flow-parameter substitution.

That direct parent-map observation is now complete for one ordinary Mac 6.0.2
export transaction. The observer binds both lens calls, their returns and
their later GPU-wait returns before reading finished UV. It captures the
actual 100x200 map per lens, all kernel-semantic parameters and both 51-pose
batches. A scalar replay of those exact inputs agrees with the finished
native maps within **0.001863 source pixel**, with identical sentinel validity
on both lenses. This verifies the model-6 projector calculation for the
captured input, not the selected player or the final stitched video.

The captured cap is 120 degrees and the horizontal mirror is +1 on both
lenses. The two pose schedules differ by 0.178161 ms; their center-composed
rotations differ from the raw lens quaternions by about 0.001223 degrees.
That small residual is real but does not explain the visible broad offset.
All three observer breakpoints were removed and the exporter detached. The
maps, poses, report and observer are preserved in ignored
`scratch/x4-parent-map-20260906-01/`; native report/map hashes match between
hosts and the reference manifest records the boundary. No source-frame
association or final-consumer-map identity was captured. Investigation now
moves downstream of the verified projector; do not reacquire these camera
inputs or treat more calibration variants as a fix. Production source and
the native player are unchanged, and owner acceptance remains outstanding.

The ordinary X4 360 exporter takes the plane-stitch/type-11 renderer, not the
archived ONE X2 type-2 render entry. Its 200x100 packed source-UV and left-alpha
uploads have now been captured repeatedly with identical bytes. Replaying
them through the existing type-2 consumer still shows an offset join; that is
not yet a faithful reproduction of the plane-stitch draw. Embedded shader
source identifies direct per-fragment map interpolation, a source transform
and coverage-weight normalization as consumer inputs.

A direct-callsite observation now captures the submitted 96-byte fragment
block and 152-byte texture block. The first texture transform is identity,
with 3840x3840 source size and a 1x1 box; the fragment block names a 200x100
lookup and 1x1 fisheye-coverage texture. The coverage texel itself was not
captured. Thus a missing source transform is not supported as the correction.
Both independently decoded lens PNGs have PTS 34572538, matching the captured
native timing numerically, but the actual draw's source-pixel identity remains
unverified. Evidence is in `scratch/x4-type11-inputs-20260906-02/` and the
reference manifest. All ten breakpoints were deleted and Studio detached.
No production fix is claimed; the next comparison bypasses the type-2
consumer instead of changing camera constants to fit its output.

That direct plane-stitch replay is now rendered from the captured inputs and
independently decoded lens images. After one common panorama registration
against Studio's AI-on frame zero, the actual owner-view crops still show a
double horizon. The shared registration is applied unchanged to both lens-only
images and alpha; no per-lens correction is fitted. Triangle-centroid checks
in the captured blend region bound the type-2 interpolation discrepancy to
2.241 source pixels in this sample, too small to explain the broad gap alone.
The replay therefore does not establish native draw parity. The remaining
boundary to authenticate is the actual source imagery and its path to this
draw, not another blind projector or triangulation substitution. The replay,
registration, lens-only images and audit are durable in
`scratch/x4-type11-direct-replay-20260906-01/`.

A following same-window observation now reads the two actual decoder records:
both are 3840x3840 VideoToolbox frames with signed PTS 34572538, zero texture
rotation and the same upside-down flag. The first TextureParam apply points
exactly to record zero's MediaTexture. Maps and submitted blocks are identical
to the preceding capture. This does not support a different-moment lens pair,
but does not yet establish the container-stream ordering, source pixel bytes
or final shader bindings. The report and bounded headers are preserved in
`scratch/x4-type11-inputs-20260906-03/`, with matching report hashes on both
hosts, all ten breakpoints deleted and the exporter detached. No player change
or visual fix follows from this observation alone.

The zero-flow roundtrip through the real CPU map merge, filtering and final
materialization contributes at most 0.182 source pixel in the neutral X4
fixture. This rules out broad deformation from that roundtrip without flow;
it does not establish that the estimated correction or camera model is right.
The diagnostic-only additions pass full workspace regression: 1128 passed,
zero failed, 30 ignored, with required GPU and real ONE X2 input
(`scratch/x4-layers-workspace-final-20260906.log`). Workspace clippy, formatting,
name-check and cargo-sources checks pass. These gates do not accept the X4
image defect; the native player remains the preceding, unaccepted candidate.

The first exact-view X4 active-capacity run at 2560x1440 completed 277.00
redraws/s, advancing all 360 source frames over 12 seconds with no reported
audio underruns. Paused capacity was 473.30. Playing p95 was 12.253 ms, p99
18.480 ms and max 39.237 ms; 370/3324 draws exceeded 4.17 ms. This is an
offscreen capacity measurement, not sustained budget compliance or native
presentation proof (`scratch/x4-capacity-20260906-01.log`).

A persistent-workgroup GPU experiment reduced isolated patch-search time but
did not consistently improve actual X4 playback capacity in alternating A/B
runs. It has been removed from production source. The baseline and candidate
binaries, measurements and rejected patch remain in ignored scratch; native
player code and binary are unchanged. Both versions still exceed the 4.17 ms
frame-time budget at the tail, so average throughput is not target completion.

**Shared-camera branch checkpoint, issue #184:** ONE X2 and X4 Air now use
one resident GPU solver with separate camera calibration inputs. The native
X4 Air player has rendered the owner's exact view, and its Scene regression
captured frames 34569..34599 through ordinary seek, stitch, draw and screenshot
ownership. Inspected first/last output shows substantially reduced central
terrain duplication. This is a candidate for owner review, not an X4 Studio
parity verdict. Local evidence: `scratch/shared-x4-review-01/`,
`scratch/shared-x4-native-held.png`, `scratch/shared-x4-native-repro.log`.
The 61-frame ONE X2 regression keeps every packed/alpha map byte-identical to
`scratch/240-final-f6339-6399`; pixels have small differences after making
source sampling dimensions dynamic (sampled frame 6369: 75.98 dB PSNR, both
actual images inspected). This is not a claim of bit-identical pictures.
The shared chart/UV convention is still native-derived, camera admission is
limited to the tested type-41/type-131 families, and no X4-specific throughput
or matching Studio export comparison has been completed. Main is unchanged.

**Original coverage gap:** the preceding resident path was ONE X2-only. The owner
called the native ONE X2 view at 212.512 s good, then supplied an X4 Air view
at 1153.452 s with yaw 132.05, pitch 3.55, FOV 63.63, locked horizon. That
file used the legacy path; the exact native screenshot
`scratch/april-riser-native-held.png` shows a central double image/horizon
offset. The original log path was reused during the new native verification;
the durable earlier context is `scratch/NEXT-SESSION-20260905-x4air-report.md`.
The reproduction does not prove Studio parity or identify the detailed cause
within the legacy path. No shared-camera change has shipped on main. Both
reported views must be checked as the engine evolves.

**Photometric lens fusion, Studio calls it Chromatic Calibration, [issue #185](https://github.com/aeharding/kjerag/issues/185):** the owner
requests parity with this setting when appropriate. It is lens color and
brightness matching, represented in Studio projects as `image_fusion`, not
optical chromatic-aberration correction or per-channel lens displacement;
`docs/research/linux-landscape.md` section 6 records the maker SDK's matching
`EnableStitchFusion` description. The exact Studio 6.0.2 Flow On and Flow Off
project blobs authenticated by the hashes in
`docs/research/studio-video-oracle-602.json` both encode Image Fusion enabled.
Read-only inspection of those manifest-named projects therefore does not
provide an isolated Image-Fusion-on/off comparison, coefficients or parity
proof. After issue #184's shared camera stitching works, add a default-neutral
per-lens photometric stage at its source-sampling/fusion boundary, then verify
the relevant Studio setting and real output. This is required work, not an
implemented or verified feature. No speculative scaffolding, new photometric
reverse engineering or photometric-isolation export belongs in the current
X4 Air stitching deliverable. Ordinary Studio verification of that camera's
reported stitching defect remains in scope.

The shared-camera workspace gate passes **1126 tests, zero failures, 30 ignored**
with required RADV and real ONE X2 input (`scratch/shared-camera-workspace.log`).
The separately enabled X4 Scene sequence passed with real X4 input; its final
repeat preserved all 31 pictures and their packed/alpha maps byte-for-byte.
Native UI verification passed **48 checks, zero failures** on the exact X4
view (`scratch/shared-camera-uitest.log`); two-file checks skip on this
single-file capture. The final native exact-view image is
`scratch/shared-x4-native-final.png`, inspected from the harness capture.
Native SHA-256 is
`aecd39b338fd7c8ce2d0c90109275d807e3ba4a29a2d1b9ce8951610040b220c`.
Owner branch review and installed Flatpak verification remain pending.

**Active-playback capacity checkpoint, 2026-09-05, issue #182:** the final
uncapped 2560x1440 Scene/ScenePipeline test completes **329.96 redraws/s**
with stitching active, versus 510.54 paused. It advances every displayed
source frame (360 changes, 12.012 media seconds in 12.001 wall seconds) with
zero audio underruns. This clears 240 fps on average in the offscreen
capacity diagnostic, NOT a 4.17 ms frame-time or native-presentation gate:
playing median is 2.018 ms, p95 10.056 ms, p99 15.598 ms and max 52.040 ms;
366 of 3960 draws exceed budget. Evidence: `scratch/240-overlap-final-1440p.log`,
view-rate SHA-256 `71d142ece122825a3619806c27d01dd05b9c2fcd7fbed7011a984dec88ac02b6`.

Flat views now rasterize the native sphere triangles on hardware; curved views
retain the original ray shader. PIS uses independent 16-row stripes with an
exact striped CPU qualification, while retaining the global reference.
This deliberately cuts vertical propagation at stripe borders and is **not
owner-accepted visual parity**. An integer-corrected hardware division estimate
keeps exact binary32 outputs; 67,300 edge/random pairs and a forced restoring
fallback pass bitwise checks. No broader Studio RE or export was needed.

The first faster-GPU native tests still ran at 22–24 source fps and accumulated
video lag. Smaller stripes, mailbox presentation and a wakeup-only change did
not fix that. The completed-stitch diagnostic reached 30 fps only by waiting
14–26 ms; that temporary blocking path and its logging are removed. The actual
handoff now allows one decoded successor to wait behind the submitted pair,
then installs/stages the predecessor and submits its successor in the same
redraw. Only one GPU stitch runs at once, every input remains sequential, and
the submitted view survives renderer recreation independently of the new offer.
Seek/step completion still requires installation. `SequentialRealtime` keeps
the audio/video clock continuous; the slow-clock policy remains a reference.

A roughly 55-second native run now reports around 30 source admissions/s after
startup, with zero dropped/starved frames or audio underruns. It has dips to
28.6 and catch-up to 31.8; worst cumulative source-admission lateness reaches
422 ms, so jitter and transient video lag remain work, not accepted tradeoffs.
These native media counters are not swapchain fps. Evidence:
`scratch/240-native-play/overlapped.log`, native SHA-256
`97f7695c4df96911d3df16b9e20356d06dc0f8d8296bc66d07cc5b6bec41b991`.

All 61 cold-seek review frames 6339..6399, packed maps and alpha maps are
byte-identical before/after the division and handoff changes. The striped/mesh
picture itself differs from the preceding global-solver branch; sampled actual
pixels were viewed and the labeled `scratch/240-final-riser-comparison.mp4`
was sent as a local review link. No new computed-trace or owner-eye verdict is
claimed. The owner has explicitly been asked about the stripe/raster tradeoffs;
main remains unchanged and PR #183 stays draft pending branch testing.

Full forced-RADV/real-media workspace gate: **1115 passed, zero failed,
30 ignored** (`scratch/240-overlap-workspace.log`). Workspace/all-target
warnings-denied clippy, format, name/source and diff checks pass. The new
real-media overlap regression proves one waiting successor, no third admission,
ordered exact drawing and renderer recreation. The full native UI gate passes
**54 checks, zero failed** (`scratch/240-final-uitest.log`), including the
actual late-clip pointer scrub to 207.908 s, exact/backward seeks, pause/resume,
screenshots, file drops, transient/stuck import failures and reopening.
The UI-tested native SHA-256 is
`3670ee1a8a93f45e87cded6e620cf0fa859300d2c9af16a825d85099f9e43a7b`.
Installed-Flatpak testing and owner acceptance remain outstanding.

**Player performance target, 2026-09-05, issue #182:** the owner requires at
least 240 fps "when I play back in player". The working interpretation is
240 fps interactive view rendering during playback, with source frames at
their recorded cadence and synchronized audio. Reuse completed stitching for
view redraws, not eight redundant solves per 29.970 fps source frame. The
display-frame budget is about 4.17 ms; report resolution, presentation limits
and tail frame times. Kernel-only throughput is diagnostic, not acceptance.
The latest 26 fps measurement counts video presentation, not view redraws;
240 fps interactive playback has not been demonstrated. Current source-video
throughput and audio underruns already fall short even before that higher gate.
Read-only `cosmic-randr list` finds the active internal panel at 2256x1504,
59.999 Hz, with no 240 Hz mode. This has been disclosed to the owner: 240 fps
rendering capacity cannot be called 240 distinct visible updates on this panel.
Keep the rendering budget without burning idle redraws. No display setting changed.
**Subsequent owner confirmation:** "240fps regardless of display", "capacity".
The acceptance target is sustained rendering capacity with stitching active,
not paused-view throughput. Display refresh does not limit the benchmark or
justify lowering the target. Actual source playback and audio must stay smooth.

**First high-refresh rendering optimization, 2026-09-05:** the selected
type-2 consumer now uses hardware linear filtering within each separate lens,
retains manual sampling across the atlas join, and avoids sampling a lens with
exactly zero contribution. No seam-map, solver or temporal change. On the
Radeon 760M at 2560x1440, `view-rate` changing-view diagnostics improve from
181 to 290 completed redraws/s paused (median 5.46 to 3.39 ms) and from 78 to
124 redraws/s during playback. These are offscreen, per-draw GPU-wait timings,
NOT native-window fps or a 240 fps pass. Playback still has a 19.92 ms p95 and
46.77 ms p99, advances source video at 26.75 fps and records audio underruns.
The paused optimized run also has 15 of 581 draws over the 4.17 ms budget.

The same 61-frame cold-start riser review has byte-identical packed and alpha
maps to the preceding checkpoint. RGB differs by at most one 8-bit code in
0.3701% of channels across that sequence. The exact 6369 side-by-side was
inspected; `scratch/view-rate-filter-riser-review.mp4` is the owner review
artifact, not a new owner verdict. Hardware sampler regression covers both
lenses, the join, outside clamp edges and the unchanged box filter, with worst
difference 0.519 code values on its high-contrast fixture. Seven focused GPU
tests pass. Detailed measurements are `scratch/view-rate-{baseline,filter}-1440p.log`.
The native UI gate passes 54 checks, zero failed, on player SHA-256
`4dc9c8d7970ec7d42fb602521ffe01ea38c823e2ea4c9981aa095a08c4b6d579`
(`scratch/view-rate-filter-uitest.log`). Workspace/all-target clippy,
format and name/source/diff checks pass. The full forced-RADV workspace gate
passes 1109 tests, zero failures, 30 ignored, including enabled real-media tests
(`scratch/view-rate-filter-workspace.log`). Installed Flatpak and owner review
remain pending. The next
performance bottleneck remains in-flight stitching and its playback scheduling;
240 fps interactive playback has not been demonstrated.

**Owner priority clarification, 2026-09-05, issue #182:** the goal is very
performant stitching that looks like Studio, not perfect reproduction.
`MANDATES.md` records the ruling. Prioritize ordinary playback speed and real
scrubber latency; exact numerical comparisons remain diagnostics rather than
the goal. Full-rate playback and audio are still unfinished. The earlier
53-check harness did not physically scrub to an unseen late frame and did not
establish responsive seeking. Visual changes require rendered-sequence review
and the owner's branch test; main remains unchanged.

**Playback/scrubber branch work, 2026-09-05, issue #182:** native GPU PIS
candidate scoring and descent sampling now run cooperatively per patch,
with ordered anti-diagonal dispatches in one compute pass. Arithmetic and
candidate tie order remain unchanged; the paired GPU and mutation tests pass.
Ordinary window playback improves from about 22 to 26 fps on the reported
29.970 fps capture. Audio underruns remain. This is not full-rate playback.

User seeks now initialize fresh temporal state at the decoder landing,
sharing immutable, already-qualified GPU kernels. This deliberately changes
post-seek history versus uninterrupted frame-zero playback, and still needs
owner visual acceptance. Drag previews use keyframes; release lands exactly.
Superseded decoder epochs cannot initialize the fresh stitch root. The real
pointer-drag UI check reaches and holds 207.908 s within five wall seconds,
including pointer setup, drag and observation, rather than replaying the
prefix for minutes. Exact release, backward steps and forward warm-state
continuation also pass through real decode/GPU regression tests.

The optional Scene review test saves all 61 frames 6339..6399 starting with
the cold landing, plus each frame's actual packed and alpha maps. Evidence
is `scratch/scrub-post-seek-f6339-6399/` and the adjacent
`scrub-post-seek-riser-review.mp4`. Sampled output frames were inspected;
this is a review artifact, not a byte-identity, computed-trace or owner-eye
parity verdict. The native exact riser view at 212.512 s also lands within
the two-second observation window. Broader optimization RE stays frozen.

The full native UI gate passes **54 checks, zero failed**, now including
physical scrubbing to unseen late content as well as exact/backward seeks.
The tested app SHA-256 is
`cda781336f2737788f8d21934be15d14d6bd610035c57fda215d2cffecb06744`.
Logs are `scratch/scrub-uitest-retest.log` and `scratch/uitest/`. The first
run exposed a harness accounting error: its sound-stop check included a
normal play report from the allowed import-retry window before the stop.
The observation baseline now starts after the actual stopped marker; no
runtime audio improvement is claimed from that test correction. Workspace
format, warnings-denied all-target clippy, name/source/diff checks pass.
The final forced-RADV workspace gate with real-media tests enabled passes
1108 tests, zero failures, 30 ignored (`scratch/scrub-workspace-tests-final.log`).
Installed-Flatpak testing and owner acceptance remain outstanding.

**Earlier native GPU integration checkpoint, 2026-09-05, issue #182:**
The newer scrubber work above supersedes this checkpoint's seek usability
implication: its exact/backward seek tests covered only a tiny prefix.

the real ONE X2 player passes all 53 headless UI checks, including exact
and backward seeks, screenshots, file drops, transient import recovery,
terminal import failure holding the last picture, and reopening afterward.
The GPU device-limit patch is committed as `7d0b312e`. The subsequent
pre-submit recovery change distinguishes temporary OS/Vulkan resource
exhaustion from invalid inputs, device loss or errors after submission.
Only a failed source import can return to idle for retry; the exact offered
frame, installed acknowledgement and GPU root are preserved. Scene uses its
existing two-second import-failure window and keeps staging the installed
picture. Errors after GPU submission remain fail-closed. No arithmetic,
temporal cadence or normal successful-frame sequence changes.

Evidence: `scratch/resident-import-recovery-uitest-pass-20260905.log`
and `scratch/uitest/` in the integration worktree; the pre-fix captures
are retained separately. Unit tests cover raw OS/Vulkan error classification
and repeated retry preserving the same next source. The native app's tested
SHA-256 is `4b35fe80cacecc1e10f3d69a485c0f0b1d61635e966873915f9acd5dc6572b67`.
Steady playback still reports about 22 fps and audio underruns. Passing
the UI checks does not establish full-rate playback or stable audio.
The full workspace test gate passes on forced RADV with real-media tests
enabled, as do format, workspace/all-target clippy and name/source/diff gates.
The vendored compositor's device-limit regression also passes separately.
The final logs are `scratch/resident-import-recovery-{workspace-tests,clippy}-final-20260905.log`.
Installed-Flatpak testing, final GPU-branch owner review and further speed
work remain; no such compromise is accepted and main is unchanged.

**Resident range verified; real-window device limit found, 2026-09-05:**
implementation commit `7c2b287d84cef120f473f30c650e50bc41120d44` consumed
source frames 0 through 6399 on the selected GPU path (6400 GPU, zero CPU,
zero dropped or starved) and saved the owner's frames 6339 through 6399.
All 61 rendered PNGs, packed maps, alpha maps and computed seam-trace PNGs
are byte-identical to the earlier owner-approved `9054e547e4ea` range.
The actual riser render was inspected. This proves that interval against
that approved baseline, not whole-file Studio parity or a new owner-eye pass.
Durable receipts and logs are in the integration worktree's
`scratch/gpu-resident-7c2b287d-{f6339-6399,trace-f6339-6399}/` and adjacent
logs. The range's 14.6 fps includes instrument waits/readbacks and is not
a real-window performance measurement. Concurrent test load also produced
an audio underrun, so this run does not establish continuous audio playback.

The actual player then failed at startup: iced requested only eight storage
buffers per shader stage, while the prepared-source layout needs eleven and
warm post-L1 needs fifteen. Headless instruments requested adapter limits and
did not expose this integration error. A local patch to the pinned
`iced_wgpu` crate requests the adapter's supported storage count, with no
other renderer or shader changes. A selected-session preflight reports a
normal raw playback error below fifteen instead of a wgpu validation panic.
The lock file and Flatpak source list are regenerated together. The real
window now opens and renders; exact/backward seeks, stills, clipboard views
and file drops pass. Its first steady playback report was 22.6 fps with audio
underruns, not full-rate playback. Failure injection exposed a separate
resident integration regression: transient pre-submit import errors quarantine
the capture and lose the held picture. The checkpoint above repairs and
retests that boundary. The existing device report also lived only in legacy
preparation; it now runs
through the common preparation entry without changing the import check.
Installed-Flatpak playback and owner-eye review remain pending. Broader Studio
optimization RE remains frozen.

**Resident camera-mask correction, 2026-09-05, implementation branch only:**
the selected resident geometry omitted the frozen CPU path's conditioned
400-by-400 camera support. On the owner's frame zero the physical mask's
first byte was GPU 255 versus CPU 0. Adding the existing camera support,
uploaded once per capture and sampled on GPU in rows [0,216) and [864,1080),
makes both physical/shared-L2 masks, Cold0 L2, its L1 seed, all three cold L1
terminals and both final public fields exact against the CPU frame owner.
The real-media before/after logs remain in the diagnostic worktree's
`scratch/resident-cold-stage-probe/`, dated 20260905.

Qualification now compares camera-conditioned masks and includes valid corner
UVs that exercise camera clearing independently of erosion. Radeon tests reject
omitting camera support or changing either row boundary. Separate bilateral
tests cover the adjacent f32 values around the promoted-f64 `1e-8` threshold
and reject the naive rounded-f32 comparison. Normal playback adds no per-frame
CPU maps or readback. The complete rendered range and computed trace are now
verified as recorded above; owner-eye review of the GPU branch remains pending.

**Resident cold frame-zero causal locator, 2026-09-01, diagnostic branch
only [CFG(TEST), ENV-GATED, REAL MEDIA; NO PRODUCTION OR PARITY CLAIM]:** an
opt-in probe now compares one exact decoded frame-zero delivery through the
ordinary resident orchestration against a fresh frozen CPU `FrameOwner`.
On the owner's reported clip, the post-Gaussian belts are byte-exact across
129,600 bytes, and both parent/preimage sections (40,000 words each) and both
retained-base sections (129,600 words each) are bit-exact. The first public
flow mismatch is downstream of those boundaries.

The narrower same-run snapshots locate the first proven arithmetic divergence
at the Cold0 paired L2 PIS terminal itself, before the L2-to-L1 seed bridge,
Cold0 L1 terminal, temporal median, dense finish or public-flow packing. The
first unequal word is A-to-B patch 0 dcol: resident `0xbf35468e`, CPU
`0xbd412190`. The next causal boundary is therefore Cold0's prepared L2 PIS
input (model, masks, work mode, admission and disparity), not post-L1 history
or final-map assembly. The probe stops at the first unequal semantic producer;
ordinary builds and playback APIs are unchanged.

The diagnostic L1 seed readback now retains the producer's per-plane stride
and removes alignment padding before comparing the four logical planes. A
padding-poison regression verifies all plane boundaries. The original Cold0
L2 failure precedes this seed comparison and is unchanged by the correction.

**Selected ONE X2 Scene resident cutover, 2026-09-01, implementation branch
only [NORMAL LIVE PATH; POST-QUALIFICATION NONBLOCKING; NO PERFORMANCE OR
STUDIO-RE CLAIM]:** an ordinary supported ONE X2 open now branches before
legacy Scene preparation and attaches its capture-owned
`ResidentCaptureFacade` to a renderer-local `ResidentSceneFacade`. Normal
prepare, redraw and window draw never construct a `FrameOwner`,
`PreparedFrame`, solver-belt readback, `ColdInputs`, CPU PIS adapter, CPU dense
map or `DirectMapDraw` upload. Only exact resident staging replaces `Shown`
and acknowledges a frame; pending and full retirement admission keep the old
exact ready drawable and schedule another redraw.

Scene performs at most one nonblocking device poll, when resident work exists,
for all active and retired attachments in a redraw, then only classifies callbacks. Seek and reopen move
the old attachment into normal draining by capture identity, including while
the new frame-zero lineage has no offered view and after reopening a
non-ONE-X2 file. Completion-proven candidates and render callbacks release
normally. Uncertain ownership is retained fail-closed without blocking or
spinning the new lineage. Pipeline recreation binds the same capture-owned
session. Screenshot takes a separate resident permit and renders through its
own offscreen pass; it neither calls the window draw nor steals its permit.

The explicit diagnostic map API authenticates the current and shown full
`FrameStamp` plus capture identity, then reads the installed packed map and
alpha for causal tooling. That instrument-only operation intentionally uses
bulk `MAP_READ` and waits. The first lazy session construction also retains
the existing synchronous target-device arithmetic qualifications, including
their bulk diagnostic readbacks and waits. After construction, ordinary
per-frame submit/redraw maps only the four-byte validity word and performs no
wait or frame-sized CPU transfer. The CPU implementation remains accessible
only as an oracle/diagnostic boundary. Studio remains the frozen correctness
oracle and no optimization reverse engineering was performed. No visual
tradeoff is proposed; rendered parity and owner-eye verification remain gates.
On the dirty branch, format, workspace/all-targets warnings-denied clippy,
workspace tests, repository name/source/diff gates, the focused forced-RADV
facade test and the five-frame real-media selected Scene test pass. A final
clean commit receipt remains pending. The 61-frame render/map/alpha/computed-
trace range and decoy/null review, ordinary app plus installed-Flatpak/UI
playback, and owner-eye review remain merge gates. The structural source test
is a tripwire rather than a transitive proof, and the five-frame test submits
real render passes but does not read back their pixels.

The first authenticated resident range attempt exposed an instrument boundary
error before publishing output: source frame 6339 had been offered while exact
frame 6338 was still installed, so the runner armed a frame-6339 screenshot and
correctly received the shown frame 6338. The runner now treats the complete
installed `FrameStamp` as capture authority. After an ordinary redraw installs
the requested transaction, it arms the screenshot and drives bounded no-pump
redraws until the exact-once callback resolves; temporary screenshot-retirement
backpressure therefore cannot advance the source or substitute a successor.
Its diagnostic map readback follows the same shown capture facade and full
stamp. Focused regressions cover the offered-N/shown-N-1 boundary and a first
capture redraw refused by full retirement admission. A fresh authenticated
real range remains pending.

**Capture-shared resident transaction facade, 2026-09-01, implementation
branch only [SUPERSEDED BY THE SELECTED SCENE CUTOVER ABOVE; FORCED-RADV
QUALIFIED; NO PLAYBACK, PERFORMANCE OR STUDIO-RE CLAIM]:** one open ONE X2 capture now has a
lazy resident session that owns the exact GPU context, authenticated render
format, calibration/orientation-derived producers, capture root, direct
pipeline, picture layout, sampler and bounded draw-retirement queue. Renderer
attachments share that session and queue but keep their one-redraw staged cell
local, so destroying and recreating an attachment neither detaches an old
installed picture from its pipeline nor loses armed decoder-surface retirement
proof. A changed device/queue or surface format refuses with its failure-site
error. At this checkpoint Scene did not yet select or construct the facade.

The facade owns at most one cold-or-warm final-map continuation. It chooses
cold start or warm continuation internally, uses the selected direction- and
level-specific READ intervals, accepts only frame zero followed by exact
same-decode-epoch adjacency, and refuses a decoded source size that differs
from calibration before import or GPU work. One nonblocking device poll per
redraw drives both the four-byte validity callback and draw retirements. While
work remains pending or retirement admission is full, the old installed ready
stays draw-capable. Full and in-flight are typed retry states; terminal device,
mapping, provenance and arithmetic errors retain their original text. Exact
full `FrameStamp` acknowledgement becomes visible only while the facade lock
holds root publication and window staging as one transition.

Every screen or screenshot permit seals a separate immutable uniform and
picture bind group around its exact `Reframe`; the retired payload owns those
per-pass resources plus the complete installed source/map carrier. Screenshot
and screen therefore cannot overwrite one another even when their command
buffers are submitted in the opposite order. The callback that supplies a
Reframe receives the exact ready stamp selected under the facade-to-root lock
order, not an offered-frame guess. Screenshot has its own bounded permit and
authenticates its root snapshot against the facade acknowledgement.

Ordinary success and semantic validity refusal use the completed four-byte map
callback to disarm the exact latest submission without another wait. An
unacknowledged `SubmissionLease` destructor never polls or blocks; cancellation,
unwind, device-poll failure and callback failure/disconnect retain the decoder
owner for process life and quarantine the facade. Install failure occurs only
after mapped completion, so it quarantines state but drops its completion-
proven source normally. Mapped semantic refusal is likewise completion-proven,
rolls back safely and preserves the prior installed ready. The focused
forced-RADV lifecycle covers Cold0 to Cold2, first warm and later warm, exact
acknowledgement timing, invalid-status
rollback with zero wait, full backpressure, private screen/screenshot
Reframes, attachment recreation with shared retirement ownership, carrier-
before-root unwind ordering and predecessor immutability.

This checkpoint did not yet implement normal seek/reopen draining.
Dropping a capture with uncertain callbacks is nonblocking and fail-closed, so
its decoder owners may remain retained until process exit. Scene integration
must keep old sessions alive and poll them to empty during normal replacement.
The first lazy session construction also runs the resident stages' existing
target-device arithmetic qualifications synchronously. Those constructor-only
diagnostics perform bulk readbacks and waits; after construction, ordinary
frame submit/redraw maps only the four-byte validity word and never waits.
Sharing or caching successful qualifications across capture sessions is a
later performance task, not part of this correctness checkpoint.
Existing explicit diagnostic packed-map, alpha and uniform readbacks remain
test/instrument-only; ordinary playback exposes no raw wgpu handle, mapped
frame payload or frame-sized CPU transfer. Studio stays the frozen correctness
oracle, and no optimization reverse engineering was performed.

**Typed GPU-resident warm post-L1 join and atomic continuation, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; FORCED-RADV QUALIFIED;
NO SCENE, PERFORMANCE OR STUDIO-RE CLAIM]:** the exact warm L1 terminal now
continues through all nine existing warm post-L1 arithmetic passes, bilateral
small-row classification, generic final mapping and atomic resident install.
One sealed consuming owner retains the original typed resident validity,
imported source, capture root and source `SubmissionLease`; it appends post-L1
and classification to one command encoder and submits once through that lease.
No raw GPU buffer, second submission owner or queue submission crosses the
join. The classifier binds the predecessor rows only through its complete
purpose-specific operation, and only the joined tail can mint the opaque
successor state consumed by attachment.

The successor retains this call's lack rows, classified bilateral rows and
their exact present/absent topology, public flow, retained L2, histogram,
FIFO, hints, paired cadence after one call and the completed cold ordinal
fixed at 3. Classification consumes predecessor pre-increment counts: the
first warm after installed Cold2 materializes rows, and the later warm consumes
that exact installed first-warm state. The installed predecessor is never
written, and carrier fields continue to drop before root rollback.

The focused forced-RADV production-chain regression runs real Cold0 through
Cold2 install, first warm full post-L1/classifier/final-map/install and a later
warm full chain/install. Both warm maps match the frozen CPU packed-map oracle
bit for bit and the capture-static alpha byte for byte; those frame-sized
copies remain test-only, while ordinary publication maps only the inherited
four-byte validity word. Exact prior `Arc`, capture identity and source-lease
continuity are asserted at each warm call. Opaque byte fingerprints covering
motion references and every retained post-L1 allocation prove predecessor
immutability. A failing word injected on the typed warm L1 terminal survives
the join and refuses publication without changing ready state, and a fully
materialized warm candidate refused by a full retirement queue likewise
preserves the old committed successor and ready draw. Studio remains the
frozen correctness oracle; no reverse engineering or optimization was
performed.

**Iced installed-resident draw adapter, 2026-09-01, implementation branch
only [SUPERSEDED BY THE CAPTURE-SHARED FACADE ABOVE; PRIVATE AND UNSELECTED;
NO PRODUCER, PLAYBACK, PERFORMANCE OR PARITY CLAIM]:** this earlier checkpoint
established bounded render-pass retirement, one-shot iced staging and typed
full backpressure. The capture-shared facade above replaces its pipeline-local
ownership and shared-uniform assumptions with capture-stable retirement and
draw-private picture resources. At that checkpoint no resident producer was
connected to Scene; the selected cutover above retires that limitation.

**Allocation-identical installed warm prior and temporal continuation,
2026-09-01, implementation branch only [PRIVATE AND UNSELECTED; THROUGH WARM
L1 TERMINAL ONLY; NO WARM POST-L1 JOIN, FINAL MAP, SCENE, PERFORMANCE OR
PARITY CLAIM]:** the capture root now installs the complete immutable post-L1
successor state rather than detached buffers and cadence counts. That state
retains public flow, direction-major pixel-interleaved retained L2, temporal
histogram and FIFO, dense hints, direction-owned lack rows, bilateral small
rows with their optional topology, the real paired cadence object and the
completed cold-start ordinal. A warm reservation alone can mint an opaque
`InstalledResidentPrior` holding the exact committed successor `Arc` and its
capture context. It exposes only size validation and purpose-specific GPU
copy/bind operations; no buffer handle or cloneable field bundle crosses the
boundary.

`GpuMotionTransaction::submit_resident_warm` derives that prior from its own
root-carried reservation and advances the same imported source
`SubmissionLease` through warm L2, the retained bridge and warm L1. Before
either level consumes work modes, one ordered GPU command constructs a new
successor-side lack allocation direction by direction: a pre-increment count
of zero selects that direction's current L1 lack rows, while every other count
retains that direction's installed rows. The installed predecessor is never
written. Installed small rows remain the warm work-mode input; updating and
attaching successor small rows belongs to the later combined warm post-L1
join.

Focused forced-RADV tests on AMD Radeon 760M Graphics, RADV PHOENIX prove an
actual Cold0 through Cold2 final-map/atomic install followed by a second
resident imported source through warm motion, L2 and L1, with exact prior
`Arc` identity, flight identity, source-lease continuity and rollback that
preserves the installed successor and ready draw. A separate asymmetric
cadence test proves same-flight count-zero refresh in one direction and
installed-row retention in the other while predecessor bytes remain
unchanged. A malformed installed allocation is refused and rolls its pending
reservation back without changing committed identity or predecessor bytes.
Studio remains the frozen correctness oracle; no optimization reverse
engineering was performed.

**Atomic resident source-plus-map install prerequisite, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; NO SCENE, WARM, PLAYBACK,
PERFORMANCE OR PARITY CLAIM]:** successful Cold2 final-map validity now has one
consuming path into a root-free installed draw. The inherited submission lease
first proves the joined compute work complete and returns the exact imported
picture/geometry owner. The post owner simultaneously removes and seals the
exact root reservation and successor into a separate installation capability.
The remaining geometry, validity, post-L1 state, public flow, resident packed
map, capture-static alpha and imported picture are inseparable in one
`InstalledOneXsDraw`; its only operations write the current `Reframe` to that
picture's exact retained uniform and bind/draw it through its exact retained
`DirectType2Pipeline`. It exposes no source, texture, buffer, bind group,
device, queue, pipeline or detached draw component.

Before binding or retirement admission, the retained direct pipeline proves
that it belongs to the map's exact graphics device. Retirement capacity is a
typed bounded admission result. A carrier-first install owner retains the
whole draw before the root candidate and permit, so refusal and unwind release
the source/map carrier before reservation rollback. Under the capture root's
one mutex, installation authenticates context, session, root allocation,
flight/frame, pending seal, generation, allocation-identical prior and
quarantine state, then replaces committed successor and whole ready draw and
clears pending as one transition. Failure publishes neither and the old ready
allocation remains available. Each redraw snapshots only an `Arc` of that
whole ready payload and reserves a fresh permit without changing history.

The focused forced-RADV chain covers real Cold0 through Cold2, final validity,
binding, permit, atomic install, old-ready retention while the next flight is
pending and while retirement is full, exact old-uniform update despite a
separately created pipeline/uniform, carrier-before-root refusal witnesses,
two exact render-pass retirements and repeated redraw without history commit.
A second forced-RADV test proves cloned-device acceptance and independently
requested-device refusal before binding or permit reservation. This remains a
private prerequisite: Scene cannot select it, warm arithmetic cannot consume
it, and it makes no playback, screenshot integration, performance, rendered
parity or Studio-parity claim. Studio remains the frozen correctness oracle;
no optimization reverse engineering was performed.

**Mode-neutral resident final-map/install adapter, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; COLD BEHAVIOR ONLY;
NO WARM OWNER, SCENE, PERFORMANCE OR PARITY CLAIM]:** the completed resident
post owner is now retained as a sealed `GpuFinalOperands<P>` through the
existing asynchronous four-byte final-map validity poll. Final-map
materialization, installed-map binding, typed draw-retirement admission and
atomic root publication no longer name Cold in their API. The reservation and
successor checks now apply to every sealed `GpuMotionResidentL2Post<P>`.
Cold2 keeps its concrete checkpoint and sole production admission, so no
caller can manufacture another mode, source owner or root identity.

After the typed owner is validated and split, only its complete
`GpuFinalDrawCarrier<P>` is erased behind the installed binding's private
lifetime/root-match trait. The imported picture remains concrete, no raw GPU
handle is exposed, and the carrier/source/root field ordering is unchanged on
refusal and unwind. Production context and upstream-install failures propagate
their original error text directly. The existing Cold0 through Cold2,
final-map, direct-draw and atomic-install path remains the only executable
behavior and is the focused regression. This adapter does not implement or
admit warm arithmetic, change Scene selection, read back frame-sized data or
perform Studio optimization reverse engineering.

**GPU-resident warm post-L1 arithmetic prerequisite, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; NO SMALL-ROW CLASSIFIER,
INSTALL, SCENE, PERFORMANCE OR PARITY CLAIM]:** the private resident post-L1
shader now has a type-sequenced warm continuation on the same unfinished
command encoder. It emits successor L1/L2 hints from the raw paired L1 PIS
terminal before temporal median, filters only dcol while preserving raw drow,
derives retained L1 directly from the allocation-identical prior public field
with the selected pairwise columns 0 through 27 and scalar-tail columns 28 and
29 associations, densifies the filtered sparse result, applies the exact
motion-selected 1/0 and 0.02/0.98 retained blend, resizes to public, repairs
the periodic boundary and emits successor retained L2 in direction-major,
pixel-interleaved `(dcol,drow)` order. It consumes the established packed-U8
L1 motion allocation in place and decodes all four byte lanes. Histogram and
FIFO are copied into new successor allocations before median, so a refused or
pending candidate cannot mutate the installed predecessor.

The continuation pauses after hints and median as an opaque
`GpuWarmPostL1Paused`. Only the classifier's sealed, exact owned
successor-state output can resume that same encoder, and that output remains
attached for the later installed-prior warm reservation; there is no loose
buffer or provenance constructor. This checkpoint does not implement the
classifier, construct a production warm prior-public owner, join the capture
root, publish or install a ready result, or touch Scene. Ordinary work adds no
CPU readback; only forced-RADV oracle qualification reads diagnostic outputs.
The oracle uses asymmetric directions, retained NaN/Inf including full-motion
multiply-by-zero contamination, mixed packed-motion lanes, refused-candidate
predecessor immutability, periodic pairs and exact retained-L2 ABI. It rejects
retained-association, raw-hint, special-value fast-path and motion-unpack
semantic mutations. Cold scheduling and arithmetic remain unchanged.

**GPU-resident mature small-disparity row prerequisite, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; FORCED-RADV QUALIFIED;
NO WARM POST-L1 JOIN, INSTALL, SCENE, PERFORMANCE OR PARITY CLAIM]:** an
isolated sealed two-pass GPU stage accepts the exact post-median paired sparse
field, common A-side L1 block mask, prior bilateral small-row allocation and
its distinct optional topology, paired pre-increment counts, GPU context,
producer frame flight and allocation-identical capture root. Its ordinary
output has no CPU-readback usage. Constructor qualification alone allocates a
diagnostic readback copy and dispatches the same pipelines through an
arithmetic-only fixture with no frame, flight, capture reservation or
successor. The returned typed owner retains its private
candidate, config, binding and identical bilateral output allocation.

One serial invocation per row and direction visits all eight patch columns in
CPU order. Masked sites are excluded; an empty count takes positive zero;
comparison is strictly `mean < 5.0`; an active NaN therefore classifies false
while a masked NaN is ignored. Pre-increment counts below three preserve the
prior optional topology and rows. Counts at least three materialize present
topology even for all-zero results. A second dispatch bilaterally ORs both
direction candidates and writes the identical canonical result to both halves;
absence survives only when both candidates are absent.

Forced RADV qualification on AMD Radeon 760M Graphics, RADV PHOENIX, Mesa
26.1.6 covers counts two and three, negative and maximum signed counts, absent
versus present-all-zero, exact-five threshold, positive and negative zero,
negative disparities, order-sensitive accumulation, masked and active NaN,
all-masked rows, asymmetric maturity, direction asymmetry and one-sided
retained rows. Live mutations reject a non-strict threshold, wrong mask
polarity, one-sided merge, late or unsigned maturity, drow consumption,
missing absolute value and reverse column order. A foreign GPU context is
refused before encoding, and the opaque result preserves its producer flight
and capture root for validation by the future attachment boundary.

This checkpoint intentionally has no production frame-input implementation or
warm work-mode binding. Only a private constructor-qualification fixture
currently implements the sealed arithmetic projection; it cannot reach the
provenance-bearing ordinary encode. The later warm post-L1 owner must implement
both sealed projections inside the module and append this work to its existing
producer submission. Atomic installation must move the whole typed result into
that producer's successor.
Only the next reservation's exact installed-prior owner may then expose those
rows to warm work modes; its consumer flight is intentionally not compared to
the stored producer flight. Studio remains a frozen correctness oracle only;
no optimization reverse engineering was done.

**Capture-scoped resident calibration ownership, 2026-09-01, implementation
branch only [PRIVATE AND UNSELECTED; NO POST-L1 JOIN, INSTALL, SCENE,
PERFORMANCE OR PARITY CLAIM]:** the sealed imported ONE X2 front transition now
names one `ResidentSourceCapture` and nothing else. Constructing that capture
from one calibration and its orientation track privately creates its parent
builder and calibration-owned readout, all `OneXsResources`-derived geometry
and final-map statics, the shared GPU pipelines and its non-cloneable resident
root. The reusable producer is no longer crate-visible. A caller therefore
cannot combine a same-device root from one capture with another capture's
parent inputs, orientation or static resources, and none of those values is a
per-frame submission argument.

The same capture is now the only source-import entry. It mints an opaque
allocation identity into the inseparable imported picture, and submission
checks that identity and GPU context before reading its frame, reserving the
root or encoding. The imported aggregate still moves whole into the existing
source submission lease. Structural tests pin the capture-only import and
submit signatures and the single calibration composition boundary. A
forced-RADV test constructs two sessions with distinct lens calibration,
readout and orientation but a colliding `FrameStamp`; a source tagged by B is
refused by A before either root or encoder advances, then encoding each
session's own frame advances only its own root generation and parent encoder.
This checkpoint changes no selected behavior and does not implement the Cold2
final-map operand join, atomic ready/successor publication, draw retirement
wiring, warm state or performance work.

**Concrete Cold2-to-final-map resident bridge, 2026-09-01, implementation
branch only [PRIVATE AND UNSELECTED; NO SCENE, INSTALL, WARM, PERFORMANCE OR
PARITY CLAIM]:** the exact
`GpuCompletedColdCheckpoint<ImportedOneXsPicture>` now has one private,
nonconstructible concrete admission into the existing final-map materializer.
Admission consumes and validates the prepared owner, structural GPU context,
flight-derived frame, imported-picture identity, allocation-identical capture
root, post reservation, four-byte validity owner, exact resident-parent
variant and every source buffer size before the materializer allocates or
encodes anything.

The geometry owner supplies only purpose-specific fixed copies. Parent and
base storage remain A then B. Final lens A receives parent words 0 through
39,999, base words 0 through 129,599 and the Cold2 public B-to-A half at word
129,600. Final lens B receives parent words 40,000 through 79,999, base words
129,600 through 259,199 and the Cold2 public A-to-B half at word zero. The
four-byte validity copy follows the unchanged final dispatch. The one inherited
imported-source submission lease is advanced exactly once and remains the
first owner on submit, validity, mapping, drop and unwind paths. No raw buffer,
queue, command, offset, caller-chosen direction or frame accessor was added.

This checkpoint joins no Scene or installation path, implements no warm tail,
does no frame-sized CPU readback, upload or reconstruction, and makes no
playback, rendered-parity, Studio-parity or throughput claim. Studio remains a
frozen correctness oracle only; this change performs no optimization reverse
engineering.

**Retained level-two producer/consumer ABI correction, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; FORCED-RADV CONSUMER
QUALIFIED; ACTUAL WARM CHAIN BLOCKED; NO SCENE, INSTALL, PERFORMANCE OR PARITY
CLAIM]:** the private warm L2 bridge now consumes the exact direction-major, pixel-interleaved
`(dcol, drow)` allocation emitted by Cold2 `make_retained_l2`. Its named
storage type and CPU/WGSL index helpers distinguish that ABI from the planar
PIS terminal, seed and hint grids, whose layouts are unchanged. Constructor
fixtures, CPU twins, nonfinite coverage and the warm hint/terminal qualifier
use the corrected contract; restoring the former component-planar retained
index is a live rejected shader mutation.

Forced-RADV qualification supplies asymmetric A-to-B and B-to-A dcol/drow
retained fields, exercises warm L2 seeds and L1 terminal bits against the
frozen CPU oracle, and proves that restoring the old planar index fails. The
allocation-identical production Cold2-to-first-warm regression remains a
mandatory gate for the later atomic-installed warm transition: this snapshot
has a `GpuColdPriorPublicLevelTwo` and retains Cold2's real allocation inside
the completed cold checkpoint, but has no warm prior-public owner or
transition that can consume it. This correction does not widen that ownership
API, invent a raw-buffer adapter, or claim the unavailable chain.

**Capture-static resident ONE X2 final-map resources, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; FORCED-RADV QUALIFIED;
NO POST-L1 JOIN, SCENE, PLAYBACK, PERFORMANCE OR PARITY CLAIM]:** the resident
source-front pipeline now constructs its geometry and final-map stages from
the same validated `OneXsResources` and exact `OneXsGpuContext`. The private
final-map materializer owns one nonconstructible `GpuFinalMapStatics`: the
calibration-derived 200-by-100 gates and coordinates for both lenses and the
exact pole-completed alpha map are uploaded once, not accepted from a frame
operand. Its only operation copies each static side into the fixed shader
input slots. No gate, coordinate, alpha, buffer or binding accessor crosses
the private resident owner.

Each pending, ready and Scene-bindable result retains the exact same static
owner by `Arc`; final alpha binding is borrowed from that owner. The sealed
per-frame operand can supply only its authenticated context and frame, fixed
copies for each side's preimage, base map and public flow, the inherited
four-byte validity copy, and the existing submission lease. A write-only
capability fixes all dynamic destinations and does not reveal the combined
input buffer, so an upstream owner cannot overwrite capture-static slots.
Carrier-first ownership also retires the upstream lease before statics on
validity, mapping and submit refusal paths.

The resident validity decoder now owns disjoint typed namespaces: success is
`u32::MAX`, existing PIS failures are words 0 through 5695, and generated
successor-hint failures use bit 31 plus level, direction, component and a
14-bit dense site. Reserved bits and out-of-range sites surface the exact raw
unknown-status word instead of being mislabeled as PIS failures. The sibling
post-L1 Rust join and its shader-layout test can use a semantic checked
encoder after integration; this branch has no post-L1 call site, and that
shader must still prove its hard-coded bit layout matches the encoder. Eleven
focused tests pass on AMD Radeon 760M Graphics, RADV PHOENIX, Mesa 26.1.6.
They cover exact production-resource upload, dynamic/static range separation,
a static-gate mutation, context and frame identity, static retention through
binding, carrier-first refusal, the full validity namespace, the unchanged
bit-exact CPU/GPU qualification and all 20 live shader mutations.

This checkpoint deliberately does not invent the missing production
`GpuGeometryFrameOwner` copy of preimage, base and public flow, join post-L1
to the materializer, install a result or select it in Scene. The capture root
also still receives `ParentMapBuilder`, orientation and readout per frame;
consolidating those with the exact calibration that constructed
`OneXsResources` is the next capture-ownership join, not a claim made here.

**Sealed source-to-resident-front transition, 2026-09-01, implementation
branch only [PRIVATE AND UNSELECTED; NO SCENE, POST-L1, PLAYBACK, PERFORMANCE
OR PARITY CLAIM]:** `ImportedOneXsPicture` now has one consuming transition
into the existing resident parent, geometry and belt front half. The caller
supplies only the resident capture, frozen parent inputs and readout. The
transition derives the opaque `FrameStamp`, GPU context and exact two luma
textures from the sealed imported owner. Context refusal happens before the
capture reservation or command encoding. Parent, geometry and belt commands
then share the existing unfinished encoder, and the same complete imported
owner moves into the first submission lease.

The crate-visible admission method accepts the concrete
`ImportedOneXsPicture`, not `SourceTextures` plus an independently chosen
owner. Its one-shot binder cannot be constructed by a caller and returns only
an opaque `ResidentImportedFront`. Its concrete motion continuation remains
private to the resident owner until that later boundary is qualified. No bind
group, texture, plane, decoder frame, device, queue or submission handle is
returned. The two temporary wgpu texture
clones remain inside the consuming implementation and name the owner's own
imported luma allocations; they exist only to end Rust field borrows before
the complete aggregate moves into the lease.

Generic field ownership still proves binding, planes A, planes B, decoder
frames and context drop in that order. Structural tests pin concrete-owner
admission, internal stamp/context derivation and the absence of raw resource
accessors. Existing GPU lease tests remain the proof that a source owner is
retained until exact submission completion. This checkpoint does not
fabricate dmabuf success and does not install a resident result.

**Resident ONE X2 Cold0 through Cold2 post-L1 checkpoint, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; NO SCENE, PLAYBACK,
PERFORMANCE OR WARM-TAIL CLAIM]:** the accepted paired L1 terminal now remains
under its original imported-frame lease, source owner, root reservation and
four-byte validity allocation through three internally typed cold ordinals.
Each ordinal emits raw L1 then propagated L2 successor hints into the full
275,400-word planar GPU allocation before temporal median, advances the paired
cadence only after successful submission, densifies without warm retained
blending, and performs the exact x2 public resize. Cold0 and Cold1 retain only
unpublished scratch public fields. Cold2 alone repairs the periodic boundary,
derives the next warm L2 retained field, and attaches public, temporal history,
hints, direction-owned lack rows, absent small rows and cadence `[3, 3]` to the
opaque pending motion successor.

The ordinary path adds no frame-sized CPU readback or upload. Warm post-L1
execution, final validity mapping, ready installation and Scene selection stay
excluded; this checkpoint constructs only their purpose-specific typed owner.
Cold work-mode preparation is now supplied by the bridge-owned GPU derivation
described immediately below; the typed loop retains only the disparity
controls minted at Cold0.

**GPU-resident ONE X2 PIS work-row prerequisite, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; FORCED-RADV QUALIFIED;
NO SCENE, PLAYBACK OR PERFORMANCE CLAIM]:** the resident cold/warm PIS packer
no longer accepts caller-supplied CPU `CostMode` arrays. It reserves canonical
zero placeholders only. A bridge-owned compute pass overwrites every private
mode slot in the same command encoder immediately before the corresponding
PIS dispatch. Cold reads the current paired direction-owned L1 lack rows;
warm reads the retained paired lack and small-disparity rows and uses
`small || lack`. L2 is not reclassified: each direction serially scans the
same 178 L1 rows and applies the exact READ contiguous-run geometry to its 88
L2 rows.

The pass exposes no buffer or bind group. Its sealed binding carries the exact
GPU context, frame flight, cold/warm meaning and target level. Four
level-specific entry points make a valid dispatch unconditional. Before the
private dynamic allocation is uploaded, the host rejects any wrong schema,
direction base, level shape, mode offset, sentinel or nonzero CPU-supplied
placeholder. Ordinary dynamic storage has no `COPY_SRC`; the per-frame
work-mode operation has no copy, map, poll, wait or CPU readback. Constructor
qualification alone uses a separate readback allocation to authenticate both
levels and both meanings.

Forced RADV tests cover asymmetric A/B ownership, both edges, singletons,
contiguous runs, alternating rows, all false, all true, cold ignoring planted
small rows, warm `small || lack`, exact L1/L2 dynamic offsets and untouched
sentinels. The CPU transcription matches all 15,931 possible single
contiguous L1 runs. Live mutations reject direction swaps, OR/polarity,
offset and propagation changes, and actual paired L2 then L1 PIS terminals
match the CPU oracle for both cold lack rows and non-vacuous warm retained
rows. Production warm ownership remains deliberately absent until the
post-L1 retained-state owner lands; the sealed warm hook refuses owners that
cannot supply those exact paired rows.

**Sealed ONE X2 source-import ownership prerequisite, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; NO SCENE, PLAYBACK,
PERFORMANCE OR PARITY CLAIM]:** the direct type-2 module now owns one private
aggregate whose only production constructor consumes `Arc<Frames>`, refuses
anything other than exactly two lens frames, imports both dmabufs itself into
`[Planes; 2]`, and creates the exact picture bind group from those imports.
There is no from-parts, raw-plane or stamp-only constructor. Its declaration
order deliberately releases the bind group, both imported plane pairs and
only then the exact decoder-frame owner, so a published aggregate cannot hand
an aliased VA-API surface back before the wgpu objects that refer to it.
The aggregate exposes no borrowed or owned wgpu handle: its sole rendering
operation performs the direct type-2 bind and draw inside `direct_type2` and
returns nothing. A caller therefore cannot clone the picture bind group and
drop the planes and frame owner out from under that clone.

The existing `VecDeque<Live>`, `Vec<Planes>` import and `RETAINED` policy are
unchanged for legacy and currently selected playback. Focused CPU-only tests
cover zero, one, two and three-lens structural admission and exercise the
aggregate's actual generic field-drop order as bind group, planes A, planes B,
then frames. They intentionally do not fabricate `DrmFrame`,
`AVDRMFrameDescriptor` or decoder allocation: constructor success, first- or
second-dmabuf import failure and real wgpu destruction therefore still require
a compatible Vulkan device plus decoder-backed test media. No resident map is
joined, installed or selected at this checkpoint.

**Resident ONE X2 drawable installation prerequisite, 2026-09-01,
implementation branch only [PRIVATE AND UNSELECTED; NO SCENE, PLAYBACK,
PERFORMANCE OR PARITY CLAIM]:** the native type-2 consumer is split into one
immutable reusable shader/pipeline/map-layout owner and a per-result binding.
The selected CPU path still allocates the same exact-size packed and alpha
buffers, writes the same bytes, uses the same shader and bind order, and
reports the same bound frame. Scene selection is unchanged.

The accepted render-pass retirement queue now has an iced-compatible shared
adapter. It reserves and polls through interior mutability and arms the
submitted-work callback on the borrowed render pass before the guarded draw.
No-submit work remains bounded and retained. Registration, draw and poll
panics quarantine uncertain owners; mutex poison is recovered only to expose
that terminal state, never to resume ordinary retirement.

The exact installed-source boundary remains deliberately unimplemented. The
current dmabuf import API returns `Vec<Planes>` separately from `Arc<Frames>`,
so a later constructor accepting those values separately could retain and bind
them but could not prove that the textures came from that exact frame
allocation. A synthetic owner test cannot close that production association.
The import site must instead produce a sealed aggregate that makes the frame,
its imported planes and their bind group inseparable. Only a validated
resident final-map result may then join that aggregate. This checkpoint does
not fake that association, publish or install a resident result, wire Scene,
or replace `live.truncate(3)` in selected playback.

The combined-tree qualification is recorded in
`docs/research/gpu-drawable-install-qualification.md`. It binds the exact code
and tree, forced-RADV focused and workspace tests, required repository gates,
test executable hashes and durable logs under repository `scratch/`.

**Asynchronous resident final-map validity gate, 2026-09-01, implementation
branch only [FOUR-BYTE READBACK; FORCED-RADV QUALIFIED; NO SCENE WIRING OR
PERFORMANCE CLAIM]:** the private final-map materializer now accepts one
additional purpose-specific operation from its sealed operands: append a copy
of the exact resident L2-to-L1 finite-center status word. It exposes no source
buffer, binding or value. The copy follows the unchanged final-map dispatch in
the same command buffer and inherited submission lease and targets one
four-byte `MAP_READ | COPY_DST` staging allocation.

Materialization now returns an opaque pending frame with no Scene-binding
operation. It retains the complete upstream/source owner, packed map, exact
capture-static owner, frame and context while `map_async` is outstanding. One
nonblocking poll calls `Device::poll(Poll)` once and checks the callback
channel once. Pending stays pending without changing prior capture state. Only
`u32::MAX` converts into the existing bindable resident frame. PIS failure
words retain the existing `(direction * 2 + component) * 1424 + patch`
contract and exact `SeedError::NonFiniteCenter`; the disjoint tagged namespace
described above carries generated-hint failures. Unknown words retain their
raw hexadecimal value. Mapping failures retain the underlying map error text.
There is no wait, polling loop, bulk readback or Scene selection on the
ordinary path.

The focused actual-RADV tests exercise bounded safe polling, valid conversion
and binding, precise invalid refusal with upstream-drop ownership, mapping
failure text, foreign frame/context refusal, the packed-map CPU twin and all
20 accepted live shader mutations. Final-map arithmetic, dispatch dimensions
and mutation gates remain unchanged. A fresh post-commit receipt follows in a
documentation-only successor.
**Root-reserved resident ONE X2 frame chain, 2026-09-01, implementation
branch only [FORCED RADV; PRIVATE OWNER; NO READY PUBLICATION]:** one private
capture root now owns the monotonic generation, exact pending seal, committed
successor, future-ready placeholder and quarantine state under one mutex. Its
linear reservation is minted before parent encoding and moves unchanged
through parent, geometry, the first and only belt submission and motion. No
second motion reservation or caller-generated flight exists. Motion derives
cold/warm state and its exact prior reference only from the reservation's
immutable allocation-identical `Arc` snapshot; the candidate successor owns
the next motion references and private space for later post-L1 storage.

Drop and explicit refusal clear only the exact pending seal. Stale or poisoned
rollback fails closed, retains affected prior/successor allocations and
quarantines reuse; generation is never rewound, and seek/reset constructs a
new capture. The submission carrier is structurally dropped first at every
motion transaction/frame boundary, including unwind and submit failure, so its
exact lease waits or quarantines before the reservation, prior or successor
can be released. A GPU witness observes completed polling while the root is
still pending and the prior snapshot is still retained for ordinary drop,
explicit abort and unwind on both transaction and frame tokens.

Tests also prove first-frame cold state, committed-prior warm state, exact
drop/retry, stale-token isolation, allocation identity rather than stamp
equality, foreign-context refusal, early duplicate/front-half admission
refusal, and the absence of production ready publication. Scene and final-map
installation remain deliberately unwired. The authenticated receipt is
`docs/research/gpu-root-capture-state-qualification.md`; it binds code commit
`7f0af2c91d1f842e4ed200eac6b513b228147448`, tree
`487de81c0fcda9148754e22ede951aa3bd3a494e`, fresh build/lint gates and 41/41
forced-RADV tests on AMD Radeon 760M Graphics (RADV PHOENIX), Mesa 26.1.6.

**Pre-submission resident ONE X2 parent maps, 2026-09-01, implementation
branch only [BIT-EXACT ON FORCED RADV; PRIVATE OWNER; NO SCENE WIRING]:** the
frozen parent arithmetic is a private child of the same owner as retained
geometry. For each exact `GpuPisFlight`, it derives center only from the
private `FrameStamp`, uploads the compact two 33-word control packs plus 51
pose quaternions, and encodes both 100-by-200 float2 parents into resident
storage. Its opaque product retains that storage, input, resources, context,
flight and unfinished encoder. It has no raw component accessor and no local
production submit, lease, readback, map, poll or wait.

The private geometry child is the only consumer. It refuses a foreign context
before allocation or encoding, then appends map merge, continuity filtering
and mask construction directly to the same unfinished encoder while binding
the resident parent output. Only the resulting encoded geometry can append
source sampling and Gaussian work; that belt transition performs the frame's
first queue submission and mints its sole decoder-surface lease. Thus parent,
geometry and belts execute in dependency order within one submission, with no
CPU `pair_bytes` reconstruction and no parent-after-belt cycle. The geometry,
source owner, parent inputs and every intermediate remain retained through the
existing frontend/PIS owner chain.

Forced RADV PHOENIX qualification compares all 80,000 parent words in seven
cold, warm, live-center, live-orientation, endpoint/clamp and changed-readout
cases; live mutations change the CPU oracle. It rejects nonlinear slerp before
encoding and rejects 13 planted semantic mutations covering quaternion order,
raster orientation, scan axis, iteration count, movement threshold, FOV,
A/B base, pose layout, both endpoint clamps, division, square root and
rounding. A composed parent-to-geometry-to-belt-to-frontend test proves source
ownership remains retained until the one inherited submission is explicitly
acknowledged. Scene remains unchanged. The remaining seam is connecting the
existing post-PIS/final-map private owners into this complete pre-submission
front half without exposing a new public transition.

The authenticated clean-build receipt is retained under the owner's repository
at `.agents/gpu-parent-evidence-topology/scratch/gpu-parent-map-evidence/`.
It binds qualified code commit
`441078dc6245e56b39748444f04e8454801f6d75`, tree
`c2cd5536d7a557e09ec2a719adf58db3621a7986`, clean pre/post status, an
initially absent dedicated target directory, the exact build and test
environments and commands, Mesa/RADV packages, ICD and loader hashes, source
hashes, adapter limits, timestamps and exit status. The complete log hashes to
`8ef951e42492f532c40ccd3b1047f1360b54eb593382910388c1ddf5a6e78a2d`,
the receipt to
`19da93bf9d0ad78d0b335c79d76f6b4d40909eefb040a09817fa6e26df2f198d`,
and the exact test executable before and after the run to
`b30c547fc9119a0061f121312f63d7562101f64788321b7f9a8f5a5fd952281d`.

**Resident L2-to-L1 ownership continuation, 2026-09-01, implementation branch
only [UNSELECTED; FINAL RADV RECEIPT PENDING; NOT SCENE-WIRED]:** the ordinary
boundary consumes the root-carried motion transaction directly rather than
sealing a separately publishable frame first. Its opaque noncloneable root
reservation and successor remain inside the post-L2 owner while the same
prepared images, masks, context and submission lease advance through L2 PIS,
resident post-L2 and L1 PIS. Carrier-first field ordering waits or quarantines
the inherited submission before any downstream refusal can roll back the root
or release successor allocations.

Temporal history alone derives Warm L2 or Cold0 L2. A sealed paired cadence
owner supplies distinct A-to-B and B-to-A pre-increment admissions unchanged
to both levels; Cold0 starts at count zero and therefore admits every patch.
Only the future post-L1 successor may advance that owner. The bridge decodes
the production temporal layout as four U8 motion codes per word. Cold retained
state plus mandatory present positive-zero L2/L1 hints come from GPU clears;
the typed warm contract binds retained state and planar dense successor hints
but has no production warm owner until post-L1 lands. L2 initial grids are
always GPU-cleared, dense hint centres fill L2, and L2 seed planes plus dense
hint centres fill L1. No ordinary CPU bulk grid, terminal upload, readback,
map, poll, raw buffer/command escape or competing lease exists.

Constructor qualification fails closed for the L2, seed and hint shaders.
The focused tests compare cold and warm bridge bits, exact seed injection and
the actual paired L2 and L1 terminals to the CPU chain; nonzero resident hint
centres and the exact temporal producer buffer are included. A production-shape
Cold0 test consumes geometry, root reservation, motion, prior state, L2 and L1
on one lease. The packed-motion fixture
records histogram `{0: 4021, 1: 5, 16: 1, 64: 5, 65: 1, 176: 2, 192: 1,
223: 1, 255: 13}` and covers zero/nonzero in every byte lane and both physical
edges. The earlier rejected bridge evidence expanded motion bytes and omitted
mandatory L1 hints; it is not an acceptance claim.

The complete three-call cold transaction remains blocked on the post-L1
owner. Cold0/1 must return a purpose-specific linear loop whose only resume
derives Cold1/Cold2, preserves this same reservation, prepared frame, lease,
post state, validity, flight and context, and binds loop-owned resident hints.
This checkpoint deliberately exposes no generic or caller-forgeable resume.

**Exact render-pass retirement checkpoint, 2026-09-01, implementation branch
only [PRIVATE OWNERSHIP PRIMITIVE; NOT WIRED TO SCENE; NO DMABUF SAFETY,
PERFORMANCE OR PARITY CLAIM]:** a bounded private owner now reserves a linear
`DrawPermit` during preparation, before a candidate or successor history can
be installed. No capacity is backpressure at that boundary and does not move
the offered payload. A prepared candidate that is never drawn simply returns
its permit because no GPU sampling needs proof; every repeated redraw reserves
a fresh permit without recommitting history.

The only production arm operation consumes the current
`wgpu::RenderPass`, permit and an `Arc` of the opaque installed-draw payload.
It stores the `Arc`, registers `RenderPass::on_submitted_work_done`, and only
then invokes a closure that borrows the stored payload to bind and draw before
returning the pass. Scene can retain one `Arc`; initial draw, later redraws and
a replaced successor each clone it without separating the exact source, map
or upstream ownership. The caller therefore cannot bind or draw before proof
belongs to iced's exact encoder and command buffer, rather than to an earlier
compute or queue prefix. The callback only publishes its monotonic generation; ordinary
`Device::poll(Poll)` drives it and payload destruction happens outside the
callback only after the exact generation is observed. Queue storage and
signals are bounded and reused with ABA protection. Out-of-order completion
releases each payload once. Poll error or panic and callback-registration panic
quarantine every prior and offered uncertain owner before terminal failure;
destruction with a possibly unsubmitted pass also fails closed instead of
waiting forever.

The generic qualification payload owns a sampled texture and a drop witness
representing the exact decoder source owner. The eventual
`InstalledOneXsDraw` must transitively own the exact `Arc<Frames>` until the
draw callback lands: cloned wgpu objects or a duplicated dmabuf descriptor do
not stop VA-API from reusing the surface. This checkpoint deliberately removes
the obsolete compute-prefix Scene retirement wiring. It does not install the
resident map binding, current frame or arithmetic into Scene and does not yet
claim that selected playback is surface-safe. The earlier queue-prefix receipt
remains only as superseded audit history in
[`docs/research/gpu-nonblocking-retirement-qualification.md`](research/gpu-nonblocking-retirement-qualification.md).

The exact code head `c81a9c3e3e6c35069e951b79d5df17ffe43278ff`
(tree `b2b8383cba88e37656dc4f3fe1130d9c1b9c1608`) passed the focused
forced-Phoenix RADV gate 8/8 with one unchanged pre/post executable hash.
Commands, identities, source/binary/driver/log hashes, timestamps and exit
status are recorded in
[`docs/research/gpu-draw-retirement-qualification.md`](research/gpu-draw-retirement-qualification.md).
This is durable qualification evidence. Independent audit accepted this
private primitive at the recorded code and docs heads, within the stated
limits; it did not accept Scene wiring or dmabuf/source-surface safety.

**Shared-context resident ONE X2 final-map materializer, 2026-09-01,
integration branch only [UNSELECTED; EXACT TARGET-GPU TWIN; NO SCENE OR
PERFORMANCE CLAIM]:** the accepted final bilateral materializer now owns the
one structural `OneXsGpuContext` used by the resident chain. Its production
entry consumes a generic sealed upstream token, advances that token's existing
submission lease, performs only GPU copies plus the exact stored compute pass,
and returns an opaque frame-bound packed-map token. That token retains the
upstream alpha and source ownership and can create the existing two-storage
direct type-2 Scene binding without exposing either raw buffer. Scene does not
construct, store or draw this token yet. Ordinary materialization creates no
mappable buffer and performs no readback or device poll; the CPU oracle and
diagnostic copies are test-only. The forced-RADV focused gate matches every
packed component bit, preserves the accepted arithmetic/order, rejects foreign
structural contexts and frame identities, and refuses all 20 accepted live
shader mutations. The materializer is now a private child of the resident belt
owner, so no flow sibling can name its operand copier, raw command or buffer
details, materializer, binding or output token. The remaining seam is the
post-L1 resident producer's implementation of the owner-private sealed operand
copier and single-lease submit method; there is deliberately no second context,
lease or CPU reconstruction route.

The authenticated receipt in
`docs/research/gpu-final-map-context-qualification.md` binds clean code commit
`e620a4cae27f1a51b8998328620594765c277dd7`, tree
`b7ce05854f507824170c1364ac4bddbc54b110c5`, the fresh retained test binary
before and after execution, exact forced-Vulkan/RADV command and environment,
ICD, adapter/driver, source hashes, timestamps and exit status. Its sealed
pre-run receipt hashes to `98be0bc640364941ab06ba3d5a53e07b07e4bea9155ee5f4ab1f717d9b9d9609`,
the complete 4/4 run log to
`34fe395b5f996a26a4b1e5bbc6943207294c03e723d04a556c41c98216df3c86`,
and the post-run receipt to
`dc1039b41b68df250f0e9bb10a6e05584c64094b2ab8067d9bf6636eb407f777`.

**Context-bound GPU ONE X2 retained geometry, 2026-09-01, implementation
branch only [UNSELECTED; NO SCENE WIRING; SEALED TRANSITIONS]:** the
shared `OneXsGpuContext` now owns a correctness-first geometry pipeline for
both 1,080-by-60 periodic map merges, the selected directional continuity
filters, physical validity seed, 9-by-9 erosion and A/B mask unification.
Static line coordinates upload once. Qualification retains a test-only
CPU-parent upload, while the ordinary boundary now consumes the resident
parent producer directly without changing geometry arithmetic or downstream
layout.

The output is one non-cloneable opaque encoded token containing its exact GPU
context, unfinished command encoder, full frame flight, both parent preimages,
both retained float2 maps, both physical masks and all bind groups. It exposes
no raw buffer, queue, submission index, detached flight or ordinary completion
hook. The geometry stage neither submits nor creates a competing lease. Its
only production handoff appends exact belt sampling and Gaussian commands to
that unfinished encoder; the belt owner then performs the one submission and
mints the chain's sole source-surface lease. A second purpose-specific
transition binds the token's packed masks directly into the sealed frontend.
The geometry allocation and imported source owner remain inseparable inside
that lease through PIS and final materialization. None of the rejected
separable flight, packed-buffer, generic-submit or completion hooks return.

The physical masks use the frontend's exact packed layout: four U8 codes per
word, lens A then lens B, followed by its explicit runtime-zero word. Belt
sampling consumes the retained map directly and the frontend consumes the
packed masks directly, with no map or mask reupload or layout conversion.
Construction compares every retained float word and packed mask word with the
selected CPU oracle on the actual adapter, including signed zero, exceptional
parent payloads, periodic edges and the native ordered FMA schedule. Four live
shader mutations cover FMA operand association, the strict directional
threshold, erosion radius and lens unification. This checkpoint makes no
playback, performance, Studio-parity, owner-eye or final ownership-readiness
claim.

**Sealed resident GPU-prepared PIS checkpoint, 2026-09-01, implementation
branch only [UNSELECTED; L1/L2 AND BOTH DIRECTIONS; NO SCENE OR INTEGRATION
READINESS CLAIM]:** the qualified paired GPU PIS kernel has a concrete
no-readback transition that consumes one complete `GpuPreparedFrame`. The
frame owner privately derives both directions' level-typed word bases, binds
every immutable allocation as a whole storage buffer, encodes and submits on
its stored exact queue, and advances the one inherited submission lease to the
actual returned submission. No prepared buffer, base, flight, device, queue or
submission index is exposed or caller-assembled. The returned opaque terminal
retains the exact flight, `PairSolveStage`, shared GPU-context identity, output buffer and
same non-cloneable frame owner; chaining consumes it back into that same frame,
while explicit terminal acknowledgement waits for the latest lease once.
The producer boundary is sealed as the same aggregate: `GpuBlurredBelts` has
no crate-visible packed-buffer, flight, generic submission or early-completion
hook. Its only front-end transition consumes the whole token, refuses a
foreign context before allocation, binding, encoding or submission, advances
the inherited lease to the exact front-end command, and moves the whole opaque
producer token inside `GpuPreparedFrame`. The frontend is a private child of
the belt owner, so no sibling module can name a bridge object or receive a raw
buffer, command, context, flight, lease, or completion operation. Foreign front-end and PIS regressions
leave both device validation scopes clean and prove the source owner waits the
latest valid fence; independently recreated front-end and PIS pipelines on the
same structural device/queue pair remain accepted.

**GPU-resident warm image-state arithmetic checkpoint, 2026-09-01,
implementation branch only [UNSELECTED; NO SCENE WIRING; NO PERFORMANCE
CLAIM]:** a render-private compute stage beneath the private geometry owner
now consumes its complete sealed geometry/belt aggregate and a capture-owned
retained reference slot.
Warm work produces the exact threshold/population motion mask, recursive L1
and L2 U8 area reductions, and the selected fused 0.7/0.3 next-reference
planes. Cold work copies both current physical planes exactly and exposes no
invented motion result. All storage bindings cover complete word-aligned
buffers, ordinary work has no CPU readback, and the full opaque `GpuPisFlight`
plus a temporal generation seal every candidate. The stage validates the
shared `OneXsGpuContext`, privately binds the physical A/B post-Gaussian belt
within that aggregate, encodes through its inherited linear submission lease,
and returns the complete geometry, masks, source owner and same lease inside
its opaque output. Production cannot enter motion from a separable raw belt
token; that narrower entry exists only in tests.

The next reference remains a private pending successor after the motion
transaction seals its opaque frame candidate. Motion has no production
publication method at this boundary: later L2, post, final-map, bind and draw
installation may still fail. Dropping either the transaction or transferred
frame candidate clears only its matching flight and leaves the
allocation-identical prior committed slot and generation unchanged. The future
capture-owner final install must publish the successor atomically with its
ready resident draw. The target Radeon 760M/RADV matched the CPU oracle for
both physical lenses, base/L1/L2 motion and next references; six live shader
mutations covering lens selection, inclusive change threshold, asymmetric
low-edge bounds, promotion population, recursive rounding and EMA weight were
all refused. Cold final-install publication, warm post-motion drop, exact retry
and warm final-install publication passed in one state regression.

Ordinary cold/warm work does not map, poll, reconstruct CPU images or create a
second submission owner. The remaining connection is one purpose-specific
private-owner transition that consumes the whole `GpuMotionFrame` into the
resident front-end and warm scheduler/post-L1 chain. That transition must bind
the already-retained geometry mask internally rather than split the geometry
carrier back into belts and masks. No current image, L1/L2 physical mask,
geometry mask, flight or lease component is exposed separately for that seam,
and Scene cannot select it until the downstream transition is qualified.

**Post-commit motion evidence, 2026-09-01:** commit `8c25706` and tree
`8afa41b9` were clean before and after a build in a new dedicated target. The
fresh binary did not exist before the forced test; afterwards its SHA-256 was
`fc6cfabb434f43fd0bc6c4a801e980eca3be6512b025ef1475b151cf5a506aad`.
All 31 private-owner tests passed under the forced RADV ICD, including final
install publication, transferred-frame rollback/retry and the exact
submission-lease drop/panic tests. The forced-run log SHA-256 is
`4dc5ca1f7830f244cb88483fddf0fd04e8619e6b16b3b4bcc2837a9a413f92b0`;
the warnings-denied fresh-target gate log is
`1d4a60ff29741c071ddec8ef42e1fda070f31a3377e561a5e96cf7ff4215a802`.
The complete source, WGSL, ICD, driver, package, timestamp, command and
pre/post evidence is tracked in
`docs/research/gpu-motion-context-qualification.md` (pre-ROADMAP-edit SHA-256
`af27ac42c5d775fd5574941d143f13a838320a7e4b4ec78c516ea31be72611e5`).

Only cost modes, initial grids, optional hints, descent admission and the
selected disparity interval are uploaded per stage. Images, physical masks,
gradients, raw weights, rolling patch sums and five-word source models are
neither reconstructed nor uploaded by this adapter. Diagnostic qualification
alone copies terminal bits to CPU. The readable CPU `Input` entry remains the
oracle and the existing selected Scene path is unchanged; there is no
direct-path CPU fallback.

The integrated target-device test creates one resident producer and complete
front end, compares exact terminal bits at both levels and directions under
asymmetric modes, hints, admissions and intervals, observes a planted shader
buffer swap, then chains two ordinary no-readback resident stages and accepts
a recreated qualified pipeline only when it carries the same structural GPU
context. Both-direction prepared
buffers and bases are one private frame aggregate, so cross-frame, level and
direction assembly is not expressible through the crate API. The remaining
integration blocker is qualification of the later resident estimator stages
against this same `OneXsGpuContext`; this direct path remains unselected in
Scene. This is not yet an ownership-readiness, performance or range claim.

**Backend-neutral paired scheduler boundary, 2026-09-01, historical superseded implementation
branch only [CPU ORACLE IDENTITY; NO NEW GPU FRONTEND; NO PERFORMANCE CLAIM]:**
cold and warm scheduling now consume a `PairedControlInputs` frame containing
only data used after or around sparse solving: the current blurred belts,
shared physical L1/L2 image views, typed lack-row classifications and one
physical-mask block map. PIS gradients, raw weights, masks, rolling patch sums
and source models are absent. `PairedSolveRequest` remains dynamic-only, and
the plain `PairedPisSolver` boundary is used through `ColdPair`, `WarmPair`,
`PairOwner`, `FrameOwner` and the capture reservation. CPU oracle sessions are
constructed with their CPU-only PIS preparation; there is no default, optional
or rebinding state. A future resident GPU session can instead own its device
frame and enter the same prepared schedule without receiving or constructing
CPU PIS preparation, and without a backend enum in `pis::Input`.

The selected Scene GPU PIS frontend is intentionally unchanged in this
checkpoint: it still constructs CPU planes for its existing upload-backed
kernel before constructing that adapter. This is retained behavior, not the
new resident frontend and not a second production solve. Complete six-stage
cold plus two-stage warm output and retained-owner bytes match the CPU oracle;
all injected stage failures, stamps, panics and receipt mismatches retain the
exact retryable owner and ready-map allocations. No target-GPU workload or
performance measurement was run for this scheduler-only refactor.

**Shared ONE X2 GPU context foundation, 2026-09-01, historical superseded implementation branch
only [STRUCTURAL OWNERSHIP; NO NEW RESIDENT STAGE; NO PERFORMANCE OR PARITY
CLAIM]:** the renderer now retains the exact iced device and queue as one
private cloneable `OneXsGpuContext`. Equality is the structural identity of
both wgpu handles: a recreated `ScenePipeline` on clones of the same pair is
compatible, while an independently requested pair refuses. wgpu exposes the
one queue returned with a requested device rather than a second-queue
constructor, so the regression uses two devices requested from one instance
and adapter; the production check still compares both handles.
The production solver-belt pipeline and its exact-submission lease now carry
that context rather than separate raw handles. Future lease advancement takes
an encoding closure only after context validation, submits on the lease's own
queue and replaces its internal completion index; callers cannot provide a
detached `SubmissionIndex`. Existing explicit completion, early-drop,
poll-error, poll-panic and double-unwind quarantine behavior is unchanged.
Selected Scene preparation validates the supplied renderer pair before even a
stopped or in-flight display can enter recovery, then uses only the context's
retained handles. A mismatch touches no retained GPU resource, selects no draw
and surfaces its raw identity error. Diagnostic picture and full-luma paths
are also context-owned and no longer accept per-call device or queue handles.
This foundation deliberately does not transplant the resident PIS front end,
direct PIS, L2 bridge or final-map materializer, and it does not remove any of
the selected path's current CPU readbacks or uploads.

**Authenticated paired PIS GPU checkpoint, 2026-09-01 [6,400 GPU
TRANSACTIONS; ZERO CPU FALLBACK; BYTE-IDENTICAL TO THE FROZEN KJERAG CPU
BOUNDARY; 10.6% MEDIAN THROUGHPUT GAIN; NOT REALTIME; OWNERSHIP FIXED;
FINAL-HEAD RANGE PENDING]:** exact clean commit
`a1594c6fc458dbee248ba2a9d0a4f9f419a22304` causally processed frames zero
through 6399 of the owner clip at the reported 71.13 yaw, -13.99 pitch and
57.95-degree locked view, Sharp sampling, band and tone enabled, and the
factory seam. It presented all 6,400 frames with zero dropped and zero
starved. Its typed receipt recorded 6,400 GPU PIS transactions and zero CPU
PIS transactions, and every one of the 61 captured map frames named the GPU
backend. The range receipt SHA-256 is
`62262de5e5ddd97c773cdf0861fb7671524b98ea53336ba904ddc2958801a107`.

All 183 production artifacts for frames 6339 through 6399, comprising the
rendered PNG, packed map and alpha map for every frame, are literal byte
matches to the separately built frozen accepted CPU boundary. Their aggregate
SHA-256 remains
`bd0eb80a82d042d641b0543ad28beb4e7b186e77e4e11048e98f424bc63dacf0`.
All 61 candidate-authenticated computed traces are also byte-identical to the
frozen traces and report zero uncovered pixels; the trace aggregate remains
`515a0fd1a9238a721942ac8eca031d048d1dc2c33c9966c48595138521b854c8`
and the strict trace receipt is
`450cff21c22c1747d22cc62e8e70d6feabc997dd9d1893bc8363e1c900b8ac60`.
This authenticates the GPU implementation against Kjerag's frozen accepted
CPU boundary over the tested interval. It does not expand the prior Studio
parity claim to other footage or settings.

The same exact GPU executable was measured against the frozen `9054e547` CPU
implementation plus the common benchmark-only harness at `4c5b91f` in four
order-balanced pairs, `AB | BA | BA | AB`. Every arm consumed 200
causal warm-up frames followed by 300 unpaced waited transactions, bound the
same source hashes, view, direct type-2 route and Radeon 760M/RADV adapter, and
presented all 300 measured frames with zero drops. The eight arms did not
overlap one another or the separately retained invalid contended experiment;
the receipts do not carry general host-load, clock or temperature telemetry.
CPU throughput was
18.787368, 18.907114, 18.687162 and 18.914921 fps; GPU throughput was
20.652659, 20.975677, 21.080096 and 20.714241 fps. The ratio of medians is
+10.60% (18.847241 to 20.844959 fps), while the primary order-balanced paired
median is +10.43%. This is an unpaced one-clip, one-view, one-machine result
from four pairs, not statistical significance, realtime playback, audio or a
visual-quality claim. It remains only 69.55% of the 29.97-fps source rate. The
complete receipt hash list has SHA-256
`1b047196e2a32523b6d53621f8ff8155a5cfa0867ea6d3744ed31082cccb80bd`.

An independent ownership audit then found that the submitted belt token could
be dropped on the selected Scene's exact-frame rejection path before waiting
for its GPU submission, returning an aliased decoder surface to the pool too
early. Integration commit
`fb177d6011a1ea66229aca3fb716ed253eca696b` fixes that production path with
one qualified device/queue owner and a non-cloneable exact-submission lease.
Successful completion releases the decoder owner once and disarms the lease;
early return waits for the exact submission, while a returned poll error or
native-backend poll panic retains the owner rather than permitting unproved
reuse. Explicit completion preserves the raw error or original panic, and
destructor-time panic is swallowed only after fail-closed retention.

Five focused lease regressions, the forced-RADV byte-exact belt twin and the
exact real owner-clip Scene ABA path passed. The two GPU logs have SHA-256
`2169499a509da9d24fe09c8fb9aa681c30ace9825ac9d8247d8251582e0e74af`
and `10fd8dbb3747c6168ce7f1b6d64265c3d6454d95ed821614a12de582f4de7163`.
An independent re-audit accepted the fix at the exact integration tree. The
full causal range above still authenticates `a1594c6`, so the eventual
shipping head needs a fresh range after the wider GPU migration. The separate
unselected resident-token checkpoint still requires this lease to be carried
through an explicit terminal acknowledgement before it can be selected. The
next performance slice keeps the post-Gaussian belts resident, constructs
estimator levels and prepared models on the GPU, and binds them directly to
paired PIS rather than reconstructing CPU `Input` behind an adapter.

**Production ONE X2 paired PIS GPU wiring, 2026-09-01, historical superseded authenticated checkpoint
[TYPED GPU RESULT; EXACT RESERVATION RECEIPT; NO CPU FALLBACK; RANGE AND
PERFORMANCE EVIDENCE ABOVE]:** the selected Scene route now lazily qualifies a
persistent paired PIS compute pipeline before any frame-specific GPU work.
Cold and warm scalar transactions retain CPU construction and upload of the
typed prepared source models as the temporary producer boundary, then consume
only fallibly admitted directional `PatchGrid` results from the GPU kernel.
Each reservation mints an exact generation plus opaque `FrameStamp`; each
stage completion must return that complete flight and `PairSolveStage` before
the typed grids can enter `commit_prepared_with_solver`. Pipeline, readback,
receipt and solver-stamp failures occur before commit, surface their original
error and restore the allocation-identical old owner and ready map without a
CPU retry.
An install failure occurs after the scalar owner has advanced, so it instead
keeps the prior ready display and makes the lineage terminal while retaining
the advanced owner only in its exact slot or quarantine. It does not falsely
restore the pre-commit estimator. An in-flight lookup returns before
constructing either GPU producer and keeps the prior completed display.

Committed `OneXsMapFrame` values now carry typed CPU/GPU PIS provenance
separately from the diagnostic count of GPU stages that returned grids. The
consecutive-range receipt contract is therefore
`kjerag.playback-consecutive-range.v2`: every recorded production map names
its backend and the run authenticates exact GPU and CPU transaction totals.
The strict computed-trace consumer emits its corresponding v2 contract and
refuses historical v1 or missing, mixed and inconsistent provenance. The
three-panel owner-review builder now requires those two v2 inputs and emits
`kjerag.owner-three-panel-review.v2`; the frozen v1 review contract is not
silently redefined. The authenticated range above is the new target-GPU v2
evidence; no frozen v1 evidence is reinterpreted as proof of GPU-only PIS.

**Second GPU ONE X2 slice, 2026-09-01, historical superseded implementation branch only [GPU
GAUSSIAN; TYPED POST-BLUR HANDOFF; 244 BYTE-IDENTICAL ARTIFACTS; SMALL NOISY
THROUGHPUT GAIN; NOT REALTIME; NO NEW STUDIO OR OWNER VERDICT CLAIM]:** the
production GPU producer now follows source sampling and the 3-by-3 reduction
with the exact separable 5-by-5 input Gaussian. A horizontal integer-Q7 pass
uses coefficients `[3, 29, 64, 29, 3]` and reflect-101 borders while retaining
one `u32` sum per logical byte. A distinct vertical pass applies the same
coefficients, rounds once with `(sum + 8192) >> 14`, and packs four final U8
codes per word. The arithmetic bounds are 32,640 for the horizontal pass and
4,177,920 for the vertical pass, both within `u32`. Ordinary playback still
reads back exactly two 1080-by-60 belts, 129,600 bytes, but they now have the
`BlurredBelts` type. The CPU estimator accepts that type directly, so a second
CPU blur is structurally unavailable on the production handoff.

Construction now authenticates three different production properties on the
actual graphics device: every sampled pre-blur byte and the retained-map FMA
witness, every resulting sampled post-blur byte, and an independently uploaded
adversarial retained-belt blur fixture covering impulses, edges, lens
boundaries, checker/ramp/constant patterns and deterministic noise. A changed
Gaussian rounding instruction refuses construction as a typed `BlurredByte`
error. There is no CPU playback fallback. On the target Radeon 760M/RADV, all
604 render tests passed with 23 corpus-dependent tests ignored, including the
real owner-clip Scene transaction through dmabuf import, exact `FrameStamp` and
`Arc` ownership, rejected ABA successor, retained last-complete display and
ordinary exact-successor recovery. The software llvmpipe adapter instead
produced 189 at the existing source-FMA discriminator where the required
answer is 190 and was refused before playback, as designed.

Clean candidate `080a03972ff3f9272cf5cc860bb11aa460711b50` (tree
`e997c818b361b5a3bc0e5a049794359a96bd71c4`, playback executable SHA-256
`06aa24e203a0b9ba773677d8cc5a50e30994207d0f3809c8286529f83d0e530b`)
causally processed frames 0 through 6399 of the owner clip at the reported
71.13 yaw, -13.99 pitch and 57.95-degree locked view, Sharp sampling and the
factory seam. It presented all 6,400 frames with zero dropped and zero
starved. All 61 PNGs, 61 packed maps, 61 alpha maps and 61
candidate-authenticated computed traces were literal byte matches to the
separately built frozen accepted CPU boundary; every trace also reported zero
uncovered pixels. The new range receipt is SHA-256
`088369b0064222a40d3d235b2dfbc81f8113d5bd339b2e87176582ff97d41fc4` and
the trace receipt is
`bbf9d166047f31e0e138b7f7a998b7cd10eda10d0b4503522a91a591d70bb0dd`.

The same authenticated transaction benchmark compared the first-slice commit
`f0db9851753415b8f69e1d6a7c97472094f4c841` against this candidate in four
order-balanced pairs, `AB | BA | BA | AB`, each with 200 causal warm-up frames
and 300 unpaced waited transactions. Every receipt binds the same two source
hashes, exact view, Radeon adapter and `one-x2-direct-type-2` route; every run
presented all 300 measured frames and dropped none. Pairwise throughput changes
were -3.31%, +16.01%, +1.84% and +1.42%, whose median is +1.63%. Aggregate
median throughput moved from 22.885 to 23.787 fps (+3.94%), while the median of
run-median preparation time moved from 29.108 to 28.172 ms (-3.22%). One slow
first-slice run makes the aggregate figure optimistic, so this records only a
small positive result under substantial run-to-run variance, not a large speed
claim. Playback remains below the 29.97-fps source clock. The four first-slice
receipt SHA-256 values are
`9f041d6c1c67650607fd95c735f7d8951093d8d80529c3c42ad0579e010801e0`,
`40934d13fd0ebcb94c88cf828b13a6e27f0934f0f71510b2d30fc08b886546c0`,
`2bc6ecd39d76a1b1546f054bebb2bd5f0356cbe0d4f8a70809a82db6df7c9450`
and `bc97443806d967f9140a21fa2d2b1db68420fb88d9db06c63ffbbf621d69ad04`.
The four second-slice values are
`b89fde6b5722c85cd893b0046845dd4e1089d74921a99e7e91acc4c078b7dfa1`,
`9ca9412960c03525341f25f66d2d6924532fda6f6cdfc91e069c7037ce20a97f`,
`d19fd7dc391e1c5d7b53bfbddbb589df212072cda421f21537e66d119d798d66`
and `d238f734a2ae8bea594d34704700ab2d71ac9fe7b401f34d7d5e482bc7bf67e8`.

**Selected ONE X2 production transaction regression, 2026-09-01, historical superseded working
branch only [OPT-IN REAL MEDIA; TARGET GPU]:** an opt-in render test drives a
real paired ONE X2 delivery through dmabuf import, the exact prepared retained
maps, compact GPU solver belts, capture-owned scalar commit, direct-map upload
and scene acknowledgement. Its adjacent-frame arm substitutes an opaque
delivery identity with the same reported index and timestamp after GPU submit,
proves the receipt is rejected while the completed owner and bound direct-map
resource remain unchanged with no legacy fallback, then proves the ordinary
production entry point can commit that exact successor. It runs only when
`KJERAG_ONE_X2_TEST_MEDIA` names either half of a paired capture and a dmabuf
Vulkan device is available, and its explicit invocation goes through
`scripts/quiet.sh`; the normal GPU arithmetic twin remains the
media-independent synthetic source fixture gate.

**Production GPU ONE X2 solver-belt bridge, 2026-09-01, historical superseded implementation branch
only [WIRED INTO PLAYBACK; BYTE-EXACT TARGET-GPU TWIN; AUTHENTICATED 61-FRAME
CPU IDENTITY; 25.6% MEDIAN THROUGHPUT GAIN; NOT REALTIME; NO STUDIO OR OWNER
VERDICT CLAIM]:** selected playback now sends the imported R8 lens textures
and each frame's retained float2 maps directly to the exact GPU sampler and
3-by-3 reducer. It reads back only the final two 1080-by-60 U8 solver belts,
129,600 bytes instead of both 2880-by-2880 luma planes' 16,588,800 bytes. The
later Gaussian slice above extends that pipeline through the exact 5-by-5
input blur and hands typed post-blur belts to the same scalar cold/warm
estimator, which builds the same typed map. Full-luma GPU readback remains
available only to diagnostics; it is not a production fallback.

WGSL does not itself promise the required FMA and exceptional-float behavior.
The lazy production constructor therefore runs the same complete 129,600-byte
adversarial CPU/native oracle plus the retained-map FMA bit discriminator on
the actual graphics device before consuming frame zero. Both execute through
the stored production pipeline and its `build_solver_belts` entry. At one
adversarial solver tap, `solver_code` writes the exact UV it passes into
`sample_source` to a two-word witness sink. Qualification reads that sink after
the same complete dispatch that produces the byte fixture; ordinary
submissions bind the same pipeline-owned eight-byte sink but do not read it,
so overlapping ordinary writes are intentionally unobserved and add no
per-frame witness allocation. Their solver output and dispatch semantics are
unchanged. Any difference refuses selected playback with the typed arithmetic
error; it neither approximates nor falls back to the CPU luma path. This is
runtime qualification, not a device allowlist.

Preparation and commit remain separate capture-owned operations. The GPU wait
happens with no capture mutex held, a pending token retains the exact imported
decoder surfaces and their opaque `FrameStamp`, and the scalar owner consumes
history only after successful readback and a second exact-frame check. A GPU,
shape or association failure therefore surfaces its own error and leaves the
previous complete display and estimator transaction available for the existing
rollback path. Existing-result lookup and successor preparation occur under
one lock, so recreating the render pipeline cannot open a ready/prepare race.

On the target Radeon 760M/RADV adapter, the complete workspace gate passed;
the render crate ran 601 tests with 23 data-dependent tests ignored. The
required adversarial GPU twin matched all 129,600 bytes, and the opt-in real
Scene regression passed against the owner ONE X2 pair through dmabuf import.

Clean candidate commit `f0db9851753415b8f69e1d6a7c97472094f4c841`
then causally processed frames 0 through 6399 at the reported 71.13 yaw,
-13.99 pitch and 57.95-degree locked view with Sharp sampling and the factory
seam. It presented all 6,400 frames with zero dropped and zero starved. Every
one of the 61 requested PNGs, 61 packed maps and 61 alpha maps for frames 6339
through 6399 was byte-identical to the separately built frozen CPU package.
The candidate range receipt is SHA-256
`a7a4e917ab6b6e986d45b73468882ecf2983eef4def7e9c9fa32b8afab776818`.
All 61 candidate-authenticated computed traces were also byte-identical to the
frozen CPU traces and reported zero uncovered pixels; that trace receipt is
SHA-256
`4a8c7d58c5702623b2b1b505872d7cb56b23d529f1b9a970c0778a3fd424b847`.
This proves identity to Kjerag's accepted CPU boundary, not a new Studio export
or an owner-eye verdict on this exact build.

**Initial GPU ONE X2 solver-belt producer, 2026-09-01, implementation branch
history [BYTE-EXACT TARGET-GPU TWIN]:** a render-internal compute pipeline
consumes the two R8 source textures and the two retained 1080-by-60 float2 base
maps directly. Each invocation evaluates four final solver bytes with the selected
scalar/native base-map and source bilinear schedules, the strict ordered UV
gate and the exact integer 3-by-3 reduction, then packs those four bytes into
one storage word. It therefore produces the final two compact 1080-by-60 U8
belts without allocating the two 3240-by-180 staging images or reading source
luma through the CPU. The pending token retains the map, bind group, packed
output, readback and a caller-supplied imported-frame owner through GPU
completion. Its packed buffer is suitable for a later GPU solver; its readback
exists for the current CPU bridge and exact oracle gate.

With `KJERAG_REQUIRE_GPU=1`, the target Radeon/Vulkan adapter produced all
129,600 bytes exactly equal to the CPU scalar oracle over unequal odd-width
source textures uploaded through 512-byte padded rows, different lens
patterns, fractional and over-one coordinates, ordered zero/negative/NaN
sentinels, infinity clamps, both lenses, the production-entry retained-map FMA
bit pattern and a source-FMA discriminator whose selected answer is 190 rather
than 189. The
output packs four logical bytes per u32 and compile-time guards pin both total
and per-lens divisibility. WGSL permits a backend to expand `fma`, and
exceptional-float handling may vary with finite-math policy, so byte identity
on a different adapter remains a required gate rather than a source-level promise.
That standalone stage made no playback, performance or wider Studio claim;
the production bridge above is the later integration boundary.

**ONE X2 GPU migration transaction boundary, 2026-08-31, working branch only
[STRUCTURAL PREREQUISITE; NO GPU OR PERFORMANCE CLAIM]:** the capture-owned
`FrameOwner` now separates fallible, non-mutating frame preparation from the
linear estimator commit. Preparation accepts the exact `FrameStamp` and source
size before luma readback exists, validates delivery continuity, and computes
the parent maps, filtered retained base maps, patch preimages and selected
camera masks without consuming `PairOwner`. Its non-cloneable `PreparedFrame`
can then travel with exact pre-blur 1080-by-60 solver belts. Commit consumes
both, rechecks the stamp against the currently committed delivery, applies the
existing input blur, cold/warm transition and map materialization, and replaces
the retained owner only at the prior successful-transaction point. The
synchronous CPU `process` path is only a wrapper over those two operations.
Focused tests compare its complete first and next-frame outputs and retained
state against explicit prepare/commit, and prove a stale prepared transaction
is rejected while the owner remains usable for the exact successor. This is a
scheduling boundary for later GPU work, not a semantic change, GPU
implementation, throughput result or Studio parity claim.

**GPU migration transaction benchmark, 2026-09-01, working branch only
[FOUR BALANCED CPU/GPU PAIRS; 25.6% MEDIAN GAIN; NOT REALTIME]:**
`kjerag-spike --bin playback`
accepts the fail-closed benchmark mode
`measure=200:300 pace=off receipt=NEW-FILE bench=0`. It consumes frames 0
through 199 causally through the production `Scene` as untimed warm-up, then
times exactly the 300 complete source/map/waited-draw transactions for frames
200 through 499. There is no pacing sleep, capture, PNG encoding or filesystem
publication inside the interval. The durable no-replace JSON receipt records
every transaction and its source, primitive, prepare and waited-draw phases;
nearest-rank median/p95/p99/maximum and interval throughput; scene presentation,
drop, starvation and redraw counts; the exact view and source identities; and
clean build, executable and GPU identities including vendor/device IDs. Every
timing vector is allocated before the interval begins. Its wall elapsed time
runs immediately before the first transaction through immediately after the
last, including only sample recording and loop bookkeeping between transactions;
each per-frame transaction excludes that bookkeeping. Source, build and GPU
adapter identities are bound before playback and reverified after timing.
Every warm-up and measured transaction now also has to expose the bound stamp
of the exact selected ONE X2 direct-map resource for its current aligned pair
after preparation. The pipeline supplies that allocation-free stamp only when
`DirectOneXs`, the bound native type-2 resource and the complete display
transaction agree. A generic camera route, missing direct resource or
mismatched delivery refuses the run. Receipt schema v2 names the ONE X2
lens-type selector, native packed type-2 map and `DirectOneXs` draw, and each
frame records that authenticated route; its negligible stamp comparisons are
included in `prepare_ns` and interval throughput and copy no map payload. This
is the stable pre/post throughput boundary for the GPU migration. It is not a
Studio parity, realtime playback, audio or visual-quality result.

The frozen CPU implementation at benchmark commit
`4c5b91fbf27eef5333984826a4f454607a6afa3b` and GPU candidate
`f0db9851753415b8f69e1d6a7c97472094f4c841` were built in separate worktrees
and target directories. Four order-balanced CPU/GPU pairs ran through
`scripts/quiet.sh` on the same source hashes, Radeon 760M, RADV driver and Mesa
build. Every run authenticated `one-x2-direct-type-2`, presented all 300 timed
frames and dropped none. CPU throughput was 19.234, 19.339, 19.363 and 19.489
fps; GPU throughput was 23.726, 24.285, 24.336 and 24.391 fps. Their medians
are 19.351 and 24.310 fps, a 25.6% gain. Median transaction time fell from
50.69 ms to 40.11 ms, entirely at preparation: its median fell from 37.44 ms
to 26.80 ms while waited draw stayed about 13.2 ms. The result is still only
about 81.1% of the 29.97-fps source clock, so it is a measured first-stage win,
not completion of the GPU migration or a realtime claim.

**ONE X2 Studio-derived playback, 2026-08-31, shipping branch:** ordinary
zero-config playback now selects the capture-owned causal ONE X2 route
automatically. It consumes every decoded pair from frame zero, carries warm
state across adjacent frames, builds the packed map and copied-pole alpha, and
draws them through the direct type-2 consumer. The Optical Flow control is a
separate legacy solver and does not gate this route. A discontinuous seek
starts a new owner and causally replays from frame zero.

The implementation uses Kjerag's orientation track as the owner-approved
substitution for Studio's unread internal stabilization provider. That
difference is disclosed and is not a Studio-provider parity claim.

The owner reported "Looks good" for the riser-continuity defect over exact
frames 6339 through 6399 at the reported view. The first optimized build also
received that bounded verdict. Subsequent archived candidates were measured
byte-identical over that 61-frame production-picture, packed-map, alpha-map
and computed-trace boundary, but the newest exact build still requires its
own owner-eye verdict.

The latest controlled 200-completed-frame prefix measured 17.992537313 fps
against a 29.97-fps source clock, about 60.04% of real time. It is a
single-machine, single-clip observation and is neither portable nor
statistically significant. Continuous sound remains unresolved.

Before merge: run the complete workspace and GPU gates, exercise ordinary
player and installed Flatpak playback, regenerate the causal range and trace
with the exact shipping build, construct its authenticated three-panel review
with `scripts/build-owner-three-panel-review.py`, and obtain the owner's
verdict on those exact rendered pixels.
No whole-video, Studio-internal, seek/reset, real-time, continuous-sound or
merge-readiness claim is made.

**Status 2026-07-31:** feasibility study complete (docs/research/), repo
bootstrapped, M0 done, M1 done, and the horizon holds still.
`cargo run --release -p kjerag-spike -- <file.insv>` decodes one 3840x3840
lens on VA-API, imports the dmabuf planes into wgpu with no copy, and
renders to PNG at 103 fps (3.4x realtime). `cargo run --release --
<file.insv>` plays the file in a libcosmic window, every frame imported
zero-copy onto the device iced created and reprojected inside iced's own
render pass: the shell, the shader widget and the wgpu-28 import all
confirmed on screen. M1 is under way: `crates/meta/` reads the trailer's calibration
(issue #2), the source tree is a workspace with one crate per layer
(issue #19), the picture is reprojected through the lenses' own Mei/UCM
models with drag to look around and scroll to zoom (issues #3, #26), and the
playback core plays it (issue #4): one demuxer, both lenses decoded in
lockstep and delivered as pairs, a presentation clock that paces 29.97 fps
content by due time, space to pause, and frames pullable by index or
timestamp. Measured over 60 s of real footage: 29.94 fps presented, zero
dropped, zero starved. **The sphere is closed** (issue #27): the shader
projects every ray into both lenses, so turning around shows the back
hemisphere, upright and unmirrored, and **the seam between them is
blended** (issue #7), which is the first of M2. The
window is a COSMIC app around it (issue #16, built to docs/UI.md): the menu
bar in the header, a welcome view, the portal file chooser, drag and drop,
recent files, the whole key map, fullscreen, Settings and About drawers, and
a control overlay that takes itself and the pointer away after 2 s of
stillness while playing and never while paused. The scrubber scrubs (issue
#5): dragging it seeks to keyframes, 21 ms each wherever in the 37.9 GB file
they land, and letting go seeks to the exact frame. **The view can be
photographed** (issue #15): `s`, the camera button and `File > Save frame`
write a 3840 px wide JPEG of the reframed view, at the window's aspect and
not its size, into the desktop's screenshots folder; `Ctrl+C` puts the same
picture on the clipboard as `image/png`. The capture is the window's own
pipeline and bind group drawn a second time into a texture of the surface's
format, so the numbers in the file are the numbers on the screen, and
everything after the submit runs on a worker thread: 13 captures over 20 s
of playback, zero dropped and zero starved in every report. **A capture says
so**, in a toast built the way cosmic-files builds its own: `Frame saved to
"Screenshots"`, `Frame copied to the clipboard`, or the reason it did not
happen (docs/UI.md, "The capture toast"). 11 captures over 30 s of playback
with the toasts in: zero dropped and zero starved in all six reports. **And
the view can be quoted**: `i` copies one line naming the video, the frame
and the framing, written as `reframe`'s own arguments, so a report about a
360 video carries the direction it was pointing rather than leaving everyone
to guess it. Every capture prints the same line, because a still's name
carries the video and the moment and nothing carries the direction. The copy
carries the file's name alone and the terminal line carries the path.

**And the line is a place, not a label.** `Ctrl+V` goes there: the frame it
names, the direction it was pointing, the horizon it was held with, as a jump
and not an animation. A reference carrying a path opens the video it names
first; one naming a video that is not open says which video it is from; a
clipboard holding anything else does nothing at all, because `Ctrl+V` over a
video means nothing in any other player either. The command line takes one
too, so the terminal line is a complete launch command:
`kjerag flight.insv time=9.576 yaw=144.40 pitch=0.90 fov=24.10 lock=1`.
All three read it with the one parser in `crates/render/src/framing.rs`, which
is also where it is written; reframe's real parser reads the same line in a
test, so no two of them can drift. A `lock=1` yaw is a direction in the
stabilized world frame, whose zero moved on 2026-08-06, so a line copied before
that date lands somewhere else and says nothing about it (docs/UI.md, the view
line). Measured under the harness: copy a view,
seek ten seconds away, zoom out a notch, paste, and the copied line comes back
to the millisecond and the hundredth of a degree with the picture byte for
byte what it was.

**M1 is done.** The seam blend (issue #7) is the first M2 quality item and
it has landed: where the two lenses overlap the pass mixes them by
longitude preference times coverage depth, with no feather width anywhere
in it, and the hard line and the tone edge where the hemispheres met are
gone. Near-field structure crossing the seam, which on this footage is the
wing and the lines, ghosts softly instead of stepping, which is parallax
and is the expected trade. Exposure is **not** corrected from the shutter
records, and that is the finding rather than an omission: the trailer's two
are parsed and kept apart (issue #7's other half, and the camera's own frame
clock for #8), but the two lenses trade shutter against sensor gain to reach
the same picture brightness, so the ratio is not a brightness ratio and the
symmetric split it implies makes the step four to twenty times worse. It is
corrected instead from what the band measures on the pixels (issue #103,
stage 3, docs/research/insv-format.md 6.10).

**The horizon is locked** (issue #8), which is the second M2 item and the
one the feasibility study called the riskiest correctness surface. The
trailer's IMU is parsed and integrated into a `world_from_body` quaternion
every 5 ms of the file, and the reprojection pass composes its inverse
between the lens mounting and the camera, so the world stays put while the
camera swings. Roll, pitch and yaw are all locked, so the view holds a
direction in the world and a deliberate turn moves the aircraft rather than
the picture (owner ruling, 2026-08-06; it was a 3 s yaw high pass until
then). Drag to look around needed no change at all: the anchor it stores is
in whatever frame the camera rotation lands in, and with the lock on that
frame is the world. Measured on rendered frames: the horizon moves 0.23
degrees peak to peak over 120 frames of calm flight and 2.86 through a
61 deg/s roll, where with the lock off it leaves the picture entirely.
`View > Lock horizon` and `h` flip it live, and it is on by default.

Two conventions were settled on the way, both against pixels rather than
against other people's tables. The IMU's axis convention is `xZY` for the
X-series in Kjerag's own frame, picked out of all 24 rotations by comparing
the accelerometer's idea of up against the horizon in unlocked frames; it
wins every stretch of two captures by 15 to 36 degrees over the runner-up.
And the quarter-turn roll datum from issue #3 belongs to the **delivered
picture** rather than to the sensor, which the IMU could tell apart because
it is bolted to the sensor: that closes the last "what 4.8 does not settle".
**Rolling shutter is fused into the same map, measured, and on** (issue #9),
which is the third M2 item. For every output ray the landing row is solved
for and the orientation used is the one at that row's own readout instant,
one round of the solve, no extra pass and nothing resampled. The one thing
the file does not record is **which way the sensor reads**, and on an X4 Air
it reads **down the delivered frame**: 1.00 +-0.12 of a whole frame in the
trailer's own 15.883 ms, measured over five stretches of a 30-minute capture
by fitting one lens against itself a few frames apart, with the horizon
lock's rigid rotation and the camera's own translation fitted out alongside.
That direction is the one the seam cannot see, because two lenses reading
down their own pictures sweep the same world direction and it cancels between
them; that is both why #42 shipped it switched off and why switching it on
cannot disturb the seam #7 blended. It costs about half a millisecond per
redraw at 2560x1440, 0 dropped and 0 starved
(docs/research/insv-format.md 6.7).

**Hemisphere-aware decode gating is half done and half cut** (issue #10),
which is the fourth M2 item and the third time this project has built
something and then measured it out. The half that shipped is in the shader:
each lens's picture is one cap around its own axis, that cap comes out of the
calibration by solving the model's own coverage boundary, and a ray further
off the axis than the cap weighs exactly nothing, so the pass does not run
the Mei map for it. One dot product per lens decides. 1.74 to 1.54 ms per
redraw at a view inside one hemisphere, 1.81 to 1.66 across the seam, and
eight rendered views are byte for byte what they were.

The half that was cut is the decoder, and the reason is arithmetic. Gating
the invisible stream is worth 2.84 points of one core and 1.53 W of SoC power
against a 6.10 W idle, which is the halving the issue predicted. But the gate
can only be on while **no** ray of the view can reach the far lens, and at the
app's default 90-degree field of view that is 16.5% of the sphere; on real
flying footage with the horizon locked, which is the default, a parked view
is not a parked geometry, and the measured duty cycle is 21.6 to 24.3% with
no hysteresis at all and 8.9 to 9.4% with the 15 degrees of margin a release
would need. Those were read under the heading follow; with the world-fixed
lock of 2026-08-06 the body turns fully under a parked view and the same
measurement on one of those captures reads 15.7% and 3.9%, which makes the
answer below no less final. Releasing a cold gate costs 195 to 340 ms, six to eleven frames
of stale far hemisphere, because HEVC has no way into the middle of a GOP and
this camera's is 29 frames. Expected saving: **0.14 W**, for a state machine
and a packet ring through the frame path. `kjerag-spike --bin gating` is the
measurement and it stays runnable.

**The lock-defect work of PR #51 was reverted the same day** (issues #44
and #45). Its drag-relative follow passed a filter-level test (a pinned
view moved under 0.05 degrees across five minutes of wandering heading)
but the owner, on that exact build, still saw the view jump back seconds
into playback: whatever the app path does to the pin, the unit test did
not exercise it. Owner's rule applied: no code on main that is not doing
its job.

**The dip was a bad start, and the seed is fixed** (issue #45,
docs/research/insv-format.md 8.7). `Filter::solve` seeded the estimate from
the first tenth of a second of accelerometer whatever it read; on the April
capture that tenth of a second weighs 1.281 g, which the running filter
refuses outright, so the horizon started **48.9 degrees** off level and
walked back over a minute and a half. The seed now searches forward for the
first window the filter believes completely and carries it back to the start
of the track with the gyroscope. Measured through the projection pass with
`kjerag-spike --bin dip`: at 6 seconds 48.9 degrees becomes **1.9** on the
April capture and 14.7 becomes 8.1 on the June one, at 30 seconds 29.2
becomes 3.9 and 6.0 becomes 2.5, and at 300 seconds both files read what
they read before to three decimal places, which is the control. What is
left is the accelerometer's own disagreement with gravity inside the seed
window, 2.8 degrees on one capture and 9.8 on the other, and that is the
residual issue #57 is about.

**And the window was the residual** (issue #152, 8.8). That fix tested the
magnitude of one second of accelerometer, which is nearly blind to the
horizontal acceleration that tilts it: the August 2 capture's launch weighs
1.039 g at 21 degrees off vertical and was believed completely, and the
horizon stayed 17 to 21 degrees off level for over a minute. The seed is now
the whole opening minute averaged, which is bounded by how much an aircraft's
speed can change in a minute rather than by what one second happened to read.
That capture now opens **3.33 degrees off level against 20.97**, inside the
4.05 it settles at four minutes in, and against a backward pass over each
file the seed error is better on all six owner flights, worst case **3.03
degrees against 24.18**. Not a clean sweep of everything: one sibling file
that was already right by a third of a degree gives up a quarter of one, and
April 10's opening reads better at the first frame and worse from 4 to 20
seconds, with the two instruments disagreeing about which.

**The instrument that missed it is repaired.** `dip` gated out any line more
than 20 degrees off level, so it never measured a defect that is 40 to 50,
and the 1.9 to 8.5 degree apparent-gravity attribution PR #51 reported was
that selection. Its injection control passed because 1 to 3 degrees was
injected where the baseline was already small: **a control has to span the
regime being measured**. The gate is off by default, every view it drops is
counted and printed, and the acceptance run injects 45 degrees and reads
back 45.101 - where the gate that shipped drops 48 of 60 views and reports
no fit at all. The apparent-gravity size and the GPS prescription (#57) are
withdrawn in place in 8.7 with pointers to the new numbers; #57 stays on
hold until the residual is re-measured.

**High-quality sampling at high zoom is half shipped and half measured out**
(issue #11), which is the last M2 item and the fourth time this project has
built something and then cut part of it. What ships is the **luma** plane:
where an output pixel has landed inside a source texel, the pass reads a
Catmull-Rom kernel instead of the hardware's bilinear tent, engaged smoothly
from 1:1 to 2:1 magnification and exactly off at 1:1 and wider. Sixteen
texels as nine bilinear fetches, which agree with sixteen point fetches to
0.14 codes RMS. How magnified a fragment is comes off the **map's own
Jacobian**, the hardware's quad derivative of the landing, so the fisheye's
uneven angular density and the output's own fall-off towards its corners are
both in it and neither had to be assumed. On real footage at the window the
player's numbers are taken at, zoomed to 50 degrees on ground and buildings:
detail (mean absolute Laplacian) 4.12 codes bilinear against 4.61 sharp,
**+11.8%**, 1.8 codes mean over 85% of pixels, and side by side at four times
life size the stone courses of a barn separate instead of smearing, with no
ringing on the roof line. It is worth **most in the middle of the zoom
range** and least at the end of it: at 5x, which is 25 degrees, the source has
nothing left to resolve and the two kernels draw the same ramp (+1.8%).

What is cut is the **chroma** plane. NV12's two planes are two grids, and
measuring them separately is what found it: chroma is half the size, so it is
magnified twice as hard and it is under 1:1 at **every** field of view this
player offers. Upgrading it is therefore not a cost paid at high zoom but a
cost paid always, and it is the larger half of the bill (0.69 to 0.90 ms
with luma upgraded, to 1.23 with both). What it buys, on 8-bit 4:2:0 chroma
that HEVC has already smoothed, is 0.41 codes on 40% of pixels and **no**
measurable change in detail at all: 4.606 either way. `Sampling::Sharp` keeps
it one line from shipping for footage that would change the answer.

**A scrub no longer waits for pictures nobody asked for** (issue #46). The
decode thread used to look at its command queue only between reads, so a
drag position arriving while it refilled the lookahead behind the last
landing waited out three pair decodes first: 33 of the 39 ms between a
keyframe seek at the reader (21 ms) and the same seek through the player
(59 ms). It now asks between packet reads and gives the read up, and a scrub
costs 26 ms: about 38 picture updates a second where there were 17. The read
a seek itself asked for is never given up, because drag positions arrive
faster than landings come out of them and a rule that always took the newest
would show no picture at all. Playback is untouched by construction and by
measurement: 29.97 fps presented, 0 dropped, 0 starved, the same decode rate
as before, and the sound goes with it, because a preempted read stops
without reading another packet and the seek behind it flushes the ring.

**The zoom goes all the way out to the blue ball** (issue #47, owner ask). The
scroll used to stop at 110 degrees, because that is where a flat window stops
being one. It now keeps going: past that threshold the output projection bends
out of perspective through **stereographic**, which is the **tiny planet**
Insta360 names and the owner calls the blue ball, and on until the whole
sphere is a ball with room around it. One
family does the whole range, `r = tan(shrink * theta) / shrink`, with `shrink`
running 1 to about 0.18, so nothing is switched over and there is nowhere to
pop. The far end is where the ball fills 0.8 of the window's **shorter** side,
which is 605 degrees of field of view on a 16:9 window and 406 on a square
one, and past 360 the frame is simply wider than the sphere: that extra is the
room the ball sits in, painted the same grey the pass has always painted where
no lens has the ray. Ctrl+0 comes back in one press.

Measured on real footage at 2560x1440 (`kjerag-spike --bin ball`): one scroll
from 20 degrees to the ball, a notch at a time, rendered, and the largest
single step is 64.3 codes at fov 402, with the largest growth of a step over
the step before it 1.32x at fov 173 - against 1.15x inside the flat range this
change did not touch, and nothing standing out at the threshold or at
stereographic. Cost, interleaved across the range and three runs agreeing
within 0.01 ms a cell, 0.71 ms/redraw at the default view against 0.81 at the
ball, on a 33 ms frame; the flat range costs what it cost, `--bin zoom` off
this branch against the same binary built off main, alternated on one box,
within 0.04 ms a cell in both directions. Playback at the ball, with three
3840 px captures taken during it: 29.97 fps presented, **0 dropped, 0
starved**, and a still of the ball is byte for byte the ball. The drag took no new mathematics: it was
already written against the projection's own rays rather than against a
tangent, so it inverts whichever map the view is in, and grabbing the ball and
turning it works because of that rather than despite it.

What is **not** fixed is aliasing. Out wide the map minifies rather than
magnifies (7.6 delivered texels to the output pixel at the middle of the
ball), so issue #11's kernel correctly switches off and the pass is plain
bilinear on a single mip level: against the same view supersampled 4x4 the
ball is 4.1 codes out over the pixels that have picture and 107 at worst,
against 1.1 and 11 at the default view. High-contrast edges will shimmer in a
moving ball. A prefilter is the fix and it is deliberately not built yet: the
imported dmabuf textures have one level and no room to generate another in
place, so it means a downsample pass per frame per lens, for a view the player
is in for a few seconds at a time. Numbers first, then the owner decides.
**A fast drag no longer freezes the picture** (issue #55). What #46 measured
and could not fix inside itself: `Player::pump` showed a frame only while its
own seek was still the newest, so a hand faster than a landing starved the
display instead of slowing it, and at 60 slider positions a second not one
picture reached the screen for the length of the drag. The pump now takes a
frame from any seek newer than the position on screen, so the pilot sees the
landings their finger has passed over rather than nothing, and picture
updates rise with the hand instead of falling off a cliff: 10.0, 15.0, 19.5,
29.0, 38.5, 45.5 and 43.0 a second at 10 to 90 positions/s, against 10.0,
15.0, 12.5, 10.5, 5.0, 0.0 and 0.0. The two questions the old rule answered
with one flag are now separate: which frames may take the screen, and which
seek is still owed one. The second is what keeps a paused window redrawing,
and it ends on the newest seek's own frame, so the release still lands the
exact frame under the handle (49 of 49 drags, both arms). At 60 positions/s
the picture now changes every 22.0 ms, which is about what one keyframe
decode costs on this camera (21.1 ms at the reader): the drag runs at the
decoder's rate, which is also the answer to #46's open question about a
100 ms drag cycle. There was no 100 ms cycle. Three landings in four were
being thrown away.

M2 is done. #44 and #45 closed against it (the seed fix, owner-verified),
and #48 has now reopened the seam.

**The seam is out by degrees, and it is calibration** (issue #48, phase 1,
measurement only). The owner supplied the one thing every earlier reading of
this was starved of: a capture from a camera that is **not moving**, and
Insta360's own export of the same capture as the parity benchmark. On a still
camera there is no parallax worth the name in the far field, no rolling
shutter and no motion blur, so what is left at the seam is the calibration.
`kjerag-spike --bin seam` measures it round the whole seam circle and splits
it into the axis parallax cannot reach and the axis it owns.

The along-seam residual #7 and #42 left open, -0.36 to -1.20 degrees, is real
and reads -0.30 to -1.25 here. It is not the problem. **Across** the seam the
two lenses disagree by **-2.4 to +2.7 degrees**, which is 43 px of the
delivered frame, and that is what the owner has been looking at: it draws a
tree trunk twice in a blended view and breaks the horizon in a hard cut, on
this capture and on his flight footage. Insta360's own export of the same
frame draws the trunk once.

The structure attributes it. Measured round the circle, a constant and one
cycle account for everything, leaving 0.012 degrees along and 0.055 across;
and the shipped map's own knob table says only a relative **lens tilt** can
put 2.7 degrees of one cycle across the seam while leaving 0.46 along it. The
fitted correction to lens 1 is a rotation of roll +0.80, yaw -2.29, pitch
-0.82 degrees plus a 15 px principal-point shift, and applying it takes the
along-seam residual from 0.766 to 0.077 degrees and the across-seam from 2.333
to 0.108. Applied and re-measured, every patch reads inside 0.02 degrees along;
rendered, **a hard cut with no blend at all is continuous** through content
that was visibly broken before, on both static captures and on flight footage
from five weeks earlier.

Controls, because #45's lesson is that an instrument which cannot catch the
failure is not an instrument: injected errors of the size being reported read
back at 0.99 to 1.06 (roll along, yaw across at r = 1.000, a 20 px principal
point on both axes); a second capture of a completely different scene, taken
minutes later with the camera moved, is fixed by the first capture's
correction; and the deck under the camera fails to pair at all exactly where
parallax says it must, because 5 to 30 cm of subject distance is 6 to 38
degrees of disparity in a 14 degree band.

Against the benchmark, each stitch scored on its own picture (gradient energy
in the seam band over the same statistic either side of it, so tone curves
divide out): Insta360 keeps **0.83 to 0.88** of its sharpness across the seam,
we keep **0.573**, and the correction takes us to **0.689**. Their export is a
square 1440x1440 reframe about 95 degrees across, mildly compressed rather
than rectilinear, not an equirect crop.

The blend width has a number now instead of a guess. Rendering the same
seam-crossing frame under crossovers of stated width and scoring each against
the front lens alone: a **2 degree** band takes 80 percent of what a hard cut
would give, against a 14 degree band today. But the two halves are not
independent, and this is the finding that orders the work: shear, the
disparity divided by the band, is 1.07 at 2 degrees with today's calibration
and 0.52 with the correction applied. **Narrowing the band before correcting
the calibration would trade a soft wide ghost for a hard visible tear.**

Nothing in the shader changed: phase 1 is measurement, and the fitted
parameters are for the owner to validate before phase 2 applies any of them.
Method and every number: docs/research/insv-format.md 6.8.

**Phase 2 applies it, per camera, and then narrows the band** (issue #48, the
owner's "2 deg looks good"). The correction is five knobs, a relative rotation
and a principal point, **fitted once against a capture from a camera standing
still and stored under that camera** rather than fitted per file. Which of
those two is right was measured rather than argued, and not on the number a
per-file fit minimizes for itself. **The per-file fits disagree with each
other**: fitted file by file, the same glued pair of lenses asks for yaws from
-1.69 to -2.58 degrees and principal points 13 px apart, which is 15 px of
picture at the seam between two answers for one camera that did not change
between April and July. **The static capture's answer, applied unchanged to
all five flights, reads the same along-seam number their own fits do** (0.15
to 0.22 degrees, within 0.002 of the per-file answer on three of five) and
well inside the 0.31 to 0.40 the per-file rotation left. That axis is the one
parallax cannot reach, so it is calibration and nothing else. On the
far-field control the per-camera answer leaves **0.022 degrees along and 0.106
across**, which is 1.8 view pixels, against 6.7 for the per-file rotation.

At this historical milestone, one explicit calibration action replaced the
old open-time wait and stored a serial-free per-camera answer. That entire
product mechanism was later retired: the current app exposes no calibration
action, saved pool or per-file fallback. Factory calibration is the parity
base, and supported ONE X2 playback uses the recovered direct type-2 route
recorded at the top of this file.

**Then the crossover, 2 degrees instead of the 14-degree overlap.** On flight
footage the doubled band goes from 10.60 degrees to 1.50 and its sharpness
against the front lens alone from 0.723 to 1.074. Against Insta360's own
stitch, the benchmark: they keep 0.92 to 0.97 of their own sharpness across
the seam, we kept **0.579**, the calibration alone took us to 0.689 and the
band takes us to **0.871** -- four fifths of the gap, closed. The pass costs
what it cost (3.77 ms per redraw before, 3.59 after, interleaved at
2560x1440), which took two attempts: reading the two lenses' angles back out
of the `Blend` array after the loop that fills it costs 5.5 ms against 3.6,
and only `--bin playback` under live decode can see that -- `--bin zoom` reads
the two as equal.

**M3 has started, and the player has sound** (issue #13). The file's AAC track
is decoded off the same demuxer as the two lens streams, resampled by
`swresample` into the device's own format, and written to a ring that a cpal
output stream drains. **The picture stays the clock.** Every device callback
asks the presentation clock where the picture will be when the samples it is
about to write are heard, and moves the sound to meet it: a splice when the
two are more than 30 ms apart, which only a start, a seek landing or a
recovered stall ever is, and a resampling ratio of a few parts per million the
rest of the time, which is what holds two crystals together over a half-hour
flight. A seek throws the ring away on the shell's thread rather than waiting
for the decode thread to reach the seek, so no stale sound survives a scrub,
and every start and stop is a 5 ms ramp rather than a step, so a pause does
not click. In the control row: a speaker button after fullscreen, opening
cosmic-player's own volume dropdown, with both settings remembered in
cosmic-config. The wheel stayed on zoom (docs/UI.md, "Conflict 2"). Nothing is
processed: a paramotor track is mostly wind, and it is played as recorded.

`kjerag-spike --bin sync` is the measurement, and it drives the real player
with no GPU in it: play, pause, resume, two scrubs and a frame step on a
printed schedule, with the app's own five-second report. Over a 325 s run of
real footage, past those, the sound sat **0.0 ms** from the picture in 40 of
44 windows and never further than **0.8 ms**, with zero underruns and zero
dropped chunks; the target was 40. The same binary with the drift correction
switched off and nothing else changed sits at **+28.1 ms** and stays there,
which is what the correction is for and is the control that says the number
above means something. Recording the output device's monitor and measuring the
sample-to-sample step at each join puts every one of the six below the 97th
percentile of ordinary playback, against 12 of 15 synthetic hard cuts spliced
into the same recording that land at the 99th or above: the fades hold.

**Looking around got three fixes at once** (issues #77, #63, #78, all owner
reported). **Fullscreen no longer resets the view.** The camera was living in
the shader widget's iced `State`, and iced rebuilds widget state whenever the
widget tree changes shape: libcosmic pushes the header bar into the same
column as the content, so hiding the bar moves the content up a place and
everything under it is built fresh. Fullscreen hides the bar, which is the
whole of the connection - and so does the two-second idle timeout, so the view
was being reset under a pilot who was only watching, too. The `Viewpoint`
lives on the `Scene` now, which is the shell's own and outlives its view. The
harness gained the checks that measured it: entering and leaving fullscreen by
both keys, and the controls hiding on their own.

**The pan goes over the top** (issue #63). The pitch had a wall at 89 degrees;
it is gone, and past a quarter turn the view is looking back over itself with
the world upside down, on round to a whole turn and round again. It is a pitch
and not a roll, so the horizon stays a level line either way up, and a
vertical drag never swings the yaw. Nothing was added to get there:
`rot_y(yaw) * rot_x(pitch)` was always the rotation and a pitch past 90 is a
perfectly good one. What the anchor solve needed is to count to the nearest
tilt **the short way round**. Checked through the pass on real footage and not
only in arithmetic: rendered at yaw 180 pitch 180, the pass draws exactly the
yaw 0 pitch 0 picture turned upside down, every pixel of it within one code of
255.

**And the wide end of the zoom drags calmly** (issue #78). Pinning the grabbed
direction to the cursor is what makes the flat range feel like a hand on the
picture and what makes the ball twitchy: the pinned rate at the middle of the
ball view is 900 degrees of world per width of window, against 164 at the
widest flat view, so a drag across the window out there turned the world two
and a half times. Past 110 degrees the pin comes off and the drag turns the
view at a fixed 164 degrees per window width, which is the rate the pinned
drag itself is going at in the last view before the handover, so the two meet
at the threshold with nothing to feel. Under 110 nothing changed at all.

**And the zoom key no longer lets go of the drag** (issue #83, pre-existing
and surfaced by the three above). `Ctrl+=` and `Ctrl+-` took a held drag's
hold again at the middle of the frame while the hand was somewhere else, so
the next move of the pointer hauled the picture over to whatever the middle
had been pointing at: 33 degrees of yaw for a cursor that did not move, in
the widget-level check that reproduced it. A held drag already knows where
the cursor is, because that is what the wide regime measures its travel from,
and the key zooms there now, which is what the wheel has always done. With
nothing held it still zooms about the middle, which is where a keyboard with
no hand on the picture is pointing.

**And nor does the wheel out in the room around the ball** (issue #92, owner
reported, the last of the same family). Zoomed out to the ball, a drag that
starts on the picture and wanders out into the grey keeps turning the view,
which is what issue #78 bought: the wide drag reads the hand's travel and not
what is under the cursor. Scrolling out there killed it stone dead. Every
zoom re-takes the drag's hold at the cursor, the room has no direction under
it to take hold of, and the whole drag was being dropped rather than the
hold: the button was still down, the pointer moved 200 px, and the camera
came back bit for bit identical in the widget-level reproduction. The hold is
kept now where there is nothing to replace it with. Nothing reads it stale:
the room only exists past 220 degrees of view and the pinned drag stops at
110, so the wide drag, which does not use it, is the only regime the room can
be seen from.

## Milestones

- **M0 Pipeline proof** — decode one lens via VA-API, import into wgpu
  zero-copy, render headless to PNG with timings. Done (`crates/spike/`,
  issue #6). Shell bring-up followed in issue #1: libcosmic window, shader
  widget, and the wgpu-28 port of the import.
- **M1 Reframing player** — dual decode, calibrated Mei reprojection,
  drag to reframe, scroll to zoom, play/pause/seek, screenshots. The MVP.
  Reprojection and the mouse are done (issue #3), and so is playback:
  dual-stream decode, the presentation clock and play/pause (issue #4).
  Full 360-degree look-around lands here too (issue #27): both lenses
  sampled, and the seam they left is blended by #7 in M2. The app shell
  around all of it is issue #16, seeking is issue #5, and screenshots are
  issue #15, whose toast has since landed in cosmic-files' idiom
  (docs/UI.md, "The capture toast"). What is left of the MVP's UI is the two
  Settings rows for the capture folder and resolution.
- **M2 Quality** — seam blend (issue #7, done: weight field in, exposure
  correction measured and rejected), gyro horizon lock (issue #8, done:
  complementary filter, `View > Lock horizon`, and a harness that measures
  the horizon in rendered frames because a Studio export was not available
  overnight), rolling-shutter correction (issue #9, done: fused into the one
  backward map, and on, the readout direction measured off the pictures
  because the file does not record it), hemisphere-aware
  gating (issue #10, done: the pass skips the lens a ray cannot reach, and
  the decode gate under the same test is measured and cut), scrub
  responsiveness (issue #46, done: a newer drag position takes the decode
  thread off the lookahead refill, 59 ms to 26 ms per scrub; and issue #55,
  done: a drag faster than a landing shows the landings it has passed over
  instead of freezing, 0 to 46 picture updates a second), high-quality
  zoom sampling (issue #11, done: a Catmull-Rom kernel on the luma plane
  wherever the map's own Jacobian says an output pixel has landed inside a
  texel, and the chroma half of it measured and cut), and the zoom out to the
  tiny planet and the whole ball (issue #47: one projection family from
  perspective through stereographic to a finite disc, capped where the
  ball clears the window's shorter side; the tiny-planet framing sits
  mid-scroll on the way there, and the owner chose to keep the extended
  range after trying a hard stop at the planet).
  Three of the quality issues under it are the interaction ones the owner
  found while flying the finished zoom: the view surviving fullscreen
  (issue #77), the pan carrying on through the poles (issue #63), and the
  wide end of the zoom dragging calmly (issue #78).
  **M2 is complete**: issue #48 reopened the seam, and both halves of it have
  now shipped. The two lenses were misaligned by up to 2.7 degrees across the
  seam; phase 1 measured that and attributed it to a relative lens tilt, and
  the now-retired phase 2 calibrated the legacy generic route from an explicit
  capture before handing the picture over in a 2 degree crossover instead of
  the whole 14 degree overlap. What was left at the seam on flight footage is
  parallax, which is depth rather than geometry, and **stage 2 of issue #103
  now measures it on every frame the pass draws**: a compute pass over the
  two imported textures
  reads the overlap band as the stereo pair it is, along the axis the file's
  own 33 mm baseline names, and each lens's ray is bent by the other lens's
  blend weight times what the two disagree by. Far field and near field take
  opposite time constants, 2 seconds against a tenth of one, which is what
  makes a per-frame reading steadier than the per-clip table phase A
  recommended rather than noisier: flicker 0.008 to 0.023 degrees rms against
  phase A's 0.22 to 0.54 for a naive per-frame table. It costs 0.3 ms a redraw
  at 2560x1440. **Stage 4 then made the crossover itself an answer to that
  same measurement**: the clamp and the width are one inequality,
  `|disparity| <= 0.9 * width`, and stage 2 held the width at 2 degrees and
  solved it for the disparity, which threw alignment away on everything
  nearer than 1.06 m. Solving the same line for the width opens the band to
  exactly what the reading needs and returns the floor bit for bit
  everywhere else, so the far field is the picture it was. It recovers up to
  11.5 view pixels of doubled edge, on 0.02 to 0.19 percent of
  direction-frames of the owner's own flights and 0.2 to 2.3 percent of the
  handheld and selfie-stick corpus, for 0.06 ms a redraw. Issue #79 opened
  a second camera: the ONE X2 writes one lens per file, and the player now
  pairs the two at open and holds an X2's horizon with that camera's own IMU
  convention.
- **M3 Export & sound** — clip export (reframed VCN encode, and lossless
  time-range remux), audio playback (issue #13, done: AAC off the same
  demuxer, cpal out, slaved to the video clock, volume and mute in the control
  row).

## Scope doctrine (owner, 2026-07-31)

Kjerag is a VIEWER: view, reframe, screenshot, and at most a simple clip
export (mark in and out, export the current view or a lossless cut). It
must be an awesome viewer before any of that export work starts, so
quality owns the roadmap until the owner says the bar is met. Keyframed
reframing and timeline editing are OUT OF SCOPE, not deferred: that is an
editor, a different product (Kdenlive's bigsh0t filters already cover
keyframed 360 export on Linux). The one editor-adjacent idea parked with
no commitment: export that follows the view the pilot actually flies
live, no keyframe UI ever.

## Decisions log

- 2026-08-09 **Deterministic per-lens exposure normalization from the trailer's
  shutter records is REFUSED, on nine of the owner's own reference views**
  (issue #103, stage 10 step P.1; docs/research/seam-blending.md 17 to 23,
  insv-format.md 6.3). The plan was to read the per-frame per-lens shutter the
  `.insv` trailer carries, ratio-normalize the two lenses before fusion, and be
  structurally unable to repeat stage 7/8's noise painting because nothing is
  estimated from pixels. Built, measured, and switched off.

  **The metadata does not describe the artifact.** Across five X4 Air captures
  the shutter ratio says the two lenses are 33 to 57 percent apart; their
  pictures of the same directions are 0.07 to 4.3 percent apart. The
  correlation between the two runs **-0.74 to +0.51 and averages about -0.05**,
  three of the nine views strongly negative, and the correction leaves **11 to
  228 times** the artifact on every single view. At the dirt reference the
  seam's step goes **2.14 to 15.26 codes** with the decoy circle unmoved,
  against the 2.265 to 1.424 stage 3's far-field gain bought at the same view.

  **The reason is physical and not a tuning miss.** The two lenses run
  independent auto-exposure loops that trade shutter against sensor gain to
  reach the same picture, so the shutter ratio measures how differently the two
  hemispheres are LIT, not how differently they came OUT. Dividing by it does
  not remove an error, it removes the camera's own correction. Closing the sum
  needs a per-lens GAIN, and the trailer carries none: record 9 is one track for
  the file, no key of the record-1 protobuf is an exposure, and the ONE X2 does
  not even write the second shutter record. **The `.OSV` answer is the same:**
  per-frame ISO, shutter and white balance are all there, one sample a frame,
  and both `djmd` tracks carry the same block, so there is no per-lens exposure
  in a DJI file either.

  **What ships is the instrument and the record**, which is PR #138's ending and
  deliberate. The application arm was built behind `KJERAG_EXPOSURE_NORM`,
  measured off-by-default against `main` byte for byte at four registry views,
  and then **deleted on the owner's ruling of 2026-08-09**: *"Feel free to
  delete dead arm on 177, I always recommend deleting dead code so we can move
  faster. It's in git history."* It was 352 lines of shipped mechanism that
  nothing drew, and it is archived in commit **8107a23** on
  `feat/exposure-normalization` - `git show 8107a23` reads it back. What is left
  in the tree is `--bin expose mode=meta`, the new instrument, which is what any
  future attempt should be pointed at first, and this record.

  **One architectural finding falls out of it and is not about this step.** The
  pooled stage-3 gain read `+0.00287` ln on both arms, unmoved, because the
  band's compute pass measures the DECODED PLANES and both gains are applied in
  the fragment shader: **anything applied at fusion is invisible to the band,
  which measures the source and not the picture.** So the two corrections cannot
  double-correct and cannot check each other either; they multiply blindly.

- 2026-08-09 **The `.OSV` support is rebased onto the flat seam, the mounting is
  baked as a constant of the camera, and the horizon lock is gated on the file's
  own gravity** (docs/research/osv-format.md, which is this format's reference
  chapter from today). The branch sat on a pre-#172 base; it is replayed onto
  #176 rather than reimplemented, eighteen commits, six of which needed a hand.
  The flat seam SIMPLIFIED the port, exactly as expected: the branch's inert
  seam-fit is moot now that nothing applies a band correction anywhere, and its
  one `band.rs` change - a `normalize` of a zero vector, which is the NaN that
  drew a DJI capture's whole forward hemisphere black - merged untouched and is
  still wanted, because `ring_at` still runs for the instruments.

  **Three owner verdicts stand on the rebased path**, re-verified 2026-08-09 at
  1920 px through the delivered pass and recorded as registry lines
  (docs/research/reference-views.md): the tower single (*"can confirm tear is
  gone"*), the far kerb joined, and the dip's horizon held (*"OSV video output
  looks good, approved"*). The counter-null for the last is the same line at
  `lock=0`, which is visibly rolled and differs byte for byte.

  **`KJERAG_MOUNT` is gone.** Candidate b - a mirror in `y` and a quarter turn -
  is `MOUNTING`, a `const`. A setting that moves the horizon is the calibration
  ritual zero-config playback forbids, and the escape hatch is replaced by
  something better: the file's own accelerometer is a second, independent
  statement of where down is, and it is held against the file's own quaternion
  **per file**. Under 8 degrees, better than the null of a camera assumed never
  to lean, and better than the same quaternion read the other way round, and the
  horizon is held; otherwise the capture comes out with an empty orientation
  track, which is the shape a capture with no inertial record at all already
  had, so it reaches the pilot as the disabled menu item and the same `level:`
  line. Measured over all seven corpus files: six unit B files at 1.55 to 3.75
  degrees against nulls of 5.68 to 9.44, and the one unit A file at 23.02
  against a null of 14.72.

  **That gate is a check on the FILE and not on the mounting** (adversarial
  review, 2026-08-09; a correction to this record and to the line the app
  prints, and not a change to any picture). The mounting's mirror and its turn
  are applied to the quaternion and to the accelerometer alike, so they cancel
  out of the angle between them: all four sign families whose change of basis is
  a rotation score the check identically, and the refuted `as written` family
  passes it with the shipped family's numbers to the last bit while composing an
  orientation tens of degrees away. What the check really sees is a file whose
  own two records disagree - which is what unit A is - plus the CONJUGATION, the
  one bit of the mounting that does not cancel, which is now a third bar and a
  third number in the printed line rather than an unstated assumption.
  **Mounting verification is offline and stays there**: the vanishing-point
  solve over 177 degrees of lean azimuth. The limitation this leaves is written
  down rather than left to be found: a third unit whose inertial frame is
  reflected the other way would render a tilted horizon and pass the gate.
  docs/research/osv-format.md 6.2, and
  `osmo::tests::the_check_scores_a_mirrored_mounting_identically` is the proof
  in code.

  **So unit A's sample capture no longer holds a horizon, and says why.** Its
  quaternion and its accelerometer disagree by 23.0 degrees where a camera
  assumed upright is out by 14.7, and reading the quaternion the other way round
  does not rescue it (11.3, past the ceiling). Its picture is untouched and its
  `lock=0` and `lock=1` renders are byte-identical, which is the check that the
  refusal is complete.

  **The GPU twin guard now runs both models.** `lens_pixel` branches on the
  block's own model field, so the Insta360 fixture never ran a line of `theta` on
  either half and the `.OSV` could have shipped a WGSL model that disagreed with
  its Rust twin with every test green. Two WGSL-only mutations were planted,
  measured and reverted; the telling one is a tenth of a percent on the leading
  coefficient, far too small to see, at 22 times the bar.

  **The `.insv` null holds.** All sixteen views of the standing null method
  (six registry views plus lock on/off pairs at four of them, rendered by
  `--bin reframe` against each build in its own target dir) render byte for
  byte identical to main at 7ef59a3, and the four lock pairs inside them
  differ from each other, so the null is not vacuous.

  **Playback is unchanged within the noise, and the claim that it improved was
  not sourced** (review, 2026-08-09). This entry said the pre-rebase branch
  "recorded 26.6 fps at 20.6 ms", and no such measurement exists anywhere in
  this branch's record. What the pre-rebase branch recorded on unit B 8k30p is
  the two-column table in the 2026-08-08 entry below, taken while another agent
  shared the box: **29.57 fps at 6.91 ms a redraw at best and 25.54 at 21.05 at
  worst**. The rebased build reads 29.36 fps at 7.25 ms, one run. So against the
  best column it is a shade slower and against the worst it is far faster, which
  together say the box's own load moves this more than the rebase does. No
  playback claim is made for the rebase in either direction.

- 2026-08-09 **The seam is flat, the handover line is held on the world, and
  the machinery that morphed the picture is deleted rather than switched off**
  (docs/research/studio-parity.md). The owner approved the architecture on
  2026-08-08 after a nine-arm eyeball loop on his own flights: *"current
  architecture approved. you can merge the existing stitch with a wide band."*

  **The shape of it is Studio's, which is the picture he compares everything
  to.** Off is calibration plus fusion plus an anchored line, and that is what
  ships; Optical Flow is the belt, and it is a separate mode, later, behind its
  own switch. One mode at a time. What shipped before was a third thing - a
  permanently on, partially trusted, live estimated morph inside a narrow
  corridor - and it is the thing he had been refusing one arm at a time since
  2026-08-05.

  **Three things come out of the render path and are deleted, not gated**: the
  corridor bend, the adaptive fade width (stage 4), and the steep blend curve
  (half of #172). The fold apparatus goes with them - `FOLD`, `SPEND`,
  `WIDEST_DEG`, `band::carried`, `band::width`, `band::affordable` - because
  every one of them existed to keep the bend from printing the picture over
  itself.

  **The band still measures and reaches no pixel.** The compute half is
  untouched. Those readings are what the belt will be seeded from, and an
  instrument that stops measuring cannot say what the belt has to fix.

  **The line is held** (`SeamAnchor`, on by default, `KJERAG_ANCHOR=off` to
  refuse). Under a world locked view the 50/50 locus sweeps across world content
  at up to 21.8 deg/s, so every static defect the seam has travels with it. One
  offset, one closed form update law, and not one event in it: the owner refused
  the two line dissolve that came before it - *"every now and then it glitches.
  We need it to be smooth, that is a requirement"* - and he was right about
  which way was simpler. The simpler arm also measured smoother than the
  picture it is drawn from, where the elaborate one was four times rougher.

  **HELD IS PARTIAL, AND THE NUMBER IS 0.62** (adversarial review, 2026-08-09;
  a correction to this record and not a change to the picture, which flat6 drew
  the same way). The anchor holds the SHARE's 50/50 line exactly; the picture
  draws the WEIGHTS' crossing, which is that share times each lens's own
  coverage depth, and the depths are fixed to the lenses. Measured over 24
  azimuths: **0.617 degrees drawn per degree commanded on the X4 Air and 0.510
  on an X2-class camera**, so the anchor reduces the seam's crawl by about 60
  percent rather than removing it, and about 40 percent survives. Three real
  defects were found inside the follow and fixed, none of them reachable by
  continuous 30 fps play and so none of them a byte of the approved picture:
  the anchor was placed on the unclamped offset while the shader drew the
  clamped one, and a seek pinned the line to a target from another part of the
  flight - backward, where the step came out as zero, and then forward too,
  where the step was capped and the follow charged with the same stale target.
  **A seek anchors afresh in either direction now**, and what separates a seek
  from a redraw is the size of the step and not its sign: a step of film past
  0.25 s, which is 7.5 frames of this corpus and a fortieth of the app's own 10
  second jump. The forward half was worth 2.90 of the 4.00 degrees on the seek
  frame, for a seek of any length, because that is the follow's own ceiling at
  the step the old code capped to. The follow's rail is the film rate's -
  `allowance / (POWER * RATE * dt)^(1/POWER)`, 3.55 degrees at 30 fps, past the
  4.00 allowance above 100 fps - and both cameras have 120 fps modes, which is
  documented with its table rather than fixed, because a dt invariant law is a
  different picture at 30 fps too.

  **The width clamp was never the safety bound, and now something is.** The
  handover's support is centred on the drawn line, so with the anchor it reaches
  a whole band off the seam at the rail: over the July-14 fast segment 202 of
  900 frames draw some of it past the coverage on his own X4 Air, and 866 of 900
  would on a camera overlapping the way the ONE X2 does. What carries that is
  each lens's
  coverage depth inside `claim`. Measured over the ring at the rail on both
  camera classes: the weights sum to one everywhere and the worst weight step is
  0.0034 per hundredth of a degree, against a fade whose own slope is 0.0017;
  a planted hole and a planted cliff are the controls.

  **The two twins are checked against each other on a real GPU**
  (`crates/render/src/twin.rs`). A review planted a bend in the WGSL half alone
  and the whole workspace stayed green while the picture changed. The guard
  compiles the shipped shader with a probe entry after it and compares every
  weight and landing against the Rust mirror; a WGSL-only bend fails it by 221
  times its bar and it is the only test that does. **The "887 times the bar"
  this entry carried is 887 times the clean RESIDUE and 90 times the bar**, and
  a ratio here now names its denominator. CI has no GPU and skips it;
  `KJERAG_REQUIRE_GPU=1` makes the skip a failure and `scripts/uitest.sh` runs
  it that way, so a release cannot be tagged without it.

  **And a guard is only a guard where its fixture reaches**: the first version
  of that probe was built on a pose with no rolling shutter, so the readout
  half of `project` sat behind a uniform test that was false on both halves and
  ran on neither, and the same review passed 226 of 226 tests with a WGSL-only
  change to `readout_share` while the picture moved. The fixture rolls now and
  the test asserts it (round 2, 2026-08-09).

  **THE ONE X2 NOW DRAWS 8.00 DEGREES WHERE IT DREW 3.94.** The bound on how
  wide a camera may hand over was its overlap minus the room a bend needed to
  carry a sample past its own ray; nothing displaces a sample now, so the bound
  is the bare overlap and the X2's 9.19 pays for the whole ask. This is the
  largest deliberate picture change in the merge and the one thing that cannot
  be byte identical to the arm he approved.

  **The null is byte identity against the arm he approved**, not against `main`:
  the same instrument source built against this branch and against the flat6
  commit, playing real film offscreen at the six registry views under two
  calibration paths. The X2 is the disclosed exception above.

  Known and disclosed rather than fixed, in
  docs/research/studio-parity.md 6: the honest doubling at the bad crossing
  (the wide band softens it, the belt is its fix), the partial hold above, the
  one sided fade truncation under sustained motion, the frame rate dependent
  rail, the seam ring still crawling away from the view centre, and the far
  field alignment the bend used to buy. That last one is the trade he made:
  alignment that moves, for a seam that stands still.

  Calibration stays v3; v6 was refused by his eye on 2026-08-08 and the parity
  line continues elsewhere. `KJERAG_HANDOVER_DEG` stays live, because it selects
  a value on a continuum the next A/B will want to sweep again.

- 2026-08-08 **The seam's temporal bundle is the default behaviour, and its
  three research toggles are deleted rather than defaulted**
  (docs/research/seam-temporal.md 9, docs/research/reference-views.md). Three
  changes the owner picked blind at six views on 2026-08-08 - a steeper blend
  curve inside the same 8 degree support, a gate on the way out filtered at 2 s,
  and a direction's first reading walked in through that filter instead of
  switching on whole - shipped together, because that is the combination he
  answered on. He said the bundle at `down1`, `down2`, `down3` and `shimmer`,
  "same" at `good`, and at `bad` picked the middle arm and then added "I like 3
  on F too"; of the round, "it actually perceptually distorts a bit less".
  `down1` and `down3` flipped from the round before, and the only thing that
  changed between them is the arrival staging.

  **The toggles disappear.** `KJERAG_BLEND_CURVE`, `KJERAG_TRUST` and
  `KJERAG_ARRIVE` are gone and so is the code they selected: an A/B harness
  that wants the old picture builds the old commit, which it can always do, and
  a configuration switch nothing reads is complexity with no reader. This is
  not the same call as `KJERAG_HANDOVER_DEG`, which stays because it selects a
  value on a continuum that the next A/B will want to sweep again; these three
  selected a behaviour that has now been chosen.

  **The null is not byte-identity against `main`.** This change moves the
  picture on purpose. What was checked instead is byte-identity against the
  ARM HE ANSWERED ON: md5-equal renders with the band live at all six A/B
  views, under two calibration paths, against the same binary the blind session
  used. What ships is what he saw.

  Known and disclosed rather than fixed: the **comb** - a dead neighbour cell
  zeroing a live cell's correction, so the corrected patch has a hole in the
  middle - gets both **more frequent and deeper**, and those are two changes
  rather than one. More frequent, 217 to 256 frames of 300 at `bad`, because
  the gate is held up for longer. Deeper because the clamp on that gate **moved
  across the mix**: main mixed the two cells' confidences and then clamped, this
  clamps each cell and then mixes, so a live cell beside a dead one is taxed by
  the mean of a 1 and a 0 instead of by the pair's mixed confidence - 0.731 to
  **0.500** on a 0.95-beside-0.00 pair, and the notch at the owner's `down1`
  pair goes 0.62 to **0.41** of the correction, 34 percent deeper
  (docs/research/seam-temporal.md 9.4). **The next build has to answer both**;
  shortening the gate's hold alone would leave every remaining notch as deep as
  it is. He was told about the stripe before he answered and not about its
  depth, which had not been measured then.

  Also disclosed, and it moved during review: the **fold inequality** is one
  line five functions solve for different unknowns, and the blend curve's
  gradient had been divided into one of them. It ships divided once, in
  `band::SPEND = FOLD / BLEND_POWER`, which all five read
  (docs/research/seam-temporal.md 9.6). **`band::WIDEST_DEG` is 4.33 degrees and
  not the 2.89 the 2026-08-05 and 2026-08-06 entries below quote**, and it is
  now exactly the width at which the clamp equals the widest reading the search
  can return, so nothing the search reports is cut on any camera at any
  handover width. No X4
  pixel moves - `affordable` has no `SPEND` in it in the roomy regime and the
  six A/B views are md5-identical either way - and the **ONE X2** changes twice
  over: nothing the search reports is clamped any more, where 0.206 degrees was
  being thrown away, and its band opens 0.17 degrees past its own overlap at the
  near field, where the coverage depth hands the picture over instead of the
  ramp. Nobody has looked at an X2 under either.

  Untouched: the roughly one degree across-seam residual on his downward arc,
  which is the expensive work.

- 2026-08-08 **The `.OSV` lock's missing piece is the IMU-to-optical mounting,
  it is measured, and it is a knob rather than a default.** *(The entry below
  left this open: "which rotation it is was NOT pinned". It is pinned now.)*
  The instrument is the one that entry built - the vertical vanishing point of a
  **lock off** render, which measures where the world's up sits in Kjerag's
  camera body from the picture alone and is the same picture whatever candidate
  is under test - run over the whole corpus instead of one dip.

  **The derivation is a measurement and not a fit.** A reading of
  `(w, x, y, z)` plus a mounting turn about the camera's own vertical composes
  as `BODY^-1 . reading(q) . BODY . Rot(up, turn)`, and setting the up it
  predicts equal to the up the picture measures leaves the turn as a
  DIFFERENCE OF TWO AZIMUTHS - one number per instant, no free parameters. The
  lean magnitudes on the two sides agree to 0.18 degrees, which is the control
  that says the mounting fixes the camera's vertical and is a rotation about
  it.

  **There are four families, not eight readings.** Negating `x` and `y`
  together is conjugation by a half turn about the file's `z`, which splits
  into a world-side yaw that `from_first_heading` removes and a body-side half
  turn - a mounting of 180 degrees. So `wxyZ` IS the shipped `wXYZ` turned half
  a circle, and `wxYZ` is `wXyZ` turned half a circle: **the eight-row sign
  table of the entry below was two families sampled at 0 and 180 only**, which
  is why none of its rows held the horizon. The answer is at 87.

  **What separates them is scatter.** On the right family the turn is one
  constant of the hardware; on a wrong one it walks with whatever that family
  is not accounting for. Over **23 instants of the three unit B files spanning
  177 degrees of lean azimuth**, weighted by each instant's own lean because an
  azimuth of a nearly upright vector is nearly undefined:

  | family | turn | rms scatter | worst | walks with |
  | --- | ---: | ---: | ---: | --- |
  | as written | -84.6 | 65.9 | 148.8 | heading 0.69 |
  | conjugate (shipped) | -107.1 | 62.1 | 133.2 | azimuth 0.73 |
  | **mirror in y** | **+86.8** | **3.3** | **10.1** | nothing 0.15 |
  | mirror x, conjugated | -90.9 | 61.2 | 170.8 | azimuth 0.89 |

  One of the four is a constant and the other three are not, by a factor of
  nineteen. It is the same constant on each file alone - `+88.3` on B003 over
  11 instants, `+84.5` on B002 over 7, `+85.7` on B001 over 5 - and **dropping
  any whole file moves it by at most 1.8 degrees** (`+85.0`, `+87.6`, `+87.1`),
  which is the leave-one-out that makes the owner-dip score below a held-out
  one.

  **Residual tilt of the horizon, degrees, median over each site**, on the same
  lock-off instrument, gated on its own control - the lean the lock-off render
  reads has to reproduce the file's own lean, which every reading agrees on
  because `1 - 2(x^2 + y^2)` carries no sign, so the gate is candidate blind and
  what it throws out is an instant where the fit found a false family of
  parallel lines. On the owner's dip that control is met to 0.13 degrees at the
  median, worst 0.53, at every consensus setting:

  | candidate | owner's dip B003 | B001 leaned | B002 leaned | flat stretch B003 |
  | --- | ---: | ---: | ---: | ---: |
  | **`a` mirror y, turn +86.8** | **0.44** | **0.66** | **1.21** | **0.66** |
  | `b` mirror y, turn +90.0 | 0.34 | 0.72 | 1.38 | 0.53 |
  | the same family, held out at +85.0 | 0.78 | 0.75 | 1.20 | 0.74 |
  | `c` conjugate, turn -107.1 | 5.06 | 5.57 | 11.63 | 1.72 |
  | `d` mirror x-c, turn -90.9 | 1.20 | 15.89 | 3.23 | 1.92 |
  | SHIPPED (conjugate, no turn) | 20.85 | 15.87 | 2.55 | 2.98 |
  | NO LOCK AT ALL (null) | 11.36 | 8.33 | 7.65 | 2.61 |
  | *instants / lean* | *9, 8.2-15.2* | *5, 7.2-11.1* | *7, 7.7-9.4* | *2, 2.1-3.2* |

  **One candidate family collapses the residual at every site**, and no other
  does: `a` and `b` stay under 1.4 degrees everywhere, where the shipped
  reading is worse than no lock at all on two of the three dip sites and `c`
  and `d` each blow up on a site the other survives. The row that matters most
  is the third: `+85.0` is derived from B002 and B001 ONLY, and it scores 0.78
  on the owner's B003 dip, which no instant of it ever saw. **The flat stretch
  is the heading control and nothing regresses on it** - every candidate sits
  within a couple of degrees of the null there, as it must, because a mounting
  turn about the camera's own vertical is a pure world yaw on an upright camera
  and `from_first_heading` takes a constant world yaw out
  (`a_mounting_turn_is_a_pure_heading_on_an_upright_camera`).

  **Unit A is a null result and is reported as one.** Its capture affords the
  instrument nothing: 3 instants of 51 fitted at all and none of those passed
  the lock-off control, and its own accelerometer is not a plumb line either
  (2.74 g mean, 1.29 sd). Nothing here is claimed for the second camera.

  **And the app's own picture agrees.** Locked renders at pitch 0 through the
  peak of the owner's dip, read by the same vanishing point with the lock ON
  and the candidate in the binary, come out level to **0 to 2 degrees** under
  `a` and `b`, where the lock-off control reads the body's own 14 to 17 and the
  shipped reading reads 30 and worse.

  **What it means: the file's inertial frame is left handed against the optical
  one.** That is also why the heading looked settled while the tilt was not - a
  mirror reverses the heading exactly as a conjugate does, so the turn
  measurement that pinned the conjugate could not tell the two apart and picked
  the one that gets the tilt wrong. The file's own two streams still agree with
  each other, because the accelerometer is written in that same frame: read
  through the mirror it misses the file's own gravity by **2.0 degrees** on
  leaned frames where the conjugate families miss it by **9.3**, against a null
  of 9.4 for a camera assumed never to lean. That check is printed per file
  whenever the knob is on, and it is the shipping verify this needs.

  **What the file does NOT say.** An hour was spent looking for the constant
  written down. The `camd` nested MP4 - a top-level box in all four captures,
  never audited before - was dumped leaf by leaf: it is a self-contained
  re-mux whose `mdat` is a **byte-identical copy of the outer `djmd` tracks**
  (all 585+585, 4384+4384, 3590+3590 and 6606+6606 samples compared, zero
  differing), and not one leaf carries a float constant. Fields 20 and 27 hold
  the same two `f32`s as each other in every entry of both units and are 0.022
  to 0.046 degrees as radians - three orders of magnitude short. Field 24 is
  `8.0` as an **`f32`**, the same on both lenses of both units, so it is not an
  octant index; 25 is the `-1000.0` sentinel beside it. Every non-`mdat` byte
  of all four files was scanned at every offset as `f32`/`f64` in both
  endiannesses for 45, 135, 225 and 315 degrees and their radian, cosine, sine
  and half-angle-quaternion encodings: every hit is a per-frame varying
  quantity (`.3.2.16.1` is a temperature, `.3.2.3.1` the ISO, `.3.2.15.2` a
  photometric value that passes through 135). The 16 unexplained bytes at
  `.1.3.1`, identical on two physical cameras, are not a unit quaternion and
  not a rotation. **Verdict: the mounting angle is not written in any field
  this audit could read**, and the HEVC/AAC payloads are the one place not
  looked.

  **Nothing is defaulted.** `KJERAG_MOUNT=a|b|c|d` selects a candidate and
  prints which one and why on stderr; unset is the shipped composition **byte
  for byte** - verified against a clean build of 61dc430 on both an `.OSV`
  lock=0/lock=1 pair and the sixteen-view `.insv` null, all sixteen hashes
  identical. Evidence and harness: `scratch/imu/{mount,solvemount,verify,plant}.py`.

- 2026-08-08 **The `.OSV` horizon lock does not hold the horizon when the
  camera leans, and the mirror that was left open is not what is wrong with
  it.** The owner's report on the entry below: "OSV now stays still when guy
  rotates in a circle, but when the camera dips the horizon is no longer
  locked", with his own view line,
  `time=139.806 yaw=-63.98 pitch=-15.06 fov=160.04 lock=1`. That instant sits
  inside a real dip - the file's own quaternion puts the camera 8 to 15.9
  degrees off vertical from 138.40 to 140.51 s - and the report reproduces.

  **A new instrument, because every angle read off the picture so far was the
  wrong angle.** A view whose axis is not horizontal makes vertical world
  lines converge, so their apparent lean changes across the frame and is not
  the view's roll: `lean.py`, a Hough vertical cluster and a shoreline fit
  were all tried and all failed their own controls. The convergence is the
  signal. Vertical world lines meet at the vertical VANISHING POINT, and the
  direction that point sits in **is** the world's vertical in the frame of
  whatever camera took the picture. Kjerag's own output map is a plain pinhole
  up to `FOV_FLAT` (110 degrees), so at 65 degrees the fit is exact.

  Measured on a **lock off** render, which applies no orientation at all: the
  picture is then identical whatever candidate is under test, and every
  candidate enters only as a PREDICTION of where world up should be. Three
  controls hold it up:

  - eight view directions 45 degrees apart, fitted independently at the same
    instant, agree to **0.37 degrees at the median** (worst 2.4);
  - the tilt it reads with the lock off is **1.02 times** the lean the file's
    own quaternion states, correlation 0.99 - it measures the body's lean when
    that is what it is looking at;
  - the tilt of a LOCKED render matches what that lock-off measurement plus
    the candidate's own arithmetic predicts, to **0.1 to 0.3 degrees**.

  **The residual tilt each reading leaves**, in degrees, on the owner's dip (9
  instants, 138.40 to 140.60 s, peak lean 15.9) and on a flat stretch of the
  same walk (6 instants, 121.0 to 123.5 s, camera upright):

  | reading of (w, x, y, z) | owner's dip | flat stretch |
  | --- | ---: | ---: |
  | **`wXYZ` conjugated (shipped)** | **21.94** | **5.00** |
  | `wxYZ` mirrored in x | 16.77 | 4.21 |
  | `wXyZ` mirrored in y | 16.38 | 4.90 |
  | `wxyZ` z negated, otherwise as written | 8.51 | 4.21 |
  | `wxyz` as written | 20.40 | 1.97 |
  | `wXYz` mirrored in z | 11.58 | 6.28 |
  | `wxYz` mirror y, conjugated | 18.73 | 5.98 |
  | `wXyz` mirror x, conjugated | 16.16 | 2.08 |
  | NO LOCK AT ALL (null control) | 11.88 | 3.44 |

  **The shipped reading leaves nearly twice the tilt that switching the lock
  off leaves.** So does each of the two mirrors that were the open question.
  Only `wxyZ` beats the null and it still leaves 8.5 degrees, which is not a
  held horizon. The deepest dip in the corpus - unit B file 1, 28.4 to 30.6 s,
  peak lean 23 degrees - says the same where the scene gave the instrument
  something to fit: shipped 21.1 against a null of 11.6.

  **What the error tracks is the lean, not the clock.** Against the file's own
  lean the shipped error fits a slope of 2.65 with correlation **0.987**, and
  its median is **1.84 times the lean**; against the body's turn rate the
  correlation is **0.102**. A lag would grow with rate and vanish at the
  bottom of a dip, where the rate passes through zero; this is largest exactly
  there. It is a standing frame error.

  **Where it is.** Low pass the file's accelerometer over 2 s - which is what
  takes the wearer's stride out of it - and the quaternion read AS WRITTEN
  predicts it to 2.1, 2.1 and 2.8 degrees on the three unit B files, on the
  frames leaning more than 8 degrees, against a null of about 10. So the
  file's own two streams agree with each other and its frame is internally
  consistent. The picture disagrees with both by about 21 degrees at the dip,
  and **essentially all of it is azimuth**: the lean magnitude the picture
  measures matches the file's own to 0.18 degrees at the median, while the
  direction that lean points, taken round the camera's own vertical,
  disagrees by about 135. The tilt is being applied the right amount the wrong
  way round the vertical - a composition between the file's inertial frame and
  the optical frame Kjerag renders in, not a handedness in the quaternion.
  Which rotation it is was NOT pinned: the scene offers the instrument
  vertical structure for only about a tenth of the capture, and the 4 to 9
  instants that survived span 29 degrees of lean azimuth, which is not enough
  to tell a turned frame from a reflected one. **So nothing was changed.** A
  sign flip would not have fixed this and no other number here is measured
  well enough to ship.

  **Why the two oracles under the entry below preferred the shipped reading.**
  Neither of them measured a distance from level. The tilt-pairs oracle scored
  the worst angle BETWEEN two locked renders half a second apart, which is a
  difference between candidates, so two readings that are wrong the same way
  both score well and the reading that moves least wins whether or not it is
  level. The accelerometer's 1.8 degrees was a median over every steady frame
  of a capture whose median lean is 4 degrees, and all eight readings predict
  the same lean MAGNITUDE - `1 - 2(x^2 + y^2)` has no sign in it - so they
  differ only in azimuth and only in proportion to the lean. On those frames
  every candidate scores within a degree of every other and of the null; read
  on the frames that lean more than 8 degrees, with the stride low passed out,
  the same instrument spreads them over 8 degrees. It was a real instrument
  used at the one operating point where it says nothing.

  Evidence, panels and the throwaway harness: `scratch/EVIDENCE_2026-08-08-osv-dip/`.

- 2026-08-08 **Horizon lock works on a DJI `.OSV`, and the file's quaternion is
  written the other way round.** *(The "what is still open" paragraph at the
  end of this entry is superseded by the entry above: the open question was
  the wrong question, and both of the readings it weighed leave more tilt
  through a dip than switching the lock off does.)* The owner's report was "the camera rotates
  when the wearer turns around", which on this format it did, because the lock
  was a designed refusal: the file's fused orientation was there and its frame
  was not pinned. It is pinned now, and the whole of it is
  `world_from_body = BODY^-1 . conjugate(w, x, y, z) . BODY`
  (`kjerag_meta::osmo`, which carries the evidence in its own doc).

  **Everything the scoping pass concluded about the orientation was measured
  under the four-coefficient lens model**, with 15 degrees of seam tear in the
  picture, so "applying the quaternions made the stitch worse" settled nothing
  and was re-derived from scratch under the five-term model above.

  **The oracle is the picture, through the app's own pass** (`--bin reframe`,
  `lock=1` against `lock=0`). Two measurements, on the owner's capture:

  1. **World stability across a turn.** 12 renders 0.25 s apart over 120.90 to
     123.65 s, during which the wearer turns 133 degrees, at yaw 0 and 90
     degrees of view. The score is how much of the picture one render still
     shares with the next, and how far the world has turned between them.
  2. **The tilt head to head.** The candidates that reverse the heading agree
     exactly for a camera held upright and differ by twice its lean when it is
     not, so they are separated on the frame pairs, half a second apart, where
     they predict the most different motion. The score is the worst of the
     three angles between the two locked renders, which should be zero.

  | candidate | hold, 133 deg turn | step yaw | max roll | tilt pairs, residual |
  | --- | ---: | ---: | ---: | ---: |
  | **`wxyz` conjugated (shipped)** | **0.718** | **0.46** | **0.89** | **5.8** |
  | `wxYZ` mirrored in x | 0.677 | 0.49 | 1.29 | 11.4 |
  | `wXyZ` mirrored in y | 0.694 | 0.86 | 1.99 | 10.8 |
  | `wXYz` mirror z, conjugated | 0.680 | 1.34 | 2.11 | - |
  | `wxyz` as written | 0.366 | - | - | - |
  | `wXYz` mirrored in z | 0.361 | - | - | - |
  | `xyzw` order | 0.224 | - | - | - |
  | `xyzw` order, conjugated | 0.269 | - | - | - |
  | LOCK OFF (null control) | 0.460 | 11.68 | - | 7.1 |

  Angles in degrees; a dash is a candidate the picture had moved too far for
  the fit to converge on, which is itself the finding. **Reading the
  quaternion as written scores below the null**: it turns the picture twice as
  far as the wearer instead of holding it, which is the defect the owner saw
  made worse rather than fixed.

  **The heading is pinned outright and needs no scoring.** With the lock off
  the view rides the body, so the camera yaw that makes a later frame show
  what an earlier one showed IS the body's turn, in the renderer's own sign.
  Searched on the picture over three half-second pairs: -16, -22 and -10
  degrees, where the file's own yaw changed by +15.7, +22.0 and +8.6. The same
  turn to about a degree, the opposite way round.

  **Cross file and cross unit.** The turn oracle on the two other unit B files
  that carry one: conjugated 0.494 and 0.712 against nulls of 0.316 and 0.439
  and an as-written 0.241 and 0.280. The tilt head to head on the same two:
  conjugated 5.9 and 3.5 degrees against the mirrors' 9.0/6.9 and 10.3/7.7.
  The conjugate wins every comparison it was scored in, on four runs across
  three files. **Unit A cannot arbitrate**: its capture is a camera bolted to
  a car driving straight down a motorway, 4 degrees of turn in the whole 23 s,
  so the world-stability oracle has no signal in it and the road rushing past
  swamps what there is.

  **Controls.** The plant: `Scene::hold_at` forced with `about_down(20 deg)`
  renders, at view yaw 0, the pixel-identical picture that the unlocked view
  renders at yaw -20 (similarity 1.0000 against 0.40 and 0.26 for yaw 0 and
  +20), so the scorer reads a known rotation back through the delivered path.
  The null: with the lock off the same 12 renders track the wearer, 11.68
  degrees a step and 133 over the segment, which is the turn the file states.
  And `lock=0` against `lock=1` on an `.OSV` was a proven no-op before this
  and now differs: that null is a counter-null.

  **What is still open.** A left-handed reading of the file's own frame
  reverses the heading the same way the conjugate does; the two are identical
  for an upright camera and differ by twice its lean otherwise. The conjugate
  wins the head to head above by about two to one and is the only one of the
  three to beat the null, but the file's own accelerometer (field 3.2.10, in
  g) agrees with the *unconjugated* reading to 1.8 degrees and so argues for
  the mirror. The picture is the oracle and the accelerometer is the
  instrument disagreeing with it. The exposure is bounded by twice the
  camera's lean, 4.3 degrees at the median and 11 at the 95th percentile over
  the corpus. **A Mimo export of one clip with lock on and off would settle it
  outright**, and nothing else in this corpus will.

  The menu item un-disables itself off `Scene::has_orientation`, which is the
  same fact it was disabled by; the refusal stays for a capture that carries
  no orientation. Playback is unmoved: 29.97 of 29.97 presented on the owner's
  8k30p file, nothing dropped or starved.

- 2026-08-08 **The `.OSV` lens model has five coefficients, and the fifth is
  field 15.** The entry below shipped the plain equidistant map and called the
  file's `k` coefficients decoration. They are not: the model is
  Kannala-Brandt,
  `r = fx * theta * (1 + k1 t^2 + k2 t^4 + k3 t^6 + k4 t^8 + k5 t^10)`, and
  four of its five coefficients sit together at fields 5 to 8 while the fifth
  is at **field 15**, seven fields later, past the yaw/pitch/roll triple. The
  reader was taking the run of four, and four of them fold the radius over
  before 90 degrees, which is what the entry below measured and refused. With
  the fifth the radius is monotone to half a turn on all four lenses of both
  units.

  **What it was worth: the seam tear.** The owner reported the picture "fixed
  off everywhere" and a doubled tower. Measured through the app's own map with
  `--bin crossing` at a 90 degree seam view, `seam=factory`, the two lenses
  drew the same far content **241 source px apart across the seam on his own
  camera, 13.1 degrees**, and 149 px on the sample unit; after, **3.2 px
  (0.18 deg)** and **7.9 px (0.43 deg)**, both within a degree of the
  `theta0 + theta1 = 180` that far content must satisfy, and what is left is
  ordinary near parallax at the range of the content. **Both readings needed
  the instrument's search opened to 20 degrees to take the before arm at all**:
  at its own 1.4 degree default the old model accepts 0 of 18 sites on unit B,
  railing or correlating nothing, against 13 of 18 after. Opened up, 7 of 18
  before and 14 of 18 after on unit B, 5 and 12 on unit A. The along-seam
  component is nothing either way, 0.6 px before and 0.05 px after, which is
  what says the 241 px is the radial map and not a pose.

  **Coverage is the check anyone can redo.** The delivered 3840 px square holds
  the image circle inscribed, so half the frame is half the coverage: the
  five-term model puts 1920 px at 98.9 to 99.4 degrees off axis, i.e. **197.9
  to 198.8 degrees**, which is the Osmo 360's published figure. Equidistant put
  the same radius at 209 to 211, a lens nobody makes, and that surplus is the
  tear. The refusal below leaned on fields 22 and 23, a fourteen-point
  polyline, read as this lens's coverage rim; those 112 bytes are byte-identical
  across all four lenses of both units, so they are a model constant - the arc
  where the camera's own body cuts the bottom of the picture - and a constant
  cannot measure a lens.

  **`image_radius` on this format is now the image circle** rather than the
  largest circle that fits around the principal point. That number moved from
  1910.2 to 1916.7 px across four lenses purely because the principal point
  wanders 10 px about the frame centre, and every delivered frame is lit past
  it: measured on frames of both units, content runs to the frame edge at the
  mid-sides and out to about 2035 px on the diagonals before the optical rim.

  **The `.insv` path is untouched, byte for byte.** Sixteen rendered views over
  three files - an X4 Air at eight views including the fitted and the stored-fit
  paths, plus a seam zoom, the nadir, the 220 degree ball and a one-stream ONE
  X2 - are identical to the branch's base at `a7b6930`, hash for hash. The
  Mei arm of the map is the arithmetic it always was; what changed under it is
  that the five coefficient slots of the uniform block are now named for the
  model that reads them, and Mei reads the same five it always did.

  **What is still open, and disclosed rather than fixed.** The per-file seam
  fit remains structurally dead on this format: the pose knobs write
  `lens.pose.*` and a DJI lens takes the mounting branch, so a fit cannot move
  the picture. With the model corrected the fitter now finds enough azimuths to
  try - the app's refusal moves from "only 2 of 72 azimuths" to "the seam
  readings do not pin a correction" - and then hits that wall. Factory
  calibration now joins at infinity without it. Near-field ghosting at metre
  range remains and is parallax, not calibration.

  Playback is unmoved: best of three 20 s runs each side on one box, 29.22 fps
  presented and 8.90 ms a redraw in the pass before, 29.32 and 8.00 after, with
  the box carrying another agent's work throughout (`--bin playback`, unit B
  8k30p, 2560x1440).

- 2026-08-07 **A DJI Osmo 360 `.OSV` plays, on an equidistant lens model and
  with no horizon lock** (branch `feat/osmo-osv`, MVP). `kjerag <file>.osv` is
  the whole of it. The calibration is in the file's own `djmd` telemetry track
  rather than a trailer, as a protobuf with no `.proto` anywhere, read by field
  number in `kjerag_meta::osmo`; both units' parsed intrinsics match the
  scoping pass's independent table exactly, to the last digit of the `f32`.

  **Equidistant only, `r = fx * theta`, and the four `k` coefficients the file
  carries are read past.** *(Superseded 2026-08-08 by the entry above: there
  are five coefficients, not four, and this paragraph's whole argument rests on
  a polyline that turns out to be a model constant. It stays as written because
  the arithmetic in it is right and only the premise is wrong.)* Each lens entry
  also writes a fourteen-point mask of
  where the camera body cuts the picture, and on all four lenses of the two
  units it sits 1804 to 1860 px out. Equidistant puts that at 98.6 to 101.7
  degrees off axis, so 197 to 203 degrees of coverage, bracketing DJI's
  published 199. The Kannala-Brandt theta-polynomial reading of the same
  coefficients turns over between 88.4 and 89.8 degrees at 1615 to 1640 px and
  folds back, so it cannot reach that ring at any angle and would make a 199
  degree lens a sub-hemisphere one. The inverse reading folds too. No candidate
  form was kept, and the refusal is a forward check anyone can redo from the
  file rather than an overlap score: the scoping pass's overlap scorer
  preferred a known 20 px principal-point error, so its preference is not
  evidence.

  **Horizon lock is off on this format and says so.** The file carries a fused
  orientation at about 1 kHz whose frame is not pinned, and applying it naively
  made the scoping stitch worse, so none is read. The menu item draws disabled,
  the key bind does nothing rather than flipping a setting that cannot move the
  picture, and the app prints one `level:` line at open. Manual pan is v1.

  **The seam is left to fit itself and refuses.** No inter-lens translation is
  recorded, so the parallax band switches off, and on both units the fitter
  found 0 of 72 azimuths with content it could match and kept the factory
  calibration. Far-field content joins cleanly; near-field shows a soft band at
  the handover. That is the accepted v1. *(Corrected 2026-08-08: far-field
  content did not join cleanly, it was 13 degrees out, and the fitter's refusal
  was the lens model's doing rather than the content's. The refusal itself
  stands, for a different reason: the fit is structurally dead on this format.)*

  Measured with `--bin playback`, rendering 2560x1440, VA-API, 20 to 30 s of
  paced playback per row; kjerag has no software decode path. **Two columns for
  every number, because this box was not this branch's alone**: a second agent
  was running its own instruments out of another worktree for most of the
  session, and the same command on the same build read 6.91 ms a redraw in a
  lull and 21.05 ms beside that agent's run. So the best of five runs and the
  worst are both here, and neither on its own is the box's answer.

  | capture              | decode, best  | presented, best | dropped | pass, best | presented, worst | dropped | pass, worst |
  | -------------------- | ------------: | --------------: | ------: | ---------: | ---------------: | ------: | ----------: |
  | unit B 8k30p         | 2.56x         | 29.57 of 29.97  |      10 |    6.91 ms |   25.54 of 29.97 |     129 |    21.05 ms |
  | unit B 8k50p         | 1.60x         | 49.45 of 50.00  |       4 |    5.11 ms |   24.43 of 50.00 |     242 |    38.46 ms |
  | unit A 8k25p         | 3.00x         | 24.87 of 25.00  |       0 |    6.51 ms |   21.89 of 25.00 |      13 |    27.53 ms |
  | X4 Air `.insv` 8k30p | 2.44x         | 29.94 of 29.97  |       1 |    7.90 ms |   27.24 of 29.97 |      82 |    20.00 ms |

  What survives the noise is the **comparison**, because the `.insv` control in
  the last row was measured in the same conditions and moves with the rest: an
  `.OSV` plays at its own rate when the box is free and falls short when it is
  not, and it does so by about as much as an `.insv` does. **Decode is not what
  runs out.** Even at its slowest the 10-bit VA-API pair decode ran at 1.17x
  realtime and no row starved for more than 33 redraws; what moves is the
  render pass, which is the same pass both formats draw through. That is worth
  saying against the scoping pass's figure of 487 percent CPU for ffmpeg to
  software-decode the same file at realtime. The app's own report, in a window
  at 1280x720, presents 30.00 of 29.97 and 25.00 of 25.00 with nothing dropped
  or starved on units B and A.

  D-Log M is out of scope: those files play with the log look, and no transform
  for it is in the container.

- 2026-08-07 **An acceptance line names the pose instead of copying it**
  (`seam=pool`, docs/research/reference-views.md). Three acceptance commands -
  the shimmer line, the May-01 crossing pair, and the `--bin step` block under
  seam-two-axis's "How to run the two instruments" - carried
  `seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`. That string is
  the knob-by-knob **median** of the owner's five-sample pool and no member of
  it: roll and cx off one fit, yaw off a second, pitch and cy off a third. It is
  the combination `SeamPool::answer` was changed to stop shipping on 2026-08-05,
  so **those three commands ran a pose the app had not drawn for two days**;
  every acceptance line written since then names the drawn one. The pose it
  draws is
  `roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91`, confirmed both by
  `config`'s own fixture test and by a `--bin reframe seam=pool` run on the
  owner's Jul-14 capture.

  At that milestone `seam=pool` was the durable acceptance argument: it read
  the app's saved state through the app's own reader and refused rather than
  falling back when the pool held nothing. It required the temporary
  `spike -> app` workspace edge. The pool, argument and upward dependency are
  now retired with the product calibration mechanism; current instruments
  default to `seam=factory` and accept only an explicitly named five-knob
  research correction.

  **The recorded readings on those three lines were measured through the old
  string and have not been re-read at the drawn pose.** They stay, flagged in
  place. Re-reading them is a job of its own.

- 2026-08-06 **The reference registry is re-derived into the world-fixed frame,
  and the seam ladder is re-baselined there** (`--bin carried`,
  docs/research/reference-views.md). Fourteen `lock=1` lines, one correction
  each, computed at the line's own frame rather than per file: `carried` runs
  from -72.41 to +79.33 degrees across the registry and moves by 3 degrees a
  second at the shimmer view, so no two lines in a file share a correction.
  Each was then checked in the picture, 67a4bcf rendering the old line against
  this build rendering the new one: twelve match at zero pixels of 1024 and two
  at 2 px, correlation 0.92 to 1.0000, against a control that leaves the yaw
  alone and lands 1.7 to 20 degrees out.

  **Two of #165's numbers do not survive that.** The shimmer view's re-derived
  aim is `yaw=162.31` and not the `160.63` published there, which was the same
  rule read off a half-second grid; the picture is 83 px out at 160.63 and 2 px
  out at 162.31. And the eight-fold improvement in `--bin shear`'s floor
  (0.0773 -> 0.0099 deg of step rms at the seam band) is neither a floor nor an
  improvement: at the corrected aim it reads 0.0687, and dropping each run's two
  worst steps leaves 0.0097 before, 0.0088 after and 0.0094 at #165's aim. One
  frame in ninety fails its correlation and decides the statistic. What the lock
  change really bought is yield, because fewer frames are refused for a seam
  past `TILT_LIMIT`: usable step pairs go 71 to 89 at the seam band and 30 to 89
  at `+60`. The seam itself did not move, `-150` reading 0.3646 -> 0.3381 deg
  applied, and `--bin crossing` at the May-01 GOOD/BAD pair moves by 0.14 source
  px across the change with its verdict and its sensitivity floor intact.

- 2026-08-06 **The horizon lock holds the world, heading and all** (owner
  ruling; `Filter::yaw_seconds` 3 s -> infinite). It supersedes the
  2026-07-31 entry below, which chose the high pass.

  The owner asked for what Insta360 Studio does, in his words hold the world
  still, and the oracle probe had already measured how far from that the
  shipped design was. Against a Studio export of the same July 14 window,
  registered chain-free frame by frame, kjerag's view swept **404.7 deg/min
  away from Studio's with r2 0.98** over ten seconds: a straight ramp, which
  is a design and not a defect. The same measurement of this build reads
  **4.4 deg/min with r2 0.04** over the same ten seconds, which is no ramp
  at all, only a 6.6 degree peak-to-peak wobble the two stitchers disagree
  by. Dense phase correlation, the second instrument, reads the picture's
  own slide over the probe's headline three seconds at **22.3 degrees before
  and 3.4 after**, against Studio's 0.1.

  **The accepted price is gyroscope drift, and it is the technique's floor
  rather than a shortfall.** Nothing observes heading: gravity cannot see it
  and no capture on the test box carries a byte of the trailer's magnetic
  record, so a locked yaw inherits the gyroscope's zero and nothing ever
  corrects it. Studio's own export drifts the same order, 2.2 deg/min on the
  probe's window.

  **It is not a steady creep, which is the thing to say out loud.** The locked
  frame turns about the world vertical at `bias . up_in_body`, so a camera
  hanging tens of degrees off vertical brings its horizontal bias components
  in, and those are the larger ones: on the July 14 file `--bin drift` walks
  the running error to -36 degrees by minute 3, +87 by minute 8 and +149 by
  minute 19, about 185 degrees peak to peak, against a signed mean of 2.08
  deg/min. Quoting the mean alone, or the body's own yaw-axis bias, describes
  a flight nobody flew. The shape is the finding and the size is not: this
  file has no still moment good enough to read a zero from, and the ten
  seconds `--bin gyro` picks instead give the same walk hundreds of degrees
  either way with a 1.40 deg/min mean.

  **The world frame's zero is the heading at the file's first IMU sample**,
  not its first video frame: 18.71 degrees of the body's own turning apart on
  that capture, and that is where `Ctrl+0` lands.

  What the pilot loses with all this is the fly-forward feel: the view used to
  settle back onto the nose within a few seconds of any turn, and now a turn
  leaves it pointed where it was, so
  a flight that turns round shows the way it came until somebody drags. What
  goes with the follow is its erosion of a pan against the world, and gyro
  drift takes that place: issue #44 was closed by the owner as the t0 seed
  transient and not as a defect in the drag or the follow (entry above), and
  the old design's own control says the same, 0.62/0.03/0.38/0.18 degrees per
  half second after a pan at `from=300`.

  **Every stored `lock=1` view line moved, and by a lot.** The yaw in one of
  those lines is measured in the stabilized frame, whose zero was the
  followed heading and is now the file's opening heading, so the two differ
  by however far the old follow had been carried: measured on the July 14
  file through `--bin lean`'s own heading column, 6.8 degrees at the first
  frame, 44 by 6.5 s and 157 by 36 s. The rule is
  `new_yaw = old_yaw + carried(t)`, confirmed in the picture at the shimmer
  view. **The rule holds and the number this entry first put on it does not**
  (corrected 2026-08-06): that view's re-derived aim is `yaw=162.31` and not
  the `160.63` written here, because 156.85 is `carried` at the 36.036 instant
  the half-second walk measured and 158.53 is `carried` at the frame
  `time=36.303` shows. The picture is 83 px of 1024 out at 160.63 and 2 px out
  at 162.31, which is where the "to 1.6 degrees" came from. The registry was
  re-derived line by line the same day, in the entry above.

- 2026-08-07 **The along-seam field is real on the unbent projection and worth
  nothing in the delivered picture, because the per-frame band had already taken
  it out. It is measured, guarded, stored and NOT applied; what ships is the
  per-reading trim and a new rule about acceptance** (issue #103, stage 9 layer
  2, docs/research/stage9.md 8 and 9).

  **What was built.** `seam::measure` reduced each azimuth's frames with a mean
  over a population that moves 0.22 to 0.48 deg by rms between frames; it now
  reduces them with `seam::left`'s own rule applied per frame, one function
  (`seam::tolerated`) shared by the ring gate and the per-frame trim. On the
  unbent projection the trim alone takes the pooled X4 leftover under the stored
  pose from **0.0828 to 0.0653 deg** and the corpus's cross-capture agreement
  from 2 pairs of 15 to 15 of 15. `seam::along_terms` then reads the five terms
  `band::Along` is written in, above the factory calibration and above no pose
  at all, and held out through the shipped functions it takes the pooled
  leftover **0.0644 -> 0.0375 deg on the X4 Air and 0.0414 -> 0.0140 on the ONE
  X2, 9 of 9 improved**. At the two May-01 crossings `--bin crossing` reads the
  along-seam median **1.29 -> 0.12 view px at GOOD and 1.47 -> 0.93 at BAD**,
  both improved, neither traded.

  **Why none of that is an applied result.** Every instrument above draws the
  **unbent** projection: `seam::measure` reads its ring through `Reframe` with
  no band, and `--bin crossing` builds its map with `Held::default()`. The app
  does not. Photographed out of the app itself, at the same clip and view, the
  delivered along-seam axis on `main` is **already at or under 0.6 view px at
  GOOD** - the band's own per-frame `Along` fit had taken the same leftover out -
  and the field arm matches it within 0.2 against an instrument shown capable at
  1 px. At BAD `main` reads **-0.11 view px** where the unbent projection reads
  1.47, and the field arm reads **-2.06: two view pixels the wrong way**. At the
  shimmer view the field arm is slightly worse on every probe.

  **The owner's blind A/B said the same thing first**: "same, both bad" at every
  view, with the `main` arm called slightly steadier at the shimmer view, which
  the instrument agrees with at 10.081 against 10.214 codes per frame over 60
  frames.

  **Why reading the table through the band did not save it.** With a table `T`
  applied and the band measuring through it, the delivered correction is
  `T + fit(L - T)` against `fit(L)` with none, so the two differ by exactly
  **`T - fit(T)`**. `Along::fit` reproduces `T` only where the ring has
  evidence, and a session's ring is an arc: planting the real pooled field,
  `T - fit(T)` reads 0.0007 deg rms with all 128 directions covered and
  **0.0333 rms, 0.0696 worst at the 27 of 128 `--bin step` reports on real
  footage** - 1.13 view px at the BAD view's scale, varying with azimuth, which
  is the size and the shape of what was measured. Reading through a table is
  necessary and not sufficient, and that binds anything that ever fills the
  `Table` uniform.

  **THE NEW BINDING RULE**: any change that applies something at the seam must
  include a **delivered-app-path comparison against `main`** in its acceptance,
  not only the unbent instruments. The A/B protocol is part of the battery and
  not only the owner's gate, and it has **two halves**
  (`~/kjerag-ab/delivered.sh`): a **difference** half, the app photographed at
  the view against the same binary run twice, which says whether two builds draw
  the same picture and cannot say which is better; and a **quality** half,
  `--bin step` and `--bin shear` with `seam=file` and the band live, which reads
  the seam itself. One control pair does not bound a spread, and a capture only
  counts if the fit landed before the shutter.

  **What ships.** The per-reading trim; `seam::along_kept`'s harvest guard,
  which refuses a sample whose own five terms compose to more than 1.2x the
  leftover they were fitted to (the July-25 flight, 170 deg of hole, reads 1.33
  and is refused outright at the app's plan); and the field stored dormant in
  `SeamSample::along_deg` against the one regime the delivered finding does not
  cover, the first frames of a session before the band has evidence. `Table`
  ships at `REST` as on `main` and the compute pass's read-through is removed
  with it. **The pool is not discarded**: what paid for that cost was the
  applied field. Measured against `main` in the delivered picture with an empty
  pool on both arms, the branch sits inside the same binary's own run-to-run
  spread at both May-01 views (2.275 codes mean against a 4.443 control at GOOD,
  7.343 against 6.740 at BAD) and outside it at the shimmer view (0.758 against
  0.105), where the trim moves that file's fit by +0.032 deg of roll, -0.079 of
  pitch and -1.31 px of `cy`. **And the trimmed fit is the better one in the delivered picture**, which the
  quality half settles: step at the seam -21.19 -> **-18.89 view px**, the
  band's own along-seam load 0.176 -> **0.159 deg** mean and 0.792 -> **0.498**
  worst, and `--bin shear`'s residuals smaller at all four bands with the
  steadiness unchanged, three runs each and deterministic; reproduced
  independently at the same aim from a different band state (no warm-up, 26 of
  128 directions against 47 to 48) at -21.97 -> -20.69 view px with the band's
  load 0.227 -> 0.199 deg mean. A cleaner pose leaves less step at the seam and
  less for the band to carry, and it does. **The claim is one camera, one flight
  (July-14), two views, two band states**: the two May-01 crossings cannot be
  read this way (line fits at 51 to 54 px rms) and the X2 view answers "no
  horizon fitted on both sides of the seam".

- 2026-08-06 **No along-seam table is fitted: above the five terms the pass
  already applies, what is left is not a static function of azimuth this corpus
  could have found** (issue #103, stage 9, docs/research/stage9.md). The
  mechanism is built, measured and ships at rest.

  Stage 9 asked for a fourth layer on the along-seam axis: one number per
  direction, pooled per camera, carrying what `SeamFit`'s five knobs and
  `band::Along`'s five harmonic terms cannot say. `kjerag-spike --bin table`
  measures the case for one, off the ring `seam::measure` already reads, and
  the case fails on every count that decides it.

  On the owner's X4 Air, six flights April to August, 299 gated readings: the
  pooled pose leaves **0.064 to 0.128 deg rms** along the seam per capture,
  which is the 1.30 and 1.43 view px `--bin crossing` reads at his two May-01
  crossings. A five-term fit takes the pooled leftover 0.0818 to **0.0739**;
  five more orders take it to 0.0712, which is **3.7 percent**. Pooled over all
  fifteen pairs of flights, the two captures' readings at the azimuths they
  share correlate at **+0.194** as they stand and **-0.014** once each flight's
  own five terms are taken off: all of the agreement between flights lives in
  the orders `band::Along` already applies, and nothing above them is shared at
  all. And the test that decides, each capture predicted by a table fitted on
  the other
  five: **a table costs the flight it was not fitted on at every width that
  resolves anything** (0.0836 to 0.0872 against a **0.0828** no-table baseline).
  The fitted column improves monotonically as the kernel narrows while the
  held-out column gets worse in step - the stage-7 striping lesson as a number.
  Swept past any width a per-azimuth field is interesting at, **the best any
  static table reaches on a capture it was not fitted on is +1.25 percent**, and
  that is the ceiling on what this corpus could ever have paid.

  **It is a refusal and not a blind spot.** A planted six-cycle table, an order
  above anything the pass applies, is put in the map and the corpus re-measured
  through it: read back at 0.894 and 0.910 of itself at 0.05 and 0.10 deg over
  107 azimuths, and through `--bin crossing table=` at the May-01 GOOD view at
  slope -1.07 and -1.26 per site with the **epipolar axis unmoved** (+0.006 and
  +0.023 src px, MAD 0.04 to 0.06) and the traced 50/50 contour unmoved. An
  order-6 field at half the size of the residual being looked for is plainly
  visible to this instrument. **And the refusal carries a size**: planting a
  static field of a known order and asking how much of its power comes back on
  a held-out capture, this corpus excludes order 3 and up above **0.02 to 0.06
  deg** of amplitude and says nothing below 0.02, which at the owner's May-01
  GOOD view is 0.37 view px against an along-seam error of 1.30. A static field
  of a few tenths of a pixel is not excluded; one large enough to be most of the
  defect is.

  The second camera says the same, **and it is the one place the answer depends
  on the gate.** The ONE X2 of issue #130, whose factory extrinsics are 2.8
  degrees out, has a leftover whose order-3-and-up structure does reproduce
  across three captures of one evening (0.0127 deg of azimuth structure against
  0.0116 of cross-capture scatter) - and held out it buys 2.7 percent at its
  best width (0.0692 against 0.0711) while a 4-degree kernel is already worse
  than nothing. With the along-seam plausibility gate **off** those same three
  captures support a table at **+10.0 percent** (0.2602 against 0.2890): what
  the gate removes is 12 to 14 readings per capture with a tail past two
  degrees, which is exactly what an ungated table would soak up, but the reader
  has to see that the sentence turns on it. **The X4 Air corpus, which is the
  one that decides, does not turn on it**: ungated it reads +0.03 percent
  (0.2985 against 0.2986), gated +1.25.

  **This corrects the #155 entry below.** "A static per-azimuth map is enough
  for along the seam" was read off the fact that the along-seam **median**
  reproduces across flights, 1.1 source px apart between May and April. It
  does. Its **per-azimuth structure above what the pass already applies** does
  not, and the median is what `band::Along`'s constant term is already for.

  **And the "does not reproduce" half of this entry is WITHDRAWN**, by the
  layer-2 preflight corpus run (`research/layer2-preflight`,
  `scratch/layer2/CORPUS.txt` and nine per-reading dumps under
  `scratch/layer2/corpus/`). It was a property of the estimator, not of the
  camera. `seam::measure` means over each azimuth's frames and the band's
  `off_epi` EMA does the same, over a population that moves 0.008 to 0.05 deg
  between frames by median absolute deviation and **0.22 to 0.48 by rms**.
  Reduced with `seam::left`'s own 4-MAD rule per reading, at full density, the
  same nine captures under the same pose and the same gate read **apart 0.0293
  against spread 0.0542 on all 15 X4 pairs and 3 of 3 X2 pairs** - where the
  mean managed 2 of 15. So "two flights disagree at one azimuth by more than
  either varies round the whole ring" and "the signal is under its own noise"
  are struck.

  **The table refusal survives, and one of its two cameras carries it.** Held
  out: **on the X2 a table costs 4 to 6 percent under every reduction** (mean
  +4.1, trimmed +5.6, median +5.2); **on the X4 the effect runs -1 to +2 percent
  depending on the estimator** (mean -0.1 and -0.6, median -0.6, trimmed +2.4, an
  independent trim +1.3), which is nothing either way. The kernel sweep is flat
  from 4 to 36 degrees on both in the table-alone arm. And what survives the five
  terms has an **amplitude of 0.004 to 0.005 deg** - the orthogonal part of the
  ladder's 0.0199 and 0.0195, not their difference - which is **0.13 to 0.16
  source px, an eighth of a pixel** and two to three times finer than `--bin
  crossing` resolves; removing all of it perfectly would improve the held-out
  residual by 1.8 to 4 percent, depending on which arm's residual it is measured
  against, and a fitted table does not get it. The
  certifying control is cross-capture, not within-session: the same test on the
  same partitions recovers the five-term field on 9 of 9 captures. So it is a
  refusal and not a blind spot. `Table::REST` ships.

  **What does reproduce is the five-term along-seam field, one harmonic order
  below where this stage looked.** Fitted on other flights only, held out on
  every capture of both cameras: X4 pooled leftover **0.0536 -> 0.0211 deg**
  (1.69 -> 0.66 source px), X2 **0.0606 -> 0.0249**, **9 of 9 improved**. That
  is the pose-order field pooled per camera, which `band::Along` computes per
  session and nothing yet carries between sessions. A pose refit on trimmed
  readings moves the pool materially (`cy` -11.91 -> -13.18, `pitch` -0.936 ->
  -1.096, per-capture leftovers 0.049-0.062 -> 0.028-0.039) but does not stack
  with it (held out 0.0208 against 0.0211): two removals of the same thing.

  **Why this stage's instrument could not see it, and the scope that follows.**
  The reproduction needs roughly ten readings per azimuth. `--bin table`'s 12
  places by 4 frames lands about two, and its `dump=` writes the ring after
  `seam::measure` has meaned it, so the artifact is baked into the recorded
  rows. Subsampling the peer's dumps to each depth and running this stage's own
  trim and gate: 12 moments reads apart 0.0938 against spread 0.0780 and 2 of 15
  pairs, 120 moments reads 0.0409 against 0.0531 and 15 of 15, all 1200 reads
  0.0254 against 0.0483. Everything this entry says about amplitude was measured
  at the thin end through the mean, so it bounds what a thin badly-reduced corpus
  could see and not what the camera has.

  **One consequence for the shipped code, for whichever stage takes it up**:
  `seam::measure` and the band's `off_epi` update average a population they
  should be filtering, and on the GPU that is one comparison against
  `held.off_epi` before the exponential average.

  What ships is `band::Table` at `Table::REST`: 128 numbers in the `Reframe`
  block, added to the band's own along-seam term before projection on the
  unwarped body ray, lens 1 whole and lens 0 not at all, tapering to exactly
  zero at any direction no reading reached. Empty, it is the picture
  `origin/main` draws, byte for byte at four registry views, and its cost does
  not measure: 0.04 ms of an 8.10 ms redraw on a quiet box, which is 0.24
  percent of a 16.6 ms frame, and a paired -1.66 to +0.72 ms interval under
  load. Nothing in the app sets one, and the loop that would is the open
  question at the top of the PR.

  **One process failure, recorded rather than tidied**: the charter's protocol
  says to freeze the kernel width before the hold-out partition is opened, and
  this stage swept it against the hold-out column instead. Nothing turns on it -
  every width is at or worse than no table, so the sweep chose nothing - but a
  corpus that had said yes would have needed the whole measurement retaken.

- 2026-08-05 **The handover is eight degrees wide, because the eye said so
  against every instrument that had an opinion** (`projection::CROSSOVER_DEG`
  2 -> 8, clamped per camera by `band::affordable`).

  The owner ran the two arms of `fade-ab.sh` label-blind on his own footage,
  arm 1 the shipped 2 degrees and arm 2 an 8 he was not told about, and said
  ***"2 is way better. Def not perfect but way better"*** of arm 2. Every
  number in this campaign says the opposite, and this entry is the record of
  both.

  **No instrument in the sweep can pick a width, and that is a measurement.**
  Five widths - 2, 4, 6, 8, 12 - at five owner reference views, on the four
  statistics that bear on the trade, and every one of them is **monotone** with
  no knee anywhere: sharpness over the overlap falls smoothly (0.686 / 0.657 /
  0.626 / 0.593 / 0.520 at the May-26 dirt view, and the same shape at all five
  views, `--bin seam mode=blend`), the doubled band grows (1.60 / 3.20 / 4.80 /
  6.40 / 9.60 degrees), the corridor's own step statistics get **worse**
  (0.0619 / 0.0673 / 0.0723 / 0.0773 / 0.0824 deg rms at the seam band,
  `--bin shear mode=probe`), and only the epipolar shear improves (disparity
  over width, so 1/width by construction). A monotone curve has no preferred
  point on it. **The instruments priced the trade; they were never able to
  choose on it, and one label-blind verdict did.** Recording that is the point:
  an agent that had waited for a number to justify 8 would have waited forever,
  and an agent that had read the numbers as a verdict would have shipped the
  arm the owner rejected.

  **Those first two rows are the instrument's own ramp and not the map's**
  (corrected 2026-08-06). `--bin seam mode=blend`'s `bands=` rows are a
  synthetic linear crossover the instrument builds itself (`Weighting::Band`),
  with the per-frame bend switched off, so its doubled band is `0.8 * width` by
  construction and grows exactly linearly. The shipped path is the same
  instrument's `shipped` row, which reads the map, so `KJERAG_HANDOVER_DEG` is
  what sweeps it. At the July-14 anchor moment (yaw 90, fov 60, the file's own
  fit), 2 / 4 / 6 / 8 / 12 asked for:

  | | 2 | 4 | 6 | 8 | 12 |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | doubled band, deg | 1.50 | 2.79 | 3.89 | **4.78** | 5.41 |
  | sharpness | 1.309 | 1.247 | 1.194 | **1.150** | 1.120 |

  So four times the width doubles **3.2 times** as much picture and not four
  times, the sharpness falls **12 percent** over that span and not 14, and the
  curve flattens above 8 because the ask is being clamped: this file affords
  9.69, so the 12 column is a 9.69. Monotone either way, which is what the
  paragraph above rests on.

  **Every row of that instrument is drawn with the per-frame bend off**, the
  `shipped` row included, and it is only the far field that makes that
  harmless: out there the bend is a fraction of a degree, so a weighting priced
  without it is the picture to within its own size. Near field it is not, and
  the near-field paragraph below uses `--bin band mode=render` instead.

  **What the sweep did settle is the other end.** 12 is refused by the optics
  on every camera in the corpus, and 8 is nearly the last width that is not.
  The handover reaches `width / 2` off the seam plus the whole bend it carries,
  and past the two lenses' shared ring it stops being a handover at all. Not by
  sampling off the end of a fisheye circle, which is what this entry said until
  2026-08-06: the coverage test is taken on the unbent ray and the bend then
  moves the sample, but a bent ray that lands outside a lens's boundary comes
  back `inside == false`, `projection::claim` returns zero for it, and the
  fragment shader reads a lens only where its weight is positive. What happens
  past the edge is that the **coverage depth** takes the weight over from the
  crossover's ramp and steps it to zero at the rim, so the picture is handed
  over by the optics instead of by the width that was chosen, and where both
  lenses miss it is transparent. The bound is conservative and it stays; the
  reason it stays is that.

  Measured with `--bin band` on the owner's own captures, re-measured
  2026-08-06: six X4 Air files overlap by **14.56 to 15.02** degrees and afford
  **9.36 to 9.82** (May-01 002 9.36, Jul-25 002 9.40, Aug-02 002 9.41, May-26
  004 9.48, Jul-14 006 9.69, Jul-25 001 9.82), the calibration fixture overlaps
  by 14.44 and affords 9.24, and the ONE X2 overlaps by **9.19** and affords
  **3.99**, under the 8 the picture asks for. The earlier "9.8 to 10.0" was one
  file's number read as a family's and was wrong on five of the six. So the
  width is the camera's now, not the picture's: `Reframe::crossover` is the ask
  clamped by `band::affordable`, it travels in the uniform block rather than
  being written into the shader source (the shader is compiled once, before any
  file is open), and the X2 hands over across 3.99 while the X4 Air hands over
  across 8. The margin inside the overlap on the fixture goes from 3.18 degrees
  a side to **0.62** (0.68 to 0.91 on the corpus files), which is the honest
  price of this and is why the bound is now asserted against the shipped width
  instead of against `WIDEST_DEG`
  (`the_widest_band_and_its_bend_stay_inside_the_overlap`). The X2 sits exactly
  on the bound, with 0.00 to spare, which is what "affords" means.

  **The width follows the calibration.** Every number above is under the file's
  own seam fit, which is what the pass draws with, and the factory calibration
  is a different answer: a fit moves the principal point, which moves each
  lens's coverage boundary, which moves the overlap. On the X2 the factory
  calibration affords 4.91 and its own pooled fit affords 3.99. So the drawn
  width is a reading and not a property of the file, and it is said twice
  rather than once: the app prints `blend:` under `seam:` at open, off whatever
  calibration has landed by then, and the render crate's own fit path prints
  `blend:  that fit moves the handover: 4.91 -> 3.99 deg` when a later fit
  moves it. The second line is not tidiness. On a camera with **nothing
  pooled** the first line is the FACTORY width, because the fallback fit lands
  a second after it: verified 2026-08-06 on the October X2 against an empty
  pool, which prints 4.91 at open and 4.91 -> 3.99 when the fit lands 1.2 s
  later. Before this branch there was no line at all, which is the state an A/B
  on the width must not be run in again.

  **Stage 4 is inert at this floor, and nothing it recovered is lost.** Its
  adaptive term opens the band to `|disparity| / FOLD` and cannot exceed 2.89
  degrees, so a floor of 8 is above every width it could ask for and
  `band::width` is a constant on every file in the corpus
  (`the_adaptive_width_is_inert_under_the_shipped_floor`). What stage 4 was for
  - a near-field reading being clamped by a band too narrow to carry it -
  cannot happen at 8 either: `carried` clamps at `FOLD * 8`, 7.2 degrees, and
  the search cannot report past 2.6. The mechanism stays because the floor is
  the camera's: a camera whose overlap forced it under 2.89 would put the
  reading back in charge.

  **Stage 4 did have work to do at 2, and the far-field views checked here are
  not where it did it** (corrected 2026-08-06). `--bin band` reports zero
  direction-frames over the floor at 8 on every stretch tried, and it reports
  zero at 2 on the same far-field stretches - but pointed at a stretch with the
  pilot's own gear on the seam it does not: at `KJERAG_HANDOVER_DEG=2` the
  May-01 file at `from=550` opens 2 direction-frames of 40 x 128 to 2.531 deg,
  recovering 8.0 view px of doubled edge on content at 0.84 m, and the May-26
  file at `from=30` opens to 2.144 deg on content at 0.99 m. Both are inside 8,
  which is why the claim above holds; what was wrong was the evidence offered
  for it.

  What is genuinely lost is stage 4's other half, that the band never opens
  further than it has to - **near-field content is now drawn twice across the
  same 8 degrees as the far field, and it pays more for it than the far field
  does**, where stage 4 would have given it at most 2.89.

  That is witnessed and not arithmetic. The pilot's harness, legs and machine
  sit at 0.8 to 1.5 m ON the seam in every corpus file, at phi 79 to 127, which
  is reached at yaw 90 or 270 and pitch -53 to -90 with the horizon lock off.
  The instrument is **`--bin band mode=render`**, whose `share` column is the
  seam band's own gradient energy over the same picture's 9 to 25 degrees off
  it, run twice per view under `KJERAG_HANDOVER_DEG`:

  ```sh
  KJERAG_HANDOVER_DEG=2 kjerag-spike --bin band -- <file.insv> mode=render \
    from=30.23 count=60 yaw=270 pitch=-53 lock=0 out=scratch/nf-w2
  ```

  | view, 2 -> 8 | share | fall |
  | --- | ---: | ---: |
  | May-26 004 gear, 0.99 m (`from=30.23 yaw=270 pitch=-53`) | 1.387 -> 1.182 | 14.8% |
  | May-01 001 under the pilot, 0.84 m (`from=550.15 yaw=90 pitch=-90`) | 0.725 -> 0.613 | 15.4% |
  | May-26 004 far field, same frame (`yaw=90 pitch=0`) | 0.695 -> 0.626 | 9.9% |
  | May-01 001 far field, same frame (`yaw=90 pitch=0`) | 0.341 -> 0.310 | 9.1% |

  So the near field pays about **one and a half times** what the far field
  pays, on the same instrument, the same frames and the same statistic. It has
  to be this instrument and not `--bin seam mode=blend`: that one draws every
  weighting with the per-frame bend OFF, including its `shipped` row, and the
  bend is exactly the near-field mechanism (the disparity under the pilot reads
  1.9 to 2.3 degrees there). Measured with the bend off, the same two views
  read a 12 percent fall and looked like the far field, which is how this
  paragraph first came to say "the same size". The `share` statistic's own
  window stops at 5 degrees while the handover reaches 6.6, so all four rows
  are floors under the effect rather than its size. Pictures in gitignored
  `scratch/near-field-witness/`.

  The along-seam findings the research arm came back with, which the width
  above rides on, follow.

  **There is only one support, and it is the crossover's.** The along-seam term
  goes to lens 1 whole and lens 0 takes none of it, and that is not a choice
  about width: it is the difference the fit measured, so it is what makes the
  two lenses draw one piece of content in one place. Wherever both lenses are
  in the picture that difference is pinned at one whole correction, so what the
  picture shows walks from none of it to all of it exactly as the weights do,
  and a ramp spread wider than the weights is a ramp that un-corrects the seam
  over the width it spread. Splitting the correction across both lenses near
  the seam, which was the other candidate, is that same un-correction written
  differently: it displaces lens 0's near-seam content by up to half the
  correction, which today is exactly zero, and leaves the crossover's own
  excursion where it was. So the knob is the crossover width, this entry is
  that knob with a name, and the width is what moved.

  **`mode=profile`'s 0.70 degree bracket is the instrument's readout and not
  the map's ramp.** What the picture carries of the along-seam correction at
  one distance from the seam is lens 1's weight, which the Rust twin reports
  with no correlation in the way: a smooth ramp over the whole crossover
  (`the_along_seam_correction_hands_over_across_the_whole_crossover`). The
  instrument reads a step because its held arm carries the two lenses' whole
  18.7 px disagreement as a double image across that same corridor: its match
  has two peaks and reports whichever leads. Doubling the map's ramp to 4
  degrees leaves the printed bracket at +24 to +60 px, exactly where 2 degrees
  put it; only 8 degrees moves it, to +12 to +72, which is 1.17 degrees of a
  ramp that is 8. **The bracket is a lower bound on the handover and not a
  measurement of it.**

  **And the ramp does not simply scale with the width.** The weights are
  cosines of the two lens axes and not a distance, so a wider corridor is a
  different slice of them: the walk from nine tenths of the correction to one
  tenth spends 0.75 of a 2-degree crossover, 0.70 of a 4, 0.65 of a 6, 0.61 of
  an 8 and 0.52 of a 12 (measured over 24 azimuths of the fixture). A handover
  four times as wide therefore spreads the correction over **3.2 times** as
  much picture, not four times, and the earlier 1/width arithmetic overstated
  what widening buys.

  What widening costs is measured and the trade has no free side. The plateau
  is unmoved at every width (0.3642 to 0.3657 deg), the null reads exactly zero
  at every probe on all 90 frames, and the plant reads its known displacements
  back inside 0.03 px, so the correction still corrects. What moves is the
  corridor: the step statistics above, and lens 0's floor, which stops being a
  floor - the band the correction never touched reads 0.0003 deg at 2 degrees
  and 0.0243 at 8, because the blend carries the correction that much further
  into lens 0's picture. Against all five reference views under the pooled
  answer, the along-seam median at the contour is unchanged with width at every
  view with the sites to say so (`--bin crossing`, `bins=180`); the 50/50
  contour itself moves about half a view pixel, because the depth term in the
  weights is not symmetric and the width scales it, and the sun view is refused
  outright on 5 to 6 accepted sites of 49.

- 2026-08-05 **What the band applies to a moving picture is an instrument now,
  and it says where its numbers came from** (`--bin shear`, issue #103's motion
  half). The shimmer campaign measured it out of tree, with two rendered frame
  directories and four Python scripts; this is the same method in one binary
  that renders both arms itself. A frame is decoded once and drawn twice
  through two `ScenePipeline`s, the delivered one and one held off by
  `hold_band` from its first frame, so the two pictures carry the same content
  by construction and what separates them is the applied field. Patches are
  placed against the seam's own row, walked onto out of the shipped map, because
  the seam sweeps 330 px down the picture over the reference window and a row
  pinned to the picture would be measuring that sweep.

  **Two modes, a null and a plant.** `mode=probe` reads four bands across the
  seam per frame with the step statistics under them; `mode=profile` walks a
  thin patch across it and brackets the handover; `null=1` holds both arms,
  which makes the two pictures one picture and every reading exactly zero; and
  `mode=plant` holds both arms and draws the second at a known yaw, so every
  band has a displacement it has to read back. Those last two are the only
  readings in the set whose right answer is known before the run, which is why
  there are two of them: 0.05 and 0.10 degrees of yaw are expected to displace
  -2.534 and -5.068 px, read back inside 0.029 px at every band, and double by
  1.9920 to 1.9963.

  **Against the reference view** (docs/research/reference-views.md, the shimmer
  line): 0.3663 deg applied inside lens 1 at 0.0047 deg step rms, 0.0619 deg
  step rms on the seam with a 0.42 deg single frame, and 0.0003 deg on lens 0's
  side, which is the floor. Those four are stated against a main and the
  registry line says which: the view is held by the horizon lock, so #158's
  reseeded orientation track moved the seam 23 to 45 px down this window, a
  mean of about 35, and took the first of them from 0.3641 to 0.3663 with no
  change to the instrument. The
  band's own state moved by 0.000002 across the same merge, which is the shape
  of the distinction: the band is fitted in the body's frame and these bands are
  read in the view's. It is reported beside them, at
  360 directions and through the shader's own `Reframe::reading_at` rather than
  a second lookup of ours, and it moves 0.0449 deg rms between frames.
  `research/freeze-dynamics` is an unmerged research branch; merged locally,
  its `KJERAG_FREEZE_DYNAMICS=0` takes that column to exactly 0.000000 while
  the bands still read the field the state is holding, which is the instrument
  telling a correction that stands still apart from one moving under the
  picture. What does **not** fall to zero under the freeze is the corridor's
  own step statistic, 0.07 and 0.12 deg rms with single frames at 0.38: the
  seam sweeps a standing field across the picture, so a band at a fixed
  distance from it reads a different part of that field every frame.

  **A step is between neighbouring frames and nothing else.** A band that drops
  readings has fewer steps than it has readings, and differencing across a gap
  reports the field's whole excursion over that gap as one frame's step, which
  on this view inflated the handover bands by 12 to 25 percent. The pair count,
  the breaks and the longest gap are printed beside every step statistic, and a
  band with fewer than twenty neighbouring pairs is refused rather than quoted.

  **Every CSV carries its source path and the whole command line that wrote
  it.** The instruments' tables outlive their terminals, and the older ones
  record no file identity at all, so a number copied out of one cannot be
  attributed to a video, a view or a calibration afterwards. That the
  calibration belongs in the stamp is not a guess: the same view fitted from
  the file reads 0.027 deg where the stored calibration reads 0.366, because
  what the band applies is what the calibration left it.

- 2026-08-05 **The seed is a mean of the opening minute, not a reading from
  inside it** (issue #152, docs/research/insv-format.md 8.8). The rule #45
  left behind tested the **magnitude** of one second of accelerometer and
  called that testing the reading, and a magnitude is nearly blind to the
  error that matters: a horizontal acceleration tilts the specific force by
  `e` and weighs it `1 / cos e`, so the whole 0.05 g of the full-trust window
  is spent by 18 degrees of tilt. The August 2 capture opens with a launch
  weighing 1.039 g at 21 degrees off vertical, which that rule believed
  completely, and the horizon stayed 17 to 21 degrees off level for over a
  minute; panning a circle swung it 40 degrees peak to peak. The seed is now
  the whole opening minute of accelerometer, every sample carried into the
  frame of the track's first sample by the gyroscope and averaged, with no
  test of any reading against anything. What makes a mean answerable where a
  reading is not is that it is bounded by flying: the mean specific force over
  a minute is gravity plus the aircraft's change in speed over that minute,
  0.025 g for a paramotor, against 3 degrees of gyroscope drift over the same
  minute. Measured against a backward pass over each file (the filter run from
  two minutes in back to the first sample with its rates negated, which moves
  0.02 to 0.83 degrees when its span goes from 120 to 240 seconds), the seed
  error falls on all six owner flights: 24.18 degrees to 3.03 on August 2,
  11.96 to 0.29 on May 26, 6.52 to 0.71 on May 1, 5.27 to 1.21 on April 10,
  3.89 to 1.48 on July 25, 3.06 to 2.66 on July 14, and on two of the three
  sibling files beside them (3.58 to 1.21 and 12.94 to 2.77, against 0.29 to
  0.56 on one that was already right). Through the render path the August 2
  capture opens at **3.33 degrees against 20.97**, and its first forty seconds
  average 3.75 against 18.86.

  **What selecting readings costs, as far as it is measured.** On that file,
  through the render path, the tilt grows with how hard the rule selects: 3.75
  degrees counting every sample, 6.73 weighting each second by
  `Filter::trust`, 9.32 keeping only the seconds inside the trust window,
  18.86 for the old rule's one chosen window. The backward pass does **not**
  agree about the middle of that ladder. In aggregate it prefers the
  trust-weighted mean, 1.61 degrees of worst case against 3.03; per flight the
  plain mean is the closer of the two on four of the six, by 0.04 to 0.09
  degrees, which is inside that instrument's own resolution, and the two the
  weighted mean wins it wins by 1.42 and 1.46. So what is settled is the
  render path's ordering on the reported file plus the shipped rule against
  the old one, which both instruments call better on every file. Picking the
  best window inside the minute is worse on both (a window can weigh 1 g by
  holding two things that are not gravity, which is what April 10 does).

  What it costs: a reading the running filter would refuse is no longer
  refused, only diluted to its share of the minute. And April 10 is not a
  clean win: its first frame improves 6.3 degrees while its 4 to 20 second
  stretch reads 3.5 degrees worse, and the two arms **do not converge inside
  the forty second walk**. The gap between their tilts is 1.53 degrees at 24
  seconds, first dips under a degree at 27.0, and is back at 1.16 by 38; the
  angle between the two measured verticals, which is the stricter reading, is
  4.25 degrees at 24 seconds, 3.47 at 38, and never below 3.28 anywhere in the
  walk. What is verified is that they are identical to three decimal places at
  240 seconds. Which instrument is lying over that stretch is unresolved.
  Nothing in the running correction changed; its gating under power is
  correct, and is why a bad seed survives so long.

- 2026-08-05 **The pool answers with a fit some capture actually took**
  (issue #103, docs/research/seam-two-axis.md 4). `SeamPool::answer` took the
  median of each knob separately, and the five knobs trade against each other
  inside one fit, so what shipped was a combination nobody had measured: roll
  and cx off one capture, yaw off a second, pitch and cy off a third. It
  answers with one of the pooled fits now, the one the rest of the pool agrees
  with most, scored as a sum of distances in probe steps
  (`seam::distance`, which was already the walk's yardstick). Re-read off the
  pixels of six of the owner's flights, at the three places in each file the
  app's own fit reads, that combination leaves **0.382 deg** along the seam on
  average where the fit now chosen leaves **0.273**, better on all six flights
  and on 15 of the 17 individual readings. In picture space, over every
  registry view (docs/research/reference-views.md) and both of `--bin step`'s
  windows, it is better on 15 of the 21 readings whose line fits describe their
  own points and worse on 6, the worst of those being 04-10 at 45.112 s, where
  the wide window's cold step goes 1.04 to 3.81 view px. A pool that is split
  evenly answers with the middle of what it is split between, which is the old
  rule's answer and is what a pool of two always is: no member of such a pool
  has the rest of it agreeing with it more, and choosing one would be choosing
  by which file was watched first. The pooling, the quality gate, the
  per-camera cache and the walk are untouched. Awaiting the owner's own test.

- 2026-08-05 **The seam can be measured where there is no horizon to measure**
  (`--bin crossing`, branch `research/crossing-instrument`). `step` needs a
  horizon and fits scenery at 51 to 86 px rms on the owner's 2026-05-01 views,
  so a seam-fix candidate could not be screened at the two crossings he
  actually looked at. This traces the pass's own 50/50 handover contour and,
  at fixed sites along it, registers the two **raw** lens pictures against
  each other on the seam's own axes. It reads the calibration's unbent
  geometry, so a reading carries no warm history.

  **It states its own floor, because the first version of it did not and was
  quoted to a precision it did not have.** Every run re-measures with the
  three angle knobs moved a thousandth of a degree each way and prints how far
  its medians travel, and how many sites each dithered run accepted: 0.00 view
  px over 36 and 36 sites on the null, 0.01 to 0.09 over equal counts on the
  four 2026-05-01 views. Nothing it says is worth more digits than that line,
  and a table taken at one `bins=` does not compare with one taken at another.

  The counts are on that line because a band is set two ways. Equal counts
  mean the dither moved the readings and the band measures that. Unequal
  counts mean it moved a site in or out of the accepted set, and then the band
  is a median stepping over a different population and is not a reproducible
  digit at all. The sun view under the pool member is the one recorded run
  like that, 12 sites against 13, and its band should be read as *at least*
  half a view pixel on the epipolar axis and not as a number. It is also an
  **angle** floor: the dither never moves `cx` or `cy`.

  That line exists because of the defect it now guards. The ported tracer
  kept, per azimuth bin, the root with the largest `min(blend.weights)`, on
  the argument that it was the one furthest inside both lenses. It is not:
  `Reframe::blend` normalizes the pair to sum 1, so at a 50/50 root that score
  is **exactly 0.5 at every candidate** and the twenty-odd candidates a bin
  holds were separated by the last bit of the bisection's `f32`. A
  ten-thousandth of a degree of calibration moved the reported medians by
  about a view pixel and a rerun of the same command did not reproduce its own
  table. A bin now keeps the root nearest its own centre, azimuth being what
  every comparison this instrument makes is keyed on, and three consecutive
  runs are identical to the last printed digit.

  **Null**: lens 0 against its own picture reads exactly `0.0000` on both axes
  at every accepted site, 36 of 37, 41 of 41 and 78 of 78 across the three
  views. **Plant**: a known calibration delta read back per site against what
  the perturbed map predicts, two sizes each - yaw 0.10/0.20 deg, roll
  0.10/0.20, cy 2/4 px. Median error 0.0006 to 0.0113 deg, which is 0.02 to
  0.36 source px, and it does not grow with the plant.

  **The owner's frame**, at `bins=180`, in view px at 1024 across. The two
  calibrations are his own pool of five resolved the two ways: the knobwise
  median that shipped until the entry above, and the member #154 now answers
  with.

  | crossing | calibration | epi | perp | accepted |
  | --- | --- | ---: | ---: | ---: |
  | the one he called good | knobwise median (was shipping) | 4.5 | 3.8 | 19/37 |
  | good | pool member (#154, ships now) | 6.1 | 1.2 | 19/37 |
  | the one he called bad | knobwise median | 12.0 | 3.3 | 38/41 |
  | bad | pool member (#154) | 10.1 | 1.4 | 38/42 |

  The two crossings differ 2.7x on the epipolar axis and are the same on the
  along-seam one. The change #154 landed cuts the along-seam error at both,
  3.8 to 1.2 and 3.3 to 1.4 view px, and moves the epipolar term by about a
  pixel, which is what "the bad one barely moved" looks like here. This is an
  independent read of that change: the entry above measured it as along-seam
  residual round the whole circle, and this measures it at the two crossings
  the owner actually looked at.

  **The bad crossing's excess is not parallax, and the first reading of this
  table said it was.** Epipolar is the axis a subject's distance displaces
  content along, so 12 view px was read off as content 2.4 m away. That
  inferred a distance from a magnitude without asking what the sites were
  looking at, and they were looking at a town, an estuary and a ridge line
  kilometres off, where a 33 mm baseline produces 0.015 view px. The excess is
  geometry.

  **Held at matched body azimuth over the whole 30-minute clip**, eight
  instants, every run's reference healthy (scatter 0.011 to 0.093 deg):

  | body azimuth | axis | across 30 min | swing |
  | --- | --- | --- | ---: |
  | +30 to +60 (the bad one) | epi | -14.5 to -13.5 view px | 0.99 px |
  | +30 to +60 | perp | -3.1 to -2.2 view px | 0.93 px |
  | -160 to -120 (the good one) | epi | -3.0 to -1.5 view px | 1.53 px |
  | -160 to -120 | perp | -4.8 to -3.7 view px | 1.11 px |

  The scenery behind those azimuths changes completely across those instants,
  from an industrial town and estuary to open farmland to clear sky, and the
  reading does not. The horizon lock is world-referenced, so a fixed view line
  looks at a **drifting** arc of the body-fixed seam; a per-time median taken
  without matching azimuth swings by 19 view px for that reason alone.

  **Three of those four rows are steady and one is not.** The good crossing's
  epipolar row swings 1.53 px on a signal of about 2, so at that crossing the
  epipolar term is **not established as constant** and nothing may be built on
  its being so. And the bad crossing's steadiness is one azimuth of one
  flight, not a property of the axis: the 2026-04-10 flight moves 7 source px
  on that same axis within itself at a different azimuth, and is the standing
  counterexample to reading that row round the whole circle.

  **Along the seam reproduces across flights and across the seam does not,
  and that is the split a residual map has to be designed around.** At matched
  body azimuth, over the runs whose reference stands and which have at least
  ten sites in the window, in source px: 2026-05-01 reads perp -10.7 to -8.8
  over 3 runs and 2026-04-10 reads -11.5 to -6.6 over 7, the medians 1.1 px
  apart, inside either flight's own spread. Epi over the same runs reads -11.6
  to -11.3 on 05-01 and -9.0 to -2.1 on 04-10, a 9 px gap between flights and
  a 7 px spread **within** the April one. A static per-azimuth map is enough
  for along the seam; across it needs a per-session channel and probably the
  per-frame one the band already is.

  What did **not** survive is the claim that the epipolar term shifted late in
  the May flight, from -9 to -3 source px. Every late-May run behind it is
  either reference-withheld or has three to six sites in the window; the three
  usable May runs are all early and all read -11.3 to -11.6. The shift was an
  artifact of contaminated and thin runs.

  **Perp is the honest broker, and it is a gate** (`perp-implausible`). No
  depth can reach the along-seam axis at any distance and a file's calibration
  does not change while it plays, so a reading far from its crossing's own
  along-seam value is a correlation that locked onto the wrong feature. It is
  refused whatever its agreement, and it catches what the agreement floor
  passes at 0.92.

  The reference is the crossing's own median over at least five readings, or
  one the caller declares, and it is **withheld** when the crossing's own
  scatter exceeds two fifths of the tolerance: sorted, 33 recorded runs scatter
  0.03 ... 0.35 then 0.53, 0.57, 1.03 of it, and the three past the gap are
  the ones whose middle meant nothing. It fires on the two runs that had put
  the honest core near refusal while keeping the junk.

  The tolerance is 0.40 deg, 12.6 source px, and it is a **chosen operating
  point in a populated continuum**, not a line between two populations: over
  750 accepted readings the departure from a crossing's own value runs p50
  1.85 px, p75 5.13, p90 22.13, and 11.9% sit in the 8-to-25 px stretch the
  cut is in. An earlier version of this entry called that stretch empty, which
  was false. What the data does say is that the choice barely matters: put the
  cut anywhere from 8 to 20 px and 4.0 to 5.7% of readings change side.

  **What the gate is not is validated.** The two-view control that was
  reported as validating it does not: `measure` builds its patches on
  body-fixed axes, so the view rotation cancels exactly and two views of one
  body direction agree to 0.0005 px whether the reading is any good or not.
  It validated the tracer's frame handling and nothing else. What stands
  behind the gate is the physical argument, a planted-site mechanism test, and
  one consequence it cannot have engineered: removing sites on the along-seam
  axis improves the **epipolar** axis's across-time reproducibility, a channel
  the gate never inspects (MAD 0.73 to 0.61 view px at one crossing, 0.65 to
  0.45 at the other). It cannot catch a mismatch that moved only across the
  seam, and the epipolar across-time range barely moves for that reason.

  **Glare refuses rather than guessing.** On the sun view, 78 sites trace and
  14 to 17 are accepted; the rest are `weak`, peaking at 0.11 to 0.48. The
  null run over the same 78 accepts every one at exactly zero, so the refusals
  are the two lenses genuinely disagreeing under flare rather than sites the
  sampler could not reach. Under the pooled fit that view's own scatter is too
  wide for a reference and the gate withholds one.

  **The search window is not a knob to widen.** At 2.60 deg the same frame
  reads a median magnitude of 18.7 source px with a spread of 19.8, because
  content two degrees away is allowed to win. A railed site is the honest
  answer. Patch, step and correlation floor are not like that: over their
  whole swept range the medians move under a pixel.
- 2026-08-01 **The descriptors describe the app, and the channel is named**
  (owner, from a screenshot of COSMIC Store). The `.flatpakref` carried
  plumbing keys only, so the page a Store draws before the remote is trusted
  had a placeholder icon, no summary and "Kjerag Developers" on it, and the
  repository summary carried no title, so an installed copy said its source
  was `kjerag-origin`. Both are filled in now from the metainfo by
  `scripts/pages-site.sh`, the repository calls itself `Kjerag (stable)`, and
  the ref file is named for the channel: `stable.flatpakref`. The half that
  was already right is the appstream branch, which had the icons, the
  screenshots and the developer name all along and was simply not reachable
  yet at the moment the owner was looking (docs/DISTRIBUTION.md 4.5).

  **And a dispatch republishes it without a tag**
  (`.github/workflows/site.yml`): the objects the last release published are
  fine, and rebuilding the app twice to fix a sentence is twenty minutes
  spent on nothing.

  **The summary is "360 video player"** and the keywords name cameras Kjerag
  refuses (GoPro, Osmo 360, Max) on purpose: somebody with one should find
  the app, meet the refusal that names their format, and send a clip toward
  support. The description says which cameras work today, mirroring the
  README's table, so that is an invitation rather than a bait.
- 2026-08-01 **The seam probe stops assuming the camera knows where its own
  lenses point** (issue #130, branch `fix/130-x2-fit`,
  docs/research/seam-two-axis.md 11). The owner's ONE X2 could never be
  calibrated: 3, 2 and 2 azimuths of 72 on his three captures against the 10 a
  five-knob fit needs, so that camera's pool stayed empty and zero-config
  playback delivered the factory calibration for good. Its two lens axes are
  recorded **2.835 degrees from opposed** where his X4 Air's are 0.308, and the
  seam reads 2.1 to 2.9 degrees along against a search window of 2.0.

  Two faults. The back patch was sampled as ONE rectangle grown by the whole
  search and refused entire if any corner left the picture, so a candidate near
  the truth was refused for where the widest candidate landed: 157 of 432 tries
  against 0 on the X4 Air, and widening the window made it **strictly worse**
  (at 3.0 by 6.0 degrees all 144 tries were refused and nothing reached the
  correlation). And the window was centred on the calibration itself, which on
  this camera is the thing that is wrong.

  The fix is one rule each and neither is a widening: a summed-area table of
  the holes makes the refusal a **candidate's** rather than the rectangle's,
  and a coarse wide pass acquires where the ring actually sits before the
  reading pass runs, along the seam only (parallax cannot reach that axis) and
  only where the offset is outside the window already searched. The three
  captures now fit 50, 42 and 65 azimuths and agree with each other to 0.06
  degrees of roll; round the ring at the owner's October reference moment the
  seam goes from 2.570 along and 2.830 across to **0.257 and 0.267**, which
  puts that camera in the same range as every other one in the corpus. Ten of
  the eleven two-lens captures on this box come back with the same fit to the
  last digit; the eleventh is the corpus X4, mildly starved too, whose fit
  improves on both axes when re-read off the pixels.

- 2026-08-01 **Kjerag has its own channel: a signed Flatpak repository at
  `kjerag.harding.dev`** (issue #137, owner). The same version tag that
  attaches two bundles to a GitHub Release now also builds, signs and
  publishes an OSTree repository on GitHub Pages, so installing is one click
  on `stable.flatpakref` and every release after it arrives
  through `flatpak update`. A bundle is a copy of a build; a remote is a
  subscription, and only one of those keeps a machine current.

  **This reverses 2026-07-31's "Flathub and nothing else"**, and not because
  the costs that ruling named were wrong. Issue #71 priced self-hosting
  correctly and the price is unchanged: no discovery, and key management and
  update delivery ours permanently. Flathub's contribution policy of
  2026-05-29 rules out this project's development process, so the route those
  costs bought is closed. The one thing issue #71 got wrong was the worst of
  it: it expected a remote with no AppStream data to be invisible in every
  software centre, and `flatpak build-update-repo` composes that data from the
  metainfo the app already ships. The listing exists; what it lacks is the
  screenshots (docs/DISTRIBUTION.md 5).

  **Nothing in it is ours.** Three published actions, the shape valent uses:
  `crazy-max/ghaction-import-gpg` for the key, `andyholmes/flatter` to build
  per arch and export an incrementally signed repository, and
  `JamesIves/github-pages-deploy-action` to push it to the Pages branch. The
  one step of shell writes the two descriptor files, and writes them rather
  than committing them because they carry the public half of the signing key:
  a committed copy is a file that names the wrong key the day the key rotates.

  **The branch is `stable`, in the repository and in the bundles alike.** It
  is the name a Flathub stable branch would have, so if that policy ever
  changes, moving is `flatpak install flathub dev.harding.Kjerag` and deleting
  our remote, with no reinstall and nothing lost. It also closes a smaller gap
  that was already there: a bundle install and a remote install are now the
  same app on the same branch, so `flatpak update` reaches a machine that
  started from a bundle.

  **The accepted cost is deltas.** flatter caches the repository in a GitHub
  Actions cache, and those are scoped to the ref that wrote them, so the two
  arch jobs of one tag share theirs and the next tag starts empty. Each
  release therefore publishes a repository holding that release alone: updates
  resolve and install correctly, and they download the app whole rather than a
  delta against the version already there. The app is 8 MB, which is why that
  is a note and not a problem.

- 2026-08-01 **Errors are the error** (owner ruling, on reading the funnel's
  own output). He watched the terminal say "trailer says lens frames are
  2880x2880 but the stream decodes 736x368" while the window said "That file
  could not be opened.", and ruled the raw message is what the pilot gets,
  everywhere, as a rule and not as a fix. So the alert's body is the
  failure's own message and the generic line is deleted rather than demoted.
  Nothing falls back to it, because there is no it: a failure nobody
  anticipated says what it says. Three lines of the app's own sit over an
  error and they are the whole list (`fail::refusal`): the format refusal
  (#107), the missing decoder (#69), the sandbox reach line (#118). Each
  names a fix the error does not know about, and each is one sentence away
  from being a mask, which is why the list is written down rather than left
  to judgement.

  **The stopped-video alert went the same way** (coordinator's call on the
  branch, applying the rule the branch had just written). Its body was "The
  picture could not be drawn, so playback stopped. Open the file again.",
  which knew less than the stall it stood over, so it did not qualify. The
  body is the stall's own line with the action on the end of it now: "61
  frames could not be imported over 2.0 s, last: Too many open files
  (os error 24). Open the file again." Added rather than substituted, which
  is the difference that makes a line of ours legitimate, and the only thing
  that half of it carries is the one fact the render layer cannot have, that
  this open is over. The terminal echo is unchanged, and so is everything
  about when the alert appears and what closing it does.

  **Two failures had no reason to show and now do.** A drop the document
  portal refused printed its answer to the terminal and put the generic line
  in the window; it carries the portal's own words now. A drop with nothing
  openable in it has no error at all to show, because libcosmic keeps only
  what converted (`dnd_destination.rs:119-120` calls `.ok()`, read at the
  pinned revision), so it says that instead of blaming a file nobody named.
  Nothing else in the shell was masking a reason; what the audit found
  besides was the opposite defect, failures with no surface at all (the file
  chooser's own errors, and issue #131's About links).

  **The engine's error strings are UI copy now.** They were always written at
  the failure site; what is new is that a person reads them, so the copy
  rules bind them and the tests do too. `kjerag-meta` checks every `Error`
  variant through a wildcard-free match, so a variant added later has to be
  looked at. The harness proves the words reach the screen rather than only
  the log: two files that fail for two different reasons must draw two
  different dialogs, which is a check that fails on the commit before this
  one.

- 2026-08-01 **The chooser hands back a document because the grant is read
  only, not because it is a chooser** (issue #123, measured, no fix yet). A
  file picked in `File > Open video` arrives as `/run/user/<uid>/doc/<id>/
  <name>`, a directory holding that one file, so a capture written as two
  files plays one lens. The reason is one permission wide. xdg-desktop-portal
  1.18.4 passes every picked file to the document portal with
  `AS_NEEDED_BY_APP` (`src/file-chooser.c`, `src/documents.c`), the document
  portal skips the store when `flatpak info --file-access` says the app
  already has the access being asked for, and the access being asked for
  includes write unless the backend answers `writable: false`.
  xdg-desktop-portal-cosmic never answers it (`FileChooserResult` has no such
  field) and xdg-desktop-portal-gtk answers it only when the pilot ticks "Open
  files read-only". So `xdg-videos:ro` says `read-only`, the request wanted
  write, and a document is registered.

  Both answers were produced, which is what makes the first one believable.
  `scripts/chooser-probe.py` makes the chooser's own `Documents.AddFull` call
  with each permission set: write asked returns a doc id for a file under
  `~/Videos` and for one on the file manager's network mount, read only
  returns an empty doc id for both, which is the real path. With the two
  grants temporarily made read-write through `flatpak override`, all four
  return the real path. `scripts/chooser-flatpak.sh` then drove a real dialog
  end to end: the whole portal stack runs a second time inside a headless cage
  session on its own D-Bus bus, so the backend draws its dialog there and
  nothing lands on the desktop, and the evidence is the portal's own
  `Request.Response` signal. Default: a document path, `not shown`. The same
  pick with the read-only box ticked: the real path, `sampling 2 of 2
  calibrated`, `2 lens streams from 2 files`.

  That harness needed one thing the repo did not have: `wtype` delivers
  character keys into a cage session and no named ones (Return, BackSpace, the
  arrows never arrive), so a dialog answered by a button could not be answered
  at all. `kjerag-spike --bin click` presses it with the same wlr virtual
  pointer `dragsource` uses.

  **The app cannot ask for less**, which was the owner's condition on widening
  anything. The finer-grained lever looked plausible: the portal's impl spec
  documents a `writable` option whose default is "no". It is documented under
  the **results the backend returns**, not the request the app makes, and four
  things say so, three of them measured:

  - ashpd 0.12.3's `OpenFileOptions` has no `writable` field, so our call site
    cannot express it at all;
  - xdg-desktop-portal 1.18.4 filters request options against an allow list
    (`open_file_options`) that does not contain it;
  - sending it by hand from inside the sandbox (`RAW=` in
    `scripts/chooser-flatpak.sh`, through `gdbus` in the bundle) is accepted
    with no error and **dropped in flight**. On the bus, the app's call carries
    `dict entry("writable", boolean false)` and what xdg-desktop-portal
    forwards to the backend is `array [ ]`. The dialog that opens has its
    read-only box unticked, which is the same fact in a picture;
  - and the GTK backend never reads such an option anyway. It only writes one,
    from a checkbox the pilot ticks by hand.

  So the write bit cannot be given up per request, and the owner's ruling is
  that it is not given up at all: **the grants stay read only and the manifest
  does not change**. The chooser keeps handing over a document, and what the
  app does about it is ask for what it cannot see. It takes more than one file
  now, the picked set is paired by name rather than by directory, a drop
  carries its set the same way, and a capture that arrived half says which of
  the two proper ways to open it the pilot has left: pick both files, or drag
  them in. Half is still played, because a pilot who asked for a file gets the
  file.

  Two facts are needed before any of that is said, and either alone is a lie.
  The capture is read as ONE lens, and the naming rule names a mate: an
  X4-class file names one too and carries both lenses in its container, so a
  name rule on its own would call every X4 capture half of one (measured: it
  stays silent). A readable folder with no mate in it earns the toast; a
  document directory earns the guidance instead, because it lists one file
  whatever is on the pilot's card.

  Composing the capture at the open was not the whole of it. The owner tested
  the branch bundle, picked both halves, and the log said both things at once:
  `2 lens streams from 2 files` from the player, and `this file carries one
  lens stream, so it has no seam` from the seam fit a line later. The fit
  reopened the capture from the picked path and looked **beside** it for the
  second lens, which is where a bare path's mate is and where a document's
  never is, so a capture opened the proper way could never be calibrated or
  harvested from. The fix is that a capture is its files: the reader hands
  back the ones it opened (`Reader::paths`), the scene keeps those rather than
  the one path it was named by, and the walk the fit reads takes them as given
  (`Walk::over`). The drop and command-line routes passed all along for the
  reason that hid this: two real paths sit beside each other, so looking
  beside worked by accident of the route rather than by anything the fit knew.
  The harness now drops both halves from a directory each, which is the
  chooser's shape without a chooser, and fails if a capture the app read as
  two files ever says it has one lens stream.

  The longer answer is issue #134, the owner's own: a folder-first shell in
  the cosmic-player idiom, where the app is given the directory and never has
  to ask. Not v1.

- 2026-08-01 **A transient import error costs a frame, and every failure the
  pilot meets goes through the alert** (issue #124, owner-reported at flatpak
  verification). One failed frame import set a flag on the pipeline for good:
  the picture was gone until the app was restarted, the sound played on over
  it, and the whole of what was said was one `eprintln!` on a terminal a
  launcher-started Flatpak sends nowhere. Measured on main's own binary under
  the headless harness, with `dup(2)` made to answer EMFILE on the app's main
  thread for 0.3 s: the picture froze for good, the clock ran on at 30.00 fps,
  and the null sink still carried the sound at 15745 of 32767. The flag is
  gone. A failed import costs that frame and the next redraw tries again; a
  run of failures that lasts two seconds stops the file, sound and all, and
  says so. The bound is time and not a frame count, because what the pilot is
  looking at is a picture that has been frozen for so long, and how many
  redraws went by inside that is a property of his display. Two seconds is the
  shell's own "long enough that a person has noticed", the same one the
  controls hide on. The run lives in the open capture's `Stalled` rather than
  on the pipeline, which iced keeps for the life of the window and which is
  why the old flag outlived every file.

  **The second half is the structure**, and it is the owner's ruling: error
  surfacing consistent by code design rather than by discipline. The alert's
  line is now private to `crates/app/src/fail.rs` and the only way to put one
  there is `Alert::raise(Failure)`, which prints the terminal echo with it, so
  a bare `eprintln!` at a failure site is strictly less than calling the
  funnel rather than an alternative to it. The engine has no way to report at
  all: a pass that gives up leaves a `Stall`, `Scene::pump` hands it out as a
  `Next::Stopped` arm every caller must match, and the shader widget will only
  give that arm to a message type implementing `From<Stall>`, so the shell
  cannot compile the video widget without a way to receive one. Adding the arm
  broke `kjerag-spike --bin playback`, which is the mechanism working: an
  instrument whose picture died was reporting a clean run.

  **A stop is final for that open** (owner ruling, on testing the branch with
  the fault left on permanently). The first shape re-armed as soon as the alert
  was closed and retried on its own, so a persistent fault meant an alert every
  two seconds: five of them in one sitting, from an app whose alert says to open
  the file again while quietly having another go behind it. Now the `Stalled`
  that gave up stays given up for the life of that capture. The pass stops
  importing into it and `Scene` hands out no player, so a play press cannot
  start the clock over a picture that is not coming back either. Reopening the
  file is a new `Scene`, a new `Stalled` and a fresh two seconds of patience,
  and it is what the alert asks for. Under the bound nothing changed: a hiccup
  still costs frames.

- 2026-08-01 **The volume popup closes on a press in the video, the way
  cosmic-player's dropdowns do** (issue #126, owner-reported). It was a
  hand-toggled bool that only the speaker button flipped, so the only way out
  of it was the button that opened it. cosmic-player's way out is
  `widget::mouse_area(video).on_press(Message::VideoAreaClick)`
  (`src/main.rs:1771-1773`), whose handler closes an open dropdown and
  otherwise plays or pauses (`1507-1513`); it also closes one on play/pause,
  on the scrubber and its release, and on fullscreen. All of that is ours now
  except the play/pause branch, which is the look-around grab here and was
  already resolved against it (docs/UI.md, conflict 1). Escape is unchanged,
  because cosmic-player's `on_escape` only leaves fullscreen.

  **A comment is what kept it out.** docs/UI.md said a press in the video
  "fires before a `mouse_area` around it could see it", which is not true and
  was checkable: the pass returns `ButtonPressed` uncaptured on purpose, and
  says in `crates/render/src/widget.rs` that capturing it would take the
  double click to fullscreen away. One line justifying a choice, read as
  settled by everyone after it (AGENTS.md, "comments record, they do not
  argue").

  **The harness grew a pointer**, because nothing in it could press anything:
  every check before this one is a key press. `wlrctl pointer` is the packaged
  tool for the job and cannot do it - cage advertises the seat's pointer
  capability only while a pointer device exists, and a one-shot client's
  device is gone before a client can bind `wl_pointer`. Measured 2026-08-01: a
  `wlrctl` wheel that should have zoomed the view did nothing, twenty in a row
  did nothing, and the same zoom off the keyboard reached the ball every time.
  `crates/spike/src/bin/pointer.rs` holds the device open for half a second
  before it moves anything, which is the whole of the difference, and the
  clicks land.

  **And a sound device**, because the speaker button is drawn disabled when
  the box has no output, and the session's own runtime directory has no
  PipeWire socket in it: every harness run until now said "playing silently",
  so the popup could not be opened there at all. The session now gets the
  desktop's socket, and the stream goes to the same null sink
  `scripts/quiet.sh` uses. Verified rather than assumed: the app's stream sits
  on `kjerag_quiet` while the harness runs, and `PIPEWIRE_NODE` is what puts
  it there, because pipewire-alsa is what plays what cpal writes.
- 2026-08-01 **The shipped Flatpak took no drops, and nothing could have
  caught it** (issue #118). A drop into a sandbox arrives as
  `application/vnd.portal.filetransfer`, which is a key the target exchanges
  with the document portal for paths it can open, and the app read only
  `text/uri-list`, whose paths belong to the source's filesystem and do not
  exist inside the sandbox. `dnd.rs` predicted exactly this in its own header
  and it shipped anyway, because there was no drop check anywhere: `wtype`
  presses keys and cannot drag. So the instrument came first
  (`kjerag-spike --bin dragsource`): a second Wayland client that performs a
  real drag with a virtual pointer and offers either shape. It found three
  things about the harness before it could find anything about the app, each
  of them something a desktop session has and a headless one does not.
  libcosmic creates the `wl_data_device` a drag is delivered over only while
  the seat has a **keyboard** (smithay-clipboard `src/state.rs:323-333`), and
  with none in the session neither this app nor cosmic-files ever asked for
  one. It reads the drop through the seat its last input event came from and
  gives up with "no events received on any seat" when there has been none, so
  a window nobody has clicked accepts a drop and then reads nothing from it.
  And wlroots drops on a button release only if the destination has already
  accepted, which is a round trip through another process, so a drag
  performed at machine speed is cancelled before the target has looked at it.
  With those three answered, the measurements: the dev build opens a
  `uri-list` drop and refuses a portal one; the released 0.1.1 bundle refuses
  the portal one, which is the owner's report, and reads a `uri-list` one and
  then cannot open the path it names ("No such file or directory" for a file
  that plainly exists). The fix is libcosmic's own two calls,
  `on_file_transfer` and `command::file_transfer_receive`, and it needs no
  permission at all: the portal exchange is what a sandbox is for.

  What the portal arm cannot fix is a source that never registers the files.
  cosmic-files 1.5.0 offers `text/uri-list` and nothing else (its source, and
  the shipped binary, which does not contain the string `vnd.portal`), so a
  drag out of the COSMIC file manager hands over a path on the host and that
  half of the exchange is the source's. It is also the owner's own workflow,
  so on his ruling the manifest grants `--filesystem=xdg-videos:ro`: the
  smallest grant that covers every way a bare path arrives, which is that
  drag, `kjerag ~/Videos/flight.insv` on a terminal, and a double click.
  Read-only because the player never writes to footage, and the videos folder
  rather than home because footage kept elsewhere is one `File > Open
  video...` away through the portal at no permission at all. The day
  cosmic-files registers its drags the grant can be reconsidered.

  What is left outside the grant used to get the words a corrupt file gets,
  and now gets its own line in issue #117's alert, only inside a Flatpak:
  "Kjerag cannot reach that file from inside its sandbox. Open it with File >
  Open video." The path decides it rather than the error, because libav's
  answer for a path with no mount behind it is "No such file or directory",
  which is a sentence about a file that is not there.

  Measured on the bundle built from the branch, with the grant, on real
  footage under `~/Videos`: a cosmic-files-shaped `uri-list` drop opens, a
  portal drop opens, `flatpak run <app> <path>` opens, and a double click's
  own `--file-forwarding` shape opens; the same `uri-list` drop of a path
  outside the folder is refused with the line above. And for the two-file
  captures of issue #123, every one of those four hands over the host path
  and pairs both lenses (`2 of 2 calibrated`, `2 lens streams from 2 files`),
  because both the document portal's `RetrieveFiles` and flatpak's file
  forwarding skip the document store for a file the app can already read.
  The file chooser is one permission short of skipping it too, which the
  2026-08-01 entry above measures: a file picked there arrives as
  `/run/user/1000/doc/<id>/<name>`, its mate is not in that directory, and
  the capture still plays one lens. Issue #123 stands for that path alone.

  And the grant was still not enough, which the owner found by testing rather
  than by reading: his footage library is on a NAS, mounted by the file
  manager, so the paths his drags carry are
  `/run/user/1000/gvfs/smb-share:server=...,share=.../...`, which a sandbox
  cannot see either. `--filesystem=xdg-run/gvfs:ro` is the standard grant and
  the read-only half is measured: the mount lists inside the sandbox and a
  file on it reads. Measured through a drop, on his own share: the file
  opens, and because the mate sits beside it there too, a two-file capture on
  the NAS plays `2 of 2 calibrated`, `2 lens streams from 2 files` over SMB.
  The instrument had to learn the shape as well: a mount's directory is
  called `smb-share:server=host,share=name`, and an argument parser that
  treats `=` as an option refuses to drag it.

  What is left outside every grant refuses once per drop rather than once per
  file, which is what a multiple selection sends: the app takes the first file
  and says one thing about it (measured: two files in, one refusal out).

- 2026-08-01 **A pane with no frame draws the backdrop, not a picture of its
  own** (owner-reported). The pass carried an animated test pattern from its
  first bring-up, a sine of the distance from the middle of the view on a wall
  clock, and drew it wherever the uniform block said there was no frame: every
  open from a window that was already up showed it until the first decoded
  frame landed, and any state where frames never arrive showed it for good. It
  is gone, and with it the clock that animated it, the two uniform fields that
  carried that clock and the flag beside it, and the redraw the widget asked
  for on every compositor refresh while nothing was open. What is left is the
  mechanism the room around the ball already uses (issue #100): no frame is no
  lens with a ray, which is transparent everywhere, which is the shell's
  backdrop - libcosmic's pane in a window, black in fullscreen. Opening a file
  is now a pane that is already there and a picture that arrives on it.

  **The harness had never looked there**, and the check that does is the
  interesting half. The command line cannot reach the state at all:
  `kjerag file.insv` opens the file while the window is still being mapped, so
  the decode thread has the whole mapping to work in and the first frame is
  there before the first pixel is drawn (measured over 80 captures from
  launch, none of them in the gap). A paste over `Ctrl+V` opens the file from
  a window that is already up, which is the pilot's own path and the only one
  a keyboard has - `Ctrl+O` opens a portal dialog cage has no portal behind -
  and naming a time 90% into the file widens the gap from one capture to
  twenty, because the first frame then comes off a keyframe walk rather than
  off the head of the stream. The pattern is told from a picture by its own
  symmetry: it scales into red by the horizontal place and into green by the
  vertical one over a blue that is neither, so two patches at mirrored places
  read the same green and the same blue to the byte and different reds, which
  no frame of video does. Against the build this branch started from the check
  fails on 41 98 211 against 170 98 211; with the pattern gone it passes on
  the backdrop, five captures of it before the frame.

- 2026-08-01 **Another camera's 360 format is refused by name, and nothing
  in the app grades the camera it does take** (issue #107, alongside #88).
  Two halves of the same honesty. The refusal: a GoPro `.360` and a DJI
  `.osv` are ordinary MP4s, so before this they opened, failed the trailer
  read, and got the line a corrupt file gets. `kjerag_meta::Format::sniff`
  now names the maker off the container before the decoder is asked for
  anything, in this order: the Insta360 trailer magic, GoPro's own `udta`
  boxes (`FIRM` `GPMF` `CAME` `MUID`), DJI's `djmd`/`dbgi` tracks, and
  Google's spherical metadata for a stitched MP4. Only where the bytes say
  nothing is the name asked, so a `.360` off a firmware that writes the
  container differently is still named, and no `.insv` can be refused for
  what it is called. The search walks the box tree instead of the bytes,
  because a raw grep for `st3d` over the sample corpus hits two genuine
  Insta360 captures inside their compressed video, and refusing a pilot's
  own footage is the one failure this must not have. Verified on 18 real
  files from seven cameras; the spherical arm alone has hand-built fixtures
  only, because no such file exists here and ffmpeg 7.1 cannot write one.
  **What says it is an alert, not the welcome view** (owner, 2026-08-01, on
  the first cut: "a normal alert, not some weird bespoke string on the splash
  page"). Every cannot-open line moved with it, the missing-decoder one of
  issue #69 included: `Application::dialog` returns the stock
  `widget::dialog` shaped the way cosmic-files shapes its failed-operation
  dialog (`src/app.rs:5665-5678`), one title, the reason as the body, the
  `dialog-error` icon, and one button, dismissed by that button or by Escape.
  A failed open now takes nothing away either: whatever was playing carries
  on playing behind the alert, where before the shell dropped the open file
  to show a line on the welcome view.
  The other half is what was **not** built: an `.insv` from a camera outside
  the verified set opens and plays with nothing said about it. The support
  tiers are the README's and the listing's, where somebody deciding whether
  to install reads them; in the app a tier is a label the pilot cannot act
  on, it would fire on files that play perfectly, and it would go stale the
  day #88 verifies a model. What the app says about a camera stays what it
  already said: the `lens:` line naming model and firmware on the terminal.
- 2026-08-01 **aarch64 is a second runner, not a second recipe** (owner
  directive). The gates that compile run on `ubuntu-24.04` and
  `ubuntu-24.04-arm`, natively, and a version tag publishes
  `kjerag-<version>-aarch64.flatpak` beside the x86_64 bundle, each built on a
  runner of its own arch. What decided the shape was the ffmpeg PPA: the
  workspace pins 7.1, Ubuntu 24.04 ships 6.1, and a PPA is often amd64 only,
  so the question was put to the arm runner before anything was written.
  `ppa:ubuntuhandbook1/ffmpeg7` publishes an arm64 index, `libavcodec-dev
  7:7.1.1-0build1~ubuntu2404` installs out of it, and `pkg-config` then reports
  61.19.101. So the provisioning block is untouched and both arches read the
  one copy of it. That left the alternative unbuilt, which was moving the
  compile jobs into the Flathub SDK container the bundle already builds in: it
  does carry ffmpeg 7.1 dev for both arches (measured), but it has neither
  rustup nor clang, and taking it would have traded two cached toolchain jobs
  for a container pull to solve a problem the PPA does not have. The release
  half was checked the same way rather than assumed: the
  `flatpak-github-actions:freedesktop-25.08` tag starts on the arm runner and
  reports `aarch64`, and `org.freedesktop.Platform`, `Sdk`, `rust-stable` and
  `llvm21` all resolve at 25.08 for `--arch=aarch64` on Flathub. One trap is
  recorded in the workflow, because it fails as a cross build rather than as a
  missing thing: the flatpak-builder action's `arch` input defaults to the
  literal `x86_64`, so the arm runner has to be told. Measured on the throwaway
  tag that proved it (`0.1.1-armtest1`, published and then deleted): a 7.2 MB
  aarch64 bundle carrying `app/dev.harding.Kjerag/aarch64/master` and
  `runtime=org.freedesktop.Platform/aarch64/25.08`, beside the 8.1 MB x86_64
  one. The two bundle jobs run side by side and the arm one finished first,
  7m22 against 9m10, so the second bundle costs no wall time of its own; the
  tag run as a whole went from 11m31 to 14m43, and that difference is one cold
  arm cargo cache in the gates. Warm, the two legs land together: on the second
  push to the branch the arm gate took 1m33 and the x86 one 1m52. What this is
  not, and the README and
  docs/RELEASING.md say so where a person reads them, is a verified build: a
  runner has no GPU, decode is VA-API against `/dev/dri/renderD128`, and most
  arm devices decode through V4L2, which this app does not use. The aarch64
  bundle is compiled and unit tested and has been run by nobody.
- 2026-08-01 **A version tag is the release, and nothing about it is ours**
  (issue #106). `cargo release patch --execute` on main bumps the version,
  stamps a dated entry into the metainfo changelog, tags the plain version
  with no `v` in front of it, and pushes; the tag makes a workflow build the
  Flatpak and publish `kjerag-<version>-x86_64.flatpak` and its `.sha256` as a
  GitHub Release. The owner's rule for the whole path was that Kjerag is the
  simplest project Flathub will ever see and its releases should be too, so
  every piece is either an upstream tool used as its documentation intends or
  it is gone. cargo-release owns the version, the changelog entry, the commit
  and the tag; Flatpak's own GitHub action owns the build, in the image
  Flathub builds with, which took a hand-written `apt-get` block and its two
  archaeology comments (`eu-strip`, gdk-pixbuf's SVG loader) out of the tree
  the day it arrived; softprops/action-gh-release and GitHub's own generated
  notes own the release. What that left of our own is four lines: a tag
  pattern, a bundle name, a `sha256sum`, and `release.toml`. An earlier
  version of this work had a `scripts/version-check.sh` holding three copies
  of the version in agreement, and it was deleted rather than kept: with one
  tool writing all three from one number, the thing it verified cannot
  disagree, and cargo-release's own `exactly = 1` on the changelog stamp is
  the guard that the metainfo was really written. The release workflow calls
  `ci.yml` rather than restating it, so a tag runs the gates a pull request
  runs, and `scripts/uitest.sh` is in neither: a runner has no
  `/dev/dri/renderD128`, so it is a cargo-release hook, which means the dry
  run that precedes every release is also the harness run. Measured on the
  pipeline's own test tags: about ten minutes end to end, an 8.1 MB bundle,
  and `flatpak install --user` of it followed by `flatpak run
  dev.harding.Kjerag --version` printing `kjerag 0.1.0`. The channel question
  is not reopened by any of this: Flathub is still where a published app goes
  (docs/DISTRIBUTION.md 4.1), and a single-file bundle is a file rather than a
  channel.
- 2026-08-01 **The room around the ball belongs to the window** (issue #100).
  The pass wrote a flat 0.10 grey wherever no lens has a ray; it now writes
  transparent black through a premultiplied blend and paints nothing there at
  all, so what fills the room is the one layer behind the video
  (`app::backdrop`). In a window that layer is empty, which leaves libcosmic's
  own pane showing: darkened translucency over the compositor's blur with a
  frosted theme, the same colour opaque without one, and no fallback of our
  own for the no-blur case, because it is the same line of libcosmic either
  way. In fullscreen the layer is black, on the owner's call: there is no
  desktop behind a fullscreen window to frost. A still is black too, for a
  different reason: the capture pass clears black and the transparent room
  flattens onto that, so a JPEG carries no alpha and needs no channel for one.

  **Nothing with a picture in it moved.** Measured over `reframe` renders from
  both builds, three fields of view (40, 90, 150) in both target formats:
  byte-identical PNGs. At the ball every differing pixel is the room and every
  one of them the same substitution, 25 25 25 to 0 0 0 on a linear target and
  26 26 26 to 0 0 0 on an sRGB one (193,084 of 262,144 pixels, 73.7%). The
  blend's cost is under this box's noise: the median ms/redraw over six
  interleaved `ball` runs a side moves between -0.03 and +0.15 ms at 2560x1440
  on a Radeon 760M, against a spread of 0.4 ms between runs of one build and a
  33 ms frame.

- 2026-08-01 **The project is Kjerag** (issue #75), in one mechanical sweep:
  five crates, the binary, `App::APP_ID`, the four `resources/` and
  `flatpak/` file names, the cosmic-config identifiers, the report prefixes,
  the harness and every doc. The previous spelling is gone rather than
  aliased (owner: "doesn't exist in any files or filenames, folders,
  anything"), so no compatibility name is read anywhere and
  `KJERAG_TEST_MEDIA`, `KJERAG_BIN`, `KJERAG_TEST_INSV` and `KJERAG_FFMPEG7`
  are the only names the scripts answer to. `scripts/name-check.sh` is the
  lock, and CI runs it: a tracked path or file carrying the old name fails
  the build. The transcripts in docs/DISTRIBUTION.md had their identifiers
  rewritten with the rest, and that document's preamble says so, because a
  transcript nobody re-ran is evidence for what it measured rather than for
  what it prints.

  **cosmic-config moves with the ID and nothing migrates.** The stores live
  under `~/.config/cosmic/<id>/` and `~/.local/state/cosmic/<id>/` and
  cosmic-config has no name-migration path, so settings, recent files and the
  seam pool are all discarded. Pre-release, and sanctioned in #75. The pool is
  a cache by construction (see the entry below), so watch-to-calibrate refills
  it silently over the next few files played; the settings are four values and
  the recents are ten paths.

  The icons kept their bytes. The name mismatch that issue #93 worked around
  is gone and an installed build resolves `dev.harding.Kjerag` through the
  icon theme, but a `cargo run` out of this tree installs no theme, and
  libcosmic answers a miss with an empty SVG rather than a placeholder. All
  three cases were measured with the harness before deciding
  (`crates/app/src/app.rs`, `APP_ICON`).

- 2026-08-01 **The calibration menu action is deleted, and the store with it.**
  Zero-config playback (AGENTS.md) leaves no room for it, and the reason is
  stronger than the doctrine: the action fitted whichever file was open, so on
  this box it stored the May 1 flight's fit and then the April 10 flight's,
  reported "Seam calibrated for this camera" both times, and never once fitted
  the static capture it existed for. A fit taken through a flight's seam
  absorbs that flight's parallax (6.8), so both answers were wrong and nothing
  on screen could show it. That is the whole explanation of the owner's "I have
  never once seen a before and after that improved". The single-entry
  `seam_calibration` is replaced by a per-camera `seam_pool` of quality-gated
  fits, medianed, filled by watching; the old key is discarded rather than
  migrated, because its contents are exactly the contamination the pool exists
  to average out. The correction is no longer landed once either: a
  `Correction` walks from what is drawn to what is asked for at 0.25 deg/s, so
  a fit that lands mid-playback is never a jump.

- 2026-07-31 **Distribution settled** (docs/DISTRIBUTION.md). A `.insv` gets a
  MIME type of its own, `video/x-insta360-insv`, glob only: the bytes that
  identify one are the last 32 and shared-mime-info offsets are
  start-relative, so the good magic rule is unreachable and the near miss
  makes `gio` segfault. The desktop entry, the metainfo, the MIME package and
  the icon theme tree install out of one `resources/` root, and the Flatpak
  builds offline from the committed `flatpak/cargo-sources.json`. **The
  channel is Flathub and nothing else** (owner): a self-hosted repository was
  worked out in full and declined (issue #71), and Flathub is reached under
  AGENTS.md's one scoped exception, owner-coordinated, previewed here before
  any outward step. The licence spelling is `AGPL-3.0-only`. The app ID is
  `dev.harding.Kjerag` (issue #66) and the whole tree carries it since issue
  #75: the ID is the cosmic-config path, the icon name, the D-Bus name, the
  Wayland `app_id` and four file names at once, so the sweep moved all of
  them in one commit rather than leave the entry naming one ID and the binary
  registering another. What is left before a submission is the owner's:
  screenshots, the X11 question, and whether `xdg-config/cosmic:ro` costs
  persisted settings.

- 2026-07-31 **`flatpak/cargo-sources.json` was stale on `main`**, and the
  rule that should have prevented it could not: it is written per commit and
  the failure is per merge. Issue #90 regenerated the sources on a branch cut
  before issue #95 bumped the ffmpeg pin; both merged clean because they touch
  different files, and `main` then held a lock file wanting ffmpeg-next 7.1
  and a source list offering 6.1.1. Found by building the Flatpak rather than
  by reading it, which is the only way it can be found. Fixed by
  regenerating, and `scripts/cargo-sources.sh --check` now compares the two
  package sets with no network and no generator, in CI and by hand. The check
  was shown able to fail before it was believed: against the stale file it
  names all four crates, ffmpeg-next 7.1.0 among them.

- 2026-08-01 **The seam's photometry: the measurement layer survives, the
  application does not** (issue #103, stage 8 final,
  docs/research/seam-blending.md 16). The owner tested the applied correction
  twice and rejected it twice, the second time on dark STREAKS across his soil:
  *"I don't think this approach is valid."* PR #138 ends as measurement
  infrastructure - the shipped crates are main's byte for byte, and the whole
  branch is one instrument file and the record.

  **The process finding is the durable part.** Every acceptance statistic this
  campaign has ever used STRADDLES THE SEAM. Stage 8 found the statistic was in
  the wrong units and fixed that; the replacement straddled the seam too. So
  nothing ever measured what an applied correction does to the picture it is
  painted OVER, and two builds were rejected on an artifact class the whole
  acceptance layer was structurally unable to see - a per-direction field over
  wide support painting each direction's own noise along its whole sweep, which
  is stage 5's scalloping on the photometric axis. **The rule: a field applied
  over an area is accepted on the area, not on the boundary.**

  **What ships is three instruments and their plants.** A perceptual lag ladder
  in Weber contrast at 1 to 128 pixels of the delivered view (a planted step
  reads back exactly at every lag; the same step spread over 64 pixels reads a
  64th of it locally). An excess-over-the-same-content statistic that names a
  line's author. And the field-interior coherence metric that was missing, which
  reads main at 0.03 percent, the rejected build at 1.01, and its own nulls at
  0.000. It is registered as the anti-acceptance for photometric work.

  **And one finding no rejection touches:** at every reference view the owner
  has given, the residual line's excess over what the same content reads a few
  degrees away is at or under the JND at the one and two pixel lags, while at
  the azimuth his own gear crosses the seam it is +5.87 percent and the entire
  photometric stage moved it from 5.94 to 5.94. **What still reads as a line is
  geometric**, which makes the local-warp-versus-pose verdict the campaign's
  next question.

- 2026-08-01 **Symmetric wide matching, and the measurement that names the
  line's author** (issue #103, stage 8 second form,
  docs/research/seam-blending.md 14-15). The owner viewed stage 8's first form
  twice. *"I dont think its aggressive enough with blending"*, and then *"to the
  eye, it still effectively looks like a line"*. Both are answered, and only one
  of them by building something.

  **The correction is split between both hemispheres and carried to the pole.**
  The first form eased it to nothing seven degrees off the seam, to keep a
  player from moving a hemisphere's black level; the symmetric split dissolves
  that objection the way stage 3's gain split did, since each hemisphere moves
  HALF the mismatch towards the other. At the owner's own wide view the
  difference between eight degrees either side of the seam goes **7.55 codes to
  2.93**, and the halo that was the priced risk of going wide did not appear:
  the long-lag Weber contrast goes 44.7 percent to 24.4 at 64 pixels. A count of
  pixels of the delivered view came out with the old shape - it decided nothing
  at any field of view the player offers, and where it bit it made the handover
  narrower than the content would bear.

  **The line that is left is GEOMETRIC, and that is measured.** The same
  statistic straddling a line a few degrees off the seam, in the same window and
  the same content, separates a photometric step (a difference in level, present
  on content with no gradient at all) from a misregistration (a difference in
  position, present only where there is content to draw twice). At every
  reference view the owner has given, the seam's excess over what that content
  reads anywhere is **at or under the 1 percent JND at the one and two pixel
  lags**; at the azimuth his own gear crosses the seam it is **+5.87 percent and
  the entire photometric stage moves it from 5.94 to 5.94**. So the photometric
  half of "no line" is done to the bar and the geometric half is the
  local-warp-versus-pose decision already pending. No local warp is built here.

- 2026-08-01 **The seam's blend, in the space an eye reads it in** (issue #103,
  stage 8, docs/research/seam-blending.md 9-13). The owner viewed stage 7's
  branch at a wide May reference view and said *"we need to do a lot better
  with blending"*, and the verdict written for him found three reasons no
  amount of tuning could: the correction was multiplicative where the
  difference is additive, the estimator weighted brightness squared so the
  content the artifact shows on carried one percent of the weight, and the
  loss was in codes while the eye reads ratios. **All three are one mistake -
  the metric - and stage 8 changes the metric first.**

  Acceptance is now the steepest local **Weber contrast across the seam**, at
  lags of 1 to 32 pixels of the delivered view, with controls that read a
  planted step back exactly and separate a step from a ramp of the same size
  to three decimals. At the owner's own wide view it goes **42.3 percent to
  16.6**, and the step the two sides differ by goes **+32.5 percent to -2.1**.
  On flat content it is at the one percent just-noticeable difference at the
  one and two pixel lags, which is where an edge lives; what is left past four
  pixels is a ramp the correction itself makes, because a player may not move
  a hemisphere's black level and the correction has to end somewhere.

  Five moves, each from a measurement: **ratio space** (the codes-space
  estimator deleted, not switched off); **a gain and an offset fitted
  jointly**, because sequentially the gain comes out at 1.15 and ruins the sky;
  **the offset per direction**, because a constant plus one cycle plus two -
  the basis stage 7 fitted through - leaves 4.2 to 5.5 codes rms against a
  frame noise of 0.8 to 1.0, so what varies round a seam is not a shape;
  **one width** in pixels of the delivered view, gated per direction by what a
  wider handover would cost in ghost, absorbing stage 4's crossover and stage
  7's colour region; and **a handover profile with no corner** plus dither
  inside it, which is the residual physics a photometry cannot reach. One hole
  was most of the improvement: stage 7 read a photometry only where the
  correlation had established what it was looking at, and that left 50 of 128
  directions blank in a continuous arc - the arc the complaint was in.

  **The debt went down.** `band::Tint` is deleted whole - its fit, its shader
  twin, its compute entry point, its pipeline and its readback - and three
  notions of "near the seam" became one function. Three new constants, one new
  derived one, one whole mechanism gone. The photometry costs +0.28 ms per
  redraw, less than stage 7's +0.38 for more work.

- 2026-08-01 **The seam hands over a colour, and one number could never have
  reached it** (issue #103, stage 7,
  docs/research/insv-format.md 6.11). The owner's verdict on the merged
  geometry work: *"the worst part now is the change in colour at the seam,
  especially on the sky or when the sun is in one of the lenses."* Stage 3
  corrects one gain for all three channels, so the SPREAD between the channels
  survives it exactly however well it is fitted, and that spread is 3.4 to 5.6
  codes on the owner's own six captures and 1.5 to 31 across four camera
  models - over the one code an 8-bit picture can carry on every one. On a
  corpus X4 it is **10.29 codes with the sun in one lens and 0.47 with the sun
  in neither**, which is the owner's own sentence measured in somebody else's
  footage. So `Tone` carries three gains in the same sixteen bytes.

  Two findings changed the design. **The pass had never read the content the
  complaint is about**: the band refuses a patch under its contrast gate, so
  20 to 64 percent of a real seam - the sky - carried no reading at all, and
  the gain was measured on the ground and applied to the sky. A flat patch has
  no geometry and the best colour on the ring, because what a displaced window
  costs a photometry is the content's own gradient across it: measured at 0.33
  to 0.76 codes rms at the residual the pass leaves, against differences of 2
  to 33. And **the difference is not one number round the ring**: a
  per-channel constant leaves 1.0 to 6.2 codes rms on the owner's captures
  against a frame-noise floor of 0.4 to 3.2, and the same five-term basis
  stage 5 fits the geometry through takes another third to a half off it. The
  null says that shape is not a window that moved: 0.15 to 0.25 codes of
  one-cycle amplitude against the measurement's 1.4 to 4.1.

  The cycles are applied as a FIELD near the seam, whole across every
  crossover the band can open and faded out by `Reframe::overlap` - the one
  angle in the problem that is a property of the cameras. Carrying it over the
  whole hemisphere the way stage 5 carries its rotation was measured and
  refused: lens shading would read the same on every scene of one file and
  this moves by 3 to 27 codes between five places in one capture, so it is
  glare, and glare has no business being painted over half a sphere. The
  glare OFFSET stage 3 priced is answered by measurement rather than by a
  build: on the content that can tell a gain from an offset the two are
  indistinguishable and the pair together buys under a tenth of a code, so no
  black level is moved.

  Eight narrow views round the seam at the owner's own reference instant:
  hue step 8.48 codes mean before, 5.22 after, seven of eight improved. It is
  not under one code and does not claim to be; what is left is the part of the
  ring that is not a constant, one cycle or two.

- 2026-08-01 **The band's cost was the fetch, not the solve** (issue #103,
  stage 2). The obvious optimisation was to score each candidate shift on a
  quarter of the patch's samples, which is what `seam::best_shift` does. It
  made the pass **slower**: 9.1 ms a redraw against 8.4. What the pass spends
  its time on is filling the two correlation grids, 3733 taps of a tiled
  3840x3840 decoder surface per direction per frame on an iGPU that is
  decoding at the same time. Shrinking the grids instead - a 0.10 degree step
  against 0.08, a search that stops at 2.6 degrees rather than 4.0 because the
  fold clamp cannot carry more than 1.8, and half the ring read per frame -
  took the whole per-frame measurement from +3.2 ms to **+0.3**. The first two
  are resolution the parabola and the seconds of averaging give back; the
  third is free because the filter is paced in seconds of media time, so a
  direction read at 15 Hz and one read at 30 settle in the same wall time.

- 2026-08-01 **The two instruments disagreed because the band was wrong twice,
  and the ruler was wrong once** (issue #103, stage 6,
  docs/research/seam-two-axis.md sections 9 and 10). Stage 5 was capped by the
  band's along-seam channel reading +0.06 to +0.20 deg where `--bin seam
  mode=residual` read -0.41 to -0.46 on the same directions of the same file,
  while the two agreed to 0.01 deg on the far side of the ring. Three faults,
  each needed for one half of that. **(a)** `Ring::perp` was built `centre x
  epi`, the negative of `seam::ring`'s own axis: the pass drew correctly for it
  because it measures and applies through the same axis, but every number it
  printed was the probe's with the sign turned over. **(b)** `reset` was a
  property of a FRAME and the state it throws away is per DIRECTION, and a
  frame reads every `SLICES`-th direction - so a seek reset half the ring and
  the other half crept toward the new content at `TAU_FAR`, reaching 0.56 of
  the truth after 120 frames. **(c)** `--bin band` had no way to be handed a
  stored fit, so the two instruments were read under different calibrations,
  which differ by 0.04 deg on the far side of the ring and 0.32 on the arc
  carrying the step. After (a) and (b) the band reads **0.99 of the probe on
  both parities**, and `--bin band` takes `seam=` so (c) cannot recur.

  **The ruler was wrong too.** `--bin step` extrapolates a straight line to the
  seam from four degrees out, on the premise that a horizon is a great circle.
  What it traces is a ridge, and the same frame with the band held off reads
  10.4, 20.9, 30.5, 32.8 and 37.8 view px at `guard` 1.2, 1.6, 2.0, 2.5 and
  3.5. Every DIFFERENCE between two builds survives that - the correction
  rotates one hemisphere and moves its whole trace by a constant, 23.2 px in
  all three windows - so the campaign's deltas stand and its absolute numbers
  carry the hill. It prints a `close:` column now, over the two degrees just
  outside the frame's own crossover, with each fit's rms beside it.

  At the owner's reference view, close-in column: **+17.3 view px on `main`,
  -5.2 cold and +8.1 warm on stage 5, -6.0 cold and -5.8 warm here**. The
  campaign's own wide column reads 32.8/30.2 on main, 10.1/23.3 on stage 5 and
  9.4/15.4 here. What stage 6 buys is that **cold and warm now agree**: 0.2 px
  apart where stage 5 was 13.3, because the reset reaches every direction.

  **Cost, priced with the box divided out** (`--bin band mode=cost`, the slope
  over sixteen extra dispatches, minimum of several runs at 1440x1440):
  **0.58-0.71 ms per redraw in steady state, 3.5 to 4.3 percent of the 16.6 ms
  a 60 fps frame has**, and 1.3-2.9 ms once on the frame a seek lands on -
  which now sweeps the whole ring where stage 5's swept half of it. Stage 5's
  form measures 0.89-0.93 ms; its reported +2.55 ms was `--bin playback`'s
  whole-redraw delta on a box building four worktrees, and six alternating runs
  of two builds under a load average of 21 came back 5.1 to 20.3 ms with the
  builds interleaved. The whole saving is the flat-sky gate, which used to be
  reached only after a candidate's entire double loop had run: a direction of
  blank sky, which on a real seam is most of the ring, paid for the whole table
  to be told there was nothing in it. A narrow re-acquisition search was built
  on top of that and **measured out** - 0.631 against 0.632 ms on a sky seam
  and 0.714 against 0.700 on a seam full of near ground, inside the run-to-run
  spread on both - so it is not in the branch. The cadence the cost ruling
  asked for has been in the pass since stage 2: `SLICES` reads half the ring
  per frame.

  **The owner's second reference view is a different defect** (issue #130). His
  October capture is a ONE X2, and that camera refuses its own fit on every
  file: `only 2 of 72 azimuths on the seam had content both lenses could be
  matched on`, 3 / 2 / 2 across three captures against the 10 a five-knob fit
  needs, so it can never build a pool entry and plays on the factory
  calibration forever. The reason is a trap: the residual there reads 1.1-1.6
  deg along the seam and 0.9-2.8 across, which is larger than the probe's
  window (and those four readings are themselves clipped by it: with the window
  moved onto the ring it is 2.1-2.9 deg along - see 2026-08-01 above),
  and widening the window makes it strictly worse because the back patch is
  sampled as ONE rectangle grown by the whole search - at `along=3.0
  across=6.0` every single try is refused for leaving the overlap. At that view
  one degree epipolar moves the horizon 12 rows and one degree along the seam
  moves it 3, the content at the seam is half a metre away, and the step is
  5 to 6 DEGREES. Measured on `main`, on stage 5 and here it moves by about a
  pixel in each direction, which is the right outcome for a fix aimed at
  another axis.

- 2026-08-01 **The seam has two axes and the campaign had only ever measured
  one** (issue #103, stage 5, docs/research/seam-two-axis.md). The owner
  rejected the horizon on `main` after stages 1 to 4 all merged on good
  numbers, and the reason is that every acceptance number those stages carry
  is a statistic of the **epipolar** axis, which is the axis a horizon cannot
  show. At his fov-20 reference view one degree epipolar moves that horizon
  **0.6 rows** and one degree **along the seam** moves it **53**, and the whole
  band campaign moves that view by 2.6 view px of 32.8. `Cell::off_epi` had
  measured the other axis since stage 2 and never applied it, and its search
  saturated: three offsets at 0.30 degrees, with 44 percent of measured
  directions cold and 67 percent warm sitting ON the limit against a corpus
  range of 0.17 to 0.67.

  Stage 5 measures it properly and puts it in the picture. The search is now
  nineteen offsets at 0.90 degrees on the same 0.10 grid the epipolar axis
  uses, with the same parabola between whole steps; nothing rails on any
  camera tried. The channel has its own confidence, refused on its own,
  because a reading pinned on the along-seam limit is a camera outside
  anything measured and refusing the epipolar channel for it would throw
  stage 2 away on that footage. One time constant, `TAU_FAR_S`, wherever the
  direction looks: parallax cannot reach this axis at any distance, so what it
  holds is the camera, and the camera does not move.

  **Two things it had to learn by being built first.** Applied per direction
  it scallops - far fewer than 128 directions correlate on a real frame, and a
  field with holes in it applied over a hemisphere warps a horizon instead of
  moving it (18.5 view px of correction at one end of a four-degree fit and
  4.7 at the other). So the ring is fitted to the shape the phenomenon has:
  constant, one cycle and two cycles, which are relative roll, principal point
  and focal aspect, the decomposition `--bin seam` has printed since #48. Five
  numbers, a ridge of one direction's worth of evidence, no time constant of
  its own. And applied only across the band it does nothing - 0.03 view px of
  32.8 - because a pose error is wrong everywhere and not only at the
  handover, so it goes where the calibration it belongs to goes: to lens 1,
  over its whole picture, scaled by the ray flattened into the seam plane,
  which is exactly the `cos(elevation)` a relative roll produces.

  The owner's reference view goes **32.8 to 10.1 view px cold** and **30.2 to
  23.2 warm**, which is short of the low single digits the ruling asked for
  and is capped by the measurement rather than by the application: at the
  azimuths carrying his step the band reads 0.06 to 0.20 degrees where
  `--bin seam mode=residual` reads 0.41 to 0.46 on the same directions of the
  same file, while the two agree to 0.01 degrees on the opposite side of the
  ring. That disagreement is the next thing to diagnose and it is what stands
  between this and a pixel-perfect horizon. Cost is **+2.55 ms per redraw**
  under live decode, which is out of the campaign's class and is the search's
  and not the application's: the same width at a 0.30 grid is +0.86 ms and
  reads 15.9 cold, and a two-pass coarse-to-fine search measured worse on both
  counts on this GPU because sixty-four workgroups' worth of extra barriers
  cost more than the candidates they save.

- 2026-08-01 **The near end of the seam correction is now the search window,
  not the fold** (issue #103, stage 4). The crossover width and the shear
  clamp turned out to be one inequality read two ways, so the band now opens
  to carry what was measured instead of the measurement being cut to fit the
  band. What that exposes is where the remaining bound sits: the clamp is
  inert for every disparity the pass can report, and what stops the
  correction at 0.73 m is `NEAR_DEG`, the search window. Measured by widening
  it to 4.0 degrees on a branch-local build: the direction-frames the band
  opens for go from 175 to **513** on the X3 sample (2.28 to 6.68 percent),
  the worst doubled edge recovered goes from 11.3 to **31.9 view px** on
  content at 0.42 m, and the pass costs **+0.20 ms** rather than +0.06. Not
  taken here, because it changes the measurement rather than the crossover
  and it halves the margin the widest band has inside the lenses' overlap
  (1.04 degrees a side on the X4 Air, against 3.22 today). It is priced and
  it is one constant: the ceiling follows `NEAR_DEG` on its own, so there is
  no second number to keep in step.

- 2026-08-01 **The seam's exposure difference is a gain, and only on the far
  field** (issue #103, stage 3). The whole earlier exposure corpus was refused
  by the audit, so it was re-instrumented from zero
  (`kjerag-spike --bin expose`). What made a trustworthy measurement possible
  is stage 2: the band's alignment is what makes two samples the same content,
  and the correlation that finds it is invariant to a brightness change, so
  the two questions are orthogonal by construction. Three findings, each with
  its own control:

  **The additive term is a near-field artifact.** Fitted across patches
  spanning 17 to 243 codes, a gain-plus-offset model beats a gain alone by 47
  percent when near-field directions are included and by nothing at all when
  they are not. What read as veiling glare in the lens with the sun in it was
  the alignment of a boot. The one exception is the X3, where an offset near
  -9 codes survives the cut; that is the priced follow-up.

  **This is what the old corpus's three inconsistent gains were.** A
  two-parameter difference read with a one-parameter estimator returns an
  answer set by the brightness of whatever content it weighted, and how much
  near-field content a seam holds is a property of the capture. Three
  captures, three numbers, no bug.

  **Pooling per-patch ratios was measured out.** The reading's slope against a
  deliberate misalignment is 0.0370 ln per degree for an average of per-patch
  ratios and 0.0013 for a pooling of totals, on the same frames: a displaced
  window's error is a boundary term and falls as the window widens. Least
  squares in codes then beat both on all nine captures, and an equal-weight
  average of log ratios is worse than doing nothing on four of them.

  What ships is one gain, far field only at the band's own knee, least squares
  in codes, smoothed at the constant the far field already has, split
  symmetrically. **No constant was added except the runaway guard**, which is
  four times the widest gain measured. Cost 0.03 ms a redraw; frame-to-frame
  flicker 69x under one code; one-lens files byte-identical.

- 2026-08-01 **The pinned seam benchmark named the wrong file** (issue #103).
  #87 and #103 both give `VID_20260501_183417_00_001.insv` as the source of
  `~/Videos/TEST.mp4`, the camera maker's export the 0:09 wing dip is scored
  against. It is part **003**: cross-correlating the two files' own audio as
  10 Hz energy envelopes over every offset gives r = 1.000 at offset 0.0 s on
  003 against 0.64 and 0.67 on the other two parts. The export's `comment` tag
  is the CAPTURE's start time, which is part 001's name, and it is not the
  clip's offset. Scored against the wrong part the projection fit never locks
  and the share reads anywhere from 0.497 to 0.932 across half a second, which
  is how a number that is not a measurement got into the record as one.

- 2026-07-31 **The sound reads on a demuxer of its own** (issue #97, owner
  defect). One file handle for all three streams was the simpler design and
  the owner's April capture disproved it: the camera left 67 MB of picture
  between the audio sample ending at 4.907 s and the next one, and
  libavformat lets a stream fall a whole second behind before it seeks out
  of file order, so the sound for that region arrived after its moment had
  passed and the splice dropped it. Measured on main: silent from 4.87 s to
  8.21 s. A second capture on this box has the same gap at 4.480 s and a
  third has one at 1445.8 s, so it is a camera behaviour rather than one bad
  file. The alternatives are all worse: a deeper ring cannot hold sound that
  has not been read, reading the pictures a second ahead needs 60 more
  surfaces than a decoder pool holds, and buffering the packets instead
  means carrying 25 MB of undecoded picture at all times. A demuxer of its
  own with the pictures discarded reads the sound at its own 190 kbps and is
  immune to any interleave, for one file handle and 0.2 s of open. **And the
  underrun count was lying**: a ring that ran dry while its head was behind
  the picture took the splice's fade-down path and counted nothing, which is
  why the hole measured 2.4 s when it was 3.3 s, and why issue #95 read 227
  underruns as a burst at startup when they were this hole in the middle.
- 2026-07-31 (late) Seam architecture revised by three owner rulings: the
  app targets ANY 360 footage (near-field moves in general, so per-frame
  band alignment is the MAIN path and the per-clip table is a prior);
  the horizon bar is pixel-perfect (calibration brings residual inside
  the band search's capture range, per-frame alignment snaps it to zero,
  far field included - which is how Insta360's own horizon is perfect);
  and correction is calibrate-by-watching (seam readings harvested from
  playback's own decoded frames, slerped in below perception, pooled
  per camera, cached per file, no user surface at all).

- 2026-07-31 **ffmpeg pin moved 6.1 -> 7.1** (owner: "Bump to 7"), which
  supersedes the 2026-07-30 entry further down. Issue #65: the Flatpak
  could not be built from the tree at all while the pin said 6.1, because
  every freedesktop runtime ships ffmpeg 7 and the 25.08 one is forced by
  libcosmic's rustc floor. The port is one file. ffmpeg 7 replaced the
  bitmask channel layout with `AVChannelLayout`, which holds raw pointers
  and so is not `Send`, and a `Track` rides its `Reader` onto the decode
  thread; it now derives the layout from the channel count it already
  keeps rather than storing one. The bill goes to the dev box: Ubuntu
  24.04 has no ffmpeg 7 and will not get one, so ffmpeg comes from a PPA
  (AGENTS.md, and the same one in CI) or, without sudo, from
  `scripts/ffmpeg7-local.sh`.

- 2026-07-31 **The app has an icon** (issue #67, seven workshop rounds
  recorded in docs/icon.md). A round teal world with a green coast and a warm
  rim, and a small figure entering it from the upper left, drawn by
  `scripts/icon-diver.py` from a joint skeleton rather than traced. The
  figure's size and how far its feet clear the rim are set together, because
  the rim crossing is what decides both: round 7 grew it 18 percent inward,
  holding the feet at the same 27.1 units past the rim.
  `resources/icons/hicolor/` is the theme tree: a scalable SVG, PNGs from 256
  down to 16, and a drawing of its own for 32, 24 and 16, because both COSMIC
  and the Pop theme redraw those sizes instead of exporting. The files are
  named for the application ID `dev.harding.Kjerag`, the one issue #66
  settled and issue #75 put in the code, so an installed build now resolves
  its own icon by name. The app still draws the scalable SVG out of its own
  bytes, because a source-tree run installs no theme to look a name up in.

- 2026-07-31 Seam bar raised (owner): "I want the best seam support out
  there." The prod gate is not good-enough but best-shipping, Insta360's
  stitcher included. Two tracks: the per-camera geometric foundation
  (static-capture 5-knob fit, #87 rework) ships first; depth-aware seam
  alignment (the overlap band is a 33 mm stereo pair, disparity gives
  metric depth - what dynamic stitching fundamentally is) is #80 phase A,
  research-first with owner-validated design before implementation.

- 2026-07-31 **The camera is the shell's state, not the widget tree's**
  (issue #77). iced keeps a widget's state in the widget tree and rebuilds
  it whenever the tree changes shape under it, which the header bar coming
  and going does on every fullscreen toggle and every idle timeout. Anything
  a pilot expects to survive the window changing shape therefore cannot live
  in an iced `State`, however natural a home it looks. Keeping it there and
  pinning the tree instead was the alternative and it is a trap: it makes
  every future layout change a chance to lose the view, silently, and the
  shell has to be free to change its layout.
- 2026-07-31 **The pitch runs all the way round, and the wall is gone**
  (issue #63, owner ask). The alternative reading of "keep looking up past
  the zenith" is to fold the crossing into pitch and yaw together - pitch
  turns back down and the yaw swings half a turn - which keeps the pitch
  inside a quarter turn and keeps the picture upright. It was rejected
  because it is not what was asked for: the owner asked to keep going
  **until he sees upside down**, and a fold never shows an upside down
  world. It also puts a discontinuity in the yaw exactly where the hand is
  moving. Letting the pitch continue is both the thing asked for and the one
  with no jump in it.
- 2026-07-31 **Past the flat range the drag is a rate, not a pin**
  (issue #78, owner ask). One threshold, `FOV_FLAT`, shared with the
  projection's own bend, and one constant: the rate the pinned drag is
  already turning at when it gets there. The alternative was to keep the pin
  and damp it - a speed limit on the solve - which reads well and breaks
  issue #63, because the same limit would have to bite hardest exactly where
  a pole crossing legitimately turns the view fastest. Two drags with one
  clean threshold beats one drag with a rule that has to know about poles.
- 2026-07-31 **A capture reports itself at the top of the window, in a toast
  drawn out of libcosmic's own pieces** (issue #15, docs/UI.md's open
  question 2). cosmic-files is the only first-party app that uses toasts at
  all, so its lines are the whole precedent and the wording, the 5 s, the
  five-line stack, the tooltip container and its spacings, and the refusal
  to carry an action unless it undoes something destructive are all its own
  (`src/app.rs:1344-1358`, `toaster/mod.rs:33-63`, `79-85`, `162-181`). The
  **placement is the owner's**, and it is a deviation from cosmic-files with
  a reason: it puts its toasts at the bottom because the bottom of a file
  manager is empty, and the bottom of this window is the transport. Shipped
  over the scrubber first, and the owner found it.
  `widget::toaster` cannot be moved: its overlay is laid out against the
  bounds iced hands every overlay, which are the window's
  (`toaster/widget.rs:199-215` against `user_interface.rs:228`), so it sits
  15 px above the bottom of the window whatever it is mounted over. Mounting
  it over a band at the top of the window was built and captured, and the
  toast did not move. So the stack is a `Stack` layer over the picture,
  which also gets the control row's overlay back
  (`overlay::from_children` rather than `Toaster`'s replace-the-content's,
  `toaster/widget.rs:137-162`). Two things that were measured rather than
  assumed: the layer is mounted even when empty, because a tree that grows a
  layer cost the toast five redraws before it reached the screen; and the
  five seconds is a sleep on the async runtime as libcosmic's own is, not a
  poll, because a 250 ms poll cost 3 to 6 redraws a second and dropped
  frames in 2 of 18 report windows against 0 of 18 without it.
  `scripts/uitest.sh` now asserts the placement instead of a reader having to
  notice it: transient chrome must leave the header band and the control-row
  band byte for byte identical, which the shipped-first placement fails.

- 2026-07-31 **A capture is not always one file, and the ONE X2's IMU is
  not mounted like an X4's** (issue #79, owner-reported). Three symptoms on
  the owner's X2 footage were two defects and one thing that was never
  broken. Half a sphere was the camera writing one lens per file: the two
  are paired at open, matched on frame index, and either file of a pair now
  opens the whole capture. The horizon being "way wrong" was the IMU axis
  convention, which fell through to the X4's `xZY` and is 121 degrees out on
  this camera; measured against pixels it is `Zxy`. "Upside down" was the
  same defect seen through a horizon lock that is on by default, and the
  delivered-frame datum it appeared to accuse turned out to be right: the
  unlocked picture is upright on a plumb reference, and the seam's own
  arithmetic agrees to 0.16 degrees.

  Two method notes worth keeping. The 24-way sweep **cannot** finish this
  job on a camera whose two best candidates are a half turn apart when the
  footage has no true horizon in it - a mountain ridge is not level - and
  what finished it was aiming the view along the accelerometer on a still
  frame and looking at whether the sky was there. And a wrong picture datum
  and a wrong axis convention are not separately observable in a locked
  view, because each cancels the other; only the unlocked picture pins the
  datum.

- 2026-07-31 **A saved still is a JPEG; the clipboard is still a PNG**
  (issue #15). Twelve encodings of five real 3840x2160 captures, plus
  libwebp and libjxl for reference, scored against those same pixels with
  ffmpeg's `psnr` and `ssim` filters. Nothing lossless got near the size a
  file that gets shared wants: PNG's own levels bottom out at 3.2 to 8.7 MB
  and take 3 to 7 s to do it, oxipng reaches 2.9 to 7.6 MB in 5 to 7 s, and
  lossless WebP, the best of them per second, 3.0 to 7.9 MB. Of the lossy
  ones only JPEG has a maintained pure Rust encoder: lossy WebP and JPEG XL
  are C libraries, and the one pure Rust JXL encoder does lossless only.
  Skipping them costs nothing measurable. At quality 93 with no chroma
  subsampling a still is 0.7 to 1.8 MB, a seventh of the PNG or less, and
  scores higher on SSIM than libwebp at quality 95 and libjxl at distance 1
  on all five captures, at 1.3 to 2.5 times their file size. The encode is
  65 to 74 ms against the PNG's 33 to 45 ms, on the worker thread that has
  already waited for the GPU and reads back 33 MB before it starts.
- 2026-07-31 **The UI harness builds the binary it drives, every run**
  (`scripts/uitest.sh`). It used to build only when `target/release/kjerag`
  was missing, so a binary left over from before a `git revert` is what it
  drove: the ball check failed twice on a tree whose source passes it four
  runs out of four, and the capture it filed was the reverted design rather
  than the restored one. A harness that reports on code it did not run is
  worse than no harness, and cargo costs nothing when the binary is already
  fresh. `KJERAG_BIN` stays the way to point it at a binary on purpose,
  which is how the stale one was identified.
- 2026-07-31 **The zoom out to the ball is one projection family, not a second
  projection** (issue #47). Perspective and tiny planet are two ends of
  `r = tan(shrink * theta) / shrink`: `shrink` 1 is rectilinear exactly,
  1/2 is stereographic exactly, and below that the sphere closes into a finite
  disc. Blending two separately-written maps was the obvious alternative and
  is worse in the way that matters, because the thing being asked for is that
  there be no seam in the scroll: a family has no crossover to hide. The
  schedule is `shrink = 110 degrees / fov`, which holds `shrink * fov / 2`
  constant past the threshold - the frame keeps the half angle of the widest
  flat view and the world shrinks into it - and that is not a taste: it is
  what makes zooming out zoom out at every point of the frame, where a
  smoothed schedule that overshoots hands back a scroll that reverses in the
  middle (`the_picture_only_ever_shrinks`).
- 2026-07-31 **The field of view is allowed past 360 degrees** (issue #47),
  rather than capping there or switching to a second control. At 360 the
  frame's edges are half a turn out and the sphere is exactly as wide as the
  frame; the owner asked for the ball to sit in frame **with room around it**,
  and room means the frame reaching further than the sphere does. Anything
  else needs a second zoom parameter with a different meaning at the far end,
  which is a worse thing to explain and a worse thing to test.
- 2026-07-31 **Which frames may take the screen and which seek is still owed
  one are two questions** (issue #55). `Player::pump` answered both with one
  epoch comparison: a frame was shown only while its own seek was the newest,
  and showing anything cleared `is_seeking`. That is what froze a fast drag,
  and the obvious repair breaks the other half, because `is_seeking` is what
  keeps a paused window redrawing and an intermediate picture would end the
  wait before the release's frame arrived. `Epochs` now carries `asked`,
  `shown` and a `Wait`, and the two questions are separate methods:
  `accepts` decides what may take the screen, and only the newest seek's own
  frame ends the wait. Three states rather than a flag, because the wait is
  not one thing: a **seek** wants a position newer than the one on screen
  (the reader is still handing over frames of the position being left, and
  they are a picture of nowhere the pilot asked to be, which the exact scrub
  measured at 79 ms of wrong picture), a **step** wants the very next frame
  of the position on screen and sends no seek at all, and **playback** wants
  whatever the clock is due. The landing is applied where the frame arrives
  rather than where the seek was asked for, so several outstanding seeks each
  get their own picture at their own time; `Presenter::advance` takes the
  seek's own frame however many pictures have already gone up, which is what
  makes the release's exact frame the last picture of a drag rather than a
  picture that never comes.

- 2026-07-31 **A picture from a seek the pilot has dragged past is better
  than a frozen one** (issue #55). Frames arrive in the order they were asked
  for, so a landing tagged after the picture on screen is a picture of
  somewhere the pilot has been since, and putting it up can only move the
  picture forwards. Sweeping the fixture end to end, 2 s a rate, interleaved
  arms, medians of 7 runs:

  | positions/s | 10   | 15   | 20   | 30   | 45   | 60   | 90   |
  | ----------- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | before      | 10.0 | 15.0 | 12.5 | 10.5 |  5.0 |  0.0 |  0.0 |
  | after       | 10.0 | 15.0 | 19.5 | 29.0 | 38.5 | 45.5 | 43.0 |

  Below 20 positions/s the decoder keeps up and both arms show one picture a
  position. Above it the old rule falls away to nothing while the new one
  climbs to the decoder's own rate and stays there: 45.5 pictures a second is
  22.0 ms each against a 21.1 ms keyframe decode at the reader. That also
  answers what #46 could not, which was what made a drag cycle cost 100 ms
  where a scrub through the same player cost 26. Nothing did: three landings
  in four were being decoded and thrown away. The release lands the exact
  frame in 49 of 49 drags on each arm, a median of 239 ms after letting go
  before and 281 ms after, which is not a difference the measurement supports
  (permutation p = 0.23): the per-drag spread is 24 to 446 ms on both arms
  and it is set by where in the GOP the release falls, since an exact seek
  decodes forward from the keyframe before it.

- 2026-07-31 **The orientation filter starts only from a reading it would
  believe completely** (issue #45). The rule that covers every other sample
  covers the first one: the seed searches forward for the first
  `accel_seconds` of accelerometer inside the whole of `trust_g`, and the
  gyroscope carries it back to the start of the track. Three alternatives
  were on the table and each was decided by a number rather than by taste.
  A **burn-in pass** would use every trusted sample of the opening instead
  of one window's worth, but it converges at `tilt_seconds` from wherever it
  started, so it needs a second time constant of its own; the forward search
  is one extra walk over at most 20 seconds of samples and no new constant.
  Accepting a **partly** trusted window, which is what the issue asked for,
  leaves the April capture 13.8 degrees off level at 6 seconds against 1.9
  for a fully trusted one, because the window it settles for is taken during
  the launch. And the search **stops at 20 seconds**, the default
  `tilt_seconds`, because past that the filter's own correction is worth
  more than a distant reading and the gyroscope has more of its own drift to
  carry back.

  A file that never reads gravity - a motor running from the first frame -
  gets the window closest to 1 g inside that search, which is a documented
  fallback rather than a panic or a silent identity, and which by
  construction is never a worse reading than the opening window the old code
  took unconditionally.

- 2026-07-31 **The sound goes out through cpal, and follows the picture's
  clock** (issue #13). cosmic-player was read first, as the doctrine asks, and
  it has no audio output code to copy: it is `iced_video_player`, which is
  GStreamer `playbin` with only the *video* sink replaced by an appsink
  (`src/video.rs:20-26`), so its sound leaves through playbin's default
  `autoaudiosink` and its volume and mute are playbin properties
  (`src/main.rs:1225-1235`). No COSMIC first-party binary on this box links an
  audio library for playback at all: `cosmic-settings-daemon` links
  libpipewire, and that is routing. GStreamer is already rejected here for the
  frame path, so the choice was issue #13's own pair, cpal or PipeWire
  directly, and cpal is the smaller by a wide margin. PipeWire still plays
  what it writes, through `pipewire-alsa`. The cost is one apt package,
  `libasound2-dev`, because cpal's Linux target links `alsa` whatever host it
  ends up using.

  The clock is **not** re-anchored on the sound, and that was not a
  free choice either: the pictures are paced by due time against a monotonic
  clock (issue #4), a reframing player must not judder, and a sound card is
  the one clock in the room that cannot be asked to wait. So the sound follows
  instead, in two corrections of different kinds. A **splice** when the ring's
  head is more than 30 ms from where the picture will be: sound whose moment
  has passed is dropped, sound whose moment has not come waits under silence,
  and the gain ramps down before the join and up after it. Only a start, a
  seek landing or a recovered stall is ever that far out. A **resampling
  ratio** the rest of the time, through `swr_set_compensation`, capped at
  0.5% and settling near 0.005%: that is the difference between the sound
  card's crystal and `CLOCK_MONOTONIC`, and without it a ring that is right
  now is tens of milliseconds out half an hour later.

- 2026-07-31 A magnified picture is sampled with a **Catmull-Rom kernel**,
  engaged on the map's own Jacobian (issue #11). The decision is per fragment
  and not per redraw because nothing about it is uniform: the fisheye carries
  1106 texels per radian down its axis and 948 radially at the rim of its
  picture, and a rectilinear output's density rises towards its corners, so
  the widest view the player offers is past 1:1 in the middle of a 2560 px
  window (1.23 texels to the pixel) and two thirds inside it at the corners
  (0.74). The shader reads that as the hardware's quad derivative of the
  landing the model just computed, which is the Jacobian by finite difference
  with the distortion, the mounting and the readout already in it, and needs
  no output size in the uniform block. Catmull-Rom rather than a B-spline
  because the kernel must pass **through** the texels it is given; a B-spline
  is what the usual four-tap trick is written for and it blurs a magnified
  picture. Sixteen texels as nine bilinear fetches, which measured 0.14 codes
  RMS and one code at worst against the same kernel as sixteen point fetches,
  on the highest-contrast view in this footage.

  Engaged **smoothly**, by mixing the kernel's weights from linear towards
  Catmull-Rom between 1:1 and 2:1, rather than by crossfading two sampled
  pictures: one kernel instead of two wherever the zoom sits in the band, and
  exactly the linear weights at the far end, so a view that is not magnifying
  takes the one fetch it always took and writes the bits it always wrote.
  Swept 70 steps of zoom across the whole band, the largest single step in
  which the sharp picture moved further than the bilinear one is 0.4 codes,
  against the 0.6 codes a kernel switched on rather than mixed would have to
  put somewhere.

- 2026-07-31 The **chroma plane is not upgraded** (issue #11), and, like the
  exposure match and the decode gate before it, the measurement is the
  reason. NV12's two planes are two grids, so they were given two thresholds
  and asked separately, which is what found the problem with upgrading the
  smaller one: chroma is half the size, so it is magnified twice as hard as
  luma and it is under 1:1 at **every** field of view this player offers at a
  window anyone uses. Its upgrade is therefore not a cost paid at high zoom
  but a cost paid always, and it is the larger half of the bill. What it buys
  is nothing anyone can see: 0.41 codes on 40% of pixels, and the detail
  metric does not move at all (4.606 with it against 4.606 without, on 4.120
  bilinear). Rendered at four times life size on the most saturated content in
  half an hour of footage, the two are indistinguishable. `Sampling::Sharp`
  is one line and stays runnable; footage with hard saturated colour edges,
  which paramotor flying does not have much of, is what would change the
  answer.

- 2026-07-31 **A scrub takes the decode thread off the lookahead refill**
  (issue #46). The thread used to look at its command queue only between
  reads, so a drag position that arrived while it was refilling the pipeline
  behind the last landing waited for three pair decodes of pictures nobody
  would ever see. `Reader::read_until` now asks an interrupt between packet
  reads and gives the read up when a newer command is waiting; nothing is
  thrown away, because the lanes keep what they decoded and the seek that
  follows is what clears them. Measured on the 37.9 GB fixture, the same 12
  places issue #5 used, medians of 10 runs per arm interleaved so that both
  saw the same box:

  | keyframe scrub             | before  | after   |
  | -------------------------- | ------: | ------: |
  | reader alone               | 20.6 ms | 20.6 ms |
  | through the player         | 59.2 ms | 26.4 ms |
  | picture updates per second | 16.9    | 37.9    |

  So 33 of the 39 ms between the reader and the player were the stale
  refill, and what is left is the thread handover plus one packet read of
  interrupt latency. The exact seek a release asks for came down with it,
  276 ms to 237 ms against a 230 ms reader. Both arms were measured before
  the sound landed (issue #13) and confirmed against it afterwards, three
  runs each: 59.2 ms to 26.5, and 276 to 236.

  **The read a seek itself asked for is never interrupted**, and that is
  load-bearing rather than an omission. A drag asks for positions faster than
  pictures come out of them (10 to 12 a second against 20 to 60 asked for, in
  the table below), so a rule that gave up whatever was newest would give up
  every landing too and a fast drag would show no picture at all.

  It composes with the sound (issue #13) without a rule of its own. A read
  that is given up stops before reading another packet, so it feeds the ring
  nothing more, and the seek it was given up for flushes the ring twice over:
  `Player::hush` on the shell's thread as the command is sent, and
  `Reader::seek` on the decode thread when it arrives. Preempting only
  shortens the gap between those two, which is the window in which the old
  position could still be decoded into the ring.

- 2026-07-31 **Skipping the map for a frame that will be overtaken is not
  worth its line** (issue #46, measured and rejected). Under
  newest-command-wins a refill frame can be decoded, mapped and handed over
  microseconds before the command that makes it stale, and the map is the
  expensive half (`av_hwframe_map` waits for the decode: 7.64 ms a frame in
  the M0 table, twice for a pair). Asking the interrupt before the map
  rather than after it recovers that. It cannot be worth much, and it is
  not: the window is one packet read wide, and interleaved runs of the two
  orderings sit inside each other's spread (26.4 ms against 26.5 for the
  scrub, 237 against 237 for the release, and the drag rates inside a
  picture a second of each other). The measurement is `--bin seek`; the
  ordering that ships is the one with the cheaper claim on it.

- 2026-07-31 **A drag is not a run of jumps, and the instrument now says so**
  (issue #46). `--bin seek` measured seeks one at a time, waiting for each
  picture before asking for the next, which is a hand that stops. A drag
  fires a position per pointer move whether or not the picture has caught
  up, and `Player::pump` shows a frame only while its own seek is still the
  newest, so a hand moving faster than a landing takes shows **nothing**.
  Sweeping the fixture end to end, 2 s per rate, medians of 8 runs:

  | positions/s | 10   | 20   | 30   | 45  | 60  |
  | ----------- | ---: | ---: | ---: | --: | --: |
  | before      | 10.0 | 10.2 |  9.0 | 3.8 | 0.0 |
  | after       | 10.0 | 12.0 | 10.0 | 5.5 | 0.0 |

  The release lands on the exact frame in all 105 of those drags, before and
  after. The interruptible read helps here too, but the ceiling is not the
  refill and this change does not move it: at 60 positions/s neither arm
  puts a single picture on the screen, and that is what a fast drag on the
  scrubber did until issue #55, whose entry above is where that ends.
  Whatever costs the difference between a 26 ms scrub and a 100 ms drag
  cycle was not found here, and an all-or-nothing epoch rule is what turns
  it into a frozen picture rather than a slow one. It is not the page cache:
  a sweep confined to one warm 36 s window of the file measures the same
  10.0, 11.5, 10.5, 5.5 and 0.0 against the full file's 10.0, 12.0, 10.0,
  5.5 and 0.0, interleaved on a quiet box. (#55's answer to the 100 ms: the
  cycle cost one keyframe decode all along, and the epoch rule discarded
  three landings in four.)
- 2026-07-31 The pass **skips the lens a ray cannot reach** (issue #10). Each
  lens's picture is one cap around its own axis; the cap is solved out of the
  calibration by finding where the model's own landing leaves the image
  circle, rather than written down as an angle that would be right for one
  camera. A ray further off the axis than that weighs exactly zero, so one
  dot product per lens replaces a Mei evaluation on the majority of the
  sphere that only one lens can see. The test is one-sided on purpose: false
  means the weight is exactly zero, true means it might not be, so a lens
  kept and weighed zero is multiplied by nothing and only a lens wrongly
  dropped would be a hole. The margin on the cap is half a degree, which is
  thirty times the worst error eight azimuths make on this fixture and costs
  0.4% of the sphere in projections that weigh nothing.

- 2026-07-31 The **decoder is not gated** on it (issue #10), and the
  arithmetic is the reason. What gating the invisible stream is worth,
  measured at playback pace on this box, two runs each, against a 6.10 W idle:

  | lanes                     | CPU, one core | SoC power |
  | ------------------------- | ------------: | --------: |
  | both decoded, both mapped |         7.06% |    9.38 W |
  | both decoded, one mapped  |         6.88% |    9.36 W |
  | one decoded, one mapped   |         4.22% |    7.85 W |

  So the full gate does halve decode power, as the issue predicted: 1.53 W of
  the 3.28 W decoding adds over idle. The cheap version that never goes cold,
  decoding both and mapping one, is worth 0.02 W and is inside the noise.

  What it is worth **on average** is the problem. A gate can only be on while
  no ray of the view reaches the far lens, which at the default 90 degrees of
  field of view is 16.5% of the sphere and 11.0% of yaw/pitch space; at 45
  degrees it is 45.6% and at 110 it is 8.4%. And with the horizon locked,
  which is the default and which the footage demands, a parked view is not a
  parked geometry: the body swings and turns under it. Measured over 40
  parked views and 60 s of two X4 Air captures, at 90 degrees, the gate would
  be on 21.6 to 24.3% of the time with no hysteresis and 8.9 to 9.4% with 15
  degrees of margin, releasing one to three times a minute with nobody
  touching the mouse. (Both under the heading follow; the world-fixed lock of
  2026-08-06 turns the body fully under a parked view and takes those to 15.7%
  and 3.9% on one of the same captures.) With the lock off it holds forever,
  but only 5 to 17.5% of parked views qualify. Expected saving at the default: **0.14 W of about
  10 W**, and 0.26 points of one core.

  Against that, releasing a cold gate is not free and cannot be made free.
  HEVC has no way into the middle of a GOP and this camera writes 29-frame
  ones, so the far lens has to be walked from a keyframe: measured at 195 to
  340 ms, six to eleven frames of stale far hemisphere, and it does not
  depend on how long the gate was on because the hold is bounded by the GOP.
  Three warm strategies were tried. Keeping the packets since the last
  keyframe and replaying them is the best of them and is what those numbers
  are. Decoding the keyframes as they pass shortens the replay by one frame
  of 29 and adds a decode a second. Re-seeking on release is slower (230 ms
  median and 447 worst, issue #5's table) and moves the demuxer the live lane
  is reading from, so it hitches the hemisphere that never stopped. No margin
  closes the gap either: the body alone reaches 551 deg/s on this footage,
  where 15 degrees of margin is 27 ms, and a drag solves for the direction
  under the cursor and so has no rate bound at all.

  A gate that is on a tenth of the time, saves a seventh of a watt, and can
  show a stale far hemisphere for a third of a second is not a trade this
  player makes. `kjerag-spike --bin gating` is the whole measurement and
  `Reframe::reaches` is the test it would have used, both kept so the loser
  stays measurable. What would change the answer is a shorter GOP or a
  format with cheaper random access, and the instrument would say so.

- 2026-07-31 An X4's sensor reads **down the delivered frame**, so
  rolling-shutter correction ships **on** (issue #9). Measured at 1.00 +-0.12
  whole-frame readouts down and 0.02 +-0.07 across, over five stretches of a
  30-minute capture, by one lens against itself a few frames apart. The
  trailer records how long a readout takes and nothing about its direction,
  and a direction applied backwards doubles the skew it should remove, so the
  bar was a control on **each** axis the fit answers on: injecting each of the
  four candidates reads back at 0.85 to 1.02 on its own axis and leaves the
  other where it was. Cameras nobody has measured keep `Sweep::Unknown`,
  which is a zero axis and no correction.

  Same day, and this is the transferable part: **#42 had the answer in its
  own tables and read it as noise**, because it controlled one of the two
  axes it fitted. The uncontrolled axis was reported as "does not repeat" on
  the strength of two stretches, one of which turns out to be a stretch where
  an injected control reads back at -0.10. An instrument that cannot see says
  so in a control column, not in a scatter.

  Same day, second lesson: **a still capture cannot answer a motion
  question.** The settling capture this issue waited on arrived with the
  camera standing on a desk (0.2 deg/s median, 1.5 worst), where a whole-frame
  readout displaces the picture by 0.02 degrees and the instruments' own
  controls can apply 0.003. `--bin rolling` now prints a `carries:` line
  before it decodes anything, which is the file's own rate distribution
  against what the measurement needs.

- 2026-07-30 License AGPL-3.0 (Alex; matches wingover). Unlocks Gyroflow
  GPL-3.0 shader reference with attribution.
- 2026-07-30 No LRV proxy dependence (Alex): full-res decode must stand
  alone; generate proxies only if ever proven necessary.
- 2026-07-30 Frame delivery via DRM_PRIME dmabuf, not hwcontext_vulkan
  (device-lost reproduced on target hardware) and not GStreamer (no
  wgpu/dmabuf sink).
- 2026-07-30 No optical-flow stitching: static calibrated warp measured
  equal or better for a player.
- 2026-07-30 Insta360 MediaSDK rejected: NDA, non-redistributable, no
  seek API, bundled cloud-calling codec.
- 2026-07-30 Primary target is AMD/Intel Mesa (VA-API). NVIDIA would need
  an NVDEC backend variant; out of scope until someone needs it.
- 2026-07-30 ffmpeg-next/ffmpeg-sys-next pinned to 6.1, matching the system
  ffmpeg. The 8.x APIs in the research notes are not present. **Superseded
  2026-07-31**: 7.1, see the top of this log.
- 2026-07-30 Zero-copy import is not a hand-rolled ash routine: wgpu 30's
  `Device::texture_from_dmabuf_fd` (wgpu-hal Vulkan) imports the VA-API
  planes as they come, 0.12 ms/frame for both. On libcosmic's wgpu 28 the
  same import is written by hand (see next entry).
- 2026-07-30 Shell: libcosmic from day one (Alex). Native COSMIC chrome is
  part of the product identity. We accept the hand-rolled ash dmabuf
  import against libcosmic's wgpu 28, and delete it the day libcosmic
  reaches wgpu 30.
- 2026-07-30 PR policy (Alex): the coordinator self-merges once CI is
  green and the diff is reviewed. Alex steers via issues, the roadmap,
  and check-ins.
- 2026-07-30 The trailer is read directly (`crates/meta/src/trailer.rs`, ~45
  lines) instead of through `telemetry-parser`. The published 0.2.6
  aborts on our X4 Air footage: it serializes the metadata protobuf's
  enum fields with `unsafe { transmute }` of the raw i32, and the file
  carries a value no enum in that schema has. The fix exists only on an
  unpublished master that pulls two further git forks. Kjerag was already
  bypassing that crate's lens profile (wrong on the Air) and would have
  had to bypass its merged exposure records (M2), so what remained was
  the record walk. `prost` decodes the eleven fields we read.
- 2026-07-30 libcosmic still pins wgpu 28 (checked, not assumed:
  `pop-os/libcosmic@dc1cf9f` vendors `pop-os/iced@7346cff`, whose
  workspace `Cargo.toml` says `wgpu = "28.0"`; the lockfile resolves
  wgpu 28.0.0 / wgpu-hal 28.0.1). The hand-rolled import stands.
- 2026-07-30 The whole crate is on wgpu 28, spike included. Two wgpu
  majors in one graph would mean two sets of incompatible types for the
  same textures, and the spike's job is to measure the code the app runs.
- 2026-07-31 Carry a wgpu fork (Alex). Nothing exposes
  `VK_EXT_image_drm_format_modifier` on the device `iced_wgpu` creates, so
  the shader widget could not import a frame on stock dependencies. We
  carry the extension-enable hunk of gfx-rs/wgpu#9366 (shipped in wgpu 30)
  applied verbatim to the v28.0.1 tag, on
  `aeharding/wgpu@v28-drm-modifier-backport`, through `[patch.crates-io]`.
  The alternative was a second wgpu device and an external-memory handoff:
  roughly 200 more unsafe lines and a CPU stall per frame.
  **Deletion condition: the day libcosmic reaches wgpu 30, delete the patch
  entry, the fork, and `crates/render/src/dmabuf.rs`, and call
  `vulkan::Device::texture_from_dmabuf_fd` instead.**
- 2026-07-31 The patch entry names `wgpu`, not `wgpu-hal`. Patching
  wgpu-hal alone leaves the rest of the tree on the crates.io wgpu-types,
  and two wgpu-types in one graph is two incompatible `TextureFormat`s.
  `wgpu` is the only crate in that workspace anything outside it depends on.
- 2026-07-31 ~~The app turns libcosmic's content container off
  (`core.window.content_container = false`)~~ (issue #22, superseded by
  issue #93 the same day). It insets the view by `border_padding` on the
  right and, because `nav_bar.active` defaults to true even with no nav
  model, by nothing on the left. Video wants both edges, and turning the
  container off is one of the two ways to get them.
- 2026-07-31 The border padding is zeroed instead, and the content container
  stays (`core.window.border_padding = Some(0)`, cosmic-player
  `src/main.rs:895`, issue #93). `main_content_padding` is `[0, 0, 0, 0]`
  either way (`app/mod.rs:632-639`), so the video still has both edges. What
  the container is worth is the window background: libcosmic paints
  `background(theme.transparent).base` only on the container branch
  (`app/mod.rs:856-874`), and that colour is what makes a COSMIC window a
  darkened pane over the compositor's blur. Without it the welcome view was
  blur and nothing else, which is what the owner saw. cosmic-files paints no
  background of its own either; it just leaves the container on
  (`src/app.rs:2352-2367`, off in desktop mode only).
- 2026-07-31 One crate per layer, in a workspace (issue #19): `kjerag-meta`,
  `kjerag-media`, `kjerag-render`, `kjerag` (the app) and `kjerag-spike`.
  The layer diagram is now a build constraint, and `kjerag-meta` builds and
  tests with no libav headers anywhere on the box, which a CI job that
  installs nothing checks on every push. `[patch.crates-io]` moved to the
  workspace root, the only manifest cargo reads one from.
- 2026-07-31 `kjerag-render` depends on libcosmic, for one file. The three
  `iced::widget::shader` impls are a foreign trait on types `render` owns,
  and coherence forbids writing them in `kjerag`. The alternative, a set of
  forwarding newtypes in the app crate, is more code for the same wiring;
  they live in `crates/render/src/widget.rs` and nothing else in the crate
  mentions iced.

- 2026-07-31 The lens pose composes as `Rz(roll - 90) * Ry(yaw) * Rx(pitch)`
  in the delivered frame's own axes (x right, y down, z along the optical
  axis). The quarter-turn datum is measured, not assumed: applying `roll` as
  `offset_v3` writes it renders an X4 Air on its side, and dropping `roll`
  renders a ONE X2 on its side. Four candidate rotations were rendered
  against plumb references on both cameras
  (docs/research/insv-format.md 4.8). This closes the last open question
  from the format study's section 10.
- 2026-07-31 The Mei forward map is written from the model description
  (Mei/Rives, OpenCV `omnidir`) rather than transcribed from Gyroflow's
  `insta360.wgsl`, so `crates/render/src/projection.rs` is plain AGPL-3.0
  with no SPDX header. The Gyroflow route stays open and stays licensed:
  any file that takes it carries its own header.
- 2026-07-31 The forward map exists twice, in WGSL and in Rust, sharing one
  `Reframe` uniform block. The Rust copy is what `cargo test` can check
  against known angles on a box with no GPU and no footage, and
  `min_binding_size` is what makes wgpu reject a pipeline whose two
  definitions have drifted.
- 2026-07-31 The camera lives in the shader widget's own iced state
  (`Viewpoint`), not in the app's model. Panning is a widget concern, the
  shell has no opinion about it, and no message round trip happens per
  mouse move.
- 2026-07-31 The drag anchors and solves (issue #29). A press stores the
  world direction under the cursor, and every move solves for the view that
  puts that direction back under the cursor: height above the horizon fixes
  the pitch, bearing then fixes the yaw. Stepping yaw and pitch by the
  cursor's own movement is only grab-the-world near the middle of the view,
  because near the pole a yaw turns about an axis nearly along the view
  ray. The horizon stays level, so where an exact answer would need roll
  the pitch clamps and the drag reads as a wall; and where a view pitched
  near the vertical sees past the pole, the solve takes the tilt nearest
  the one it is already at rather than the mirrored view that fits equally
  well.
- 2026-07-31 A ray is in the picture only where the Mei map stays one to
  one, `cos(theta) > -1/xi`, as well as inside the image circle (issue
  #30). Past that turning point the map folds rays from behind the camera
  back inside the circle, which showed as a raw circular fisheye hanging
  behind the reframed view. The bound comes out of the calibration's own
  xi, so no maximum field of view has to be guessed at. It is per lens: a
  fold in one lens is now a ghost printed over a picture the other lens is
  drawing correctly.
- 2026-07-31 ~~The pick between lenses is nearest axis, and it is a branch
  rather than a blend~~ (issue #27, superseded by #7 the same day). One lens
  was sampled per output pixel, which halved the texture fetches; the cost
  was a hard seam. Nothing is left grey either way: the two 97.4-degree caps
  overlap by about 14 degrees. Sampling still uses an explicit mip level,
  because a `textureSample` needs uniform control flow to compute one and
  every imported texture has a single level anyway.
- 2026-07-31 The lenses are mixed by `cos^2(theta/2) * (image_radius -
  landing_radius)`, normalized (issue #7). Longitude preference times
  coverage depth, which is the field docs/research/insv-format.md 6.6
  recommends, and neither factor is a feather width: the band that gets
  blended is the overlap itself, 83.4 to 97.4 degrees off the front axis,
  and the rim of the image circle, where vignetting lands and where the
  distortion polynomial is least trustworthy, is down-weighted for nothing.
  The longitude factor is what puts the crossover on the seam great circle
  rather than wherever the two image circles happen to end, which is 8 px
  apart on the X4 Air.
- 2026-07-31 Exposure is NOT corrected from the shutter records, and the
  measurement is the reason (issue #7). The plan was the symmetric split
  the format study recommended, `front /= sqrt(g)` and `back *= sqrt(g)`
  for shutter ratio `g`. Measured first, on two 30-minute X4 Air captures
  by comparing the mean luma of each lens's overlap annulus, which holds
  the same world directions in both: the real step is 0.9 to 3.5 percent
  and `g` swings 0.54 to 1.81, uncorrelated, and the split makes the step
  14 to 20 percent. The two lenses trade shutter against sensor gain to
  reach the same brightness, so `g` measures how differently the two
  hemispheres are lit and not how differently they came out; the per-lens
  gain that would complete the sum is not in the trailer. What is left,
  three percent laid across a 14-degree band, is under the eye's threshold.
  A measured luma ratio off the overlap band is the fallback and costs a
  readback per frame: do not build it before a capture needs it.
  docs/research/insv-format.md 6.3 has the method and the table.
- 2026-07-31 The trailer is read through record 0's index, not by walking
  (issue #7). Walking is not merely slower on the X4 Air: its trailer
  leaves 163 to 250 KB of slack between records, so the chain stops making
  sense after the three records nearest the footer and the exposure records
  are unreachable. The ONE X2 writes no index and packs its records tight,
  so the walk is still there for it, and where both exist they agree.
- 2026-07-31 The blend's loop runs MAX_LENSES times whatever the file
  holds, and the lens count zeroes a slot rather than shortening the loop
  (issue #7). A loop the shader compiler cannot unroll indexes its local
  arrays dynamically, which puts them in scratch memory: 1.82 ms per redraw
  against 1.68 at 2560x1440, which is more than the blend's own second
  texture fetch costs.
- 2026-07-31 Lens 1's nominal arrangement is a half turn about the body's
  vertical, multiplied on the right of the block's own angles (issue #27,
  docs/research/insv-format.md 4.9). The file does not contain the flip at
  all, and the two orders differ by twice lens 1's roll residual, so this
  was measured rather than picked: each lens rendered alone across the
  seam, and the far-field content correlated between the two pictures.
  0.4 degrees of along-seam residual for this order against 1.5 for the
  other, and the half turn about x, which is a rear sensor upside down,
  correlates with nothing at all.
- 2026-07-31 The shell's design is written down before it is built, in
  docs/UI.md, from the cosmic-player / cosmic-files / cosmic-edit sources
  (issue #16). Every decision in it cites the first-party file it copies,
  and the places where no COSMIC precedent exists are listed as open
  questions rather than answered by us. The written guidelines turned out
  to cover almost none of this: `system76/hig` is one README about dialogs
  and copy that defers to the elementary HIG, which has no keyboard,
  header-bar or media page either. The source is the guideline.

- 2026-07-31 Frames are delivered in pairs, not one stream at a time
  (issue #4). `Frames` carries every video stream at one PTS, so the two
  lenses cannot drift apart: there is no code path that delivers lens 0
  without lens 1. Both are imported into wgpu, and since issue #27 both are
  sampled.
- 2026-07-31 Playback is paced by due time, not by counting refreshes. Each
  frame's due time comes off a monotonic clock anchored to the first frame
  presented, and the shell sleeps until it (`RedrawRequest::At`). The
  shell-side alternative, a `window::frames()` subscription pumping the
  clock on each redraw message, was built first and measured: 33-46
  redraws/s on a 60 Hz display and 1-18 dropped frames per 5 s, because the
  event has to leave iced and come back. The clock is pumped inside the
  redraw pass instead, in `kjerag_render`'s shader widget, which costs a
  `RefCell` in `Scene` and buys 30.0 redraws/s with nothing dropped.
- 2026-07-31 The engine holds container PTS as the frame clock for now. The
  trailer's `pts_type = 2` (`VideoPtsEexposureFile`) suggests the per-frame
  exposure records are the camera's authoritative clock; #4 is pacing, not
  gyro alignment, and #8's Studio-diff harness is what can tell. Only
  `Frames::timestamp` changes if it turns out otherwise.
- 2026-07-31 `Reader::lookahead` is 2 (issue #4). Mapping the oldest queued
  surface rather than the newest hides the `vaSyncSurface` inside
  `av_hwframe_map`: 2.19x realtime at depth 0, 2.46x at depth 2, 2.47x at
  depth 4, so depth 2 takes the whole win. docs/ARCHITECTURE.md's "2-3
  frames in flight" now has a number behind it.
- 2026-07-31 The shell is built to docs/UI.md, which reads the first-party
  COSMIC apps and cites one for every call (issue #16). Three things in it
  are ours rather than theirs, each for a reason recorded there: scrubbing
  seeks keyframes until release (#5), the video does not toggle playback on
  click because the press is the look-around grab, and there is no nav bar.
  Two more the implementation added: the auto-hide timeout is checked by a
  250 ms timer that runs only while playing, because this player sends no
  per-frame message to hang it on (that is the whole point of the pacing
  design), and the `text/uri-list` drop is handled while the portal's
  file-transfer mime is not, because that one is a D-Bus round trip rather
  than a payload and nothing here is sandboxed.
- 2026-07-31 There is no hand-built keyframe index, which is what issue #5
  expected. libavformat parses the whole of `stss`/`stco` out of `moov` when
  the file is opened, so the index is already in memory and `av_seek_frame`
  is a lookup in it: measured at 0.1 ms for the seek call itself, and 70.6 ms
  to open the 37.9 GB file. A second copy of that table would buy nothing.
  `cargo run --release -p kjerag-spike --bin seek` is the instrument.
- 2026-07-31 A drag on the scrubber seeks to keyframes and the release seeks
  exactly (issue #5, docs/UI.md's one deliberate deviation from
  cosmic-player). Measured on the 37.9 GB file: 21 ms to a keyframe against
  230 ms median and 450 ms worst to an exact frame, which is what an accurate
  seek per slider tick would cost. A keyframe lands 455 ms early on average,
  which is half a GOP.
- 2026-07-31 A seek hands its first frame over with no lookahead
  (`Reader::landing`). The lookahead is a pipeline and its depth is paid
  before the first picture comes out of it: 46 ms per scrub against 21 ms.
  Playback refills it behind the landing frame.
- 2026-07-31 `media::first_frame` is gone. Everything reads through
  `Reader`, which takes a `Cue` (frame index or timestamp), seeks to the
  keyframe at or before it and walks forward without mapping what it
  passes: 0.22 s cold to any frame in a 3 GB file, position-independent.
  This is the entry point #5's seek and #8's harness build on, and the
  `reframe` instrument now takes `frame=` and `time=`.

- 2026-07-31 Horizon lock is **on by default** (issue #8). The footage
  decided it: this camera is clamped rolled about a quarter turn and pitched
  down, so an unlocked view of a paramotor flight has its horizon running
  down the picture and swinging out of it, and the reframed view inherits
  every swing of a camera hanging under a wing. `View > Lock horizon` and
  `h` flip it live and the choice is remembered. `h` is bare and is ours,
  like `s`: no COSMIC app locks a horizon, so there is no precedent, and the
  owner asked to be able to flip it while watching.
- 2026-07-31 The camera-body orientation is a **complementary filter**, not a
  Kalman filter (issue #8). Integrate the gyroscope, turn the estimate
  towards the accelerometer with a 20 s constant, and believe the
  accelerometer only near 1 g. A Kalman filter estimates the same two states
  with a covariance nobody can populate from a file that records no noise
  figures. Every constant is measured on real footage and the reason for each
  is in docs/research/insv-format.md 8.5; the one that is a judgement rather
  than a measurement is the yaw constant, and the numbers either side of it
  are there too.
- 2026-07-31 Yaw is **high passed, not locked** (issue #8). A view welded to
  the heading the file starts on fights every deliberate turn; a view that
  follows the body exactly inherits every swing. At 3 s the view's worst
  heading swing inside a second is 29 degrees against 103 unstabilized, and
  it still follows 946 degrees of real turning a minute against 986.
  **SUPERSEDED 2026-08-06**, top of this log: a deliberate turn carrying the
  picture round is the thing the owner wanted gone, and the swing this entry
  worried about was never what the high pass caught. On the July 14 file the
  3 s constant took the view's worst swing inside a second from 239.9 degrees
  only to 178.6 (`--bin gyro`), while following 986.8 deg/min of the turning.
- 2026-07-31 The IMU axis convention is **measured, not transcribed** (issue
  #8). A three-letter convention string is only half of a convention; the
  other half is the frame it lands in, which is whatever the project it came
  from composes next, and Kjerag's composition is its own. All 24 rotations
  were compared against the horizon in unlocked rendered frames; `xZY` wins
  every stretch of two captures, by 15 to 36 degrees over the runner-up.
- 2026-07-31 The quarter-turn roll datum belongs to the **delivered picture**,
  not to the sensor (issue #8, closing the open question in
  docs/research/insv-format.md 4.8). The IMU is bolted to the sensor and can
  tell the two readings apart: held level by its accelerometer alone, an X4
  Air comes out a quarter turn on its side through `Rz(roll - 90)` and level
  through `Rz(roll)`. `kjerag_meta::Pose` now carries both.
- 2026-07-31 The **gyro is aligned to the exposure records' clock**, and
  playback still paces on container PTS (issue #8). `pts_type = 2` means what
  it says: the camera's own timestamps drift from the container's nominal
  30000/1001 grid at 6.4 ppm, 11.5 ms by the end of a 30-minute file. What
  the choice is worth is 0.10 to 0.15 degrees of camera orientation on
  average and 0.95 to 1.48 at the worst instant; rendered, the two are
  indistinguishable, so the case for the camera's clock is that bound plus
  the fact that the gyro timestamps come off the same clock.
  `FrameClock::Container` is kept so the loser stays measurable.
- 2026-07-31 The orientation track is stored at **200 a second**, not per
  frame and not per IMU sample (issue #8). Per sample is 1.8 million
  quaternions and 72 MB on a 30-minute capture; per frame is too coarse for
  issue #9, which needs an orientation part way through a frame. 5 ms is
  three times finer than the 15.9 ms readout it exists to serve.
- 2026-07-31 Verification is **physics in the footage**, with a Studio export
  as a later drop-in (issue #8). The owner was asleep and no reference export
  existed, so the references are that a horizon is level and an accelerometer
  at rest reads 1 g. `kjerag-spike --bin horizon` measures the horizon's
  angle in rendered frames; its own tests are its positive control, and a
  deliberately wrong axis convention is the negative one, reading 54 to 65
  degrees of standard deviation against 0.04 to 0.68 for the right answer. A
  Studio export becomes one more row in the same table.

## Measured on the target box (AMD Phoenix, RADV, 3840x3840 HEVC)

Per frame, 300 frames, one lens, one frame in flight (`crates/spike/`):

| path      | demux | decode | deliver | import | render | fps   |
| --------- | ----: | -----: | ------: | -----: | -----: | ----: |
| zero-copy | 0.14  | 0.85   | 7.64    | 0.12   | 0.91   | 103.0 |
| copy      | 0.10  | 0.57   | 45.3    | 2.25   | 3.05   |  18.4 |

`deliver` is `av_hwframe_map` (zero-copy) or `av_hwframe_transfer_data`
(copy); `import` is the dmabuf import or `write_texture`. The map stage is
dominated by the `vaSyncSurface` inside it, so it is really decode wait: a
player that keeps 2-3 frames in flight gets that time back. The copy path
does not reach realtime for even one of the two lenses.

Playback, both lenses, 60 s of a 3840x3840 29.97 fps file, rendering
2560x1440 (`cargo run --release -p kjerag-spike --bin playback`):

| lookahead | decode        |
| --------- | ------------- |
| 0         | 2.19x realtime |
| 2         | 2.46x realtime |
| 4         | 2.47x realtime |

| what                        | measured |
| --------------------------- | -------- |
| presented                   | 29.94 fps |
| redraws                     | 30.0 /s   |
| dropped                     | 0         |
| starved                     | 0         |
| worst late                  | 6.6 ms    |
| reprojection pass           | 1.31 ms/redraw |
| CPU (decode + import + pass) | 9.1% of one core |

Sampling both lenses (issue #27) costs the pass about a quarter more, and
nothing else: measured back to back against the same binary before the
change, three runs each at 2560x1440, 1.26 to 1.48 ms/redraw with one lens
against 1.59 to 1.90 with two, still 0 dropped and 0 starved either way.
Every output pixel ran the Mei map twice, once per lens; **since issue #10
it runs it once wherever only one lens can have the ray**, which is most of
the sphere. Three 30 s runs each side of the change, same binary, 2560x1440:

| pass, ms/redraw       | before | after |
| --------------------- | -----: | ----: |
| yaw 0, fov 90         |   1.74 |  1.54 |
| yaw 90, fov 90 (seam) |   1.81 |  1.66 |
| yaw 45, fov 110       |   1.80 |  1.64 |

0 dropped and 0 starved in all eighteen runs, 29.92 fps presented and 30.0
redraws/s throughout, and eight rendered views are byte for byte what they
were before. The seam view saves nearly as much as the axis view because a
90-degree window on the seam is mostly not seam: the band is 14 degrees
wide and everything either side of it drops a projection. The saving is
smaller than issue #9's numbers suggest a Mei evaluation is worth, and that
is the correction rather than a disappointment: the 1.11 ms a readout round
costs is a `turned` (a sine, a cosine and a cross product), a normalize and
a Mei, and the Mei is the cheap part of it.

Blending them (issue #7) costs about a twentieth more again, and only a
little of that is the second texture fetch. Three 60 s runs each of the
same binary either side of the change, 2560x1440, at two views: yaw 0,
which is down the front lens's axis and holds no seam at all, and yaw 90,
which puts the seam down the middle of the picture.

| pass, ms/redraw | yaw 0, no seam | yaw 90, seam through the middle |
| --------------- | -------------: | ------------------------------: |
| hard pick (#27) | 1.61 to 1.62   | 1.61 to 1.63                    |
| blend (#7)      | 1.67 to 1.70   | 1.73 to 1.76                    |

0 dropped and 0 starved in all twelve runs, 29.94 fps presented and 30.0
redraws/s throughout. The seam costs 0.06 ms of that and the rest is
structure: an earlier shape of the same shader, whose loop was bounded by
the file's lens count and so could not be unrolled, measured 1.82 ms at
yaw 0, because a loop that is not unrolled indexes its local arrays
dynamically and they go to scratch memory. Away from the seam the pass
takes the one texture fetch it always did, and writes the same bits: a
one-lens ONE X2 file renders byte for byte what it rendered before the
blend, at three yaws, and so does the front hemisphere of a two-lens file.

The windowed app over the same 60 s: zero dropped and zero starved in
every 5 s report, 30.0-30.2 redraws/s, 13.4% of one core and 295 MiB RSS
for the whole libcosmic process.

Rolling-shutter correction (issue #9) costs what a second pass through the
lens model costs, because that is what it is: the landing row is solved for
rather than computed, and each round of the solve is one more turn of the ray
and one more Mei projection per lens per pixel. Two 60 s runs each at
2560x1440, yaw 90 so the seam is down the middle, forced on through the
harness hook before the direction was known:

| pass, ms/redraw | measured |
| --------------- | -------: |
| correction off | 1.84, 1.85 |
| one round of the solve | 2.96, 2.97 |
| two rounds | 3.99, 3.97 |

0 dropped and 0 starved in all six runs, 29.97 fps presented and 30.0
redraws/s throughout. Switched off it was not nearly free but exactly free:
the same binary rendered two of three test views byte for byte against the
pass before the change, and the third differed in one channel of one pixel of
a million by one code, which is the compiler's scheduling of a refactored
function and not the map.

**Re-measured when it was switched on** (2026-07-31, an X4 reads down the
frame): five 60 s runs of one build, the same view, alternating the file's
own readout against `playback ... off`, which is the pass as it was before
issue #9. **4.00 and 4.23 ms off, 4.28, 4.82 and 5.00 on**, so about half a
millisecond per redraw, and 0 dropped and 0 starved in all five with 29.97
fps presented. Both arms are dearer than the table above because issues #10
and #11 changed what a redraw does; what the table above is still good for is
the shape, which is that a second round would cost as much again as the
first.

Hemisphere gating (issue #10), `cargo run --release -p kjerag-spike --bin
gating`. How much of the sphere a view could gate a lens off for at all, with
the body held still, and what the view axis has to be within for it:

| fov | cone half-angle | view axis within | of the sphere | of yaw/pitch |
| --: | --------------: | ---------------: | ------------: | -----------: |
|  20 |          11.4 deg |         70.7 deg |         67.5% |        54.1% |
|  45 |          25.4 deg |         56.7 deg |         45.6% |        33.6% |
|  90 |          48.9 deg |         33.2 deg |         16.5% |        11.0% |
| 110 |          58.6 deg |         23.5 deg |          8.4% |         5.5% |

And what a **locked horizon** does to that, which is the finding: over 40
parked views and 60 s of each of two X4 Air captures, with nobody touching
the mouse, the body's own swing takes the gate off and puts it back.

| fov | margin | gated       | releases/min | median run  |
| --: | -----: | ----------: | -----------: | ----------: |
|  45 |  0 deg | 48.4, 49.9% |     2.6, 4.5 | 0.60, 1.43 s |
|  45 | 15 deg | 32.8, 31.1% |     1.5, 2.4 | 1.77, 2.30 s |
|  90 |  0 deg | 24.3, 21.6% |     0.9, 2.6 | 1.90, 2.10 s |
|  90 | 15 deg |   9.4, 8.9% |     1.4, 2.0 | 0.57, 1.17 s |
|  90 | 30 deg |   0.9, 0.3% |     0.9, 0.3 | 0.47, 0.40 s |
| 110 | 15 deg |   3.2, 2.5% |     0.5, 1.0 | 0.70, 0.90 s |

**Read under the heading follow that shipped until 2026-08-06, and the world-
fixed lock makes every row of it worse.** The body now turns fully under a
parked view instead of partly, so the gate comes off more. Re-run on
VID_20260714_193252_00_006, before against after: at fov 90 and no margin
18.2% gated becomes 15.7%, at 15 degrees of margin 5.2% becomes 3.9%, at fov
45 and no margin 49.1% becomes 44.7%, and the longest single run over the file
falls from 14.21 s to 9.41 s at the default field of view. Releases a minute
barely move (5.5 to 5.6 at the default). **The decision the table gated is
unchanged and better supported**: an expected saving of 0.14 W was already too
little for a state machine and a packet ring, and there is less of it now.

Releasing a gate, packets held since the last keyframe and replayed, nothing
mapped on the way:

| gated for | held packets | catch-up      | frames stale |
| --------: | -----------: | ------------: | -----------: |
|     0.5 s |           13 | 195 to 204 ms |       6 to 7 |
|     2.0 s |           28 | 237 to 335 ms |      8 to 11 |
|    10.0 s |           28 | 280 to 339 ms |      9 to 11 |
|    30.0 s |           28 | 293 to 340 ms |      9 to 11 |

Screenshots (issue #15), 3840 px wide at the window's aspect. In the
windowed app, 20 s of playback with a still saved every 2 s and copied
every 7 s: zero dropped and zero starved in all four 5 s reports, 28.9 to
30.0 fps presented. Headless, five captures over 10 s
(`playback <file> 10 60 5`): 29.77 fps presented, zero dropped, zero
starved, and `prepare` costs 2.19 ms on the redraw that takes a capture
against 0.57 ms on one that does not (worst 6.26 ms against 3.29 ms). What
is left is the readback and the encode, and both belong to a worker
thread: 45 to 53 ms to encode one 8.5 MB PNG.

High-quality zoom sampling (issue #11), `cargo run --release -p kjerag-spike
--bin zoom`. How far the view magnifies the source, down the front lens's
axis at 2560x1440, and how far each plane's kernel engages there:

| fov | centre | corner | luma engaged | chroma engaged |
| --: | -----: | -----: | -----------: | -------------: |
|  20 |  0.152 |  0.150 |         1.00 |           1.00 |
|  60 |  0.499 |  0.435 |         1.00 |           1.00 |
|  90 |  0.864 |  0.626 |         0.18 |           1.00 |
| 100 |  1.030 |  0.686 |         0.00 |           1.00 |
| 110 |  1.234 |  0.743 |         0.00 |           0.86 |

Texels per output pixel: under 1 is magnifying. Two things to read off it.
The player is **already magnifying this camera at its own default view**, by
16% in the middle of the picture and 60% at the corners, before anybody
touches the wheel. And the chroma plane, at half the grid, is under 1:1 at
every field of view on offer, which is what cut its half of the upgrade.

What the two halves are worth, at 2560x1440 on real footage, as the mean
absolute Laplacian of the luma ("detail") and as the difference between the
pictures:

| view                          | bilinear | luma          | both planes   |
| ----------------------------- | -------: | ------------: | ------------: |
| ground and buildings, fov 50  |    4.120 | 4.606 (+11.8%) | 4.606 (+11.8%) |
| wing and lines, fov 50        |    1.126 | 1.214 (+7.8%) | 1.217 (+8.1%) |
| wing and lines, fov 25        |    0.676 | 0.688 (+1.8%) | 0.694 (+2.6%) |

| view                         | luma moves            | chroma adds          |
| ---------------------------- | --------------------- | -------------------- |
| ground and buildings, fov 50 | 1.835 codes over 85.4% | 0.412 over 39.9%    |
| wing and lines, fov 50       | 0.475 codes over 42.5% | 0.259 over 25.6%    |
| wing and lines, fov 25       | 0.453 codes over 40.1% | 0.268 over 26.5%    |

The upgrade is worth most in the **middle** of the zoom range and least at
the end of it, which is the opposite of what the issue expected: at 5x, on a
white canopy, the source has nothing left to resolve and the two kernels draw
the same ramp. Textured content at 1.6x is where a bilinear tent is
measurably the wrong shape.

The pass alone, one process, a still frame, the three settings interleaved
render by render so a laptop that throttles throttles all of them, least of
39 renders a cell:

| pass, ms/redraw | bilinear | luma (ships) | both planes |
| --------------- | -------: | -----------: | ----------: |
| fov 20          |     0.56 |         0.76 |        1.00 |
| fov 25          |     0.53 |         0.69 |        0.93 |
| fov 35          |     0.54 |         0.70 |        0.96 |
| fov 60          |     0.61 |         0.79 |        1.07 |
| fov 90          |     0.69 |         0.90 |        1.23 |
| fov 100         |     0.69 |         0.87 |        1.19 |
| fov 110         |     0.70 |         0.79 |        1.07 |

And under playback, which is the same pass with a cold texture cache and two
decoders running beside it (`--bin playback <file> 20 60 0 0 file <fov>
<setting>`). Every row here presented 29.9 fps with **0 dropped and 0
starved**; rows the box spoiled are left out, and the box was shared for part
of this session, which is why the controlled table is the one above.

| pass, ms/redraw | bilinear | luma | both planes |
| --------------- | -------: | ---: | ----------: |
| fov 20          |     2.07 | 2.83 |        3.39 |
| fov 45          |     2.06 | 2.52 |        3.41 |
| fov 90          |     2.23 | 2.47 |        4.68 |
| fov 110         |     2.23 | 2.40 |        3.57 |

Three properties, measured rather than argued:

- **It touches only what is magnified.** At fov 110 and 2560x1440, 47.7% of
  the picture is under 1:1 and 20.1% of it moved; the least magnified pixel
  that moved sits at 1.0008 texels to the pixel, where 1.0 is the switch-off.
  Rendered small enough that the whole picture is past 1:1, at 640, 960 and
  1280 px wide, the shipped setting is byte for byte the picture bilinear
  drew. Upgrading chroma as well is not byte-identical even at 960 px wide.
- **It does not pop.** Over 71 one-degree steps of zoom, the shipped setting
  is 0.179 codes from bilinear at fov 110 and 1.838 at fov 40, and the
  largest single step in that difference is 0.047 codes, 2.5% of it, where a
  kernel switched on rather than mixed in would put all 1.838 in one step.
  Step for step the picture itself moves 1.050x as far sharp as bilinear at
  the median and 1.063x at the worst, spread along the whole sweep: a sharper
  picture moving, not a kernel arriving.
- **A still gets it without being told** (issue #15). A 3840 px capture off a
  2560 px window magnifies 1.5x harder (0.293 texels to the pixel against
  0.440) and is byte for byte a 3840 px render of the same view, because the
  magnification is read off the hardware's quad derivative in whatever target
  the pass is drawing into rather than out of a resolution in the uniform
  block.

Seeking, 12 places from 1% to 97% of the 37.9 GB file, warm
(`cargo run --release -p kjerag-spike --bin seek`):

| what                          | median   | worst    |
| ----------------------------- | -------: | -------: |
| open the file (moov, once)    | 70.6 ms  |          |
| `av_seek_frame` alone         | 0.1 ms   |          |
| keyframe seek, reader         | 21 ms    | 49 ms    |
| exact seek, reader            | 230 ms   | 447 ms   |
| keyframe seek, through Player | 26 ms    | 54 ms    |
| exact seek, through Player    | 237 ms   | 473 ms   |

The worst case is not the far end of the file: it is whichever seek runs
first after the decoders warm up, and 97% costs the same as 1%. The player
used to cost 59 ms against the reader's 21, because the decode thread
finished the lookahead it had started behind the previous landing before it
read the next command; issue #46 made that read interruptible and the two
numbers above are what is left.

The same instrument measures a drag, which asks for a position per pointer
move rather than waiting for each picture, sweeping the file end to end for
2 s per rate:

| positions/s     | 10   | 15   | 20   | 30   | 45   | 60   | 90   |
| --------------- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| picture updates | 10.0 | 15.0 | 19.5 | 29.0 | 38.5 | 45.5 | 43.0 |

A picture reaches the screen while its own seek is newer than the position on
screen (issue #55), so a hand faster than a landing sees the landings it has
passed over: the rate rises with the hand to the decoder's own ceiling of
about 45 a second, which is one keyframe decode each. Under the rule that
shipped before #55 the same sweep read 10.0, 15.0, 12.5, 10.5, 5.0, 0.0 and
0.0, the last two being a frozen picture for the length of the drag. The
release lands on the exact frame every time either way.

## Ideas parked (complexity needs an observed failure first)

- Decoded-GOP cache in GPU memory for instant reverse scrubbing
  (~44 MB/frame; a 30-frame window is ~1.3 GB).
- Vulkan Video decode (drops VA-API plumbing; blocked on wgpu exposure
  and Rust HEVC support anyway).
- Batch screenshot/export queue across multiple files.
