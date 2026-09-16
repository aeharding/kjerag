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

## Remaining work

- **X3 horizon convention, issue
  [#88](https://github.com/aeharding/kjerag/issues/88):** actual generic X3
  playback exposes an inverted horizon with the unverified X4 `xZY` fallback.
  A finite picture comparison of all 24 proper axis mappings, followed by a
  different camera pose, supports an X3-specific `xzy` calibration input.
  ONE X2's mapping does not transfer. The branch changes model selection only,
  retaining the generic projection/readout policy and existing X4/X2 mappings.
  [The evidence and limitations](research/x3-orientation-20260915.md) include
  the skyline check that could not rank this footage. The model-selection
  regression fails on the old fallback and passes with the correction. Full
  device-hidden workspace gates pass with 1,520 reported passes and 52 ignored,
  including unavailable-GPU/media returns, not hardware qualification. All six
  CI jobs pass on `5fc99fb2`. Native X3 playback captures are upright and moving;
  its UI suite has 46 passes and one exact-picture clipboard-return failure
  whose cause remains unproven. A host monitor/dock hotplug also coincides with
  an AMD display warning, so further GPU checks are paused for health review.
  Owner review, established-camera UI regressions and cumulative Flatpak
  qualification remain pending. This does not qualify all X3 firmware or add
  X3 to the resident stitcher.
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

### Current private installation, September 16

Merged main `1d5e9e46` is now installed as a private test Flatpak. It includes
the owner-accepted drag-release fix and the merged capture-pairing, file-opening,
view-validation, screenshot-safety and error-reporting changes. The selected
stitching/color arithmetic, source cadence, dependencies and Flatpak manifest
are unchanged from the previously accepted `90721189` package. The unqualified
X3 horizon candidate in PR #212 remains excluded.

The actual bundle passed its separate payload audit. Both before installation
and through the actual installed package, separate runtime suites passed
43 X4 Air and 44 ONE X2 checks with zero failures. Every launched player
executable was authenticated, and both motion captures from each of the four
suites were inspected. The documented isolated sound-device, portal, exact-view,
import-fault and cross-mount pair-fixture skips remain. This is functional
qualification, not new owner acceptance, a capacity result or a public release.
Exact identities, health qualifications and rollback are recorded in
[MERGE_READINESS.md](MERGE_READINESS.md#current-private-test-package-september-16).

The previous accepted `90721189` bundle is retained for rollback. Installation
preserved data, runtimes and remote settings; the origin remains `kjerag-origin`.
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
