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

- **Single signed release build, issue
  [#146](https://github.com/aeharding/kjerag/issues/146):** branch work replaces
  the independent unsigned-download build with app bundles exported from the
  signed multiarchitecture repository. Forty-three local release tests pass on
  Flatpak 1.14.6 and 1.18.1, including signature-preserving bundle import and
  isolated older-channel-to-newer-bundle upgrades, fresh bundle installation,
  settings retention and subsequent update, signed
  payload tampering, reference races, draft retries and version rollback guards.
  The workflow now stages both native
  builds explicitly and gates publication on a signed combined payload. The
  artifact-failure and publication-retry checks use disposable repositories and
  fake publication commands, not public writes. Device-hidden workspace checks
  report 1,513 passes and 52 ignored, including unavailable-GPU/media returns;
  formatting, Clippy, vendor warnings and source-list checks pass. Draft PR
  [#202](https://github.com/aeharding/kjerag/pull/202) initially passed the six
  existing CI jobs but exposed a client-version assumption in both new release
  jobs: Flatpak 1.14 refuses reinstalling an identical commit. The lifecycle
  test now requires a real different-commit bundle upgrade, checks that precise
  same-commit refusal without suppressing other errors, and verifies retained
  settings, origin and later channel updates. Publisher version validation also
  rejects malformed SemVer before contacting GitHub. All eight CI jobs pass on
  correction `ec3087a4`. Follow-on branch work adds a no-publication dispatch
  mode using a disposable signer through the same native build and artifact
  path, with an independent final download/verification job. Eight additional
  local checks exercise context validation, signer continuity and publication
  guards; together the fifty-one release checks pass on Flatpak 1.14.6. Dispatch
  from the reviewed branch succeeds, and all eight PR CI jobs pass at `4f769ab8`.
  Its first real validation run fails before compilation because Git rejects
  the container-owned checkout. Both native identity checks now trust only the
  exact workflow checkout for that command; a regression reproduces the ownership
  refusal and requires wrong paths and source revisions to remain rejected.
  Actual native build handoff and fresh CI for this correction remain open.
  No release, tag, signing secret or installed player was changed locally.
  This does not retire #186's performance target or pending picture reviews.
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
  PR [#199](https://github.com/aeharding/kjerag/pull/199) carries this diagnostic
  change; CI and review remain separate merge gates. This enables
  investigation of the reported July sky boundary, not a seam-quality fix
  or evidence of a difference from Studio.
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
At the owner's request, the qualified PR
[#197](https://github.com/aeharding/kjerag/pull/197) test Flatpak temporarily
replaces the signed release for a clipboard retest. Source `519e7545`, installed
OSTree `94688e18d630`, and executable SHA256 `9ed9b69b730c` identify this package;
it includes the merged cleanup fixes and changes no stitching or color
arithmetic. Both cameras passed four focused clipboard checks against the
actual installed ref, including matching restored view text and pixels.
This reduced-service qualification does not cover audio, portals or performance.
The test package remains installed pending owner retest; PR #197 is unmerged.
The verified signed 0.3.1 ref and release bundle are retained for rollback,
and normal-channel restoration is due after that retest. Issue #146 tracks
deriving downloads from the signed build, avoiding separate payload qualification.

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
