# Roadmap

> **History:** the complete roadmap and experiment log through 2026-09-12 is
> preserved verbatim in
> [ROADMAP-HISTORY-20260912.md](ROADMAP-HISTORY-20260912.md). Date-based links
> from older research and source comments refer to that record.

This is the current product and delivery map. GitHub issues remain the work
queue. Update this page in any PR that changes product status, qualification or
release state.

## Product direction

Kjerag is a native COSMIC/Rust player for 360 camera files. The product priority
is smooth, zero-config playback with a correct horizon and Studio-like visible
stitching. Exact numerical reproduction of Insta360 Studio is useful evidence,
not the shipping requirement.

The selected design is one shared GPU stitcher with camera-specific calibration
and geometry. Qualified ONE X2 and X4 Air routes use hardware decode, GPU-resident
source preparation, stitching at source cadence, and reuse of the completed
stitch result across view redraws. The detailed ownership and frame path are in
[ARCHITECTURE.md](ARCHITECTURE.md).

The architecture guide now separates the active filtered, resident spatial and
generic frame paths. Its complete prior text is preserved verbatim in
[ARCHITECTURE-HISTORY-20260912.md](ARCHITECTURE-HISTORY-20260912.md), including
rejected experiments and historical measurements. This documentation cleanup
changes no released code, accepted picture or performance qualification.

## Current delivery

October 9 playback-stats option
([#265](https://github.com/aeharding/kjerag/issues/265)): the owner requests a
toggleable on-video buffer/playback overlay. The branch adds a stock `View >
Playback stats` checkbox and `Ctrl+I`, off by default and remembered. Bare `I`
still copies the view reference. A fixed, input-transparent stack layer leaves
the video viewport and camera unchanged and remains visible when controls hide.
Opt-in 500 ms snapshots report source promotions/checks, compressed-input lead
and bytes, retained decoded successors, completion-proven stitched FIFO lead,
live audio queue and callback health. Source progress is not physical display
FPS, and compressed lead is not completion-proven playback runway. Diagnostic
try-locks report unavailable rather than wait, consume queues, change wakes or
poll the GPU; seeking masks unacknowledged input lineage. No stats timer runs
with the option off or no video open. Formatting, workspace Clippy, vendor
warnings, name/source checks and the device/driver-hidden workspace pass
(1,718 tests, zero failures, 53 ignored; unavailable-device returns are not
hardware coverage). The owner confirms normal desktop behavior after the earlier
dock/display warning, allowing bounded one-at-a-time checks to resume. The native
X4 overlay regression passes nine checks: real held picture, default-off and
saved toggle, unchanged video pixels and view, panning through the panel,
preference persistence and clean exit. Actual playing/fullscreen captures are
retained for inspection. The regression is part of `scripts/uitest.sh` and can
run alone with `KJERAG_UITEST_ONLY=playback-stats`. All eight CI jobs pass on
`88897ef7`; the hardware-hidden SDK build succeeds. SDK app-path preflight passes
the full X4 suite (59 checks) and focused ONE X2 overlay suite (9 checks), with
actual pictures inspected. The exact SDK package is installed from a separate
signed local test origin: app commit `df699be5fdb1`, executable SHA256
`496c171baaac`. Actual-installed overlay suites pass nine checks on each camera
without an app-path override, with playing pictures inspected. No new kernel
entries or scoped memory-limit/OOM events are recorded. Official release trust,
shared runtimes and permissions are unchanged; signed 0.3.4 is retained for
rollback. Normal automatic Kjerag updates pause while following the local test
origin. The initial installer wrapper stops because Flatpak adds a blank INI
separator; its failed receipt is retained, and verification of actual trust
fields and the installed payload succeeds separately. Owner review remains
pending. This is not full ONE X2 UI-suite, sandbox-audio, ARM-playback,
network-smoothness or rendering-capacity qualification; no merge or release.

October 9 hover-bar release ([#261](https://github.com/aeharding/kjerag/issues/261)):
the unchanged, authenticated 0.3.3 Flatpak reproduces the owner's picture jump.
The stock header consumes 48 logical pixels; its appearance changes the video
rectangle from 1280x720 to 1280x672 at y=48 while the camera stays unchanged.
The owner confirms the recorded down/up jump. The candidate keeps the stock
COSMIC shell and projects only its video against the full window viewport,
clipping to the visible content. Mouse rays use the same projection; header
input cannot start a video grab or zoom. There is no media or stitching change.
Three device-hidden widget regressions pass for draw/input agreement, changing
header heights, header hit testing and real window resizing. The actual-input
regression rejects all three released header transitions, then passes three
actual pointer wakes on each of X4 Air and ONE X2, with captured stock headers
and no new kernel entries. The device/driver-hidden workspace reports 1,705
passes and 53 ignored; unavailable-GPU paths are not hardware test coverage.
Formatting, workspace Clippy, vendor warnings and source/name checks pass.
The full native X4 UI suite passes 60 checks, with no new kernel entries or
scoped memory-limit events. The owner tests the frozen branch executable
(`a67f6506` runtime source, SHA256 `723a745d7a89`) and accepts it as "looks good",
requesting normal review and merge. Review of the fix finds no outstanding
defects; all eight exact-head CI jobs pass. PR
[#262](https://github.com/aeharding/kjerag/pull/262) merges at `f7eade02` with a
tree identical to the checked head. The exact-runtime-source SDK candidate
passes 50 X4 app-path UI checks before publication, with actual moving pictures
inspected. Standard native release hooks pass 60 X4 and 64 ONE X2 checks.
Tag `0.3.4` names `20bd8a8c`; only version/lock/metainfo metadata changes after
the reviewed merge, and the offline source list regenerates identically.

Both signed builds, tagged-source CI and assembly pass. Initial publication
fails after creating an empty draft with "created release cannot be found";
publication-only retry succeeds with the original signed artifacts. The failed
attempt is retained, not relabeled green. This remaining publisher defect is
tracked separately in [#263](https://github.com/aeharding/kjerag/issues/263).
Both architectures' exact download/channel app and Debug commits, canonical
HTTPS release record, permissions, license, metainfo and FFmpeg linkage
authenticate against the preexisting release key. No ARM execution is claimed.

Signed 0.3.4 installs at x86_64 commit `5f4935d4c47f`, executable SHA256
`602d7ea5ed09`, preserving the canonical signed `kjerag` update origin, shared
runtimes, manifest permissions and trust. Verified 0.3.3 is retained for rollback.
The normal app-only channel update retains the exact bundle commit. Canonical
HTTPS summary bytes and their signature container match the authenticated sealed
publication; both architectures' public app and AppStream refs are present.
Actual-installed suites pass 50 X4 and 54 ONE X2 functional checks, with both
moving pictures from each inspected. The original hover-bar regression passes
all three actual pointer wakes on each camera, retaining full-window projection
and unchanged camera values. Scoped memory-limit/OOM and CPU-quota-throttling
counters are zero. Existing sound-device, preload-import, exact-view and X4
single-file-pair skips remain. During the ONE X2 suite, the kernel logs an AMD
display `REG_WAIT` timeout around a dock/monitor connection, followed by hotplug
messages. No desktop-freeze cause or harmless-warning verdict is established;
further GPU runs stop pending desktop-health confirmation. These are functional
UI results, not a new playback-capacity, universal network-smoothness or
all-camera qualification.

October 9 follow-up ([#258](https://github.com/aeharding/kjerag/issues/258)):
the owner's actual 0.3.2 Play failure is traced to an
empty audio ring retaining nonzero gain during a pending reset. The callback
cannot fade without PCM, while the reset refuses refill; coordinated playback
therefore waits indefinitely despite six completed successor pictures. The
fix finishes silence-target fades when PCM runs out,
preserving ordinary running gain, epoch rejection, timestamps and all stitching
arithmetic. New callback and Player/independent-producer regressions cover the
observed failure. The device/adapter-hidden workspace reports 1,702 passes with
53 ignored; unavailable-GPU paths are not hardware coverage. The exact-source
SDK candidate passes 50 X4 and 54 ONE X2 functional app-path checks, then is
installed through an owner-approved separate signed local test origin. Normal
automatic updates are paused for that test; official release signature trust
and the signed rollback remain intact. An actual-installed original network-clip
check uses real audio callbacks routed to a null sink, lands three clipboard
seeks and continues playback without counted underruns, new kernel entries or
scoped memory-limit events. The owner reports "seems fine so far" and explicitly
authorizes shipping without waiting for extended intermittent validation.
PR [#259](https://github.com/aeharding/kjerag/pull/259) merges at `59fd56e1` after
all eight CI checks pass. **0.3.3 is published and installed**, with tag source
`07edcdec`. Both signed native builds and assembly pass. Publication attempt 1
cannot rediscover its newly created empty draft; a failed-job-only retry succeeds
using the unchanged signed artifacts. No rebuild, asset replacement or tag change
occurs. Pages deployment succeeds, and the initially stale public release marker
is retained before a fresh fetch authenticates the new signed record.

Both architectures' downloads and channel commits authenticate against the
preexisting release key. Installed x86_64 commit `fc5fe232a94b`, executable
SHA256 `1339e0f13ce8`, follows the canonical signed `kjerag` remote again.
The bundle installer retains the disabled test-origin name, so an explicit
reinstall from the existing official remote restores normal updates at the same
commit. Both installers exit normally; shared runtimes and permissions remain
unchanged. Actual-installed suites pass 50 X4 and 54 ONE X2 functional checks,
with both moving pictures from each inspected, no new kernel entries and zero
scoped OOM/limit or CPU-throttling counters. Existing sound-device, import-fault,
exact-view and X4 single-file-pair skips remain. This fixes the reproduced Play deadlock, not
all network stutters or the 240-capacity goal. The separately requested hover-bar
picture-jump fix is not included.

Earlier October 9 checkpoint: **0.3.2 is published and qualified**, following the owner's October 8
acceptance of `c524ad0e` as "acceptable enough for release" and explicit merge/
publication approval. Cumulative PR [#254](https://github.com/aeharding/kjerag/pull/254)
merged at `efdd18a9`; tag `0.3.2` names `08f0246a`. Application sources remain
the accepted candidate's, with only release version metadata changed. The two
parked color files are excluded. Native release hooks pass 60 X4 and 64 ONE X2
checks. Both signed native builds, all eight CI gates and payload sealing pass.

The original release run `37941712593` remains **failed**, not relabeled green:
GitHub suppressed the public signing-key job output, then local recovery exposed
the publisher's draft-lookup bug. The exact same-run signed payload was separately
authenticated against the already trusted channel key. All four download files
were uploaded without replacement, downloaded and compared before publication.
PR [#255](https://github.com/aeharding/kjerag/pull/255) deployed the identical
signed repository; Pages run `37946013169` passes. No rebuild or tag change occurred.
Issue [#256](https://github.com/aeharding/kjerag/issues/256) corrects those two
automation defects without changing the installed app or release payload.

Installed x86_64 commit `963a8fd5adc2`, executable SHA256 `a2c1149d82ab`, follows
the official signed HTTPS channel again, with shared runtimes unchanged. Actual
installed suites pass 50 X4 checks after bundle installation and 54 ONE X2 checks
after the signed-channel update confirms the same commit. Both motion captures
from each suite are inspected; no new kernel entries or scoped OOM events occur.
The bundle installer itself exited with a libgobject protection fault after
deploying the intended commit; that anomaly is retained, not a clean install-exit
claim. The subsequent signed-channel update exits normally. ARM signatures,
metadata and public refs are checked, not ARM GPU playback.

This release does not close #186, certify all network conditions or meet the
240-capacity target. The scoped 188-redraw/s result and occasional buffering
hold remain recorded. Functional UI checks are not physical-scanout, audible
continuity or universal hitch-free qualification.

PR [#183](https://github.com/aeharding/kjerag/pull/183) is the cumulative GPU
stitching delivery. It merged at `53ecc929` after all six fresh CI checks passed
on final cleanup head `2afef25c`, followed by 50 native X4 UI checks. Tag
`0.3.0` names release commit `c21afcd0`; its dry run and execution each passed
47 real-footage UI checks. All jobs in release workflow `34678413630` passed,
and the GitHub bundles and signed channel are public. Post-publication package
qualification remains open in [issue #187](https://github.com/aeharding/kjerag/issues/187);
publication alone is not a completed qualification verdict. Packaging-only
PR [#188](https://github.com/aeharding/kjerag/pull/188) then merged at `4b3bcbf1`
after review and all six CI checks. Its **0.3.1** tag names `1fb97b12`; dry run
and execution each passed 47 real-footage checks. All ten jobs in its release
workflow `34682255167` passed; both downloads and the signed channel are public.

The owner tested the installed Flatpak built from source
`48e1d741f1bf7e7d8882624dff87754227bb8c7d` and accepted the actual branch
player: "looks good. not perfect but pretty damn good." That acceptance covers
the reviewed installed result, including source-owned world-coordinate temporal
correction. It does not establish exact Studio parity, all-camera coverage or
hitch-free playback.

The delivery includes:

- automatic GPU stitching and lens color matching for the qualified ONE X2 and
  X4 Air routes;
- source-owned alignment, temporal correction and photometric controls, paired
  exactly with the source frame that produced them;
- horizontally periodic temporal processing and rotation-stabilized world
  coordinates independent of mouse direction, window size and horizon display;
- first-picture preparation and responsive seeks, with completed GPU work and
  decoder resources retired by completion rather than by submission;
- readable reference implementations, real-path regressions and retained
  research evidence.

The exact package identities, accepted tradeoffs, test qualifications, evidence
paths and release review are recorded in
[MERGE_READINESS.md](MERGE_READINESS.md). That page is the authority for claims
about this delivery; private captures and receipts remain in ignored `scratch/`
paths and are not release assets.

## Qualification summary

Issue [#251](https://github.com/aeharding/kjerag/issues/251) covers the
owner-confirmed startup arithmetic refusal on the current Flatpak graphics
runtime. The exact failing Gaussian fixture word distinguishes one-rounding
multiply-add from an unfused two-rounding result. WGSL permits the latter;
our existing compiler fork now marks explicit SPIR-V Fma results with
NoContraction rather than relaxing the exact startup guard. A CPU regression
detects the previously missing precision decoration. The same focused
front-end GPU qualification fails before and passes after the compiler edit
on unchanged Flatpak Mesa 26.2.2, with no new kernel entries or memory pressure.
The patch uses native GPU arithmetic, not software emulation. Source `56232400`
passes 1,697 device-hidden workspace tests with 53 ignored, all local CPU gates,
a native release build, and all eight CI jobs. The clean committed-color SDK
package passes 49 X4 and 50 ONE X2 functional UI checks and is installed at
OSTree `250b429d3e69`, executable SHA `973fae2d5337`. Real screenshots are
inspected; no new kernel faults, scoped memory-limit events or CPU throttling
occur in the camera suites. The original September 23 network clip passes
startup with no counted audio underruns and near-source-cadence warm processing.
The actual-installed finite input-interruption check also passes startup and
resumes, without counted underruns. That unpaired run starts at a different
time/view from the earlier control and records three additional processing
holds just after its initial seek. A separate ordinary network control still
has roughly 47–55 ms completed-picture intervals. Neither its averages nor the
completed-render receipts establish physical scanout, audible continuity,
universal smoothness or the 240-capacity target. Owner branch retest and the
broader network-stutter requirements remain outstanding. No shared runtime
downgrade, merge or release. Draft [#252](https://github.com/aeharding/kjerag/pull/252)
is stacked on the approved conditional-pause test in #250; the compiler patch
is an owner-fork-only draft PR, with no upstream interaction.

The `fix/filtered-view-startup-preparation` follow-up under issue #186 moves
existing corrected-view shader/pipeline construction from the first completed
picture's draw to initial renderer attachment while autoplay remains held.
It uses the actual window format and the existing immutable pipeline cache.
A bounded real-X4 regression fails before this change and passes after it;
the corresponding ONE X2 check also passes. Both after runs have no new kernel
entries or memory-pressure averages. These native checks include the unchanged
parked color variation and prove preparation order, not clean SDK provenance
or a general hitch fix. The full bounded device-hidden workspace passes 1,699
tests with 53 ignored; all CPU gates and the native release build pass.
All eight CI jobs pass on runtime source `039c0a31`; draft
[#253](https://github.com/aeharding/kjerag/pull/253) is stacked on #252.
The clean committed-color SDK package passes 49 X4 and 50 ONE X2 functional UI
checks, with the existing service/fixture skips, authenticated executable
identities, no new kernel entries, memory-limit events or CPU throttling.
Actual output pictures are inspected. It was installed at OSTree `3f02b2bf26fe`,
executable SHA `1abb8f35f623`, retaining working `56232400` for rollback without
changing the shared graphics runtime, origin or permissions. The actual-installed
original September 23 `_003` network/view check runs near source cadence with
no buffer holds or counted audio underruns. These are scoped functional results,
not audible-continuity, physical-scanout or 240-capacity qualification.
A matching `_002` forced-input-interruption pair gives one hold in the candidate
and five in the earlier installed control. The control's post-resume map work
includes 53–80 ms wall-time samples, while the candidate's selected interval
stays below 36 ms. Run-to-run input/GPU variation is not isolated: shader
preparation alone is not established as the cause of that recovery difference.
Both whole-run cadence verdicts remain false, including frameless startup.
The random network-stutter goal and owner branch acceptance remain open.

The experimental `fix/completed-picture-recovery-runway` follow-up under #186
starts from the current delivery source. Two bounded native diagnostics in the
unchanged Flatpak graphics runtime split CPU submission from map completion.
One reproduces five natural picture holds well after an intentional input
interruption. Slow map transactions reach 91 and 83 ms, while all completion
polls/checks account for about 6 and 5 ms respectively. This excludes a long CPU
polling call for those samples, but does not identify a particular GPU kernel
or scheduling cause. The old two-picture restart reserve is about 67 ms at
30 fps. The candidate requires six completed successors after an actual hold,
with a bounded nine-successor decoded horizon to supply temporal dependencies.
Startup remains a two-picture gate, work admission remains four, source GPU
lifetimes remain two, and temporal history/source cadence/arithmetic are
unchanged. A strengthened real-X4 Scene recovery test fails before at only two
completed successors and passes after; ONE X2 passes after too. Both after
runs have no new kernel entries or memory-pressure averages. These native
checks contain the unchanged parked colors and prove readiness, not clean SDK
provenance or smooth network playback. More retained memory and potentially
longer holds were initially unaccepted tradeoffs. The earlier removed
three-picture prototype did not demonstrate a useful fix and is not acceptance
of this larger candidate. At that native checkpoint it was not installed;
clean package qualification follows below and owner retest remains due.
All 1,699 device-hidden workspace tests, 53 ignored, CPU gates and the native
release build pass. A bounded
old/new/old network-interruption comparison records respectively one initial
extra hold, no extra holds, and three initial plus two post-restart extra holds.
Every run has zero counted audio underruns and no new kernel entries or
pressure averages. All restart about 0.76–0.80 seconds after input returns,
estimated from adjacent presentation receipts; the deliberately unavailable
input causes longer total holds. Map work varies substantially between runs,
so absence of extra holds in the candidate is promising, not causal proof or
owner acceptance. Its deduplicated GPU-allocation snapshot is about 2,004 MiB
versus 1,669/1,738 MiB in the controls; these are not peak-memory measurements.
The actual candidate output image is inspected. These native comparisons
contain parked colors and do not replace clean SDK or owner testing.

Clean SDK source `c524ad0e` is built, excluding parked colors, and draft
[PR #254](https://github.com/aeharding/kjerag/pull/254) is stacked on #253.
All eight runtime-source CI jobs pass. The package passes 49 X4 and 50 ONE X2
functional UI checks, with the existing service/fixture and Rust-twin provenance
limits. Both suites have no new kernel entries, memory-limit events or CPU
throttling; actual pictures are inspected. Its bounded `_002` network outage
run resumes once with no extra holds or counted audio underruns, even while
two source-map transactions take 118 and 112 ms. It restarts about 0.66 seconds
after input returns; the deliberately unavailable input makes the total hold
about 5.07 seconds. The exact reported `_003` network view runs near source
cadence without holds or counted underruns and without a delay hook. All exit
normally with no new kernel entries. Outage pressure averages are zero except
0.02 over 60 seconds; the natural run's averages are zero. The SDK allocation
snapshot is about 2,016 MiB, not a peak-memory qualification. Full cadence,
audible continuity, 240 capacity and branch retest remain open. The owner then
explicitly approves installing this test with roughly 300 MiB extra GPU
allocation and potentially longer buffering holds. Source `c524ad0e` is installed
at OSTree `53263015fe29`, executable SHA `a39f4d521eec`; the working `039c0a31`
package is retained for rollback. Origin, permissions and shared runtime refs
remain unchanged. This is approval of the test tradeoff, not of its playback.

One bounded actual-installed check uses the exact September 23 `_003` network
view and the existing finite eight-second input interruption. It resumes after
the interruption with zero counted audio underruns, but also has a later
approximately one-second processing hold. Both holds resume; warm source
progression returns near 30 advances/s. Normal exit, actual executable identity,
output pixels and postflight with no new kernel entries or pressure averages
are verified. Whole-run cadence integrity remains false; no audible-continuity,
physical-scanout, universal network-stutter or 240-capacity verdict is inferred.
The owner subsequently accepts this installed build for release preparation
on October 8. This does not erase the additional hold or qualify general
smoothness. No merge or release at that checkpoint. Receipt:
`runtime/completed-runway-installed-moab003-outage-01`
under `scratch/playback-independent-20260927/`.

Packaged ONE X2 ordinary playback at the reported riser view also maintains
source cadence without holds or counted underruns, with no new kernel entries
or pressure averages. A bounded network `_003` changing-view capacity check
completes about 188 sourced redraws/s at 2256x1504 while advancing 30 consecutive
camera sources/s, with no holds or counted underruns. Completion-spacing
p99/max is about 22.92/33.65 ms. Cohort integrity passes, not the 240 target;
the result remains below that target and is not physical-scanout proof. Its
postflight has no new kernel entries or pressure averages; actual output is
inspected. No further code or installed-package change is inferred from this.

A separate unchanged installed network diagnostic clarifies the ordinary
47–55 ms picture intervals above: despite advertising 60 Hz, the isolated
headless compositor delivers callbacks at 61.921 Hz, median 16.045 ms. Two ticks
are about 32 ms and three about 48 ms, so 29.97 fps video needs occasional longer
dwells. This protocol trace has no presentation-feedback requests; concurrent
debug output corrupts one completion JSON record and the cadence parser fails.
It is not physical-scanout or smoothness qualification. The genuine initial
processing stalls, random network-stutter report, audible continuity and
capacity requirements remain open; no thresholds or source cadence are changed
to make the headless statistics look regular.

Issue [#229](https://github.com/aeharding/kjerag/issues/229) covers a malformed
saved volume reaching the audio callback. An isolated real COSMIC/RON config
test confirms that `NaN` is admitted, and a failing-before public Pipe test
shows positive samples inverted and amplified beyond full scale within 10 ms.
The media boundary now maps NaN to zero through its existing fade, preserving
finite and infinity clamps. The reproduction uses memory only, with no audio
device or GPU access; it is not an owner-reported audible defect or an
established cause of playback hitches. The device-hidden full workspace passes
1,598 tests with 53 ignored and no failures, including unavailable-device
returns rather than hardware coverage. Full formatting, Clippy, vendor warning,
naming and dependency-source gates pass. Independent source review approves
the bounded guard. PR [#230](https://github.com/aeharding/kjerag/pull/230)
merged at `b4af2fa8`; the qualified private Flatpak below includes the guard.

Issue [#227](https://github.com/aeharding/kjerag/issues/227) covers the generic
player's end-of-timeline seek. Its inclusive duration endpoint requested a
nonexistent source after the final frame, and terminal EOF/disconnection could
leave a seek marked outstanding. Deterministic tests reproduce both failures
through the actual Player pump. The candidate clamps known-length captures,
preserves requests for unknown-length streams, and retires terminal waits
without accepting a superseded epoch or hiding replay errors. The selected
resident route already clamps endpoint seeks. The real X3 scrubber reproduces
the old player's failure: its shown view stays at 84.885 seconds instead of the
final source at 85.152. Native candidate `965e35dd` passes six focused endpoint
checks each on X3, X4 Air and ONE X2, including stable final pictures, backward
navigation and return to the end. All candidates quit normally; the failed
baseline's forced cleanup triggered a private-compositor assertion, with no
new kernel entries or scoped memory-limit events. Captures were inspected.
The device-hidden workspace reports 1,595 passes, 53 ignored and no failures,
including unavailable-device returns rather than hardware coverage. These are
focused native checks, not full UI suites, Flatpak delivery, a performance
result or a stitching/color change. PR
[#228](https://github.com/aeharding/kjerag/pull/228) merged at `f0eee9e1`;
the qualified private Flatpak below includes the fix. The separate X3 horizon
review is unchanged.

Issue [#163](https://github.com/aeharding/kjerag/issues/163) corrects missing
evidence in the legacy `colour` profile diagnostic. Out-of-window controls,
unsampled lags and missing side fits no longer masquerade as measured zeros.
Each unavailable control is reported explicitly, and the excess requires all
four declared controls; their positions and the default reach are unchanged.
CPU regressions exercise the actual reducer, including failing-before cases,
real zero contrast and the complete four-control mean. This changes no player
picture, stitching/color arithmetic or performance qualification. The older
diagnostic's compacted sparse-bin spacing remains a separate limitation.

Issue [#151](https://github.com/aeharding/kjerag/issues/151), a camera drag
continuing after release over the scrubber, is reproduced in the accepted
`90721189` native player through real overlay routing. At one paused source
frame, releasing over the controls and then moving the same pointer without a
button changes the view; the Scene trace still reports an active grab. A queued
window-level release candidate was rejected before playback testing: iced can
process subsequent pointer events before applying its delayed release message.
The branch UI harness includes the observed same-pointer gesture and rejects a
missing initial drag or a moving source frame. The selected candidate replaces
the bar and volume-popup MouseArea shields with stock iced `opaque`: presses
remain shielded, while unowned releases reach the video in normal event order.
Global left-press activity preserves blank-padding timer rearming. No renderer,
media, dependency, stitching or color changes are needed. The tracked X4 native
regression fails on the unchanged player and passes on the candidate, with an
unchanged view after bare motion and normal candidate exit. The focused route
does not itself generate same-batch release/move/new-press input. The device-hidden
workspace passes 1,546 tests with 52 ignored and no failures, plus full Clippy,
vendor warnings, formatting, source/name checks, five startup regressions and
18 synthetic playback checks. Unavailable GPU/media paths are not hardware
qualification. Full native UI suites pass 52 X4 and 53 ONE X2 checks with no
failures; the existing portal, single-file-pair, cross-filesystem hard-link and
explicit-view-fixture skips remain. The owner then tested the frozen native
candidate from head `5b87ed90` and reported "Yes, fixed." That verdict covers
the reported drag-release defect in that exact candidate. It does not assign an
owner verdict to a later combined SDK build. PR
[#215](https://github.com/aeharding/kjerag/pull/215) subsequently merged at
`1d5e9e46`, with the accepted drag patch unchanged in final integration
`ddb19f04`. Its device-hidden gate reports 1,583 passes and 53 ignored;
all eight final-head and post-merge CI jobs pass. The integrated native suites
pass 53 X4 and 54 ONE X2 UI assertions. After the X4 wrapper rejected a routine
pointer-helper rebuild, the focused two-check drag run also passed with the
correct helper identity; the frozen player and source were unchanged.
The fix is now included in the qualified private Flatpak described below.
This is not a release or a seam/performance claim.

Issue [#86](https://github.com/aeharding/kjerag/issues/86) has a reproduced
post-decode pairing inconsistency: the analysis `Walk` matched raw timestamps
using its first source's clock, while `Reader` normalized each source's origin
and matched frame indices. A regression through an extracted production queue
adapter fails with starts 0/900 and passes after per-source normalization. The
candidate shares the alignment decision without changing Reader's lookahead,
mapping, stamps or scheduling. Capture discovery and sibling validation now
also share Reader's existing policy: named pairs stay in lens order, selected
companions take precedence, unsuitable discovered siblings fall back to one
lens, and unsuitable explicitly selected pairs are errors.

The device-hidden media gate passes 122 tests with four ignored and no
failures. Generated CPU-only MOVs exercise
real container admission, software decode and repeated forward/backward demux
seeks through Walk's production clock/queue adapter with starts 0/60000. Both
lenses deliver every expected frame after each cue. A separate H.264 MOV probe
found extra preroll with the existing raw seek target but no wrong at-or-after
packet; seek targets are unchanged. These checks do not cover hardware decode,
negative-origin containers or arbitrary-origin playback. The final-source
device-hidden workspace gate passes 1,566 tests with 53 ignored and no failures,
plus formatting, full workspace Clippy, naming and dependency-source checks.
Unavailable GPU/media returns are not hardware coverage.

The separate public Reader/Walk hardware comparison now passes on both named
X4 Air and ONE X2 captures. It checks 15 near-start deliveries per camera across
five forward/backward/repeated cues: indices, normalized times, lens counts and
nonempty downloaded Walk planes agree. This is not image equality, full-clip or
arbitrary-origin hardware qualification. Native player UI suites pass 51 X4 and
52 ONE X2 checks with zero failures, including visible playback, real late
seeks, pause/resume, spaced-path view restoration and import-failure recovery.
Both cameras' motion captures were visually inspected. The exact-view, portal
and cross-mount paired-fixture checks retain their documented skips; the
separate Radeon shader/Rust-twin test passes.

No new kernel entries or scoped memory-limit/OOM events appeared. GPU heap
counters varied across the UI suites; these are bounded functional checks, not
a resource-containment or performance verdict. The X4 outer wrapper rejected
the harness's routine pointer-helper rebuild after all 51 checks passed; both
helper identities were authenticated, and the frozen player and source hashes
remained unchanged. ONE X2 then passed the complete wrapper with that helper.
Private receipts remain in `scratch/frame-alignment-86-20260915/`.

Independent review confirms that production capture discovery, sibling
selection, admission and alignment now have single shared implementations,
meeting #86's capture-pairing scope without unifying decoder/delivery APIs.
PR [#216](https://github.com/aeharding/kjerag/pull/216) has all eight CI jobs
passing on runtime head `2df9eea1`; final documentation-head CI is a separate
merge gate. No installed app/package/release change, seam-quality change or
performance improvement is claimed.

Both actual 0.3.1 x86_64 distribution routes independently pass 40 X4 Air and
44 ONE X2 installed UI checks. Their reported views match the accepted
`48e1d741` package byte-for-byte, and both include the exact source LICENSE,
fixing the signed 0.3.0 omission. Permissions and runtime metadata are unchanged.
The documented sandbox sound-device and import-fault skips still apply.

The signed 0.3.1 baseline was installed from the official GPG-verified channel. Its
2256x1504, 40-second active-playback cohorts with 300 Hz requested view rate are:

| Camera | Completed redraws/s | Source advances/s | Completion p99 / max |
| --- | ---: | ---: | ---: |
| X4 Air | 258.199 | 27.075 | 14.757 / 21.956 ms |
| ONE X2 | 314.999 | 29.975 | 10.971 / 24.469 ms |

X4 exceeds 240 redraws/s but falls behind its 29.97 fps source, reaching
3.83 seconds of lag. The same slowdown was reproduced in 0.3.0 and the exact
restored accepted package. No runtime/shader change is included in 0.3.1, and
this is not established as a new-release-only regression. Earlier successful
cohorts remain historical evidence, not a current unconditional capacity pass.
MERGE_READINESS preserves every earlier cohort and the host/Flatpak comparison;
environment contributes to the difference but no unique cause is isolated.

Both architectures' app and AppStream refs are authenticated and published;
the x86 AppStream client reports 0.3.1. aarch64 install/playback is untested on
hardware, not inferred from querying its refs on this x86_64/i386 host.

An unchanged signed 0.3.1 recheck on 2026-09-13, with AC online and the battery
charging throughout, measures 285.024 completed redraws/s and 29.950 source
advances/s at the same 2256x1504, 40-second pan. Worst reported lateness is
10.2 ms; completion p99/max is 13.645/26.368 ms. That cohort meets the average
throughput/source requirement, but does not explain or erase the earlier
failures, prove hitch-free output, or establish a code fix. Native controls
also regain full source cadence while remaining below 240 redraws/s. Issue
#186 retains the unexplained variability and native/runtime difference.

The exact local gate counts, qualified Radeon environment, retained llvmpipe
failure and logs are in MERGE_READINESS. Final cleanup CI passed all six jobs in
run `34677590261`; tag CI passed all jobs in release workflow `34678413630`.

## Accepted tradeoffs and boundaries

The owner-approved tradeoffs are listed at the top of
[MERGE_READINESS.md](MERGE_READINESS.md#accepted-tradeoffs). In brief, the
accepted installed result can differ from readable CPU/global references through
reduced-resolution temporal correction, source-rate prefiltering, independent
patch-row alignment, GPU sampling and rounding, and history restart after seek.
First-picture preparation adds roughly 0.1 seconds to avoid a later activation
pause.

Frame-time spikes and sustained source lag are limitations, not accepted
performance tradeoffs. No seam-refresh cadence reduction or invented gradual
color-update policy is selected.

## Retired queue entries

- **Application identity, issue
  [#66](https://github.com/aeharding/kjerag/issues/66):** the owner-approved
  `dev.harding.Kjerag` identity landed in `9c878337` / PR #104. The app,
  Flatpak manifest and desktop, metainfo, icon and MIME resources agree;
  [DISTRIBUTION.md section 3.6](DISTRIBUTION.md#36-the-app-id-is-devhardingkjerag-and-the-whole-tree-says-so)
  records the settled choice. No new rename or settings migration is pending.
- **Direct pointer testing, issue
  [#84](https://github.com/aeharding/kjerag/issues/84):** the native Wayland
  pointer helper landed in `28e7e442` via PR #128. The UI harness now injects
  real clicks, held scrubber drags and drag-release gestures. This retires the
  missing-input instrument, not every UI defect or all input coverage.
- **Duplicate seam-pool samples, issue
  [#156](https://github.com/aeharding/kjerag/issues/156):** the fitted seam pool
  was removed in `c72579af`, delivered by PR #183. `ConfigState` retains only
  recent files and ignores old pool keys. This issue is obsolete by removal,
  not repaired by adding deduplication or reviving per-capture fitting.

The generic decoder-arrival wake is also merged, but issue #136 remains open:
its original NAS symptom is not established as fixed by that narrower change.
This queue reconciliation does not retire #186's performance requirements,
change installed code or add qualification claims.

## Remaining work

- **Playback scheduling and audio starvation, issue
  [#186](https://github.com/aeharding/kjerag/issues/186):** after the owner
  restored CPU boost by reconnecting external power, a fresh installed-player
  foreground run on the 4K display sustains consecutive source cadence without
  accumulating lag or reporting audio underruns. This short idle-view result
  does not establish the 240 fps target or resolve the owner's broader report.
  A separate desktop run receives redraws at about 1 Hz: its bounded
  video channel blocks the decoder, which also stops refilling audio despite
  the independently running media clock. The fullscreen video in that first
  desktop capture belonged to another app, and the measured surface was
  1331x998. The owner reports Kjerag was visible on the 4K display; without
  contemporaneous window-state evidence this must not be dismissed as hidden
  playback. An already-late sequential session does not discard its
  backlog when normal throughput returns.

  Branch work gives audio its own bounded producer, preserving decoding and
  correction arithmetic, with seek-authorized writes and explicit failure
  and shutdown handling. The filtered X4/ONE X2 branch path now also moves
  source admission and ordered completion out of renderer preparation into a
  playback event owner, with absolute deadlines and coalesced readiness wakes.
  The owner approved skipping obsolete
  completed screen updates to catch up with audio while retaining every source's
  stitching and color processing. No stitching inputs,
  source cadence, color law or installed package changes in this slice.
  The device-hidden media gate passes 137 tests with four ignored and no
  failures, including eight new producer/seek/lifecycle regressions. The
  starvation test uses the real bounded video-delivery loop and audio ring with
  a controlled PCM producer, not AAC decoding or a physical output device.
  Independent concurrency review found and corrected a startup authorization
  race. The integrated device-hidden working-tree gate reports 1,618 passes,
  53 ignored and no failures, including unavailable-device returns rather than
  hardware coverage. Full workspace Clippy, formatting, source/name checks and
  20 portable UI regressions pass. Those CPU-only gates did not qualify
  native/Flatpak playback. Generic/spatial video
  paths retain their existing scheduling, while audio supply is independent
  for all live Readers.

  September 29 qualification of exact source `68591c10` establishes a narrower
  real-player win. In the same X4 session at 2256x1504, eight seconds of 1 Hz
  compositor updates followed by restored 60 Hz causes the old package 735
  audio underruns and almost eight seconds of draw lag. The candidate reports
  no underruns, retains 988 contiguous completed source transactions, and
  restored draw age stays within 16.65 ms of its pre-slowdown anchor. These are
  authenticated draw/completion records, not physical scanout or a capacity
  qualification. Both full app-path UI suites pass, 43 X4 and 44 ONE X2 checks,
  with the documented isolated-service/fixture skips and inspected pictures.
  The private candidate is now installed; owner acceptance is pending.

  A separate delayed-packet test still reduces input to about 23.7 fps and
  accumulates 3.332 seconds of video lateness while audio stays supplied.
  Normal reading lets it catch up. This explicitly leaves slow-input buffering
  and sustained sub-realtime processing unresolved; do not describe the new
  scheduling as a complete A/V-sync fix. The owner was asked about holding
  picture and sound together to refill, but that policy is not implemented or
  accepted yet. Receipts: `scratch/playback-independent-20260927/`.

  The owner's September 20 clip at 1281.413 seconds, FOV 166.23, reproduces
  a severe failure in the installed `68591c10` player: 2.8 source advances/s,
  almost 18 seconds of accumulated video delay and hundreds of audio gaps.
  A later repeat reaches near source cadence but still accumulates 710 ms
  lateness. Live thread samples show audio and video waiting for GVFS reads;
  high GPU use on the wide view remains a separate unresolved factor. These
  measurements do not isolate the network or projection as the sole cause.

  Dedicated branch `refactor/playback-compressed-input` separates compressed
  reading from video decode and PCM refill with bounded per-file packet
  queues. It is based on the installed scheduling, not the unaccepted clock-
  hold prototype. Seven CPU regressions cover read-ahead during a blocked
  read, byte/count limits, in-flight seek invalidation, raw failures, shutdown,
  panic wakeup and byte/metadata identity against real libavformat input.
  The media gate passes 148 tests with four ignored and Clippy passes.
  No stitching/color arithmetic changes. Clean SDK source `88f0d350` is built
  but not installed. Its actual NAS/full-size wide-view run still accumulates
  912 ms lateness with 28.8-29.6 source advances/s and no audio gaps. Input
  read-ahead alone is not the fix; both-camera runtime/UI checks and owner
  retest remain due.

  A complete local copy isolates a separate rendering failure in the installed
  player. At the exact reported 166.23-degree view and 2256x1504, lateness
  grows to 654 ms. At 1128x752 with the same aspect ratio, camera pose and file,
  source cadence remains 29.8-30.0/s and worst lateness holds at 34 ms. Both
  controls have no reported audio gaps or new kernel entries. The smaller
  surface is a diagnostic, not an accepted picture-resolution reduction.

  Branch `perf/source-map-view-cache` moves native endpoint position and packed
  map evaluation from curved screen fragments to one source-owned GPU cache.
  It preserves full-resolution sampling, triangle/alpha rules, panorama input
  and source history; narrow mesh rasterization remains unchanged. A same-owner
  uncached draw is retained for rendered regression comparisons. The bounded,
  device-hidden working-tree workspace passes 1,626 tests with 53 ignored and
  no failures, plus full Clippy, vendor-warning, formatting, name/source gates.
  These results include unavailable-device returns and the separately parked
  color variation, not clean-package qualification. Separate bounded real-GPU
  Scene cases compare 31 consecutive sources per camera against the uncached
  draw of the same completed owner, with unchanged coverage. At the reported
  X4 wide view, 47 pixels across the 31 1280x720 captures differ by at most one
  RGB8 code; the ONE X2 riser sequence is byte-identical. A paired X4 frame was
  inspected. These working-tree tests isolate view arithmetic, not the parked
  color variation or actual surface performance. Subsequent clean SDK runs
  demonstrate only a limited win: the local stationary view maintains source
  cadence, but NAS delay still reaches 1.14 seconds. At 300 Hz changing-view
  stress, the cache reaches 210.5 completed redraws/s with only 18.47 source
  advances/s and 16.79 seconds of growing delay. At normal 60 Hz, the local
  pan maintains 29.95 source advances/s and worst lateness stays at 82 ms,
  but completion gaps reach 37.1 ms. None of this is a hitch-free verdict.
  The clean package remains uninstalled; both-camera UI qualification and
  owner retest remain due. Issue #186 stays open.

  The compiled-triangle follow-on was rejected after its clean package reached
  only 205.9 redraws/s and 18.12 source advances/s in the same changing-view
  stress, with continuing delay. Passing its two-camera pixel comparison did
  not establish a useful performance improvement. Its code is not included in
  branch `refactor/filtered-source-admission`.

  That branch replaces per-source shared-worker messages with one capture
  actor draining a bounded source queue. Admitted successors no longer require
  an intervening shell handoff; stitch/temporal work retains the existing
  two-source and four-output bounds. Busy refusal precedes graphics-device
  polling and GPU retirement reservations. A real-decoder/GPU regression holds
  the worker, admits two sources, then requires both to finish without further
  shell progression or renderer callbacks. The device-hidden render suite
  passes 1,119 tests with 43 ignored. The separate actual-decoder/GPU case at
  the exact September cue passes: both sources finish without further shell
  progress or test-side device polling. No new kernel entries, sampled memory
  pressure averages zero. Final device-hidden workspace passes 1,630 tests
  with 53 ignored, plus full fmt/Clippy/vendor/name/source gates. These include
  no-device returns and parked color variation, not hardware qualification.
  Final-source no-shell GPU cases pass on both cameras. Separate 31-source
  rendered sequences at the reported X4 view and ONE X2 riser view are
  byte-identical to their retained parent captures; first frames inspected.
  No new kernel entries and postflight pressure averages zero in those runs.
  Clean source `635e9b04` has all eight CI jobs passing. At2256x1504 its
  60Hz local pan keeps29.95source advances/s without growing lag, worst25.2ms;
  its NAS stationary-view repeat also keeps up. Neither is a general fix:
  the300Hz pan reaches only213.9redraws/s and19.4source advances/s, with14.1s
  delay. Separate app-path UI suites pass43X4 and44ONE X2 checks, with the
  documented service/fixture skips and inspected captures. It is installed as
  an interim test package, retaining68591c10rollback. The final actual-installed
  NAS60Hz pan FAILS: roughly20source advances/s, delay grows to10.91s, despite
  no audio underruns or new kernel entries. Its strict capacity parser also
  rejects a skipped screen source, separately from the genuine cadence failure.
  One successful cached range does not establish robust playback. No merge,
  owner acceptance, hitch-free verdict or complete A/V-sync fix.

  Retired branch `perf/curved-view-cell-hints` evaluated a renderer follow-on. A static
  indexed screen grid supplies only native cell search hints. The exact pixel
  ray must pass the existing watertight cell test; a missed hint uses the
  complete original search. No picture, lens UV, alpha or temporal field is
  interpolated from this grid, and no source work or cadence is removed. This
  unqualified candidate was not installed. Its clean SDK actual-player checks
  failed: NAS60Hz pan reached18.6s delay, local60Hz pan reached588ms, and
  local300Hz pan reached only193.2redraws/s,15.3source advances/s and20.1s delay.
  Conditions were not paired, so these do not establish causally slower shader
  performance. They do establish no useful fix. The extra draw code is removed;
  the corrected transparent coverage diagnostic remains. Evidence and source
  stay in Git and scratch, not a selectable production path.
  Device-hidden workspace gates pass1,632tests, with53ignored. Three separately
  bounded31-source Scene comparisons pass: the reported X4 wide view, ONE X2
  wide view and ONE X2 riser. The riser is byte-identical; X4 differs by at most
  two RGB8 codes. The X2 wide view additionally fills one reference coverage
  hole, with a49-code difference there, and one other three-code difference.
  Transparent-target checks report zero removed coverage in all three cases.
  This replaces the earlier opaque-black alpha check, which could not measure
  coverage. First rendered frames inspected; no owner quality acceptance or
  smoothness result. All three rendered runs have no new kernel entries.

  Removing PIS's storage-dependent zero barrier fails its existing adapter
  qualification by one terminal bit and is rejected before packaging. The
  barrier and all arithmetic qualification remain unchanged. Retired branch
  `perf/exact-power-two-division` evaluated an exact integer exponent
  shortcut for normal values divided by normal powers of two, only when the
  result stays normal. Subnormal/overflow cases retain the full divider.
  Its 841,492 adapter divisions pass bit-exact checks, but native full-size
  300Hz playback reaches only193.9redraws/s,18.2source advances/s and9.46s
  accumulating delay. This does not establish a useful fix. The shortcut and
  trial-only test expansion are removed before packaging.

  Branch `refactor/filtered-work-admission` separates bounded CPU work from GPU
  lifetime reservations: four decoded sources may queue, while only two GPU
  lifetimes remain in flight. Only the worker polls/reserves those slots, not
  the UI; its cancellation-aware wait holds no Scene/state lock. The existing
  four-output reservation and exact source/history law remain unchanged.
  Sixteen targeted CPU tests pass. The actual September decoder/GPU test admits
  and completes all four sources without further shell events or test-side
  GPU polling. Native2256x1504 local60Hz pan maintains29.96source advances/s,
  worst27.1ms lateness and no audio gaps. The300Hz pan still FAILS:
  212.3redraws/s,13.3source advances/s and13.95s accumulating delay.
  Neither run has new kernel entries. This structural change is uninstalled
  and does not establish a reliable playback fix or the240capacity target.

  A source-rate stitched-cube prototype evaluated producing one
  completed corrected image per source and reprojecting that immutable texture
  during view redraws, instead of repeating native map/lens fusion per pixel.
  It uses the existing corrected rectilinear mesh draw for six faces and keeps
  every source's stitching, color and temporal history. It adds RGB8
  quantization and texture resampling, with2560px faces for3840px lens inputs
  and1920px faces for2880px inputs. Separate31-source prototype comparisons
  completed on the reported X4 wide view and ONE X2 riser. First frames were
  inspected, and the X4 moving comparison was sent to the owner. Six-face GPU
  intervals average10.76ms on X4 and6.46ms on ONE X2, with maxima22.04/11.72ms;
  these isolated intervals do not establish actual-player capacity. Neither
  run has new kernel entries; sampled memory-pressure averages are nonzero.
  The integrated native300Hz pan FAILS:152.71redraws/s,9.96source advances/s,
  20.45s accumulating delay and53.74ms maximum draw interval. No new kernel
  entries were logged. The prototype and its live integration are removed,
  with source/binary receipts retained in scratch. No owner acceptance or
  packaging is claimed.

  A subsequent prototype evaluated caching only lens coordinates and the native chart
  in six512px float32 faces, during the existing source-preparation encoder.
  Curved drawing interpolates this geometry while retaining full-resolution
  source sampling, alpha, photometric and temporal correction. It adds no
  submission/wait and leaves rectilinear drawing unchanged, but adds50.3MB
  GPU memory per retained source and a disclosed interpolation tradeoff.
  Producer/combined shader CPU checks and31 real September sources pass, with
  no lost coverage or new kernel entries. Pixels differ (maximum93RGB8codes),
  and no owner quality acceptance is claimed. Native300Hz pan FAILS:
  177.92redraws/s,15.50source advances/s and12.61s accumulating lateness.
  The prototype is removed; source and both binaries are retained in scratch.

  The worker's five intermediate queue-prefix waits also wait for unrelated
  view submissions. Removing those CPU round trips preserves the six bounded
  command chunks, GPU ordering and final validity ownership. With the coordinate
  prototype still present, local300Hz pan improves source cadence to28.87/s and
  worst872ms lateness, but achieves only112.54redraws/s with48.04ms maximum draw
  interval. This is not the capacity target or a reliable playback fix. Local
  60Hz pan maintains29.92source advances/s with33.3ms worst lateness; NAS60Hz
  pan still falls to roughly25source advances/s and4.46s lateness. The NAS
  strict capacity parser rejects an omitted completed screen update, which is
  owner-authorized; the independent cadence failure remains. No new kernel
  entries. A packet-only NAS read sustains roughly35source frames/s.
  Cube-free continuation passes full device-hidden CPU gates (1,629 tests,
  53 ignored). A following decoder-wake guard avoids arming input-ready wakes
  when source admission is full; six targeted CPU tests and Clippy pass.
  Its actual NAS60Hz pan still FAILS at24.4-25.6source advances/s and4.354s
  worst lateness. Source import averages0.362ms and map-valid preparation
  22.34ms. The mean gap between panorama submission and the next worker source
  is16.27ms; this is not a sole-cause verdict. No new kernel entries.

  The next unqualified change removes the temporal worker's per-output shared
  queue-prefix wait. A private four-byte completion marker follows each final
  output submission; no image bytes are read. The executor records ordered
  successors while polling a bounded completion monitor, including while
  CPU-idle. Exact FIFO-front completion still gates installation, startup and
  seek acknowledgement. This changes scheduling, not image/source/history laws.
  The real-decoder no-shell regression now covers seven-source startup and GPU
  output completion. Build and runtime qualification are pending. No installed
  change, package qualification, owner retest, merge or release yet.

  The per-output completion change passes its real seven-source no-shell
  regression (no new kernel entries), but NAS60Hz pan still FAILS:25-27source
  advances/s,3.635s worst lateness. Local control maintains29.92source advances/s
  and27.4ms worst lateness, with61.21redraws/s and26.81ms maximum draw interval.
  The high local progress count follows the harness's1,000Hz mouse input,
  not a proven decoder-wake loop. App dispatch previously reran the scheduler
  for every UI message. Restricting it to media/worker `SceneReady` events
  passes full device-hidden CPU gates (1,631 tests,53 ignored), but NAS60Hz
  still FAILS after initially maintaining30fps:27.6-29.6/s later and988.7ms
  accumulating lateness. Neither result establishes smoothness or240capacity.

  Retrying six-face RGB8 source materialization after removing shared-prefix
  CPU waits still FAILS NAS60Hz pan:20.58 source advances/s,9.63s accumulating
  lateness and94.75ms maximum draw interval, with memory pressure. That cache
  is removed and its source/binaries retained in recovery artifacts. Direct
  corrected-source drawing is restored. No cache image tradeoff was accepted.
  Neither the restored candidate nor installed635 is a playback fix.

  Exact-cue dual-lens Reader controls without stitching/rendering decode900
  pairs at37.5/s from NAS and70.6/s from its local copy. Both exceed29.97fps
  in those bounded runs; neither proves input-jitter immunity. No new kernel
  entries or memory pressure. An existing seven-source timestamp regression
  isolates warm map work at6.76-11.14ms (including stage-submission gaps),
  source snapshot preparation at5.81-9.52ms and temporal output at1.43-2.44ms.
  These are diagnostic intervals, not a native-player capacity verdict.

  The next unqualified renderer candidate rasterizes a subdivided sphere to
  locate native cells on front-facing curved views. Fragment sampling still
  uses the original exact cell test and full fallback; no RGB cache or reduced
  picture resolution is selected. Rear/ball views retain the original path.
  Source/history/color behavior stays unchanged. CPU shader validation passes;
  coverage, moving comparison and actual-player capacity remain pending.

  The first sphere-rasterizer revision FAILS pixel inspection with severe
  overlapping geometry. Its alpha-only coverage test reports no holes but
  cannot detect overwritten opaque pixels. NAS runtime also fails and is
  rejected; no new kernel entries or pressure. The next revision removes
  negative perspective-w folding, projects rear vertices to a finite rim and
  rejects rear fragments before picture sampling. It is rebuilding, not a fix.

  The corrected broad phase passes31-source coverage and pixel inspection,
  differing by at most3RGB8codes, but still FAILS native NAS60Hz pan:
  17.4-19.6source advances/s,6.126s accumulating lateness. It is replaced,
  not kept as a selected alternate. The next candidate follows native triangle
  positions/packed coordinates with four edge subdivisions and lets hardware
  interpolate them on front-facing curved views, replacing fragment ray
  intersections. This is an explicit subpixel sampling/projection approximation;
  the previous broad-phase quality result does not qualify it. Full-resolution
  originals, source/history cadence, alpha/color and residual laws remain.
  Its moving-image and actual-player qualification are pending.

  The first native-mesh31-source comparison passes coverage at16:9 and differs
  by at most6RGB8codes, but NAS60Hz still fails. Inspection exposes a selection
  error: the actual fullscreen1.5aspect after controls hide has slightly
  rearward corners at166.23deg, disabling the front-only fast path. The runtime
  result therefore does not isolate native-mesh performance. The replacement
  clips hidden cells outside the visible cone before the curved projection
  singularity, with a native-cell safety margin, covering both header-visible
  and full-window states. The real-source comparison now captures1.5aspect.
  Final moving coverage/quality and real-player capacity remain pending; no
  build is installed and no new quality tradeoff is accepted.

  Corrected full-window native-mesh images pass31-source coverage and differ
  by at most8RGB8codes, but actual playback still FAILS: local pan falls to
  about16source advances/s and14.23s worst lateness; NAS pan is worse and
  eventually underruns sound. There are no new kernel errors or memory pressure.
  Neither storage nor renderer selection alone explains the failure. The next
  architectural correction ties source-snapshot retirement to a unique mapped
  submission marker, not a shared queue callback that can attach to concurrently
  submitted display work. CPU tests cover pending-proof refusal and raw-error
  quarantine. GPU/actual-player qualification remain due. No install or merge.

  Source-specific retirement passes the real seven-source no-shell decoder/GPU
  regression. The native local-copy2256x1504 player now maintains30source
  advances/s during ordinary playback and29.94/s under60Hz pan, without
  accumulating lateness or audio underruns. The previous local mesh-only pan
  managed about16/s and14.23s lateness. Uncapped local pan maintains30.00
  consecutive source advances/s with155.06completed redraws/s; callback
  p99/max27.33/32.78ms. This still FAILS240/4.17ms capacity. Network pan still
  FAILS at roughly3.6-6source advances/s despite a contemporaneous decoder-only
  control reaching34.2paired frames/s. Do not claim the input architecture is
  fixed or blame network bandwidth alone. ONE X2's31-source riser comparison
  is byte-identical to its uncached reference, with no removed coverage.
  Workspace CPU gates and both-camera moving coverage pass. Native X4 UI
  qualification passes 55 checks with zero failures;
  all eight CI jobs pass on committed head `dd908324`. After trying the frozen
  native preview, the owner reports "Performance looks much improved" and,
  when asked to check the moving picture/seam and audio sync, "Good enough".
  This accepts that preview for the next delivery step, not network playback,
  the 240 fps capacity target or arbitrary-camera coverage. Its parked periodic-color
  optimization is preserved but excluded from the commit. Clean committed-source
  SDK/Flatpak verification subsequently passes 43 X4 and 44 ONE X2 UI checks,
  zero failures, with the documented isolated-service/fixture skips. The real
  exported bundle passes executable, metadata and license authentication in a
  separate device-hidden import. The clean package is now installed, retaining
  the 635 rollback; origin, permissions and shared runtimes are unchanged.
  Actual-installed local 2256x1504 pan maintains 29.94 consecutive sources/s,
  worst 45.1 ms lateness, zero audio underruns and no new kernel entries. Clean
  package NAS pan still reaches 6.05 s growing delay, and 300 Hz local pan reaches
  only 129.31 redraws/s with 29.44 sources/s and 236.9 ms worst lateness. These current
  failures do not erase the native preview win or satisfy 240 fps capacity. No merge
  or release. Exact identities and qualifications are in MERGE_READINESS.

  September 30 network follow-up isolates an input contribution: the installed
  package falls behind with normal NAS audio/video; removing only its private
  sound access holds source cadence; restoring sound fails again. Keeping sound
  active but redirecting only its input to the authenticated local counterpart
  also holds cadence. This supports sharing file bytes below the independent
  demuxers, not muting sound or combining the two timelines. The branch adds one
  file handle and a bounded 16 MiB byte cache with independent AVIO cursors.
  Initial native NAS pan holds approximately 30 sources/s after startup, with
  no audio underruns or growing delay at both 1281.413 and 1381.413 seconds.
  Startup still hitches, reaching 701.5 and 1231.0 ms worst lateness respectively;
  the later run temporarily processes 36 sources/s while catching up. There are
  no new kernel entries or memory pressure. All 1,644 workspace tests pass,
  with 53 ignored, plus Clippy, formatting, vendored warnings and source-list
  checks. Six byte-cache regressions include actual independent audio/video
  packet workers and raw callback errors. Clean-source sandbox qualification
  and installation remain due.
  This is not yet an accepted network fix or 240 fps qualification.

  The first clean `2e4466a8` package also holds NAS cadence after startup,
  but its X4 UI suite crashes inside libavformat on a pasted reopen. Installation
  is held. MOV retains per-stream pointers to its original AVIO context; the
  post-inspection context replacement is unsafe and removed. Capture inspection
  now opens on its final custom IO, shared by Reader and synchronous Walk.
  Requalification of this corrected ownership path is required before delivery.
  Corrected ownership passes all 1,645 workspace CPU tests, 53 ignored, and
  the remaining CPU gates. The new allocation-churn regression repeats open,
  seek and close 32 times with other AVIO contexts alive. Native NAS playback
  again reaches 30 sources/s with no audio gaps or new kernel entries, but
  startup reaches 1844.2 ms worst lateness and catches up before steady playback;
  its whole-run cadence parser fails. Clean SDK/UI qualification remains due.
  Exact corrected source `591cf695` subsequently passes the clean SDK build,
  43 X4 and 44 ONE X2 app-path UI checks, zero failures, including the reopen
  that crashed the first candidate. All eight CI jobs pass. The exported bundle
  passes private payload authentication. Normal-audio SDK NAS checks settle at
  30 sources/s at both 1281.413 and 1481.413 seconds, without audio gaps or growing
  delay after startup. The first reaches 1238.9 ms worst lateness; the later
  reaches only 36.9 ms. The package is installed with origin, permissions and
  shared runtimes unchanged; the authenticated `dd908324` rollback is retained.
  Actual-installed NAS playback also settles at 30 sources/s, zero audio gaps,
  but startup reaches 2284.5 ms and its whole-run cadence parser fails. The
  installed ONE X2 riser smoke reaches 30 sources/s, worst 39.8 ms and zero gaps.
  No new kernel entries or postflight pressure in these corrected runs. This is
  a steady-playback input improvement, not a hitch-free or 240 fps verdict.
  Owner network retesting, startup spikes and capacity remain open. No merge.
  The owner subsequently rejects network performance: "seek takes forever and
  there's still tons of hitches". This is not startup-only acceptance. A new
  installed-player reproduction requests actual forward/backward/copied-view
  seeks and observes 4.1 and 3.5 seconds before the destination enters a draw;
  returning to the already-read region takes approximately 0.6 seconds. A
  diagnostic trace locates read amplification: small sparse audio requests
  fetch whole 1 MiB blocks, often rereading video bytes. A longer trace reads
  approximately 2.2 GiB for 1.1 GiB of unique blocks. The cache now retains
  64 KiB pages within the same 16 MiB limit, coalescing large video requests up
  to 1 MiB without rereading cached intervals. A failing-before CPU regression
  exercises sparse shared reads; packet/timestamp/seek behavior remains under
  the existing actual-demux regressions. Runtime qualification is pending;
  the installed package is unchanged. This is not a hitch or seek fix verdict.
- **File-chooser failures, issue
  [#141](https://github.com/aeharding/kjerag/issues/141):** the real FileOpen
  task reproduces a silent missing-session-bus failure. The branch routes
  non-cancellation errors to the existing alert and terminal, preserving the
  underlying portal error instead of libcosmic's generic dialog wrapper.
  Cancellation remains a no-op and a failed chooser retains the current video.
  Real Cancel-button clicks on the installed GTK and COSMIC pickers each
  returned response code 1 with no files, observed on isolated software-rendered
  sessions. COSMIC required a nested COSMIC compositor; cage alone lacks its
  required protocols. This is qualified for those installed backends, not every
  portal implementation. Playback and stitching arithmetic are unchanged.
- **Non-clobber frame saves, issue
  [#222](https://github.com/aeharding/kjerag/issues/222):** actual-save CPU
  regressions reproduce an existing capture being overwritten after filename
  collisions and a dangling filename symlink being followed. Saving now encodes
  first, reserves a new file exclusively and advances numbered names without
  falling back to an occupied original. JPEG bytes and ordinary names are
  unchanged. Concurrent captures and raw noncollision IO errors are covered;
  this does not add atomic complete-file publication or crash durability.
- **Failed pasted-view opens, issue
  [#220](https://github.com/aeharding/kjerag/issues/220):** the real paste
  handler could mistake the retained old video for a successful new open,
  then change its time, camera and horizon and show a false success toast.
  A failing-before application-message regression reproduces the unwanted
  seek. Loading now returns an explicit success result and only a successful
  target open applies the pasted view. A normal-window UI regression checks
  failure while actual footage remains open; stitching is unchanged.
- **View-reference numeric validation, issue
  [#219](https://github.com/aeharding/kjerag/issues/219):** CPU regressions
  reproduce malformed NaN angles reaching the real camera-pointing path and
  invalid times being silently reset to zero. The shared parser now rejects
  non-finite values and failed Duration conversions before CLI/paste admission.
  Negative finite times still clamp to zero; ordinary view terms and raw path
  handling are unchanged. This is an input-validation fix, not a reproduced
  footage defect or a stitching/performance change.
- **About-link spawn errors, issue
  [#131](https://github.com/aeharding/kjerag/issues/131):** branch work routes
  the existing raw launcher-spawn error to both the terminal and an on-screen
  toast. A device-hidden regression through the actual application message
  handler reproduces the missing notification with no launchers in PATH.
  Detached launching remains unchanged: a launcher that starts and later fails
  is outside this error API's coverage. This does not alter playback or the
  separate file-chooser cancellation behavior in #141.
- **Legacy band comments, issue
  [#179](https://github.com/aeharding/kjerag/issues/179):** three stale comments
  now describe retained instrument measurements rather than the deleted
  `band_bend` draw path. Confidence decays when a channel is refused while its
  measurement remains available. This is documentation-only: no executable
  Rust/WGSL, stitching, color, scheduling or installed-player behavior changes.
- **Single signed release build, issue
  [#146](https://github.com/aeharding/kjerag/issues/146):** branch work replaces
  the independent unsigned-download build with app bundles exported from the
  signed multiarchitecture repository. PR
  [#202](https://github.com/aeharding/kjerag/pull/202) passes all eight CI jobs at
  `4aa5dadc`. Its no-publication validation
  [run 34918992469](https://github.com/aeharding/kjerag/actions/runs/34918992469)
  also passes both native builds, signed architecture handoffs, combined
  repository/bundle assembly and independent final artifact verification.
  A separate device-hidden audit of the downloaded real packages verifies that
  each bundle preserves its channel app commit and signature, both channel
  Debug commits remain signed, and licenses, permissions, runtime metadata and
  ELF architecture/FFmpeg dependencies match the manifest and source.
  No application was installed or executed by that audit.

  Fifty-one local release/workflow checks pass on Flatpak 1.14.6; the original
  forty-three release checks also pass on 1.18.1. They cover isolated signed
  install/update mechanics, settings retention, payload tampering, reference
  races, publication retries, source identity and version guards. The older
  client refuses an identical-commit reinstall; the test accepts only that
  precise refusal and still requires a real different-commit upgrade and later
  channel update. Device-hidden workspace checks report 1,513 passes and 52
  ignored, including unavailable-GPU/media returns, with formatting, Clippy,
  vendor warnings, naming and source-list checks passing.

  The owner explicitly approved withholding the whole release if signing or
  either architecture fails, and subsequently delegated qualified merges.
  Publication across GitHub and Pages remains non-atomic, as in the current
  workflow. The new downloads-before-Pages ordering narrows existing failure
  states; it is not a newly owner-accepted atomic-publication guarantee. Validation used
  a disposable signer, not production credentials, and published nothing.
  Tag-event publication, live HTTPS updates and actual-player qualification
  remain distinct release checks. The installed player is unchanged; this
  infrastructure work does not retire #186.
  The earlier main integration retained its playback qualification instructions
  and the release branch's authenticated retry policy. Release scripts and
  signing/publication workflows are unchanged. The integrated tree passes 1,519
  device-hidden workspace tests (52 ignored), all51 isolated release checks on
  Flatpak 1.14.6, and format/lint/vendor/source/name/syntax checks. Unavailable
  devices are not hardware coverage. All eight CI jobs passed on that exact
  `32b5c881` integration in run `34969694574`; the earlier native artifacts retain
  their actual `4aa5dadc` source identity rather than that of this integration.

  Final merge preparation incorporates accepted main `e8089f9a`, including the
  cumulative player delivery and CI timeout. The sole roadmap conflict retains
  all three entries. Runtime, shader and test-harness sources match main exactly;
  release scripts and release/site workflows match the previously qualified
  `32b5c881` branch exactly. The updated device-hidden workspace gate reports
  1,544 passes, 52 ignored and zero failures; format, full Clippy, vendor,
  source/name/whitespace and all 20 CI shell-step checks pass. All 51 isolated
  signed-release tests pass on Flatpak 1.14.6. All eight fresh CI jobs passed on
  final head `0a4b9130` in run `35032370751`; PR #202 merged as `c5462d21`, and
  post-merge main CI `35032879851` also passed. The merge author was explicitly
  set to the mandated address and the resulting tree verified against the
  reviewed head. No production release or installed-player change followed.
- **Decoder starvation wakeups, issue
  [#136](https://github.com/aeharding/kjerag/issues/136):** delayed video-packet
  delivery reproduces repeated overdue-empty redraws in the actual generic X3
  player, about 62 redraws/s for 23 source frames/s. The same bounded installed
  test does not reproduce that mechanism on the selected filtered X4 Air path;
  its late deadlines accompany new sources. A branch change uses a one-shot
  decoder-arrival wake through the existing Scene subscription, retaining
  ordinary deadlines without a listener and leaving sequential stitching
  scheduling unchanged. The device-hidden workspace passes 1,535 tests with
  52 ignored, including deterministic Player and Scene wake regressions.
  A private candidate in the same Flatpak runtime reduces Scene pumps during
  the 16-second delay from 981 to 382, with no expired-deadline callbacks in
  that interval (964 before). Source delivery remains input-limited near
  23 fps and recovers to about 30 fps afterward. This is an idle-work reduction,
  not a measured CPU-percent, seam-quality or 240-capacity result. PR
  [#211](https://github.com/aeharding/kjerag/pull/211)'s exact code head
  `d3a03071` passed all six CI jobs. Separate bounded native UI suites passed
  47 X4 Air and 48 ONE X2 checks; both motion captures were visually inspected
  for each camera. Exact-view and portal checks retain their documented skips,
  as do cross-mount paired-file hardlinks on ONE X2. No new kernel entries or
  memory-limit events appeared, and post-exit GPU-memory counters matched each
  suite's preflight. These are functional checks, not installed-bundle or
  performance qualification. The cumulative PR #213 test package was
  installed and qualified on both cameras as recorded below. The owner accepted
  the combined normal-use check and authorized landing it after CI.
  This is not a fix for X4 source/GPU contention in #186.
- **CI dependency-fetch hang, issue
  [#160](https://github.com/aeharding/kjerag/issues/160):** branch work bounds
  the reported `add-apt-repository` stall to five minutes, followed by a
  30-second forced-stop grace period. The command keeps its underlying output
  and nonzero exit status, and the other architecture continues independently.
  A genuinely slow fetch can now fail and need a rerun. This does not remove
  the external dependency or bound the later package update/install commands;
  no package version or player behavior changes. The original PR #203 head
  `05ac017a` passes all six CI jobs, including normal provisioning on both
  architectures. Current main is integrated without changing the timeout.
  Its device-hidden workspace gate passes 1,519 tests with 52 ignored; format,
  Clippy, vendor warnings, source/name checks and shell syntax also pass. These
  unavailable-device returns are not hardware qualification. The final head
  `a3c51f75` passed all six CI jobs; after owner acceptance and explicit approval,
  PR #203 merged at `005d4dcc` and issue #160 closed on September 15.
- **CPU sampler bounds, issue
  [#204](https://github.com/aeharding/kjerag/issues/204):** synthetic decoded-plane
  tests reproduce chroma sampling row padding outside the image and both samplers
  accepting NaN coordinates. A branch fix checks logical image bounds before
  integer conversion, retaining decoder strides, valid NV12/P010 values and
  bilinear luma support. All seven focused sampling tests pass with GPU devices
  hidden. This is an agent-found CPU sampling defect, not an established cause
  of an owner-reported seam artifact or a GPU stitching/performance change.
- **Capacity input precision, issue
  [#208](https://github.com/aeharding/kjerag/issues/208):** the benchmark's
  whole-pixel sine pan repeats a coordinate for about 25 ms at each turn.
  Every outside-present gap over 8 ms in the recovered X4 and ONE X2 traces
  coincides with those turns; the app need not redraw an unchanged view.
  A pointer-only branch preserves subpixel input and records its precision.
  Its regression fails on the original 25-sample hold, and all five CPU
  pointer tests pass after correction. The device-hidden workspace gate reports
  1,515 passes and 52 ignored tests, including unavailable-GPU/media returns.
  Separate bounded installed-player controls at 2256x1504 passed with the
  unchanged combined review package: 291.274 completed redraws/s and 29.900
  consecutive source advances/s on X4, 327.474 and 29.975 on ONE X2.
  Outside-present gaps over 8 ms fell from 76 to 1 on X4 and 130 to 0 on ONE
  X2. Both cameras' actual before/after pictures were inspected, and kernel,
  memory-pressure and post-exit GPU-memory checks found no new failure.
  This fixes the benchmark's input hold, not player performance: X4's
  begin-to-commit p99 rose from 1.926 to 7.985 ms, and completion-spacing
  p99/max remains 12.958/25.197 ms (X4) and 7.799/12.333 ms (ONE X2).
  The changed input workload and different cooling state preclude a clean
  application-speed comparison; X4's slight source shortfall also remains.
  The nominal-300-Hz headless workload is also compositor-callback-paced, not
  an uncapped maximum. Existing sourced throughput counts remain valid, but
  these gaps cannot alone establish a scheduling defect or retire #186.
- **Visible playback qualification, issue
  [#206](https://github.com/aeharding/kjerag/issues/206):** retained real UI
  captures show that a painted backdrop can satisfy startup, and either the
  first video frame appearing or changing controls can satisfy the old motion
  check. A test-only branch now requires a positive playback report plus a
  valid non-flat video area before measuring motion. Both captures are checked
  and compared without controls; dark textured footage remains admissible.
  Eighteen portable synthetic regressions pass without GPU or personal media;
  the old harness fails the backdrop-transition and controls-only controls.
  The device-hidden workspace reports 1,513 passes and 52 ignored, including
  GPU/media-unavailable returns rather than hardware coverage. Formatting,
  full Clippy, vendor warnings, naming and Cargo-source checks pass.
  One-lens fixtures cannot qualify motion because automatic opening advice can
  draw over an empty pane; this restriction does not change player support.
  The installed player and stitching arithmetic are unchanged. After host
  recovery, the exact tracked harness at `b0264533` passed 37 X4 Air and 38
  ONE X2 installed checks in separate bounded runs. Both motion captures were
  visually inspected for each camera; every launched player authenticated as
  the unchanged combined review package below. No new kernel entries appeared.
  Sound-device, portal, exact-view and cross-mount paired-fixture checks retain
  their documented skips. The standard native shader/Rust-twin check also
  passed; it is separate from installed shader provenance. These are functional
  checks, not a performance fix or owner picture acceptance. The four additional
  PR #197 spaced-path checks remain covered by the earlier combined-package
  suite; that unmerged route is absent from this main-based harness.
- **Filtered-map inspection, issue
  [#198](https://github.com/aeharding/kjerag/issues/198):** the actual X4
  filtered Scene reproduces a missing displayed-map diagnostic. That API
  queried the separate spatial facade, not the retained corrected frame.
  Branch inspection now follows the exact filtered owner and retains handles
  only to the map/alpha/ratio allocations already sampled by its bind groups.
  Ordinary playback performs no new readback, GPU allocation or stitching
  arithmetic. Both current-delivery and retained-display contracts have
  real-camera regressions passing separately on X4 and ONE X2. The device-hidden
  workspace reports 1,513 passes, 52 ignored and no failures, including unavailable
  GPU/media returns rather than hardware coverage. Full Clippy, vendor warnings,
  formatting, naming and Cargo-source checks pass. The native X4 UI suite passes
  45 checks and ONE X2 passes 46. Audio and portal services are excluded;
  cross-bind-mount paired-file hardlinks also skip in this isolated ONE X2 run.
  PR [#199](https://github.com/aeharding/kjerag/pull/199) merged at `286b2b9f`
  after all six CI jobs passed and the owner explicitly approved. This enables
  investigation of the reported July sky boundary, not a seam-quality fix
  or evidence of a difference from Studio.
- **View references, issue
  [#174](https://github.com/aeharding/kjerag/issues/174):** the unchanged player
  reproduces failed clipboard navigation with a space-containing filename.
  Branch parsing now preserves the raw path before the view-term suffix;
  application classification tests and a real-window spaced-path copy/paste
  regression cover it. Branch verification passed, and on September 15 the
  owner confirmed that copying and restoring a view with spaces works in the
  installed Flatpak. That response did not name an exact package identity.
  The owner subsequently accepted the combined build and authorized its merge.
  This does not implement shell quoting or tilde expansion
  ([#157](https://github.com/aeharding/kjerag/issues/157)), change the written
  reference format, or change any stitching, color or rendering arithmetic.
- **GPU resource safety, issue
  [#195](https://github.com/aeharding/kjerag/issues/195):** the owner reported a
  frozen desktop requiring a hard reset during a real-GPU workspace gate on
  September 13. The preceding kernel log records AMD command-allocation
  failures. Broad hardware test loops remain disabled. The owner has resumed
  continued work with the full computer available; merge preparation uses a
  device-hidden workspace test gate and separate bounded player UI checks.
  Source review identifies a teardown leak that keeps even completion-proven
  draw owners alive; a branch correction and CPU-only regressions address
  that bounded defect without polling the device or
  weakening unresolved-work quarantine. Its contribution to the desktop
  failure is not established. The performance experiment is parked, and the
  stitching arithmetic is unchanged. Additional source audit found
  no second completion-proven leak in the worker, history and pending-map
  paths. Failure tests intentionally retain unresolved GPU work for process
  life, so a new opt-in runner admits one exact test per process under host
  resource/time bounds. Its fake-process regressions do not qualify a GPU run
  or establish GPU-memory containment; the runner requires separate approval.
  After the September 14 resumption, the single one-pixel callback/teardown
  regression passed on the Radeon 760M in 0.08 seconds. No new kernel messages
  appeared and post-test VRAM/GTT counters matched their pre-test values. This
  qualifies that exact cleanup regression only. The owner subsequently approved
  one-at-a-time real-footage GPU tests with resource limits and health checks.
  X4 and ONE X2 draw-deferral checks passed in 5.98 and 2.61 seconds; the X4
  overlap/renderer-recreation case recorded as failed in the interrupted gate
  passed alone in 12.39 seconds. After each process exited, VRAM/GTT counters
  matched the preflight values, memory-pressure averages stayed zero, swap
  remained unused, and the kernel journal had no new entries. These isolated
  integration checks do not qualify a full workspace gate, playback performance,
  or a release. A subsequent device-hidden full workspace run passed with
  1,511 reported passes and 52 ignored tests; unavailable-GPU and absent-media
  returns are included, so this is not hardware qualification. Full workspace
  Clippy and vendor warning checks passed, and the branch player rebuilt.
  PR [#196](https://github.com/aeharding/kjerag/pull/196) subsequently passed
  both camera UI suites and all six CI jobs, then merged at `6cf8963f` after
  the owner's explicit approval. The desktop-freeze cause remains unproven.
- **Native/SDK performance gap, issue
  [#186](https://github.com/aeharding/kjerag/issues/186):** current X4 runs show
  shared source-cadence slowdowns across the published and restored accepted
  packages. Host/Flatpak measurements show an environment contribution but do
  not isolate the cause.
- **Native-grid source-filter candidate, issue #186:** evaluating the source
  box at original texel centers has two horizontal phases and one vertical
  phase. The candidate derives its four bilinear sample positions and separable
  weights directly, retaining the original native-plane textures, atlas
  boundaries, pass count, source cadence and temporal/color ownership. It does
  not add a lower-resolution source or another intermediate image. The original
  loop shader remains a same-device reference. Floating-point reassociation is
  not bit-identical: full-camera patterned plane checks differ by at most one
  code, and the named 31-source X4 world/blotch and ONE X2 riser sequences differ
  by at most three RGB8 codes in the displayed picture. Source indices/times,
  current/filtered fields and camera-coordinate bytes match the preserved
  reference exactly. These measurements are not owner picture acceptance.
  A sequential native X4 baseline/candidate/baseline test at 2256x1504 measures
  239.349 / 287.299 / 232.024 completed redraws/s with 29.950 / 29.950 / 29.900
  source advances/s. Completion p99 is 14.962 / 12.229 / 15.244 ms. The candidate
  was retained for qualification at that stage: the measured gain
  does not retire the 4.17 ms tail target or establish every-camera performance.
  AC was online throughout, but fan and battery-status endpoints differed.
  The device-hidden full code gate reports 1,514 passes, 52 ignored and no
  failures, including unavailable-GPU/media returns rather than additional
  hardware coverage. Full Clippy, vendor warnings, formatting, naming and
  Cargo-source checks pass. Native UI passes 45 X4 and 46 ONE X2 checks,
  with sound-device, portal, exact-view and paired-file fixture skips recorded.
  The odd-width chroma fixture is byte-exact against the original shader.
  Draft PR [#200](https://github.com/aeharding/kjerag/pull/200) carries the
  candidate; all six CI jobs passed on code commit `0f4ce99b`. The separate
  native ONE X2 controls retain recorded source cadence near the requested
  300 Hz view rate, with no claimed additional throughput headroom.
  The exact SDK package also builds. Without replacing the installed app,
  `--app-path` runtime checks measure 305.224 X4 and 307.299 ONE X2 completed
  views/s with 29.950 and 29.975 source advances/s at 2256x1504. Completion
  p99/max remains 12.143/30.465 ms and 12.562/32.786 ms, not hitch-free.
  The healthy initial packaged X4 control measures 265.974 views/s; the final
  control falls behind after external GPU-memory conditions change. Package
  bases also differ by the pending clipboard and merged diagnostic changes,
  so the runtime comparison corroborates, rather than independently isolates,
  the phase optimization. The terminal redraw counter counts only Player
  pumping after the readiness gate, not every reuse of the displayed source.
  These isolated performance packages preceded the installed combined review
  package recorded below. On September 15 the owner reported "They look the
  same" for the supplied moving old/new comparisons. This accepts those
  viewed comparisons, not bit identity, all-camera coverage or performance.
  Private source/executable identities, controls and moving comparisons remain in
  `scratch/source-prefilter-phases-20260914/`.
- **Exact periodic-wrap simplification, issue #186:** a follow-on candidate
  replaces the temporal motion shader's inner signed remainder pair with one
  conditional subtraction. The existing outer landing wrap and admitted image
  dimensions prove the smaller coordinate range; source cadence, sample order,
  temporal/color policy and resource ownership are unchanged. An exhaustive CPU
  coordinate test and a same-device original-shader comparison pass. The named
  31-source X4 world and ONE X2 riser replays preserve source/coordinate fields
  and every displayed RGB8 sample exactly against the native-grid candidate.
  The full temporal Stream cyclic-shift regression also passes. A bounded
  native X4 baseline/candidate/baseline at 2256x1504 measures
  282.474 / 282.449 / 271.549 completed views/s, with 29.975 source advances/s
  throughout. Completion p99 is 13.275 / 10.487 / 13.913 ms; completion-spacing
  p99 is 12.683 / 11.319 / 12.279 ms. This supports further qualification of an
  upper-tail improvement, not a proven throughput gain or the 4.17 ms target.
  Median spacing does not improve, and thermal/fan state differs between runs.
  The device-hidden workspace reports 1,516 passes, 52 ignored and no failures,
  including unavailable-GPU/media returns rather than hardware coverage.
  Full Clippy, vendor warnings, formatting, naming and Cargo-source checks pass.
  Native UI passes 45 X4 and 46 ONE X2 checks with no failures; isolated runs
  exclude sound and portal services, and the recorded exact-view and paired-file
  hardlink checks skip. Draft PR [#201](https://github.com/aeharding/kjerag/pull/201)
  is stacked on PR #200; all six CI jobs pass on code commit `4ede9f30`.
  The exact SDK package builds. Separate package `--app-path` X4 controls,
  with identical metadata and baseline code apart from this change, measure
  300.649 / 309.374 / 311.149 completed views/s and consecutive source advances
  at 29.950 / 29.975 / 29.950 per second. Completion p99 is
  14.990 / 10.774 / 13.001 ms; spacing p99 is 12.468 / 11.010 / 11.718 ms.
  This corroborates the narrower upper-tail observation, not a throughput win
  or the 4.17 ms target. Thermal/fan state differs. All strict bounded capacity
  cohorts pass; broader cadence reports retain startup issues and, in the first
  control and candidate, terminal non-cohort issues.
  One candidate-only ONE X2 package run measures 317.224 views/s with 29.975
  consecutive source advances/s, completion p99/max 9.884/20.460 ms and spacing
  p99/max 10.267/23.350 ms. This is second-camera coverage, not an X2 speedup
  comparison; AC remains online but battery status changes to discharging.
  These isolated package results preceded the installed combined review
  package below, whose normal-use review the owner subsequently accepted.
  The work is included in cumulative PR #213. Private identities, comparisons and receipts
  are retained in `scratch/temporal-periodic-wrap-20260914/`. This changes no
  July sky-line diagnosis or owner picture-acceptance status.
- **Frame-time spikes, issue #186:** throughput averages do not retire the 4.17
  ms capacity budget or hitch risk. Do not call playback hitch-free.
- **Exact photometric parity, issue
  [#185](https://github.com/aeharding/kjerag/issues/185):** automatic lens color
  matching is implemented and visually reviewed. An isolated proof of exact
  Studio Image Fusion behavior is not complete.
- **Coverage, issue
  [#88](https://github.com/aeharding/kjerag/issues/88):** qualification covers
  the named ONE X2 and X4 Air footage and tested AMD Radeon 760M Flatpak setup,
  not every camera, recording mode, bit depth, architecture or GPU. aarch64 is
  compiled and unit-tested in CI, not playback-tested on hardware.

## Delivery next steps

### Conditional network-stall recovery, October 1

Latest exact-runtime review, `c428658f`: the owner also rejects the restored
`6dbbf16b` build. Its actual Moab 2026/VID_20260923_081735_00_002.insv network
path reproduces growing picture/sound lag and repeated audio underruns. The
candidate's ordinary run is clean, but a repeated unchanged control is also
clean, so that natural bracket does not establish a causal improvement.

With a single four-second packet-delivery outage after twelve seconds, the
installed control reports 462 then 436 audio underruns in successive intervals
and 1.621 seconds worst picture lag. The retained buffering candidate instead
holds picture and sound together once at fixed PTS, then recovers with zero
counted underruns in every interval. An ordinary moving-view candidate run
also holds once and recovers with zero counted underruns. Both waits are about
1.8 seconds under this artificial outage: one second of compressed-media lead
is not a one-second wall-clock wait guarantee. Packets, timestamps and source
order remain unchanged. Quiet-sink accounting is not audible owner acceptance.

Clean SDK package `c428658f` / executable `a2a0cdb2` / OSTree `43fd2f93`
now passes 49 X4 and 50 ONE X2 app-path UI checks, zero failures, with existing
service/fixture skips. Both motion-picture pairs are inspected. Both UI scopes
have no memory-limit/OOM events, postflight pressure averages or new kernel
entries. The native shader-twin helper remains separate from clean SDK
arithmetic provenance. Actual bundle import authenticates commit, executable,
metadata and license without installation, execution or signing qualification.
These results supersede the corresponding remaining-gate statements below,
not the earlier high-rate/240-capacity failure or spontaneous-stutter limits.

The owner explicitly accepts a possible short already-queued audio repeat for
this test build only. Qualified `c428658f` is now installed, with executable,
metadata and the normal origin authenticated; the checksum-verified rollback
is retained. However, actual-installed ordinary playback of the same network
clip logs ten picture hold/resume pairs, with zero counted audio underruns.
The run reaches only 29.91 media seconds in its roughly 48-second wall window.
Replacing stutters with frequent buffer pauses is not an accepted solution.
The player exits normally with no new kernel entries; pressure avg60 is .02,
avg10/300 zero. The actual after picture is inspected. This latest run does
not isolate network input from processing/admission delay. Owner branch retest
and the underlying smooth-playback fix still precede any merge. The separate
unpushed cache change is not part of this package or qualification. Evidence:
`scratch/coordinated-buffering-20261001/STATUS.md` and the exact-package runtime,
UI and payload-audit receipts it names.

October 2 follow-up: a transition-only native trace reproduces seven holds
with both lens packet caches empty, only two or three future decoded sources,
and admission open. Those holds are actual input shortages, not the suspected
full-source-queue classification mistake. Normal native and native-in-runtime
controls do not reproduce them and are not fixes. A no-delay C demux trace
corrupts one concurrent JSON log record; player exit is normal, but its parsers
fail, so it is not a passing runtime gate.

Reader-only controls on the same path can fall below realtime without decode,
stitching or rendering. Actual MPV selects one lens and audio and buffers seven
times in its 30-second null-output control. This does not contradict the owner's
earlier smooth MPV experience or excuse Kjerag's cutouts. Input conditions vary:
identical 640 MiB filesystem reads range from 17.6 to 25.1 MB/s. Native GIO
packet reading takes 21.27 then 37.59 seconds, bracketing a normal FFmpeg read
of 42.76 seconds for the same 30-second prefix. All packet bytes, metadata,
order and counts hash identically. The slower GIO repeat prevents a stable
backend-speed claim; no GIO backend or Flatpak permission change is selected.
Private controls and their limits are retained in the STATUS record above.

The already-written shared-cache candidate is now being evaluated on
`fix/buffering-input-ownership`, retaining coordinated buffering rather than
changing its thresholds or history. Its current device-hidden media suite
passes 186 tests, four ignored and no failures. Its native network follow-up
still has seven picture holds, despite zero counted audio underruns; the cadence
parser reports failure. It exits normally with no new kernel entries or
postflight pressure averages, and the actual after picture is inspected.
The native preview includes the parked color variation and is not clean SDK
arithmetic qualification. Installed `c428658f` is unchanged. The shared-budget
change is not established as the smooth-playback solution and is not delivered.

A separate uninstalled `fix/prepared-input-reserve` follow-up reuses the existing
one-second compressed-input readiness gate before startup/seek autoplay, not
only after a shortage. It retains two completion-proven successors and sound,
the same cache bounds and EOF escape, and unchanged explicit paused landings.
The blocked real-packet-worker regression fails before the change and passes
afterward; all 186 media CPU tests pass, four ignored. The native preview
includes the unchanged parked color variation, not clean SDK provenance.
Its same-path ordinary network cohort has no buffer holds or counted audio
underruns. A four-second injected read interruption produces one hold, about
.76 seconds by neighboring presentation records, then regular source cadence
with zero counted underruns. Three actual clipboard seeks into another range
show their requested destinations in about .96/.75/1.80 seconds, with no buffer
holds, counted underruns or growing delay in later playback. All exit normally
with no new kernel entries or postflight pressure averages; actual pictures
are inspected. These unpaired native observations do not establish causation
for every prior pause, audible continuity, physical scanout or 240 capacity.
The broad cadence parser still rejects frameless startup renders; these are
scoped playback observations, not a full gate pass. Slow-share autoplay can
wait longer before opening or seek playback starts. That tradeoff, both-camera
qualification, clean SDK delivery and owner retest remain due. Installed
`c428658f` is unchanged and the overall network fix remains unproven.
The bounded, device-hidden full workspace check passes 1,694 tests with 53
ignored and no failures. Formatting, workspace Clippy, vendor warning, naming,
dependency-source and whitespace checks also pass. These CPU checks include
the parked color variation and unavailable-device returns, not executed GPU
coverage or clean-package qualification.

Draft [#248](https://github.com/aeharding/kjerag/pull/248) now retains the clean
offline SDK package from exact source `65e00f72`, excluding the parked colors.
Its app-path suites pass 49 X4 and 50 ONE X2 checks, with existing service and
fixture skips. The native shader twin remains separate from clean SDK shader
provenance. All eight exact-source CI jobs pass, including both architectures.
On the actual network `_002` path, ordinary playback and a real two-second
pause/resume have no buffer holds or counted underruns. Three real clipboard
seeks show their requested pictures in approximately .84/.55/1.73 seconds,
then regular source cadence, zero holds and zero counted underruns. The same
four-second input interruption causes one approximately 1.13-second hold,
then regular playback and zero counted underruns. This exceeds the requested
zero-to-one-second buffer goal; it is not reported as completion. All exit
normally with no new kernel entries or pressure averages, quiet output verified
and actual pictures inspected. Both UI scopes have no memory-limit/OOM events.
Whole cadence analysis still rejects frameless startup and the last incomplete
presentation at process exit; the seek run additionally crosses deliberate
timeline discontinuities. These are scoped observations, not complete cadence,
audible continuity, physical scanout or 240-capacity verdicts. The actual bundle
payload is privately authenticated without installation or signing qualification.
The owner now accepts the autoplay-preparation wait for this test build only.
The exact qualified `65e00f72` package is installed as OSTree `0deeacde`, with
executable `a3b56c4d`, unchanged runtime/permissions and origin `kjerag-origin`.
The replaced `c428658f` bundle is retained for rollback. Owner branch retest
remains required; no merge, release or general stutter-fix claim.
The one actual-installed network smoke check exits normally, authenticates the
running package and quiet output, and maintains approximately 30 source
advances/s after startup with no buffer holds or counted audio underruns.
There are no new kernel entries; memory pressure averages after the run are
0.00 over 10 seconds and 0.02 over 60 seconds. Its simulated pause/resume keys
leave the final picture paused, so it does not qualify resumed playback. The
whole-run cadence verdict remains false, including frameless startup and the
terminal incomplete presentation. Owner testing of ordinary pause/resume and
seek remains required, not replaced by this scoped smoke check.

The owner still reports brief micro-stuttering after buffering ends in the
installed `65e00f72` build. A retained exact-SDK run has 65–82 ms source dwells
in the first second after restart, despite later 30-source/s reports. A
three-completed-picture restart prototype respects the existing six decoded
successors and three-future-source temporal window, without enlarging GPU or
packet retention. Its device-hidden workspace passes 1,696 tests with 53
ignored; all CPU gates pass. One native network interruption check recovers
with no counted underruns but does not demonstrate a clear improvement over
the current installed control. That prototype is not a confirmed fix or
delivery candidate, and includes the preserved parked color variation.
The unproven extra-picture prototype is removed, with its reference in branch
history. The more direct display-path finding is that actual installed redraws
advance through catch-up pictures while the common clock remains held for
buffering. That can expose slow recovery steps as if playback already restarted.
The existing real-Scene regression now redraws during every held step and
checks the exact shown owner, then resumed presentation and retained source
history. It fails before the fix on the owner's latest actual Moab `_002`
network path, showing frame 3 instead of the held frame 0, and passes after the
narrow display-selection fix. Both focused X4/ONE X2 GPU tests pass normally,
with no new kernel entries or postflight memory pressure averages. The
device-hidden full workspace reports 1,694 passes, 53 ignored, no failures;
formatting, Clippy, vendor warnings, naming, source-list and harness CPU gates
pass. These native checks include the unchanged parked color variation, not
clean SDK arithmetic qualification. The window now selects the actual last shown
picture while buffering; camera/horizon controls and logical source processing
remain live. Readiness thresholds, cache/surface bounds, history and source/color
arithmetic are unchanged. Clean SDK source `5e385619` then builds normally;
its package passes 49 X4 and 50 ONE X2 functional UI checks with fixture/portal
skips disclosed, no new kernel entries and no memory-limit events. All eight
CI checks pass on that runtime source. The qualified test Flatpak is installed
with the preceding `65e00f72` package retained and the update origin unchanged.
An actual-installed eight-second forced network interruption now holds source
447 through all 40 buffering redraws, where the earlier installed control
advanced through sources 476, 477 and 478 during its hold. No counted underruns
occur; later source progression reaches approximately 30/s. Those are separate
unpaired runs, not physical scanout or audible verdicts. Crucially, the new run
still shows 82 ms and 63 ms first-second picture dwells after clock restart.
Stitch/map preparation at sources 468 and 469 takes 81.9 ms and 65.1 ms wall
time in that interval. The actual cause inside that work/queue remains open;
the full owner micro-stutter report is not fixed. A read-only CPU-counter repeat
has no quota throttling, held source 488, and approximately 47 ms examples after
restart, not the larger spike. It exits normally with no kernel entries or
counted underruns, no memory-limit events and small nonzero pressure averages.
The owner clarifies that the symptom occurs randomly, not at a particular
timestamp. A copied view is therefore not a prerequisite for further diagnosis.
Draft [#249](https://github.com/aeharding/kjerag/pull/249) contains the narrow
shown-picture change; no owner acceptance, merge, release, general network
smoothness or 240-capacity completion is inferred. Recent-file inspection confirms the
original September 23 `_003` exists in the share's `SSD/` subfolder, not at the
previously checked root path; the earlier whole-clip absence assumption is retired.

The separate uninstalled `fix/processing-stall-buffering` candidate then tests
the completion-aware clock decision, not another network backend or a larger
buffer. Previously admitted stitch inputs exempted an overdue unfinished
picture from a common-clock hold. The existing real-Scene blocked-actor case
reproduces that on the actual X4 path before the policy edit; after the edit,
X4 and ONE X2 hold only after the picture deadline is missed, retain the shown
owner during refill, consume all expected sources and resume in the same epoch.
The normal pre-deadline and completed-prefix catch-up controls do not hold.
The media suite passes 187 tests with four ignored; the bounded device-hidden
workspace passes 1,695 with 53 ignored, plus all CPU gates and a native release
build. These native checks include the preserved parked color variation and
unavailable-device returns, not clean SDK or broad hardware qualification.
A bounded native eight-second input-interruption run has one hold, retains
shown source 486 throughout it, no counted underruns and no repeated buffering.
It exits normally with no kernel entries or memory-limit events and small
nonzero pressure averages. Its first restart second has 47–50 ms examples,
not the earlier 82/63 ms pattern, so this unpaired run cannot establish general
micro-stutter resolution. Stitching and color arithmetic, source cadence,
restart lead and cache bounds are unchanged. The conditional processing pause
is explicitly put to the owner before testing. Clean SDK source `37ca091b`
then builds successfully; its functional UI suites pass 49 X4 and 50 ONE X2
checks with the disclosed sound-device, portal and fixture skips. All eight
CI jobs pass on that runtime source after retrying an x86 dependency-setup
failure. A bounded actual-window network interruption through the clean,
uninstalled package holds shown source 488 through 39 redraws, resumes once
and has no counted underruns. No new kernel entries, memory-limit events or
CPU-quota throttling are recorded. Roughly 47 ms first-second picture intervals
remain; the larger preceding 82/63 ms pattern is not reproduced. This unpaired
run is not a universal micro-stutter, audible-continuity, scanout or capacity
verdict. The owner then approves installing the conditional-pause test.
Installation succeeds, but the actual-installed network check refuses playback
at the shared GPU PIS front-end arithmetic check: horizontal scratch word 41
is `0x43618a56`, expected `0x43618a57`. A local no-delay control refuses
identically. The retained `5e385619` is restored and also refuses identically,
so rollback does not establish working playback. Flatpak history shows a
concurrent Mesa 26.2.2 update immediately before candidate installation;
earlier passing tests preceded it. That is compatibility evidence, not yet
a completed runtime-cause isolation. No arithmetic guard is bypassed and no
shared runtime is changed. The owner rejects shared graphics rollback;
the in-app compiler precision fix is tracked separately in #251 above.
Owner branch retest and the broader random stutter requirement remain
outstanding. No merge or release.

The owner reports stuttering across network footage, including spontaneous
onset at 23.190 seconds in the September 23 `_003` clip. A natural installed
pause/resume/seek sequence did not reproduce sustained bad recovery. A separate
temporary demux slowdown through the installed app does: after reads recover,
picture and sound remain seconds late until manual pause/resume clears the bad
state. This is a related clock-recovery reproduction, not proof of the sole
cause of every owner-reported stutter.

The owner conditionally permits automatic recovery only if it removes the
stutters and does not act as an always-on workaround. Branch
`fix/network-stall-recovery` adapts the earlier unshipped clock-hold/PCM-history
prototype to the current single-reader architecture. It resumes only with
completion-proven picture lead, not merely queued GPU outputs. Media CPU tests
pass 174 cases with four ignored, including two simulated minutes without a
normal-playback hold or clock reset. Real network A/B, audio restart, both camera
UI and package gates remain due. The bounded device-hidden workspace passes
1,676 tests, 53 ignored, with no failures; unavailable-device returns and the
two parked color edits are included, not clean-source GPU qualification.
Full lint/format/vendor/name/source and portable harness checks pass.
Repeated pauses or unnecessary normal-playback
holds do not qualify. The installed `6dbbf16b` app is unchanged; no fix, merge,
performance-capacity or all-camera verdict is claimed. Private evidence:
`scratch/resume-stutter-20261001/`.

Exact prototype source `1d34630b` now has all eight CI checks passing and a
clean offline SDK package, excluding the parked color edits. The installed
baseline and candidate each replay the reported September 23 clip through
23 seconds without reproducing the owner's sustained bad state. Under the
same controlled transient input slowdown, the candidate recovers before manual
pause/resume, with reported worst lateness bounded at 68.3 ms and zero reported
audio underruns; the baseline previously accumulated seconds of lag and hundreds
of underruns. This is a scoped recovery improvement, not a general stutter fix.
The candidate logs 25 hold/resume pairs, including one startup hold and repeated
holds during prolonged slow input. A separate normal network control also holds
once at 0.145 seconds, then maintains source cadence with no further holds.
It therefore does not meet the zero-unnecessary-hold/repeated-pause qualification
gate and remains uninstalled. Startup priming and input-refill readiness need
attention before delivery, without reducing stitching cadence or history.
All three completed player runs exit normally with no new kernel entries.
Transient-candidate postflight memory pressure has zero avg10 but nonzero longer
averages; both normal controls have zero averages. These receipts do not include
completed cgroup memory-event accounting or physical scanout proof. Remaining
both-camera and package gates are not waived.

The next uninstalled revision adds recovery-only compressed-input high water
across each required lens/file, using their existing normalized container
clocks. It waits for one second of packet lead, with EOF and existing audio/video
cache bounds as escape conditions, without increasing decoded/GPU retention.
Startup and exact-seek autoplay instead prime two completion-proven pictures
and sound before starting the clock; they do not use that input threshold.
Paused exact landings remain immediate once their requested picture completes.
Seven new CPU cases cover real blocked packet delivery through Player recovery,
separate lens clocks, wake/cancel, seek invalidation, errors and unattainable
lead. The media gate passes 181 tests with four ignored. Full Clippy, vendor,
format, name/source and 28 portable harness checks pass. The device-hidden full
workspace passes 1,683 tests, with 53 ignored and no failures, including
unavailable-device returns and the parked color variation, not clean-source
hardware coverage. Actual-player qualification remains pending. No installed app,
stitch/color arithmetic, source cadence or history changes. The startup-delay
and sustained slow-input behavior still require measurement, not acceptance
from these CPU checks.

Exact follow-up `6f5b80d0` passes all eight CI checks and has a clean offline
SDK package. Its natural September 23 network run has zero buffering entries,
including startup, with regular 30 source advances/s, 54.5 ms worst reported
lateness and no audio underruns. The spontaneous owner-reported bad state is
still not reproduced. Under the same controlled slowdown, it reduces hold/
resume pairs from 25 to four, with no startup hold, and restores regular source
cadence before manual intervention. Reported worst lateness is 49.3 ms and
audio underruns remain zero, but the intentional pauses still interrupt sound.
That is a scoped recovery improvement, not a hitch-free verdict or acceptance
of repeated pauses under slow input. Both actual-camera Scene recovery/history
regressions pass separately with no new kernel entries or pressure averages.
The clean app-path suites pass 49 X4 and 50 ONE X2 checks, separately, with the
existing isolated-service and fixture skips. Both cameras' motion captures
are inspected. Neither suite has new kernel entries, postflight pressure
averages or scoped memory-limit/OOM events. The native shader-twin helper is
separate from clean SDK shader provenance.
All inspected network runs exit normally with no new kernel entries and zero
postflight pressure averages, but those wrappers lack completed scoped memory
event receipts. The natural trace still fails full integrity at startup/quit;
its trimmed source cadence is diagnostic, not physical scanout proof. Captured
pictures are inspected, not a moving owner review. Installed `6dbbf16b` remains
unchanged; audible restart behavior and owner acceptance are still pending.
The exported bundle also passes a private device-hidden import/authentication
of its commit, executable, metadata and license. That audit installs and runs
nothing, and does not qualify signing of this private test package.

Further changing-view qualification prevents treating that candidate as ready.
At 2256x1504 with a nominal 250 Hz headless output and the September 23 NAS
clip, with each app limited to four CPU cores by the test scope, `6f5b80d0`
completes about 123 sourced view renders/s and enters recovery
15 times, including before the pointer pan. Its unchanged installed baseline
completes about 144 renders/s under the same requested load while retaining
regular source cadence. Neither meets 240 renders/s in this capped test; this
does not establish full-hardware capacity. These
conservative GPU callback times do not isolate rendering-kernel cost. The
candidate's repeated picture holds are not accepted as a performance solution.
Both processes exit normally with no new kernel entries; the candidate has
nonzero postflight memory-pressure averages, unlike the baseline.

A separate failing CPU regression identifies an unnecessary-recovery case:
a late shell wake can stop the common clock despite multiple completed
successors already waiting. The follow-up permits ordered catch-up through
that completion-proven prefix before declaring a picture shortage. Missing
sound and exhausted or pending picture work still trigger actual recovery.
Media coverage passes 183 tests, four ignored; the device-hidden workspace
passes 1,687 tests, 53 ignored. Full workspace lint, formatting and portable
harness checks pass. Both-camera actual-Scene catch-up regressions pass with
real decode and GPU work, but both log sound-device underrun/overrun errors.
They prove the tested clock/source ownership, not audio-device continuity;
their native binaries also include the preserved parked color variation.
New clean runtime/package qualification remains due. This does not establish the
cause or resolution of the owner's spontaneous network stutter. Installation,
merge, the 240-capacity verdict and owner acceptance remain withheld.

Exact correction `9cae1bed` now has a clean offline SDK package excluding the
parked colors. Repeating the changing-view network test gives two picture-hold
entries rather than the preceding 15-entry observation. The first resumes;
the second occurs after the pan and remains held when the test quits. This
does not meet a zero-unnecessary-hold gate. Its strict 24-second pan cohort
has 3,059 completed sourced view renders, about 127.46/s, with 716 consecutive
source changes, about 29.83/s. Regular reports show roughly 30 source/s,
27.0 ms worst lateness and no counted audio underruns. The actual app cgroup's
CPU quota records no throttling during this run; it cannot explain away that
capacity result. Both app-scope memory-event snapshots have no limit/OOM events,
and shutdown is normal with no new kernel entries. Postflight pressure has
zero avg10/300 but avg60 .01. This is a bounded diagnostic improvement, not
resolution of the spontaneous owner symptom, moving acceptance or 240 capacity.
The installed app remains unchanged and delivery is still withheld.

A further real-pipeline regression reproduces an unnecessary clock hold with
all three required future sources already admitted but the stitch actor
deliberately blocked. This isolates admitted processing delay from missing
camera input. The follow-up uses that exact source/epoch ownership and the
existing seven-source stream's lookahead to exempt in-progress work from input
buffering. Completed current pictures check their next output; queued finish
owns the clipped EOF tail. Actual completion remains mandatory for display and
restart, and missing sound always holds. Expired deadlines wait on worker
completion rather than causing a timer loop. Media passes 184 tests with four
ignored; the device-hidden workspace passes 1,692 tests with 53 ignored, plus
the full lint/format/vendor/source/name and portable harness gates. These
include unavailable-device returns and the parked color variation, not clean
SDK hardware coverage. Separate real-Scene cases pass on both cameras with
admitted stitch work delayed, without a common-clock hold. Both cameras also
pass actual missing-sound hold/refill/restart checks with picture inputs
already admitted, retaining the same source history. These fixtures enable
sound through the quiet sink rather than using the mute readiness exemption.
Three of the four runs log ALSA underrun/overrun warnings, so they do not
qualify sound-device continuity. No new kernel entries appear; X4's recovery
run has nonzero pressure averages, while the other three have zero averages.
Clean SDK and actual-player qualification remain due. This does not establish
the cause of the owner's spontaneous stutter or meet the 240-capacity
requirement. Installation and merge remain withheld.

Exact follow-up `c428658f` passes all eight CI jobs and builds an offline SDK
package excluding parked colors. Its 2256x1504, nominal-250-Hz September 23 NAS
pan fails throughput: 73.87 completed sourced views/s and 16.37 consecutive
source advances/s. Picture lateness reaches 6.426 seconds. One actual missing-
sound hold resumes at the same PTS; three counted audio underruns remain.
A matching unchanged-installed control also fails, at 72.37 views/s and 16.50
sources/s, accumulating 16.244 seconds of picture lag and hundreds of audio
underruns per later interval. Live GPU samples report 98%/97% busy at 800 MHz
for candidate/control, with temperatures around 62 C in the candidate. This
does not isolate the reason for that clock or prove it is the sole cause.
Both app CPU quotas record zero throttling and both memory-event snapshots
record zero limit/OOM events. Both players exit normally with no new kernel
entries. Candidate pressure avg60 is .03, with avg10/300 zero; the control's
averages are zero. Both after pictures are inspected, not moving acceptance
or scanout proof. This is evidence of a shared severe throughput failure and
scoped recovery behavior, not a deployable performance or spontaneous-stutter
fix. Further input-stall and both-camera package gates remain due. No install.

The subsequent shared-divider reuse experiment passes arithmetic checks and
retains both cameras' 31-source captured outputs, but is retired unmerged in
PR #244. Its exact SDK player repeat achieves 164.00 views/s versus the
preceding package's 163.83, both at 29.96 ordered sources/s with no holds.
An earlier candidate run achieves 194.37 views/s but only 20.79 sources/s,
with four picture holds. No useful player-speed benefit or 240 capacity is
demonstrated. These battery-powered runs do not attribute the preceding
plugged-in 800 MHz failure to code. All exit normally with no new kernel,
CPU-quota throttling or memory-limit/OOM events. The prototype is archived,
not installed; active work returns to the playback bottleneck.

A packet-order audit of the reported September 23 clip identifies unused
compressed-cache capacity. With consumers stopped at 14.828 seconds, its actual
packet order reaches the old 128-audio-packet cap with only about 60 MB retained,
despite roughly 128 MB of combined budget. A static shared-budget model extends
source lead from about 2.72 to 5.99 seconds without raising that total. This is
not measured runtime cache state or proof of the spontaneous stutter's cause.
The next candidate shares the existing byte/count budget across audio and video
while both consumers are attached. It leaves video-only limits, packet contents,
source processing and image arithmetic unchanged. A failing-before regression
through the real packet worker confirms that the old audio partition stops
read-ahead with video capacity unused. Candidate checks cover actual producer
progress, the shared byte/count bounds, refill wakeups, retained packet order,
and restoring the video-only budget after audio closes. All 186 media CPU tests
pass, with four ignored and no failures, including real MOV packet/seek and AAC
PCM comparisons. Full formatting, lint, vendor/source/name and 28 portable
harness checks pass. The device-hidden workspace passes 1,694 tests with 53
ignored, including unavailable-device returns and the parked color variation,
not clean-source GPU coverage. Actual-player network qualification remains
pending; the installed app is unchanged. The cache change will be qualified
separately on the installed runtime. The October 2 follow-up evaluates it with
the retained coordinated-buffering prototype, not as an accepted installed fix.
This is not an accepted recovery workaround or a performance verdict.

### Pre-window startup delay, September 30

October 1 clarification: the owner wants the normal transparent/blurred window
pane throughout opening, not black followed by transparency followed by video.
The branch now uses the existing window/fullscreen backdrop in both preparation
and playback; fullscreen's existing black surround remains unchanged. The CLI/
FIFO regression additionally compares the opening pane with the closed window's
pane. Clean source `6dbbf16b` is now qualified and installed. All eight exact-source
CI jobs pass; SDK app-path suites pass 49 X4 and 50 ONE X2 checks, zero failures,
with existing fixture/service skips. The actual-installed startup suite passes
all six checks. Motion and startup captures are inspected. No new kernel entries
or scoped memory-limit/OOM events appear. X4 postflight pressure has zero avg10
but nonzero longer averages; X2 and installed startup averages are zero. Origin,
permissions and recorded shared runtimes remain unchanged; the authenticated
preceding `5aaf97a7` bundle is retained. Headless checks verify the normal pane,
not desktop blur. The owner's reopen retest remains the merge gate.

October 1 playback report: footage can enter persistent stuttering after pause/
resume or seeking, or apparently at random, until another pause/resume or seek.
This is separate from opening styling and remains unlocalized under issue #186.
No throughput or functional UI result above establishes that it is fixed.

Previously, the owner rejected the installed "Opening video..." welcome screen and requested
the normal window with a first picture or blank video area immediately. The
branch now shows a blank black player during initial preparation, with no logo,
message or open button. The no-file welcome screen and failed-replacement
preservation are unchanged. Clean source `5aaf97a7` was installed. All eight
exact-source CI jobs pass; separate SDK app-path suites pass 48 X4 and 49 ONE X2
checks, zero failures, with existing fixture/service skips. The actual-installed
CLI/FIFO startup check also passes all five checks, including blank picture and
responsive Close. Motion and startup captures are inspected. No new kernel
entries or scoped memory-limit/OOM events; postflight pressure averages are zero.
Origin, permissions and recorded shared runtimes are unchanged; authenticated
`5ecc9946` rollback is retained. The owner's startup retest remains the merge
gate. No new first-picture latency, all-camera, hitch-free or capacity claim.

After reporting the installed single-reader build as "better", the owner
identifies a remaining opening delay specifically before the window appears.
The unchanged installed `78075a42` NAS trace places the first surface render
attempt 4.81 seconds after app exec. UI-thread startup includes a 2.76-second
roughly 36 MB motion-track read and a 0.17-second ISO read, while decode has
already started video read-ahead. This is a confirmed synchronous startup
contribution, not proof of a stitching bottleneck, cold-cache latency or a
paired causal result about the concurrent reads.

Branch `fix/nonblocking-file-open`, stacked on PR #241, moves complete file
inspection/calibration to one background worker with one replaceable pending
choice. The shell returns from initialization without waiting; the existing
welcome view shows opening status. Decode/audio start only after preparation.
Ordered source processing, horizon integration, stitching/color arithmetic
and first-picture readiness remain unchanged. Closing/replacing a choice
invalidates stale results; failed opens preserve the current picture and view.
The device-hidden workspace passes 1,656 tests with 53 ignored, including
unavailable-device returns and the two preserved parked color edits rather
than clean-source hardware coverage. Full lint/format/source/name and CPU
harness gates pass. The real CLI/FIFO regression fails pre-window paint in the
installed baseline and passes five native checks, including responsive Close
and canceled-result preservation. Normal native NAS opening attempts its first
surface draw at 0.758 seconds, before the large metadata read; recorded cadence
and zero underruns follow. Both video pictures are inspected. Native and
installed runtimes differ, so this is not a matched package-speed comparison.
No new kernel entries appear; native NAS postflight memory pressure is nonzero.
All eight CI jobs pass on implementation `5ecc9946`. The clean SDK app-path
suites pass 48 X4 and 49 ONE X2 checks, zero failures, including the five
blocked-open checks. Existing fixture/service skips remain; the native shader
twin helper is not a clean SDK shader test. All motion captures are inspected.
The actual exported bundle is authenticated and installed, preserving origin,
permissions and shared runtimes, with authenticated `78075a42` rollback retained.
Packaged NAS first surface preparation occurs 0.777 seconds after app exec,
before the large metadata read, which now runs on the opening worker. This is
not cold-cache or click-to-window latency, nor first-video-picture readiness.
The final actual-installed NAS run restores the exact reported CLI view and
subsequently reports 30 source advances/s and zero audio underruns; worst
reported picture lateness remains 131.0 ms. It exits normally. Both UI suites
and the packaged/installed NAS runs have no new kernel entries and zero
postflight memory-pressure averages. The owner's startup retest remains the
merge gate; no hitch-free, all-camera or 240 fps verdict is claimed.
The [startup record](research/nonblocking-file-open-20260930.md)
retains exact scope, negative cleanup and invalid first-assertion details;
private evidence remains in `scratch/playback-independent-20260927/`.

### Preceding single-reader private installation, September 30

Exact source `78075a42` from draft
[PR #241](https://github.com/aeharding/kjerag/pull/241) was installed and is now
retained for rollback beneath the startup change above. It replaces
the competing live audio/video demuxers and custom byte cache with one normal
FFmpeg input per container, routing bounded compressed queues to video and
audio consumers. Stitching/color arithmetic and ordered source processing are
unchanged. The clean SDK archive excludes the two preserved parked color edits.
All eight exact-source CI jobs pass. Its separately run app-path UI suites pass
43 X4 and 44 ONE X2 checks, zero failures, with existing fixture/service skips;
all motion captures are inspected. The actual exported bundle is authenticated.

Packaged NAS forward/backward seeks take about 1.36, 1.36 and 0.75 seconds, then
playback holds recorded cadence through the previously failed region, zero
audio underruns and no growing delay. Worst picture lateness remains 42.4 ms.
A local 1 Hz redraw/recovery check preserves source processing and audio, then
returns to a current picture at 60 Hz. Neither test is a hitch-free or 240 fps
capacity verdict. No new kernel entries appear; UI and recovery postflight
pressure averages are zero, while the NAS seek postflight avg60 is 0.02.
Installation preserves origin, permissions and all recorded shared runtime
identities; both `591cf695` and `dd908324` rollback bundles are authenticated and
retained. The actual installed 2256x1504, 60 Hz NAS pan holds 29.95 consecutive
sources/s, zero audio underruns and 41.2 ms worst lateness without growing delay.
It exits normally with no new kernel entries or postflight pressure averages.
Display timing is still uneven: steady sourced-picture dwell p99/max is
48.12/51.00 ms. Owner retest and the 240 fps/tail requirements remain open.
No merge or release is claimed. [MERGE_READINESS.md](MERGE_READINESS.md) records
the package identities and qualification limits.

### Rejected byte-cache build and diagnostics, September 30

Exact source `591cf695` was installed from draft
[PR #241](https://github.com/aeharding/kjerag/pull/241). It shares one bounded
file-byte cache beneath independent audio/video demuxers, created before capture
inspection. It includes the prior source-completion/native-mesh work, excluding
the two parked color edits. Both clean-source SDK camera suites, all eight CI
jobs and the actual bundle audit pass. Normal-audio NAS playback now reaches
30 sources/s after startup in the tested regions; installed startup still
reaches 2.3 seconds of picture lateness before catching up. Actual installed
ONE X2 playback reaches recorded cadence without growing delay or audio gaps.
The `dd908324` rollback is retained. Origin, permissions and shared runtimes
are unchanged. This is not a hitch-free, 240 fps, owner-accepted, merge or release
verdict. Exact identities and limits are in [MERGE_READINESS.md](MERGE_READINESS.md).

The owner has since rejected this build's network seeking and ongoing hitches.
The request-sized byte-cache follow-up above is not installed or accepted yet.
Its clean SDK long NAS pan still accumulates 4.45 seconds of delay, whereas
the same local-file control holds source cadence with worst 38.8 ms lateness.
The owner's requested processing-disabled control also fails over NAS: both
lenses and audio remain active, but the raw fisheye display has no seam solver,
color matching, temporal filter or projection. It runs around 25 to 27 fps and
accumulates 6.53 seconds of delay. The defect therefore does not require those
processing stages; investigate input/demux/decode waits before changing stitch
arithmetic. This diagnostic is not a speed or capacity qualification, and its
temporary bypass has been removed. Details and limits are in
[the network-input follow-up](research/network-file-buffering-20260930.md).

The next unqualified branch candidate replaces competing live audio/video
demuxers with one container reader routing separately bounded compressed packet
queues. Audio decoding/refill remains independent. Video compressed retention
increases from 64 to 128 MiB (still 512 packets) to pass the measured 67 MiB
interleave gap without holding decoder surfaces. Seeks clear both queues and
gate pre-target audio in the same transaction; causal video history is preserved.
The device-hidden media suite passes 165 tests, four ignored, plus all-target
media Clippy. Real MOV/AAC packet/seek comparisons and decoded PCM against the
audio-only reference pass. Initial native full-size NAS pan holds approximately
30 source advances/s, with 39.6 ms worst lateness and no audio underruns across
the 40-second moving-view cohort. No new kernel entries appeared; postflight
memory-pressure averages are small but nonzero. This is one working-tree result
including parked owner color edits, not clean SDK qualification, an arbitrary
network guarantee, 240 fps capacity or owner acceptance. Real seek and historical
audio-gap controls, both-camera qualification and delivery remain due. The
installed Flatpak is unchanged.

That first single-reader seek control still has a later failure: about 2.04
seconds of lag and 266 audio underruns, despite quicker individual seeks. It is
not ready to install. The April interleave region passes through the full player
without audio gaps. The next candidate removes the custom AVIO/byte cache and
uses normal FFmpeg-owned input. Its 155 device-hidden media tests pass, with four
ignored and the ten retired cache-only tests removed. Runtime qualification is
pending; no reliable network or capacity result is claimed.

The standard-input native follow-on completes those three seeks in about 1.23,
1.43 and 0.82 seconds and holds later source cadence without audio gaps or
growing delay. Another 40-second NAS pan holds 29.95 consecutive sources/s with
46.1 ms worst picture lateness and no underruns. ONE X2 paired-file playback and
the April historical audio-gap region also hold cadence without audio underruns.
No new kernel entries appear and postflight pressure averages are zero. Actual
before/after pictures are inspected. These previews include parked owner color
edits and do not qualify a clean package, instant seeks, all networks or 240 fps.
Complete workspace/SDK/UI qualification and owner retest remain due; installed
code is unchanged.

Final device-hidden workspace gates pass 1,648 tests, 53 ignored and no failures,
plus formatting, full Clippy, vendor warnings, source-list and naming checks.
This includes the raw-filename admission guard and all packet fanout tests, with
unavailable-device returns counted separately from hardware qualification.
That local gate includes the two parked color files, not the clean archived
source. The later exact-source CI, SDK/UI, slow-compositor and packaged NAS
qualification is recorded in the current installation checkpoint above.

### Previous source-completion private installation, September 30

Source `dd908324` was installed as an interim test Flatpak after separate clean
SDK app-path suites pass 43 X4 and 44 ONE X2 checks, zero failures, and the actual
bundle passes a private payload import/audit. It brings the source-specific
completion and finite curved-mesh changes above, not the parked periodic-color
edits from the owner-accepted native preview. The actual installed local X4 pan
holds recorded source cadence without growing delay or audio underruns.
Network-backed pan and the 240 fps capacity target still fail. Executable, OSTree,
permissions, origin and unchanged shared runtimes are authenticated; source 635
is retained for rollback. No merge or release. Exact identities and qualification
limits are in [MERGE_READINESS.md](MERGE_READINESS.md).

### Previous private installation, September 29

Source `635e9b04` was installed. It adds bounded compressed read-ahead,
source-owned native endpoint caching and a capture actor draining admitted
sources to the earlier independent-audio/redraw-independent design. The clean
SDK archive excludes parked color edits and the unaccepted clock-hold prototype.
Both app-path camera suites pass43X4 and44ONE X2 checks, zero failures, with
documented sandbox skips. Executable and OSTree identity are authenticated;
origin, permissions and shared runtimes remain unchanged. The68591c10bundle
is retained for rollback. The actual-installed NAS moving-view check still
falls behind at60Hz; this is an interim test build, not the promised reliable
playback fix. Owner acceptance and merge remain due. Exact identities and
qualification limits are in [MERGE_READINESS.md](MERGE_READINESS.md).

### Earlier September 29 private installation

Exact source `68591c10` adds the independent audio producer and redraw-independent
filtered playback described above. It changes no stitching/color arithmetic.
Its app-path camera checks passed before installation. The installed executable
and OSTree identity match the candidate; the previous `f557ee59` bundle is
retained for rollback. An old installer unexpectedly updated five shared runtime
refs; all five exact pre-installation versions were subsequently restored and
verified. Future installs must disable dependency and related-ref updates.
Origin remains `kjerag-origin`. This is an interim private test build, not a
merge, release, complete performance fix or owner acceptance. Exact identities
and qualification limits are in [MERGE_READINESS.md](MERGE_READINESS.md).

### Previous private installation, September 16

The installed private test Flatpak was built from `f557ee59`, whose tree is
identical to merged main `748ea008`. Relative to the previous `1d5e9e46`
package, its only production changes are PR #228's generic endpoint-seek and
terminal-wait correction and PR #230's NaN-volume guard. It retains the
owner-accepted drag-release fix. Selected stitching/color arithmetic, source
cadence, dependencies and the Flatpak manifest remain unchanged; the unqualified
X3 horizon candidate in PR #212 remains excluded.

The actual bundle passed its separate payload audit. Both before installation
and through the actual installed package, separate runtime suites passed
43 X4 Air, 44 ONE X2 and six focused X3 endpoint checks with zero failures.
Every launched player executable was authenticated; both motion captures from
each full suite and both X3 endpoint captures were inspected. The documented
isolated sound-device, portal, exact-view, import-fault and cross-mount pair-fixture
skips remain. X3 coverage is generic endpoint behavior, not horizon or seam
qualification. This is functional qualification, not new owner acceptance,
a capacity result or a public release.
Exact identities, health qualifications and rollback are recorded in
[MERGE_READINESS.md](MERGE_READINESS.md#current-private-test-package-september-16).

The immediate prior `1d5e9e46` bundle and earlier owner-accepted `90721189`
bundle are retained for rollback. Installation preserved data, runtimes and
remote settings; the origin remains `kjerag-origin`.
Normal signed-channel restoration remains due after test review. Issue #186's
performance limitations and #187's capacity/handoff decision remain open.

### Previous accepted integration, September 15

The owner-approved cumulative PR [#213](https://github.com/aeharding/kjerag/pull/213)
test Flatpak was installed. Tested source `90721189` combines main `f3f83a4f`
with the earlier installed `0029252d` composition and PR #211's generic decoder
arrival wake. It preserves PR #197's clipboard changes and PRs #200/#201's
stitching/color arithmetic, and includes merged CPU bounds and test cleanup.
The still-unqualified X3 horizon candidate in PR #212 is excluded.

This exact source passed the device-hidden workspace gate (1,544 reported
passes, 52 ignored, zero failures), full Clippy, formatting, vendor warnings,
naming and Cargo-source checks, eighteen synthetic harness checks and all six
CI jobs in run `34985135862`. Unavailable-GPU/media returns are included in
the workspace count, not additional hardware coverage. A separate SDK build
and actual bundle import authenticated the payload, metadata and license.
Native UI suites passed 51 X4 and 52 ONE X2 checks; app-path runtime suites
passed 41 and 42; after authorized installation, actual installed suites also
passed 41 and 42, all with zero failures. Every Flatpak suite authenticated
each running player executable, and both motion pictures from all six suites
were personally inspected.

The isolated suites retain their documented sound-device, portal, exact-view,
sandbox import-fault and cross-mount paired-fixture skips. Native import-failure
recovery and the separate Radeon shader/Rust-twin guard passed; that twin does
not establish SDK-shader provenance. No new kernel entries or scoped
memory-limit/OOM events appeared. Host GPU-memory, swap and pressure counters
varied. These are 1280x720 functional checks, not a new capacity result,
hitch-free verdict or proof that the historical desktop-freeze cause is fixed.
Private receipts remain in `scratch/cumulative-review-20260915/`. The then-installed
artifact was source `90721189`. After the requested normal-use check of
playback, seeking, pause/resume and seam/color regressions, the owner replied
"lgtm". This accepts the combined build, not a measured capacity result or
unresolved hitches. The owner then authorized merging #213 after the
documentation conflict and CI were cleared, adding "you dont need permission".
Routine qualified merges are delegated; new visible tradeoffs still require
owner acceptance and release qualification remains separate.

Merge preparation integrated main `005d4dcc`, bringing only the already-merged
CI timeout and roadmap changes. The sole conflict was resolved by retaining
both the decoder-wake and CI-timeout entries. All runtime and test-harness
sources remained byte-identical to the installed/qualified `90721189` artifact;
that package was not rebuilt or reinstalled for those documentation/CI changes.

That cumulative PR #213 subsequently merged at `e8089f9a` after all six CI jobs
passed on final integration head `60b5ed3e`; main's tree is identical to that
checked head and its post-merge CI also passed. PRs #200/#201 are marked merged,
and #197/#211 were closed as incorporated, with their evidence and branches
retained. Issue #174 is closed; #136/#186 remain open. That merge left the
installed package unchanged and was not a release publication.

Issue [#193](https://github.com/aeharding/kjerag/issues/193) hardens filtered
capture failure and cancellation. Restart honors a worker error recorded before
the final retirement transaction; terminal normalization preserves the first raw
error and installed picture even after state-lock poison. Unpublished temporal
history is canceled outside the state lock, with worker-exit cleanup covering
cancellation that loses the initial try-lock race. Deterministic regressions use
the real X4/ONE X2 filtered Scene, source history, stopped screenshots and seek
path. This changes failure-path lifetime and reporting, not successful stitching
arithmetic, source cadence or color policy, and is not a performance fix for #186.

Issue [#191](https://github.com/aeharding/kjerag/issues/191) removes unused and
unreachable code from the two local UI-library patches and adds an explicit
compiler-warning gate for them. Because they are excluded from the workspace,
`bash scripts/check-vendor-warnings.sh` selects each package through the root
lockfile and denies compiler warnings in the resolved application feature
graph. It does not claim a standalone all-features matrix or introduce a new
Clippy policy for the copied dependency code. No stitching, cadence or live
presentation behavior is changed by this cleanup.

Feature release **0.3.0** and packaging patch **0.3.1** are published through
GitHub and the signed channel. Complete issue #187's capacity/handoff decision;
publication, installed UI and license checks pass, while issue #186 retains
the unresolved performance evidence.
At the owner's request, that cumulative review Flatpak temporarily replaced the
signed release. Source `907211897664904b24d74e544c78c7aad84bb5d2`, then-installed
OSTree `660d46ac2815a721109ef7e2c76555de6830b499ba27b1f4d316f61d61c5c322`,
and executable SHA256
`440c490f6e3d1d847033fec0a758cc82e71542f1885dda6eb07fd0beeaae95ec`
identify the prior accepted private integration artifact, now retained for rollback.
It combines PR
[#197](https://github.com/aeharding/kjerag/pull/197)'s clipboard fixes with
PRs [#200](https://github.com/aeharding/kjerag/pull/200) and
[#201](https://github.com/aeharding/kjerag/pull/201)'s GPU optimizations,
PR #211's decoder wake and merged cleanup/diagnostic ancestry. It is not a
new published release. Installation preserved user data, runtimes, related
refs and remote settings; the origin remained `kjerag-origin`. The exact prior
`0029252d`/`b547c4e2`/`20982c8d` package is retained for rollback, as are the
verified signed 0.3.1 ref and release bundle. Owner clipboard and sampling
feedback and subsequent combined-build/merge approval are recorded above,
without accepting hitches or authorizing a new release.
Normal-channel restoration is due after review. Issue #146 tracks
deriving downloads from the signed build, avoiding separate payload qualification.

Before the cumulative installation, after the host GPU recovered on the same
boot from the observed 800 MHz / `0x604` throttle condition, the then-installed
`0029252d` combined package sustained 310.875 X4 and 317.824 ONE X2 completed
redraws/s in separate 2256x1504, requested-300-Hz,
40-second pans. Both advanced consecutive sources at 29.950/s. Completion
spacing p99/max was 10.941/22.136 ms for X4 and 9.094/22.475 ms for ONE X2.
Those notifications include queue/callback delivery, not precise shader time
or physical scanout; app-side spacing is also uneven. These runs meet the
average capacity/source-cadence requirement, not the 4.17 ms tail budget or a
hitch-free guarantee. No software fix or environmental trigger is established.
Issue [#186](https://github.com/aeharding/kjerag/issues/186#issuecomment-5675525548)
retains the measurements and remaining timing-consistency work.

The signed 0.3.1 60 Hz X4 control maintains 29.975 source advances/s under
the same 1,000 Hz mouse input, with 17.1 ms worst reported lateness. Earlier
lifecycle diagnostics show a longer composite stitch span in the high-rate
cohort without UI-event starvation; the remaining contention owner is not
isolated. This is not a 240-capacity or hitch-free verdict (#186).

On 2026-09-13 the owner directed continued performance work and other justified
code-quality attention after the published-release handoff. This is not
acceptance of the slowdown or a reason to close #186; the source cadence,
picture and actual-player verification requirements remain. The release flow
is recorded in [RELEASING.md](RELEASING.md).

On September 15 the owner approved and the coordinator merged CI-only PR
[#203](https://github.com/aeharding/kjerag/pull/203) at `005d4dcc`, after all
six CI jobs passed. It bounds FFmpeg PPA registration to five minutes; a
genuinely slow request may fail and need a rerun. This does not change the
installed player. For PR [#202](https://github.com/aeharding/kjerag/pull/202),
the owner also explicitly accepted withholding the entire release if signing
is unavailable or either x86_64/ARM build fails. That policy acceptance
did not waive qualification. That PR subsequently merged after its integration
and CI gates, as recorded above. The separately disclosed non-atomic GitHub/Pages
publication boundary is not newly accepted by that reply.
