# GPU stitching delivery: merge and release review

Current checkpoint: 2026-09-12. PR #183 is merged and 0.3.0 is published.
Release qualification remains open because high-rate X4 playback falls behind
in both the release and a restored accepted-build control. This page
records the cumulative delivery without requiring reviewers to reconstruct
the chronological experiment log in ROADMAP.

The active owner goal includes cleanup, merge and release. The living roadmap
is now separated from its verbatim [historical record](ROADMAP-HISTORY-20260912.md),
with a [research navigation index](research/README.md). The shared playback
slowdown, native/Flatpak differences and frame-time spikes are tracked in
[issue #186](https://github.com/aeharding/kjerag/issues/186).

## Accepted tradeoffs

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

The branch is based on main `67c7fea25ec57c0eb6e8d6d63940e9d63303dd67`, verified
against the remote on 2026-09-12. There are no main-only commits to integrate.
Keep the tested runtime intact rather than reorder its dependent implementation
commits. Preparation changes documentation, store description and ignores for
local build/Python caches, not player code, shaders or dependency versions.

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
is pending a follow-up PR and release; it does not retroactively change 0.3.0.
The next published package must be checked for that exact file and content.
The local Builder successfully resolves the updated manifest; staging its
actual license-install command produces a byte-identical copy of `LICENSE`
(`0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0`).
This is a manifest/command check, not a newly published-package pass.

The follow-up tree's unchanged Rust code passes the complete local gates in
`scratch/merge-readiness-20260912/release-record-gates-01/`: 1,496 workspace
tests with 52 ignored, formatting, workspace/all-target Clippy, source/name
checks and the ancillary gates above. Source/name and whitespace checks pass
again after the documentation and explicit license-install changes.

Release notes describe:

- Automatic GPU stitching and lens color matching for ONE X2 and X4 Air.
- Reduced flickering seam artifacts, including rotating-camera and panoramic
  wrap-boundary cases.
- Reuse completed stitch results while reframing; prepare the first picture
  before playback and seeking, and retain video state when controls appear.
- The known X4 high-rate panning slowdown and unfinished combined capacity
  qualification, without turning earlier good measurements into a guarantee.
- Tested ONE X2/X4 Air scope and Studio-like rather than identical output.
