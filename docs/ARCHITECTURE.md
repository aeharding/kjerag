# Architecture

## Layers

One crate per layer, in a cargo workspace (issue #19). A layer cannot use
a dependency it does not declare, so the diagram below is enforced by
`cargo build` rather than by good intentions.

```
crates/app      kjerag         libcosmic shell + window. The view is an
                               `iced::widget::shader` around a Scene, and
                               the mouse reaches it through that widget.
crates/render   kjerag-render  wgpu: dmabuf import, final WGSL pass (NV12 ->
                               RGB + projection); shared ONE X2/X4 Air GPU
                               luma sampling, sparse PIS, retained state, dense
                               map construction and direct resident draw;
                               camera state and offscreen screenshot rendering
crates/media    kjerag-media   ffmpeg demux, dual VA-API HEVC decoders in
                               lockstep, presentation clock, play/pause,
                               frames by index or timestamp. One demuxer per
                               file of the capture, which is two on the
                               cameras that write one lens per file. No UI.
crates/meta     kjerag-meta    .insv trailer, read directly: per-lens Mei
                               calibration, gyro track, per-frame exposure,
                               global ISO observations for denoising.
                               No UI, no ffmpeg, no wgpu.
crates/spike    kjerag-spike   the headless instruments, which nothing else
                               depends on: `spike` (M0 frame-path timings)
                               and `reframe` (the projection pass to a PNG,
                               no compositor needed)
```

`app` -> `render` -> `media` -> `meta`, `render` -> `meta` as well for the
calibration the shader runs on, and `meta` depends on nothing but `prost`.
That last one is the point of the split: `cargo test -p kjerag-meta` passes
on a box with no libav headers, and a CI job that installs nothing proves it
on every push.

`CalibrationSet::denoise_iso` holds the record-9 observations and summary
decoded by the Studio-read constructor law. This is a global denoising input,
not per-lens gain or a photometric correction. Its millisecond origin discards
the native 40-item prefix, independently of the shutter clock, preserving the
native signed/wrapping operations. Missing data supplies no observations;
metadata parsing chooses no ISO100 default. Frame-time interpolation, invalid
ISO handling and backend parameter selection belong to the temporal consumer,
not this parser. `temporal_fusion::iso::Lookup` now owns a single open-time
snapshot and implements the recovered binary64 bracketing/interpolation,
forward-zero cache and invalid-value correction. Empty or unordered input is
refused; backward query time requires an explicit reset, which also clears the
cache. It does not select filter parameters or invent missing ISO values.
`CalibrationSet::source_group_type` preserves the presence and int32 type of
protobuf FileGroupInfo (tag26/type1), including an explicitly present zero.
`settings::Provider` combines source metadata, source fps and the ISO lookup
for the authenticated X4 Air and ONE X2 selector routes. It returns per-source
radius, motion-confidence inputs and fusion settings using the recovered native
table interpolation. Unsupported sources or missing ISO are errors, not guessed
defaults. Reset clears the ISO lookup's chronology/cache. Ordinary file opening
selects this provider once during live opening, before dropping the full
calibration. Authenticated supported sources select the temporal stream below;
unsupported selectors explicitly retain the existing spatial stitch. Invalid
ISO on a supported source is an error, not a silent fallback. Stepped stills do
not construct a temporal provider.
`docs/research/studio-denoise-iso-602.md`
records the binary and real-file authority.

`spike` has no dependency on `app`. Its stitch instruments default to the
factory calibration, which is the parity base. A research run may name the
five seam knobs explicitly, but the removed `file` and `pool` values cannot
silently select a content-fitted pose. The app's saved seam pool and the
upward dependency that exposed it to instruments are gone.

`media -> meta` is one function wide and it is issue #79's: a capture is not
always one file, and which file holds the other lens is a fact about `.insv`
naming, which `meta` owns. `media` owns what a container is and does the
verifying half with it (`Shape::pairs_with`), because the second file of a
per-lens pair carries no trailer to check against.

`media` and `meta` know nothing about the shell. `render` names libcosmic
for exactly one file, `crates/render/src/widget.rs`: those three
`iced::widget::shader` impls put a foreign trait on types `render` owns, and
Rust's coherence rules will not let the shell crate write them. The
alternative was a set of forwarding newtypes in `app`, which is more code
for the same wiring. Nothing else in `render` mentions iced.

The shell is libcosmic, which pins wgpu 28, so `render` is written against
28 and owns the one module that wgpu 30 would delete
(`crates/render/src/dmabuf.rs`).

The shipped player draws full-resolution prefiltered source/map
samples with a reduced full-sphere temporal residual. The owner accepted the607
quarter-field and subsequent prefilter moving comparisons, with exact responses
recorded in ROADMAP; this is not broad footage or live smoothness acceptance. The two
low-resolution terms pass through the same gamma RGB/NV12/RGB conversion. Their
signed difference is bilinearly sampled, added to explicitly quantized direct
RGB8, clamped, then transferred to the surface in one render pass. Rectilinear
views use the existing native mesh; curved views use the existing body-ray map.
Correction textures extend picture group0, retaining map group1 and colour
group2 within the native three-group limit. Pipelines cache by surface format.

The shipped player materializes the selected temporal input in canonical
world coordinates, before filtering, for the1147 moving-line report. The owner
accepts both the stabilized-input diagnostic and, on2026-09-12, the installed
player ("looks good. not perfect but pretty damn good"). That exact package
passed both-camera UI and an earlier rendering-capacity cohort. Current release
qualification reproduces the accepted pictures but does not sustain full X4
source cadence, including after restoring the accepted package; those cohorts,
the unresolved target and the separately rebuilt signed package are distinguished
in ROADMAP. This is tested-build visual acceptance, not a current unconditional
performance guarantee, all-footage coverage or exact Studio parity. Scene retains actual
`body_from_world` independently of the display's horizon toggle. Its source
Reframe has zero camera yaw/pitch and a fixed aspect, so mouse movement and
window size cannot steer the temporal field. A separately cached world shader
specialization samples the existing source/map with that transform; ordinary
body/full reference constructors keep their body-fixed chart.

`WorldPanorama` seals the exact prepared source-uniform columns to its stamped
RGB texture. `CorrectionInput` retains those columns with the same source/map
owner while the coordinate-neutral `RgbPanorama` enters the unchanged temporal
stream. Each completed center source carries its own48-byte immutable matrix
buffer. The final correction lookup transforms the actual displayed body ray
back into that source's world chart; it never derives the transform from the
redraw Reframe. This adds binding9 to picture group0, not another bind group or
render pass. The matrix buffer is allocated at source cadence and shared by
view redraws. Temporal weights, history, raster resolution, source cadence and
high-detail prefilter remain unchanged. The world chart's different sampling
grid can change motion/noise decisions; the owner tested the installed version
separately rather than transferring the diagnostic's acceptance automatically.

The source-rate prefilter changes only the corrected
display's high term: it evaluates the existing native box at every source
texel centre in the existing snapshot passes, then uses atlas bilinear reads
at view time. Full dimensions do not mean unchanged detail: prefilter storage
quantization and later interpolation can soften detail or change noise. This
was disclosed and accepted for the607 movie and test build, not all footage.
A private snapshot constructor and corrected-only shader specialization keep
prefiltered storage away from raw
body/map/color/temporal inputs. Native-box reference draws default to unchanged
sampling; both paths share the original atlas boundary and box WGSL functions.

The selected correction raster preserves the preceding half-field candidate's
original-source ownership and display rules. Its five-level field is1920x960
for X4 and1408x704 for ONE X2. The latter rounds the natural
quarter height down to a multiple of32, preserving a2:1 full-sphere raster
without partial finest/output motion blocks; it does not crop source coverage.
Level3 still supplies exactly full-field/16 luma coordinates. Five and six levels use the
explicit reduced finest-size entry; the full seven-level oracle keeps its
original minimum. Five through seven matching nonempty pyramids are admitted,
never padded or fabricated levels. The larger angular search support and
changed noise/motion-edge output were disclosed before the owner accepted the
607 moving Studio comparison ("Yes, looks acceptable"). That approval is not
acceptance of all footage, live-player performance or a merge. The
preceding quarter bundle passed40 X4 and44 ONE X2 UI checks; ROADMAP records
that dated prefilter package's installed qualification and exact source/executable
identity. The preceding package is retained for recovery.

The selected quarter path uses a periodic-horizontal temporal specialization
for the owner-confirmed680-second sky defect. The visible boundary follows the
body panorama's0/360-degree cut, not a lens handover, and the owner confirms it
is absent from the moving Studio comparison. Pyramid reduction, motion search,
final motion packing, reference fusion and both residual terms' chroma
reconstruction now wrap actual image X; Y keeps its existing boundary rule.
Coarse predictor grids wrap only when their blocks cover the complete physical
image width. Partial coarse tails retain endpoint predictors. Only the selected
quarter constructor opts in; full/half and native saved-input reference paths
remain clamped. The owner accepts the moving Studio comparison and calls the
old/new result "Looks fixed" for sky680. The installed source `dad5d709` Flatpak
contains this same specialization;
ROADMAP records its exact package identity, both-camera installed UI checks
and separate historical capacity measurements. Its installed live-player verdict
was pending at that checkpoint; the later world-coordinate package received the
owner verdict recorded above. History, source cadence and filter weights do
not change. Evidence and moving comparisons: `scratch/sky680/quarter-periodic-01`.

The resident source worker encodes the reduced RGB body and display snapshots
of both original lens planes in one command buffer. The previous quarter build
used exact texture-load copies; the current branch and installed build prefilter
in the same two MRT passes, reading both original lens planes at the internal
atlas join. Imported dmabufs are
sampled-only, so no unsupported COPY_SRC use is invented. The first body pass
arms submission-complete retirement before any source sampling. Its retirement
covers the later snapshot passes too; a later encode rejection explicitly closes
admission and quarantines uncertain owners while preserving the original error.
Snapshots are ordinary GPU-owned R8/RG8 textures, not aliases of VA-API surfaces.
The snapshot path explicitly requires8-bit planes; higher-bit-depth snapshot
support has not been qualified. The installed map snapshot clones only immutable
read/fusion bind groups, exact frame and context, not the heavyweight carrier.

`CorrectionStream` owns the low unfiltered controls and reduced-level temporal
Stream. `CorrectionSequence` retains at most seven full-resolution display/map
snapshots and moves each into its exact completed `CorrectedFrame`. Readbacks and uploaded
maps exist only in the older paired review harness, not this live path. Scene
publishes and screenshots these typed frames. There is no coefficient EMA,
skipped source refresh or interpolation between correction updates. Halving the
field changes angular block support and motion decisions, so it is deliberately
not claimed to be an equivalent arithmetic optimization. The full-resolution
seven-level and preceding half-field Stream constructors remain test oracles.

`render::temporal_fusion` supplies the stream's post-stitch primitives.
It consumes explicit full-range NV12 image arrays,
current-to-reference displacement/confidence grids, a luma-index grid and
effective fusion parameters. The full-plane `GpuFuse` reference records two
render passes producing GPU-owned R8/RG8 planes. The selected `packed::Encoder`
instead records one half-size MRT pass producing RGBA8 Y quartets and RG8 UV.
Each even2x2 footprint shares its flow/luma lookup while retaining the same
ordered per-component fusion and normalized attachment quantization. The packed
converter reads the correct Y lane directly and retains full-resolution centered
chroma reconstruction and RGB output, without an intervening unpack pass.
Neither primitive submits, waits or reads pixels back. Studio's selected
implementation uses compute; these render targets are Kjerag's execution
choice. The caller must supply prepared images and own source association,
history, seek epochs and completion. None of that scheduling is supplied by
this primitive. Saved native input/output tests establish its bounded
arithmetic result, not a complete temporal pipeline or performance verdict.

Its `stream` child supplies worker-owned execution over the requested field.
Seven real sources emit startup
centers0..3, steady center3 and flush4..6. The source's matrix and retained
automatic settings travel with its exact stamp. Full RGB/NV12 conversion,
GPU pyramid/search/refinement/fusion now use resident images throughout. Arrival
encodes the selected number of packed motion-pyramid levels and submits without
waiting for CPU pixels. Coarse levels down through one produce GPU records; histogram,
global prediction and seed interpolation feed the next level directly, followed
by the finest search. Motion packing reads the retained level-three luma texture,
not an uploaded CPU copy. Source stamps and effective settings remain in the
same seven-source history. Same-queue ordering establishes preparation/search
dependencies; only final filtered-output publication waits for completion, using
callbacks and nonblocking device polls on the temporal worker.

The first validated source push prepares both coarse-search pipelines on that
worker, before adding history. Later ISO transitions therefore reuse compiled
pipelines instead of compiling during playback. This records no GPU work and
does not eagerly build motion images for radius-zero sources. A fresh seek
epoch prepares its own pipelines during buffering; preparation is not free
startup work. The measured first-source delay and narrow activation-hitch
comparison are recorded in ROADMAP, separately from overall capacity.
The owner accepted the roughly0.1-second first-picture delay for this test
build after its meaning and the unmeasured extra seek delay were disclosed.

This candidate deliberately changes coarse search execution semantics: blocks
read immutable same-level predictors instead of serially updated neighbours,
and omit the row-major bad-block counter and adaptive UMH recovery. Level order,
strict candidate ties, penalties, fixed-center radius-two search, global
prediction and interpolation remain. Both the preceding serial CPU oracle and a
readable CPU reference for the new independent-block candidate remain available.
GPU/CPU candidate equality does not establish Studio-like moving output. The
owner accepted the current 607-second moving Studio comparison on 2026-09-10;
the separate 612 comparison and live performance remain unqualified. GPU work
still has dependent levels and one large filter submission/completion boundary
per output; GPU residency alone does
not establish smooth drawing or full-rate playback.
The preceding full-resolution reference producer draws to packed-Y and UV
attachments, avoiding the disposable full-size RGB panorama. It samples four
full-resolution centres per fragment, explicitly quantizes each gamma RGB
sample to RGB8, then applies the existing full-range NV12 conversion. History
unpacks the four Y lanes and copies UV into the arriving source's reserved
layer. The typed single-layer luma view supplies the pyramid. The old RGB
producer and direct RGB-to-history path remain test oracles.
The compact owner seals device, source stamp, matrix and geometry. Its shader
reuses the exact lens-sampling Reframe for size and matrix; the caller rejects
scaled targets before allocation. This keeps the existing three bind groups,
including photometric fusion, within the native renderer's limits. The initial
four-group prototype passed adapter-max tests but failed native startup; a
three-group GPU creation regression now exercises that boundary. Compact
sampling is not byte-identical to full-resolution RGB rasterization and still
requires moving-output review; the owner's preceding607 acceptance does not
automatically cover this representation change.

That compact reference producer additionally caches the 51-by-101 native mesh
vertices once per source/map. A164,832-byte GPU buffer retains position and
packed-map samples, including the distinct column100 endpoint. Its compute
prepass and panorama draw share the existing encoder and source retirement.
Per-pixel triangle selection, interpolation, alpha/fusion sampling and source
cadence remain unchanged; installed map buffers remain private and frame/device
sealed. This removes repeated vertex trigonometry and map sampling from millions
of body pixels, not full-resolution sampling or temporal work. The uncached
compact and RGB producers remain diagnostic oracles. Both-camera comparisons
find sparse one-code YUV differences, so numerical identity is not claimed.
Native new/old/new playback improves from14.57 to16.12–16.28 source fps, but
maximum pre-prepare `native-pump` shown-report gaps are worse in those cached
samples. Those reports sample the prior installed frame before the current
redraw's prepare and therefore do not by themselves establish visible pauses.
A later unchanged cached control reaches15.66fps without that long gap, so
cache causation is not established. Reusing one workspace across sources gave
no further throughput benefit and was removed; the retained producer allocates
one per source/map.
Neither smoothness nor active240fps capacity is established, and this candidate
is not installed.

Deriving the initial body cell from panorama UV was evaluated and removed.
Direct latitude disagreed with the existing GPU inverse on both camera rasters;
the longitude-only refinement preserved the tested seeds and rendered sequences
but gave no dependable native throughput gain. The retained renderer therefore
uses the original inverse and search, without the extra body-selector wrapper.

A radius-zero
source initially needs no motion inputs, but a later center may reference it.
Missing inputs are then reconstructed once from its exact retained unfiltered
NV12 layer; no lens sampling or colour-coefficient update runs again.

`FilteredCaptureFacade` separates processing from presentation. The resident
stitch worker prepares source/map/color and body panoramas while a single temporal
executor filters the preceding source. At most two sources may occupy those
stages within an epoch. Admission reserves the four-picture ready capacity,
including the seventh arrival's four startup outputs; EOF waits for both stages
to drain before reserving its three tail outputs. Only completed outputs publish.
The executor and its capacity-one channel are shared across seek restarts. Each
epoch owns a fresh sealed Stream, and only that executor may mutate it. A full
channel blocks the stitch worker, never the UI, bounding old work across rapid
seeks to one executing job, one queued job, one blocked stitch handoff and one
queued stitch job. Alongside current and last-shown owners this retains at most
six distinct epoch owners in the product seek path. A successful restart now
cancels its predecessor's temporal epoch and clears unpublished ready outputs.
Idle history drops immediately through a nonblocking try-lock; executing history
drops after its current operation. Old completed Shown remains independent.
Errors stay with their
epoch and preserve the first underlying failure. There is no per-seek worker
thread, UI join, changed source cadence or changed filter arithmetic.
Panorama source import retries only classified resource exhaustion before the
stitch transaction begins. The exact decoded pair and reserved draw slot remain
on the bounded stitch worker, with a 1 ms scarcity backoff and the existing
two-second import limit. The deadline is checked before another attempt after
waiting. No successful import sleeps; invalid descriptors fail immediately.
Exhaustion beyond the bound passes the last underlying error to the existing
capture-terminal handoff. No later stitching or filtering failure is retried.
At most four ready corrected frames and one installed frame remain per epoch.
Original source snapshots move from temporal pending into those outputs rather
than being duplicated. Decoder leases retire after body/plane-copy preparation;
corrected outputs own all sampled resources independently of decoder surfaces. Only the
exact due FIFO front may install and acknowledge Player's current delivery.
Scene drains six prepared successors even during paused startup/seeks, never
advancing picture/audio time merely to satisfy the filter. A completed panorama
and its acknowledgement survive renderer recreation together; a seek replaces
the facade while the old shown facade retains its last picture. ISO restarts
share immutable observations but reset lookup chronology and zero-cache state.

The live panorama projector renders that typed completed texture into the
existing surface pass with the same gamma/linear convention as direct drawing.
The packed-filter integration retains all62 saved607/612 Scene frames exactly
and improves controlled native source throughput from15.12 to17.32–17.39fps.
These1280x720 runs still show81–91ms maximum source-frame gaps; they establish
neither full-rate playback nor the2256x1504/4.17ms capacity target.
Changing view only changes its projection binding, not filtering or colour
history. Screenshots use the exact installed panorama and surface format. EOF
flush waits until all real prepared inputs have been accepted. Near-EOF exact
seeks decode the last seven real sources, holding picture/audio time at the
requested target and preventing pre-target outputs from replacing Shown. Whole
captures shorter than seven sources still produce an explicit unavailable-output
error. Full X4 and ONE X2 geometry use seven nonempty 16x16 search grids; odd
pyramid dimensions floor-halve and discard the unmatched tail. Different native
level-count routes remain unsupported. Automatic selection is not a playback
capacity or installed-quality verdict.

The `history` child owns seven resident NV12 array layers and the exact source
stamp occupying each slot. Arriving NV12 images can be copied once; the selected
compact producer unpacks/copies directly into the same validated slot. A complete borrowed
window supplies center/reference stamps and physical layer indices in logical
c-3,c-2,c-1,c+1,c+2,c+3 order. An explicit `window_at(center, radius)` also
supplies the clipped intervals needed for startup and tail frames: ascending
logical order excluding the center, with no repeated or fabricated neighbors.
The ring must still contain seven real sources. A zero radius has no references
and cannot be sent to the fusion primitive; `encode_copy_current` instead copies
the selected physical Y/UV layers into an independent sampled output. Reference
phases use the recovered binary64 raised-cosine law over the actual clipped
interval. Neither accessor grants processing
or presentation authority; the caller supplies the center and effective radius.
It records copies without submitting or waiting.
Its caller must submit earlier consumers before a later overwrite and create a
fresh owner for a new decode epoch. It rejects gaps, duplicates and invalid
storage before recording. Resources must share one wgpu Instance: the pinned
native device equality only diagnoses different devices within that Instance,
not separate Instances with colliding local IDs. This is storage ownership,
not player startup, seeking, padding or scheduling policy.

Its `pyramid` and `motion` children retain readable CPU references.
`pyramid` takes an explicit gray base image and level
count, retaining Studio's two separately rounded reduction passes. `motion`
takes explicit raw displacement/cost, luma, geometry, tables and reference
phase; it implements the verified16x16-block,2x expansion/confidence path.
Unsupported geometry is rejected, not assigned new semantics. Neither is a
motion search, calibration policy, image-history owner or player scheduling
component. Hash-sealed native tests cover seven pyramids and six packed
motion grids. The selected worker uses resident GPU pyramid preparation and
coarse search; CPU readbacks belong to the separate offline reference path.
View redraw does neither.

The `search` child is a readable serial CPU implementation of the selected
seven-level, gray, pel-1 motion search. Its GPL-3.0-or-later adaptation preserves
the pinned MVTools attribution and the separately read native changes. It
reproduces the standalone combined adapter, not every native vector. It owns
neither source history nor player policy. Its 16x16 SAD leaf uses checked row
slices, byte absolute differences and an exact u32 sum, converted back to the
existing i64 penalty arithmetic. This exposes automatic compiler vectorization
without unsafe code, CPU-feature dispatch or changed search decisions. The
original scalar leaf remains a test oracle; the separate parallel-refinement
CPU oracle is unchanged. The `color` child supplies an explicit
diagnostic gamma-RGB/full-range-NV12 conversion; it is not a claim that Studio
uses that matrix inverse, quantization or centered chroma footprint.

The separate `parallel_refine` candidate changes the finest motion-search
algorithm: blocks read immutable coarse seeds instead of newly searched
neighbors, and omit the serial bad-block/UMH recovery. Its readable CPU oracle
and GPU kernel keep the initial candidate order, SAD penalties, bounds and
fixed radius-one ring. The GPU assigns one workgroup to each block/reference,
with distinct immutable input and output buffers. Runtime slices accept one
through six matching references/seeds/globals; allocation and dispatch use that
actual count. Each reference has one dispatch with a directly bound reference
texture, removing the six-way texture switch inside each SAD load. The dispatch
uniform selects its original contiguous seed/output slice. Typed output carries its reference
count into motion packing. Eight teams of eight lanes
cover each candidate's complete 256-byte SAD. Gray inputs use typed, immutable
`PackedGray` textures from the pyramid producer: four horizontal bytes in rgba,
with logical dimensions distinct from physical packed width. Each lane handles
two rows, using four aligned or five unaligned texel reads per row and exact
component swizzles. The current block is 64 vec4 words, loaded once per workgroup.
Team sums remain bounded exact u32 arithmetic, followed by the same ordered
strict-tie decision. No padded tail byte enters a legal SAD.
This is not Studio-exact
search or an accepted quality tradeoff. Coarser preparation still uses the
existing CPU reference; no player scheduling or color-update rule changes.

`pyramid::gpu` now records the matching brightness preparation on the GPU.
An R8Unorm full-Y input is averaged into a half-size R8Uint base; alternatively,
an explicit R8Uint base can be copied without that bridge. Each later level
uses distinct vertical and horizontal R8Uint render targets, preserving the
intermediate byte rounding. The builder validates geometry before encoding,
returns owned logical-level textures, and never submits, waits or reads back.
Its separate `encode_packed_base` records R8Uint to Rgba8Uint packing with
explicit zero tail lanes. The offline Scene packs once per arriving source in
the existing pyramid submission, retaining that typed image instead of its R8
base. Original R8 levels still supply CPU readbacks. This preserves source
stamps and seven sampled bindings; it adds no history owner or queue wait.
Frame stamps, source ownership and history remain the caller's responsibility.
The optional offline Scene selection still reads those levels for CPU motion
search, so it is not a GPU-resident complete temporal pipeline. Its exact
reported-view comparison preserves all filtered and unfiltered artifacts.
Native captures verify the reductions from the half-size base; they still do
not provide same-input native authority for the initial full-Y bridge.

`motion::gpu` expands supplied raw search vectors and packs displacement plus
independent Y/UV confidence into an Rgba16Sint texture. The scalar phase and
threshold preparation remains on the CPU in the reference operation order;
the per-pixel GPU work needs no optional wide numeric types. Its explicit
subset requires i16 raw displacement, nonnegative 16x16 byte SADs and positive
thresholds at most46340. Other thresholds, including the CPU reference's
wrapping-square cases, are refused rather than approximated. Encoding submits
and waits for nothing. Raw vectors and luma are still CPU uploads, so this is
not GPU motion search. The offline caller may separately run the six pure CPU
reference searches concurrently, preserving their supplied order. Searches
within each reference retain their serial predictors and candidate order.
An alternative typed handoff accepts the validated parallel-refinement output,
selects one of its actual reference slices and records packing without an
intermediate readback. Geometry, device, phase and confidence validation remain
at that boundary; arbitrary unvalidated GPU buffers are not accepted.

The test-only `scene::panorama_review` path materializes each exact displayed
source/map/fusion into a body-fixed RGB panorama, then projects it through the
same locked view. An unfiltered NV12 round trip isolates representation changes.
The body-image producer also accepts a linear `ResidentScreenshotDraw` from
the selected player carrier. It draws the exact resident source, final map and
fusion bindings without map readback/reupload, deriving the output stamp from
the imported source and checking it against the installed map. The existing
bounded draw retirement holds the complete source/pass until completion;
abandoned command buffers retain the same fail-closed ownership. Linearized
output is refused before arming. The shared body pipeline is lazily cached.
`KJERAG_PANORAMA_RESIDENT_INPUT=1` selects this route only in the offline review;
its first source checks the entire panorama against the uploaded-map oracle.
Supported live playback consumes the same upstream resident stitch through the
filtered panorama owner; explicit spatial controls retain the direct-map draw.
The separate `ResidentPanoramaIngest` is the non-presenting source producer for
that integration. It owns a fresh resident session and cannot share a root with
the display facade. A typed job on the existing bounded stitch-worker channel
imports one exact decoded pair, runs the unchanged stitch/color transaction,
then commits only its computational successor. Neither the raw future nor ready
display slot is populated. A draw-retirement permit is reserved before admission
or computation; its immutable source/map/fusion pass survives through panorama
submission completion. The returned panorama owns independent compact YUV
textures and its source stamp, not a decoder surface. The RGB reference and
compact live producer share the same import/map/commit/reframe transaction.
The filtered facade above supplies bounded admission and publication.

`Player::prepare_ahead` supplies the corresponding explicit source horizon:
up to six successors may be retained without presenting one, including while
startup or a seek landing is paused. The current source must belong to the
newest requested epoch; pending seek notes stay available to ordinary promotion.
Every accepted successor must be adjacent. Stale notes are discarded, while gaps
and decoder failures remain errors. `is_input_exhausted` distinguishes decoder
EOF from presentation EOF; if six slots are full, the trailing EOF note is read
after a slot frees. The ordinary two-frame lookahead, reader depth, presentation
clock and startup acknowledgement policy remain unchanged. These preparation
interfaces do not themselves publish filtered frames. The selected facade and
Scene route above supply that separate publication boundary.

The offline temporal review keeps seven contiguous source-stamped NV12 images
and gray pyramids. Settings are either the explicit captured ISO100 regime or,
with `KJERAG_PANORAMA_TEMPORAL_TRACK=1`, the source-track provider above. These
diagnostic selectors are mutually exclusive, not user calibration controls.
After seven real inputs it emits startup centers0..3, then center3 per successor,
and flushes centers4..6 at the end. References are radius-clipped, source-ordered
and checked against history stamps. Radius0 copies current after the same gate.
Fewer than seven sources produce no filtered outputs at this backend boundary;
this does not establish Studio's higher exporter short-seek behavior.
The first output records seven arriving copy pairs, each subsequent arrival one,
and additional startup/tail outputs none. Copies precede consumers in their
encoder. Optional GPU
pyramid/packing and concurrent CPU search routes change execution only;
CPU search and explicit readbacks remain offline
reference execution, not the player architecture. The source image and its
color corrections do not change between comparison arms. This diagnostic does
not select a player seek/publication policy or gradual color update, and its
explicit CPU waits are not a performance path. Production playback instead
selects the separate worker-owned stream above, not this offline controller.
The additional explicit parallel-refinement diagnostic is different: it selects
the changed spatial algorithm described above, retaining each GPU base beside
its exact source stamp and NV12 image. Refinement, packing and fusion share one
encoder. Its receipt discloses the algorithm change, and its timing separates
CPU coarse preparation from GPU work plus completion/readback.

The test-only `KJERAG_PANORAMA_GPU_TIMING=1` selector adds encoder timestamps
around history copies, finest refinement, motion packing, fusion, conversion,
projection and picture-readback copy. The timestamps share the original
submission; their resolve/map is reported after picture completion, not a new
wait between stages. The intervals exclude host work and implicit queue-write
uploads preceding that command buffer. With the selector absent, instrumentation
allocates no GPU resources and changes no device feature requirements. These measurements
locate expensive diagnostic stages, not actual-player capacity.

The optional view-scissor diagnostic keeps full-sized fusion Y/UV and converted
RGB targets, their absolute coordinates and the unchanged panorama projector.
`temporal_fusion::regions` conservatively bounds one exact rectilinear `Reframe`;
unsupported or uncertain geometry uses full coverage. RGB bounds include the
projector's horizontally periodic linear-sampling footprint. Separate fusion
bounds expand by two Y texels, clamped on both axes, so halving them supplies
the centered-chroma reconstruction halo. No shader arithmetic or intermediate
quantization changes. Scissors restrict fragment execution, not allocations,
full source history or motion search. Cleared pixels outside those bounds are
not valid filtered picture data: the caller may project only the prepared view,
not reuse the partial panorama after a view change. Ordinary playback selects
none of this diagnostic and gains no changed-view cache or scheduling policy.

The shell's pinned `iced_wgpu` renderer is locally patched to request the
adapter's supported storage-buffer count. Its fixed default of eight caused
the resident ONE X2 pipeline to panic at startup in the window, despite
passing headless checks whose devices requested adapter limits. This is only
device configuration: no renderer arithmetic or stitch layout changes.
`vendor/iced_wgpu/KJERAG.md` records the source, license and removal condition.
The selected capture session checks its fifteen-buffer requirement before
pipeline construction and reports an ordinary failure when it is unavailable.

The same local renderer patch owns native presentation readiness. Window
rendering prepares visible custom shader primitives before acquiring a surface
image or preparing iced's built-in UI batches. When the selected Scene has a
previous resident picture but preparation has no exact resident draw, it
reports the window unavailable. Exhausting its two draw-retirement slots is
the measured frequent cause. The compositor skips that physical
presentation, so the previous complete compositor buffer remains visible;
there is no cleared or UI-only replacement. Ready windows retain one combined
UI preparation and render submission, and offscreen rendering remains ungated.

The first unavailable preparation always asks the shell for an immediate
redraw. This bridge is required because `Scene::pump` runs before preparation
discovers the first Full result. On that next tick, a Full pending refresh is
scheduled for `now + 1 ms` instead of requesting another immediate redraw.
Scene advertises self-scheduling only while its post-prepare pending-refresh
flag is set; Empty and target-mismatch states keep the renderer's default
per-attempt redraw fallback. A ready aggregate resets the episode. The vendor
trait defaults to that fallback and explicitly supports one independent
self-scheduled widget per window; Kjerag has one Scene. This is admission and
host retry scheduling only: it neither waits for GPU completion nor changes
the two-slot capacity, source lease, shader arithmetic or presentation clock.
The 1 ms interval is a measured Kjerag policy under qualification, not a Studio
constant and not proof of 240 Hz physical presentation.

When the only remaining work is an admitted due source's running stitch actor,
Scene can now sleep until that actor commits the exact result. A single
coalescing wake belongs to Scene; its shell subscription stays alive during
paused seeks and final-frame landing. Registration shares the capture-state
lock with temporal commit, and refuses to sleep if any publishable future
already exists. Only an explicitly awaited due stamp wakes the shell, not
speculative lookahead. The subscription message requests a normal redraw;
the presentation clock remains in the widget's redraw event. Mouse and UI
redraws remain independent. Missing listeners, admission backpressure, retired
resource drains, ready futures and full draw slots retain their existing
retries. No source is skipped and no older map is applied to newer video.
The real-camera sequence tests exercise worker notification without renderer
polling and compare the original images, maps, alpha and fusion ratios.

`Size` and `Fallible` live in `media`: they are frame types, and `render`
depends on `media` rather than the other way round. `render` re-exports both
and adds the `Extent` trait, which is the `wgpu::Extent3d` half of `Size`
that cannot live in a crate with no wgpu.

The pinned iced core also has a local named-child reconciliation correction.
When COSMIC restores its header, its named header is inserted before its named
content container. The original algorithm diffed the retained content in its
old slot, then overwrote that slot with the new header without appending the
content. The next redraw had no Scene widget and therefore no media deadline
request; a controls Tick rebuilt it roughly 250 ms later. The patch reconstructs
child state in new widget order, retaining named survivors across insertions
and removals. It changes no shell layout, frame clock, stitch calculation or
GPU queue policy. `vendor/iced_core/KJERAG.md` records its exact provenance and
removal condition. The app has a direct regression for the actual dependency
operation; `scripts/uitest-controls-wake.sh` exercises repeated real pointer
wakes in a private native compositor. Its 100 ms pump-gap rejection is a
specific quarter-second-pause regression guard, not the 4.17 ms capacity gate.

The resident ONE X2 draw path has a separate, private source-import
owner in `direct_type2`. Its only production constructor consumes the exact
`Arc<Frames>`, requires two lenses and imports both descriptors directly into
`[Planes; 2]`. It has no raw-plane or stamp-only association boundary, and
exposes no bind-group or texture handle. An actual render pass creates its
own immutable picture binding, retained with that exact source owner until
completion. Field order releases the pass binding before its source owner,
and the imported planes before their decoder frame owner.
Selected Scene playback reaches it only through the capture-owned resident
session and renderer attachment; the legacy `VecDeque<Live>` path remains an
explicit diagnostic/oracle boundary and is not a selected fallback.

### Shared camera boundary (issue #184)

Live admission produces one immutable `ResidentCameraProfile` while opening
the capture, before selecting sequential playback or constructing GPU state.
It owns the resolved parent inputs, source dimensions, static maps and image
support, without retaining the full raw calibration in production. Renderer
attachment consumes those prepared resources;
seeks share the same profile while creating fresh temporal/frame ownership.
Scene no longer carries a second optional calibration alongside the capture.
The lens-only constructors remain available to reference instruments, but
live GPU attachment does not independently rebuild or choose camera geometry.

The resident solver now serves ONE X2 and X4 Air. `stitch_camera` selects
the two tested lens families without changing their metadata identity. The
ONE X2 law retains its recovered Template mounting, crop centers, alpha and
housing masks. X4 Air parents use the native model-6 Template mounting and
13-coefficient distortion from `offset_v6`. The adapter assigns native record
1 to delivered stream 0 and native record 0 to stream 1, then applies a fixed
body-to-sphere datum once in camera packing. Decoder order, IMU calibration,
view controls and downstream A/B ownership stay unchanged. A v3-only X4 stays
on ordinary projection instead of entering the resident solver.

X4 currently retains Kjerag's existing blend weights and image-circle support,
not a claim of Studio's complete fusion law. The support applies across the
whole solver belt instead of inheriting ONE X2's pole-only housing exclusion.
Checking peripheral support against the associated model-6 centers remains
follow-up coverage work; the centers differ from the v3 support circles.

Everything after that camera boundary is shared: source import, parent GPU
mapping, belt sampling, sparse PIS, temporal history, final map construction,
frame ownership and direct draw. The final luma/chroma sampler uses the actual
per-lens dimensions. Both live routes use sequential source presentation and
automatic stitching; the legacy optical-flow toggle cannot select a second
solver on an admitted capture. Camera inputs are validated before changing
presentation policy. A capture without usable orientation stays on the older
projection path rather than entering a parent mapper that requires it.

The source color matrix is container metadata, not a camera-profile constant.
Media carries it beside depth/range in `Samples`; `Reframe::with_samples`
packs its four coefficients for the shared `source_rgb` shader helper.
Ordinary and resident drawing both use that helper. The tested ONE X2 stream
declares SMPTE 170M (601); the X4 streams declare 709, matching their respective
Studio source uniforms. Untagged or unhandled matrix tags retain the ordinary
renderer's historical 709 fallback, not a claim of color-space support. This
change leaves range normalization, the resident box footprint and the inner
photometric solver's separate BGR/YCC transform alone.

The fixed solver chart, displacement gates and native parent UV convention
remain shared implementation choices, not evidence of Studio's X4 setup.
Admission is deliberately limited to type-41 and model-6 type-131 pairs;
other camera families need calibration/coverage validation, not a new PIS
implementation. In particular, arbitrary mounting residuals could move a
different camera's seam outside this chart. Existing native oracle APIs and
module names remain available for comparison.

Studio's Chromatic Calibration means photometric lens color/brightness
matching. The shared resident capture applies it at the geometrically aligned
source-sampling/fusion boundary. The shipped automatic implementation is included
in the owner-accepted installed-player result on the named footage. Exact
isolated Studio Image Fusion parity remains open in issue #185. The old
projection path's unaligned color estimator is not reused as a parity
implementation.

`field_interior` is a shared CPU diagnostic, not an estimator or playback step.
It contains the existing dark-field coherence arithmetic previously private to
the `colour` instrument. That instrument retains its legacy picture-generation
path, while Scene's opt-in GPU review supplies the exact displayed ON/neutral
pixels and their prepared Reframe directly. Both call one arithmetic core.
The review retains all admitted azimuth bins and runs the original zero and
0.5/2-code ripple controls. Insufficient coverage is reported explicitly; no
new image-quality threshold is selected. Its fixed body-chart interpretation
must be checked against the capture's alpha before treating it as off-seam
coverage. It does not qualify arbitrary two-dimensional fields by itself.
An optional diagnostic observer records the exact eligible pixel/bin membership
in that same traversal, before the per-bin population gate. Scene can retain it
as a row-major little-endian u16 raster, with 65535 marking ineligible pixels.
This is a test-only artifact, not another rendering pass or playback allocation.

The explicit saved-map diagnostic can attach an `image_fusion::RatioPair` to
its `OneXsMapFrame`. A separate shader variant samples these two 200x100 RGB
ratio maps in the original spherical chart, before the packed-UV/alpha
centering transform, and corrects each lens before blending. Missing maps use
an actual arithmetic bypass with no extra binding, not a constant-one map
whose rounding or RGB clamp could change the neutral picture. The diagnostic
recreates its private binding when correction presence changes. Ordinary
resident playback selects this variant with its own computed coefficients,
never captured coefficients.

`image_fusion::solve` is a readable Windows selected-X4 inner MGP reference,
not the production estimator. It takes already-aligned BGR8 working images;
it does not own decode, source geometry or scheduling. Its metric-reset flag
and warm-solution population are separate: an empty observation after reset
can run the first 100-step solve using retained spatial admission, without
consuming the next valid observation's direct metric seed. The separate
`image_fusion::spatial::Reference` now wraps this with the selected current-row
replication, periodic 200-to-212 extension, periodic-edge join and center crop, same-ordinal ratios,
ROI-local box filtering and original-chart remap. Its full ratio arrays start
at one and retain the previous filtered left boundary row 40. The corrected
binding/constant contract is `docs/research/studio-image-fusion-spatial.md`.
The reference and GPU distinguish measurement from equation emission. Robust
color bounds use supported rows 49/50 and columns 18..193; those bounds then
admit correction equations over **all** rows 48..51 and columns 0..211,
including the wrapped extension. NCC and sticky invalidity select quantile
observations, not a second mask on the final equations. Reusing the narrow
measurement domain for the solve omitted native constraints; the Mac read and
saved-input regression are recorded in `studio-image-fusion-temporal-602.md`.
Both the readable reference and GPU retain the separately solved six-column
periodic copies when joining prepared bytes before ratios, as the Mac selected
caller does. The GPU joins within its existing ratio dispatch, with the same
FMA and byte truncation; the current-image denominator remains unjoined.
The native-input Scene diagnostic validates saved native coefficients before
camera rebasing and changes only diagnostic draw bindings. It is not a
production fallback or accepted flicker fix.
The detached `prepare_one_xs_fusion_inputs` API composes the final packed map
through the recovered lookup into 200x4 source UVs,
then expands them endpoint-aligned and samples two 800x16 BGR bands. Each lens
is bilinearly sampled locally, not through drawing's atlas box footprint.
It retains private bindings, the exact decoded source owner and the immutable
camera profile through a consuming readback. Stepped scenes retain that profile
without enabling a live transaction. Ordered lens-local UV range comparisons
produce the four validity rows; no map sentinel or image-circle substitute is
invented. Container-driven float RGB conversion and ties-to-even bytes are
Kjerag choices, not authenticated native conversion-branch arithmetic.
`spatial::Reference::observe_bands` accumulates sticky invalidity before testing
the three 66x16 strips per lens against the last admitted means. Only an
admitted observation area-reduces the full bands into working rows 48..51 and
advances the solve/ratio history. The CPU gate retains binary64 mean arithmetic.
Source-map provenance and video scheduling remain the reference caller's responsibility.

The admitted camera profile now carries the photometric coordinate boundary
as well as geometry. Native X4 fusion ordinals are delivered streams `[1,0]`,
and its native sphere differs from Kjerag's established chart by `Ry(pi)`,
the same fixed datum used in `x4_model6_static`. Composing the native source
lookup gives `Ry(-pi/2)` against the Kjerag packed map. Ratio publication
instead preserves the consumed texture's centers: Kjerag `(r,c)` reads the
native output table at `(99-r,(99-c) mod200)`. Re-evaluating the producer's
endpoint lattice with `Ry(+pi/2)` would introduce a texel-center discrepancy.
This fixed coordinate table is prepared once, not remapped each frame.
Input bindings and final
texture bindings perform the lens exchange, without copies or additional
passes. ONE X2 keeps its original positive input/negative output lookups and
`[0,1]` lens order. The solve and temporal history remain in native fusion
order for both. `FusionInputs::new_reference()` creates a matching cold CPU
diagnostic with renderer-ordered outputs; `spatial::Reference::new()` remains
the unconverted native replay boundary. Detached sampling replaces its cache
when the admitted camera changes, and a seek preserves the camera conversion
while resetting color history. The X4 correction is included in that reviewed
installed result, not accepted as exact Studio parity; see
`studio-image-fusion-temporal-602.md`.

Existing sparse arithmetic, box
reductions and trigonometric implementations are not claimed numerically
identical to Studio. Neither reference is selected by ordinary playback.

`image_fusion::gpu::Producer` owns the analogous retained GPU state inside one
`ResidentCaptureSession`. The final-map materializer appends source sampling
and photometric dispatches before its existing asynchronous validity copy,
advancing the same inherited source lease with one final-map submission. No
ordinary source or ratio readback, host solve or new host wait is added. Only
resident status `u32::MAX` authorizes history changes; a global failure leaves
baseline, invalidity, solve and ratios untouched. Coordinate invalidity on a
globally valid but content-skipped observation remains sticky.

History follows successful source processing, not display publication. This
matches the existing computational lookahead: a future may compute before it
is due, but its immutable ratio pair remains attached to that exact map and
source. A later install refusal does not roll photometric history back. A seek
gets a new producer and causal root; renderer reattachment retains both.
Pending/ready ownership and uncertain-completion quarantine include the color
inputs and outputs. Installed draws retain their own ratio binding across newer
source computation and draw retirement. Diagnostic installed-map readback can
explicitly include the ratio pair; ordinary redraws only sample it.

The GPU content gate deliberately uses integer sum delta `>3168`, so it does
not reproduce the CPU reference's binary64 threshold-rounding edge (9 to 3177).
CG and box reductions use GPU float arithmetic. These disclosed implementation
choices require real-output and playback-capacity qualification, not further
optimization reverse engineering.

The selected photometric implementation writes final ratio values into immutable
RGBA32F textures in the existing remap pass. Hardware bilinear interpolation is
selected only when optional full-f32 filtering is enabled on the device;
otherwise the consumer uses explicit texture loads and wrap/clamp interpolation.
Saved-map CPU replay uses the same format and feature policy. Both the
half-storage and due-source-priority trials were removed after they failed to
improve the combined source-cadence and changing-view performance result.
No half-precision storage or arithmetic remains. Full-f32 hardware interpolation
rounding is separately qualified; neither that arithmetic check nor whole-player
visual acceptance establishes exact Studio parity.

The full-f32 hardware consumer's adversarial fractional-UV test uses a
separate half-of-one-8-bit-code bound; the explicit consumer retains `2e-6`.
On the test Radeon, their measured maxima are `0.00054196` and `1.79e-7`,
respectively. The hardware maximum is at the synthetic map's discontinuous
periodic join. Actual Scene comparisons retain their one-code-per-channel
bound; none of these gates establishes whole-video or owner acceptance.
The selected simplification publishes only those textures,
removing two redundant 320,000-byte output buffers and their stores/bindings.
The color producer needs eleven storage buffers instead of thirteen; the
complete stitch path's fifteen-buffer requirement is unchanged. Explicit
diagnostics copy the exact texture texels into temporary padded staging and
strip row padding. No live staging, readback or wait is added. All 51 color,
three consumer and two actual-Scene checks pass. Across the two saved sequences,
all 31 X4 and 61 X2 frames' pixels, packed maps, alpha and ratios remain
byte-identical to the preceding implementation. At that implementation
checkpoint, capacity qualification showed no material gain: the next native run
reached
29.175 source fps and 239.20 changing commits/s on X4, while X2 reaches
29.950/261.12. It removes duplicated publication, not the measured cadence
defect. The owner's packaged review build was unchanged at that checkpoint;
the later installed-player acceptance and current capacity limits are recorded
at the top of this page and in ROADMAP.
A bounded indexed-mesh trial preserved every triangle and all 460 saved frame
artifacts, but did not produce a useful X4 gain: 29.475 source fps, 238.95 changing
commits/s, commit p99/max 8.79/23.00 ms and still-growing lateness. X2 reached
29.975/262.82, with p99/max 8.47/19.17 ms. Both strict pointer cohorts fail one
source-less commit. The additional index-buffer code is rejected; the existing
triangle draw is retained. Trial source and evidence remain in scratch.
At that implementation checkpoint, the retained texture-only implementation
passed full gates (1,238 workspace tests, zero failures, 30 ignored) and
50 X4/54 ONE X2 native UI checks. The same
backward-seek and real mouse-scrubber checks now run on both named fixtures;
earlier harness results covered those two interactions only on X2. These are
correctness and interaction gates, not proof of uniform presentation timing.

## Failures the pilot is told about (issue #124)

There is one way a failure reaches the pilot and it is the alert, and that is
a property of the types rather than a rule anybody has to remember, which is
what the owner asked for: "why is this not consistent by code design?"

Three pieces hold it up.

- **The line is private.** `crates/app/src/fail.rs` owns the alert's title and
  body in a struct nothing outside that module can build, and the only way in
  is `Alert::raise(Failure)`. A new failure site cannot write its own sentence
  into the window; it adds a `Failure` variant, and the compiler then makes it
  give that variant a title, a line for the pilot and a line for the terminal.
- **The terminal echo comes with it.** `raise` prints as well as shows, so an
  `eprintln!` at a failure site is not a shortcut past the funnel, it is
  strictly less than calling it.
- **The engine cannot report at all.** `render` has no way to say anything to
  a person. A pass that has given up leaves a `Stall` in the open capture's
  own slot, `Scene::pump` turns it into a `Next::Stopped` arm that every
  caller has to match, and the shader widget will only hand that arm to a
  message type implementing `From<Stall>`. The shell cannot compile the video
  widget without a way to receive one.

**What the alert says is not the funnel's to write** (owner ruling,
2026-08-01, AGENTS.md "Errors are the error"). The funnel decides the surface
and the title; the body is the failure's own message, shown word for word. A
sentence written in the shell can only ever say less than the one written
where the failure happened, and the shape that proved it was live: the pilot
read "That file could not be opened." while the terminal read "trailer says
lens frames are 2880x2880 but the stream decodes 736x368".

Three lines of the app's own survive over an error, and they are the whole
list (`fail::refusal`): another camera's format (issue #107), a codec whose
decoder is one install away (issue #69), and a path the sandbox was never
shown (issue #118). Each is allowed because it knows something the error does
not and turns it into something the pilot can do. There is no line under them
for a failure nobody anticipated, which is the part that makes this hold:
there is nothing to fall back to, so nothing can be masked by falling back.

A fourth line of ours is added rather than substituted, and the difference is
the whole test: a stopped video reads the stall's own line and then "Open the
file again.", because that open being over is a fact about the shell that the
stall has no way to know. An addition hides nothing. The sentence it replaced
("The picture could not be drawn, so playback stopped.") was a substitution,
and it knew less than the stall underneath it.

That makes the error messages in `meta`, `media` and `render` pilot-facing
copy. They are still the layer's own to write, and they are still written at
the failure site; what changed is that they are read by a person, so plain
words and no em dashes bind them the way they bind `strings.rs`.

A stop is final for the capture it happened to, which is the owner's second
ruling on the issue (2026-08-01). The `Stalled` that gave up says so for the
rest of that capture's life: the pass stops importing into it, and `Scene`
stops handing out its player, so nothing the pilot presses can run a clock over
a picture that is not coming back. Opening the file builds another `Scene` with
another `Stalled`, which is the whole of the recovery and the only thing the
alert asks for. The shape before it re-armed on the next redraw and raised the
same alert every two seconds.

Deliberately not in the funnel: a capture that could not be written says so in
a toast, because the picture is still there and the pilot is still watching it
(docs/UI.md). The funnel is for the failures that leave him with no video.

## The frame path

```
VA-API decode (two 3840x3840 HEVC streams, one demuxer)
  -> av_hwframe_map(dst = AV_PIX_FMT_DRM_PRIME, MAP_READ | MAP_DIRECT)
  -> AVDRMFrameDescriptor { fd, offset, pitch, format_modifier } per layer
  -> two single-plane wgpu textures per frame:
       R8Unorm  from layer 0 (luma,   DRM_FORMAT_R8)
       Rg8Unorm from layer 1 (chroma, DRM_FORMAT_GR88 - note GR, not RG)
  -> imported textures remain the render sources
  -> one fragment pass to the swapchain at display resolution
```

On the generic projection path the shader consumes both lenses (issue #27),
so a view anywhere on the sphere has a picture in it, and it mixes them across
a crossover on the seam that is 8 degrees wide on an X4-class file and the
camera's own width on any other (issues #7 and #48, and 2026-08-05 for the
width). Outside that crossover one lens weighs exactly 1 and the other exactly
0 and only the first is fetched: a pixel away from the seam costs what it cost
before the blend, down to the bits it writes.

Since issue #10 the second lens is not projected there either. Each lens's
picture is one cap around its own axis, and how wide that cap is comes out of
the calibration by solving the model's own coverage boundary
(`coverage_floor`), so a ray further off the axis than that weighs exactly
zero and one dot product says so before the model runs. The test is
deliberately one-sided: a lens kept and weighed zero is written and
multiplied by nothing, which is what it was before, and only a lens wrongly
dropped would be a hole. Worth 0.20 ms of a 1.74 ms pass at a view inside one
hemisphere and 0.15 of 1.81 across the seam, and every picture it writes is
byte for byte the one the pass wrote before it.

No queue-family EXTERNAL acquire step is needed, on either wgpu version:
`create_texture_from_hal` offers no hook for it, `TextureUses::
UNINITIALIZED` works, and the spike's output is byte-identical to the
copy path's. wgpu 28's `create_texture_from_hal` has no `initial_state`
argument at all (that is wgpu#9496, new in 30), so the layout-discard
hazard is unavoidable there; it stays benign for the same two RADV
reasons, and the byte-identical PNGs are the check.

There is no viable fallback. The copy path measured 45.3 ms/frame of
delivery (18.4 fps) in the M0 spike: it cannot sustain realtime for even
one lens. Zero-copy import is a requirement, not an optimization. (An
earlier research note put `vaDeriveImage` at 0.53 ms/frame; that was the
map call alone, with nothing reading the pixels through it.)

The selected ONE X2 path samples those imported R8 textures directly and keeps
the ordinary stitch transaction resident through retained-map sampling,
reduction, blur, paired sparse PIS, temporal continuation, dense native-map
materialization and direct draw. Normal post-qualification frame preparation
maps only the four-byte validity word. It performs no solver-belt or sparse
terminal readback, CPU PIS, CPU map materialization, frame-sized transfer or
native-map upload. The decoded picture and every installed map stay in the
capture-owned resident session until bounded render-pass retirement proves
their last draw complete.

Every selected ONE X2 GPU stage is rooted in one render-private
`OneXsGpuContext`, constructed from the exact device and queue iced gives the
`ScenePipeline`. The context compares those wgpu handles structurally, so a
renderer-pipeline recreation on clones of the same pair remains compatible;
a replacement device or queue within that wgpu Instance is a different context.
The resident producer and its `SubmissionLease` retain that context. The lease submits
later resident consumers on its own queue and replaces its own completion
index; no consumer may hand it a detached `SubmissionIndex`.

wgpu supplies one queue together with each requested device and has no public
constructor for an independent second queue on that same device. Tests
therefore prove cloned-pair acceptance and independently requested-pair
refusal. The context nevertheless compares both structural handles because
the device-and-queue pair, not either handle alone, is the ownership boundary.
Selected Scene preparation authenticates that pair before terminal-display or
in-flight recovery can bind, write or encode anything. After authentication,
all selected work uses the retained context handles. A mismatch selects no
draw, preserves the last complete display untouched and surfaces the raw
identity error. Diagnostic picture preparation and full-luma readback are
context-owned too; their per-frame APIs accept no replacement device or queue.

The pinned wgpu's structural device/queue equality compares per-Instance IDs,
not Instance identity. First allocations in two separate Instances can compare
equal, so foreign-pair tests request two devices from the same Instance. This
is a boundary limitation, not proof that separate Instances share resources.
Normal iced renderer and surface recreation clone the existing compositor's
Engine/device/queue. Constructing a new compositor creates fresh primitive
storage, so current playback does not carry a resident attachment across that
boundary. Any future cross-compositor attachment reuse must add Instance
identity rather than rely on these structural comparisons.

The shared PIS front end caches its rolling patch sums instead of replaying
each row/column prefix for every patch. One horizontal recurrence per source
row feeds one vertical recurrence per patch column. Both retain every
intermediate rounded update, including positions between the stride-three
outputs. The 3,376-word public patch-sum prefix and downstream bindings stay
unchanged; a 10,260-word private scratch tail uses the existing allocation.
An additional dispatch establishes the cache dependency. The CPU oracle and
GPU mutation checks cover the complete public result. This removes repeated
work without changing the solver's input arithmetic, admission or temporal law.

The geometry mask is bilateral. A horizontal nine-column validity scan combines
both source lenses and packs four boolean results per word. A second pass ANDs
the nine neighboring row words, reproducing the original clipped 9-by-9
conjunction before applying the unchanged camera support. It writes the final
word to both lens sections; a disjoint invocation writes the validity sentinel.
The mask allocation has a private 64,800-byte scratch tail, without an added
buffer or binding. Its public two-lens prefix and sentinel offsets are unchanged;
the prepared-source front end checks the actual enlarged allocation size and
indexes only that prefix. Retained maps are never overwritten for scratch.
Full public-mask and retained-map qualification still uses the direct CPU oracle.

## Playback (issue #4)

One demuxer per file feeds every decoder and hands out `Frames`: every lens
at the same instant, mapped and ready to import. A lens is never delivered
without its partner, so the two cannot drift apart; if a head ever lacked a
partner the reader drops it rather than pairing two instants.

Two demuxers is the ONE X2 case (issue #79), where a capture is two files of
one lens each. They share a frame grid exactly, so the lanes are matched on
frame **index** - the one number that means the same thing in two timelines
with their own `start_time` - and the reader pumps whichever file is behind
so neither runs away with the memory. The lens 0 file is a frame longer than
its partner in every pair measured, and that frame is dropped: it has no
partner, and half a sphere is not a picture.

**The sound has a demuxer of its own** (issue #97): the same file, opened
again with the pictures discarded. The reason is measured, not stylistic.
Two of the four large captures on this box have one place where the camera
left tens of megabytes of picture between two audio samples, both of them
about four and a half seconds in, and libavformat reads a file interleaved
like that by letting one stream fall up to a second behind
(`mov_find_next_sample` keeps to file order until the timestamps differ by
more than `AV_TIME_BASE`). Sound a second late is sound the splice drops, so
the owner heard three seconds of silence. Nothing on the picture side can
fix it: no ring can hold sound that has not been read, and the pictures
cannot be read a second ahead of themselves when a decoder's surface pool is
20 frames deep. On its own demuxer the sound reads 190 kbps of a 180 Mbps
file, seeking straight to each audio chunk (40x realtime over the whole 30
minute capture, measured), and the pictures no longer cross the file for
packets nobody takes from them. It costs one more file handle and one more
open of the container, 0.2 s on the 36 GB capture.

`Player` runs that reader on its own thread behind a two-deep channel and
answers one question per redraw: which frame belongs on screen at this
`Instant`. Nothing counts ticks. 29.97 fps divides evenly into no refresh
rate anyone ships, so a frame has a due time and the shell sleeps until it
(`iced`'s `RedrawRequest::At`, requested by the shader widget in
`kjerag_render::widget`). Pumping the clock from a shell-side
`window::frames()` subscription instead was written and measured first: 33
to 46 redraws a second against a 60 Hz display, and 1 to 18 dropped frames
every 5 s, because the redraw event has to leave iced and come back before
the next redraw can be asked for. The clock must be pumped inside the
redraw pass, which is why `Scene::pump` takes `&self` and holds the player
in a `RefCell`.

Playback still paces on container PTS, which is what a presentation clock
wants: a monotonic grid to sleep against. **The gyro does not.** `pts_type =
2` turned out to mean what its name says, so a frame's orientation is looked
up at the camera's own timestamp for that frame
(`ExposureTrack::frame_time_us`), which drifts from the container's nominal
grid at 6.4 ppm and is 11.5 ms away from it by the end of a 30-minute file
(issue #8, docs/research/insv-format.md 8.6).

### ONE X2 Studio-derived stitch path

An ordinary open of a supported ONE X2 selects this route automatically.
There is no calibration step, setup ritual or quality toggle. The Optical
Flow setting controls only the legacy solver and does not select or modify
this route.

The player changes to `PresentationPolicy::SequentialRealtime` and one
capture-owned resident facade starts at frame zero. Selected startup uses the
same exact-landing hold as seeking: Scene retains autoplay intent, but the
source/audio clock stays paused until frame zero's resident map is installed
and acknowledged. A pause during startup cancels that autoplay intent. Generic
projection opens still start immediately. This prevents cold GPU setup from
charging time against a picture that has not been prepared yet; it does not
solve sustained processing slower than the source cadence.
Scene branches to this route before legacy prepare, import or draw. Admission
and publication stay on the UI thread. The capture-shared worker now owns the
whole computational transaction: the exact imported pair, existing GPU chain,
final validity acknowledgement and unpublished temporal commit. This autonomous
worker is the selected production path. Current source-cadence and overall
release qualification remain open in issues #186 and #187. Its bounded channel
permits one active capture-service job and one queued job across capture
restarts. A full channel
does not authorize a source admission that has no worker to service it.
UI preparation never waits for the worker or consumes an unfinished map.

The source/audio clock does not reanchor on each source frame. The original
slow-clock `EveryFrame` policy remains available to diagnostics. During ordinary
play, `Player` keeps the current due frame and exposes at most two already-decoded
successors without presenting either or moving the clock. The capture root
holds one completed unpublished source/map pair separately from its displayed
pair. One renderer preparation can admit both decoded successors. The worker
advances the computational temporal prior and starts the next admitted source
without another renderer visit. The total accepted but unpublished population
is bounded at two, including queued input, active computation, the committed
future and any parked completed result. If the second result finishes while
the future slot is occupied, the worker parks it and ends that service job.
Publication frees the slot and schedules service again; there is no waiting
thread or polling loop for a full future slot. This reserve can absorb uneven
per-source work but cannot solve a sustained throughput deficit. The larger
three-result FIFO/pre-roll trial remains archived in `8c6921cf`, not selected:
its additional memory/startup cost had no qualified overall smoothness benefit.
Restoring this installed-build scheduling boundary preserves source order,
seam arithmetic and refresh cadence while isolating the controls-tree correction.

Completion cannot publish a picture. Preparation reserves an ordinary draw
permit and binds the exact future before moving it to the displayed slot,
only when Player's current delivery supplies the same opaque `FrameStamp`.
Screenshots and redraws continue sampling the displayed pair, never the newer
computational prior. Player does not promote another frame until its current
one is acknowledged. Two decoded successors, one unpublished completed result,
one additional active or parked transaction and two render-retirement slots are bounded;
none treats computation as display or permits an unbounded queue.

Idle playback schedules its next redraw at Player's exact media deadline,
not on every display refresh to poll speculative successors. An old-picture
presentation just before that deadline arms winit's Wayland frame callback and
can delay the due picture until a later refresh even when its map is ready.
Input and UI redraws retain their independent scheduling; this is not a cap on
changing-view rendering capacity. Preparation admits bounded decoded lookahead
whenever a redraw occurs, but completion and successor execution no longer
require redraws. If the exact currently offered, unacknowledged source
is not installed after preparation, Scene requests a follow-up through the
renderer while keeping the old picture presentable. This flag is recomputed on
every prepare and cleared on errors; it does not bypass compositor callbacks,
publish early, or reject a drawable old picture. Draw-retirement refusal keeps
its separate existing retry policy. Worker autonomy is intended to preserve
preparation headroom while an idle window sleeps until its next deadline.
Source readiness and ordinary playback still require qualification alongside
active-view capacity; the architectural split alone is not proof of smoothness.

The capture-owned session and its exact source owners survive a
renderer-pipeline recreation on the same device and queue. Pausing hides decoded
lookahead from preparation but does not discard a job or completed future result;
resuming returns publication authority to Player's due delivery. Seeking and
stepping still require installation, create a new causal root where required and
drain the replaced facade without reusing uncertain source surfaces. At EOF the
clock stops, but redraws continue until the last offered transaction installs.
An adjacent forward replay re-anchors its landing without discarding the remaining
already-decoded successors; a real seek clears all decoded slots. Pending work
and full render-retirement admission always leave the prior exact
shown result drawable; there is no legacy recovery route.

Selected PIS uses independent 16-patch-row stripes. Vertical candidates do not
cross stripe boundaries; this is an explicit propagation approximation, not
Studio's global schedule. The global scalar/GPU reference remains available,
and selected GPU construction qualifies exactly against its striped CPU twin.
Parent mapping and PIS compile one shared `one_xs/f32_div.wgsl` helper.
Binary32 division estimates a normalized quotient on hardware and corrects
the exact integer remainder before the unchanged RN-even/exponent handling.
The bounded estimate follows [WGSL's division accuracy contract](https://www.w3.org/TR/WGSL/#accuracy-of-concrete-expressions);
integer residual correction replaces the ordinary 24-step restoring loop.
Edge/random and forced-fallback tests compare complete output bits to CPU.
Sharing this helper removes the parent's separate divider implementation;
alternating whole-Scene measurements did not establish a capacity improvement.

Flat perspective views rasterize the native 100-by-50 sphere triangles directly,
sampling packed maps at vertices and alpha after perspective interpolation.
Curved/ball projections retain the ray-based shader; no intermediate panorama
is introduced. Native fixed-function interpolation is not bit-identical to
ray intersections. Regression requires exact coverage and bounds each mapped
channel by CPU samples within 1/64 output pixel, plus the original arithmetic
tolerance. Actual-footage review remains a separate owner gate.

At most one nonblocking renderer-side device poll drives draw retirement for
the active attachment and every normally draining attachment replaced by seek
or reopen. The worker independently drives its exact final validity callback
with nonblocking polls and a 100-microsecond sleep fallback. It never uses a
blocking GPU fence wait. Renderer retirement never waits. Completion-proven
owners release normally; uncertain owners remain fail-closed without blocking or
repeatedly scheduling the new lineage. A discontinuous user seek creates a new
temporal root on the decoder's landing frame, sharing immutable GPU kernels but
no old history. Drag updates request keyframes; release requests the exact
destination. Sequential consumers reject superseded decoder epochs before they
can initialize that new root.
This intentionally differs from uninterrupted frame-zero history, following the
owner's 2026-09-05 priority of performant Studio-like stitching over perfect
reproduction. A single forward step keeps the adjacent warm state. The old
display remains visible until the destination's source-specific map
is acknowledged. The direct type-2 draw consumes the native map and alpha
without routing through the legacy seam-band displacement.

Screen and screenshot reserve separate immutable resident draw permits. A
source import owns only the exact imported planes, decoder frames, context,
and capture identity. It does not allocate a picture binding or upload view
uniforms; those are created only for an actual immutable draw permit. A
screenshot prepares the exact shown capture and stamp, then draws its permit in
an independent offscreen pass; it never calls the window draw. Explicit seam
diagnostics may authenticate that same installed identity and read back packed
map and alpha bytes, but that bulk transfer and wait are instrument-only.

The type-2 picture consumer uses the GPU's linear sampler within each imported
lens. A filter footprint crossing the two-texture atlas join still uses four
explicit loads, since clamping either separate texture would change that join.
The recovered box filter and native map remain unchanged. Pixels whose alpha
is exactly zero or one read only the contributing lens. Hardware subpixel
filtering can differ from software interpolation by small rounding amounts;
bit identity of rendered RGB is no longer required by the owner's current
performance goal. Real-sequence review remains required.

The owner's 240 fps target is interactive view rendering during playback, not
240 new source pictures from a 29.970 fps file. Completed source/map pairs are
already reusable for view redraws, but processing shares the render queue and
can still delay them. The offscreen `view-rate` diagnostic reports paused
redraw capacity separately from changing-view playback and its tail latencies.
It waits for each GPU draw and has no compositor; its results do not prove
native-window presentation at 240 Hz.

That diagnostic now observes a queue-completion callback using nonblocking
polls and 100-microsecond receive timeouts, with one redraw in flight. Polling
and wakeup costs remain in the measurement. Concurrent stitch work may join
the callback's queue prefix and delay observation; it can never make the draw
appear complete early. The previous offscreen blocking poll held wgpu's fence
read lock throughout GPU execution, preventing the stitch worker from acquiring
the write lock needed for submission. It therefore disproportionately delayed
chunked scheduling. Comparisons of worker schedules must rebuild both arms
with the same nonblocking measurement; old blocking-poll figures remain raw
instrument observations, not causal evidence of native scheduling behavior.

The first lazy resident-session construction runs the existing target-device
arithmetic qualifications synchronously. Those constructor-only probes perform
bulk readbacks and waits. Once the session exists, the UI-side submit, redraw,
draw and retirement paths perform no bulk readback or wait; only the four-byte
validity callback crosses to CPU. The worker submits each GPU stage normally,
except resident L1, whose unchanged wavefront schedule is encoded into chunks
of at most eight dispatches. Its six submissions have five callback-completion
waits on the registered worker only. These waits hold no wgpu fence lock; a
100-microsecond receive timeout drives nonblocking polling if no UI is active.
The earlier one-millisecond fallback delayed completion observation between
chunks in normal compositor-paced playback; the uncapped diagnostic's frequent
polling had hidden this cost. The timeout is a host scheduling interval, not
stitch arithmetic or a source-release proof.
The callback is only a scheduling signal, not a source-release proof. The
exact lease stays armed through every chunk and final validity acknowledgement.
UI draws may interleave, but are not guaranteed a submission between chunks.
Earlier blocking-wait pacing and diagnostic-only pacing were removed. Selected
L1 runs through `GpuL2BridgeOutput::submit_l1_pis`, not the CPU-grid diagnostic
`submit_pis_stage`. A test-only counter at the successful-submission site lets
real Scene cold, warm and cached-redraw tests prove the selected worker route.
The bulk legacy CPU stitch implementation is
retained solely as the frozen oracle and explicit diagnostic surface; small
control, pose, identity and lifecycle state remains on CPU.

The selected ONE X2 basis, calibration packing, 51-pose schedule and Metal
parent-map law are READ from Studio. Kjerag uses its existing orientation
track in place of Studio's unrecovered `PrecomputeStabilization` pose-cache
producer, an owner-approved implementation substitution recorded in
`docs/research/studio-seam-re.md`. The masks, cold and warm estimator, map
materialization, alpha and type-2 consumer implement the recovered semantics
around that boundary. This is a disclosed implementation difference, not a
claim that Kjerag reproduces Studio's internal provider.

The bounded visual gate covers the owner's reported riser-continuity defect
at the reported view over frames 6339 through 6399. The owner reported
"Looks good" on that sequence and again after the first internal concurrency
change. Later archived candidates were measured byte-identical over that
61-frame production, map, alpha and computed-trace boundary, but the latest
exact build still needs its own owner verdict. This does not establish
whole-video, Studio-internal, seek/reset, real-time or continuous-sound
parity.

## Trap list (each verified in the 2026-07 study)

- A passing solver primitive test does not prove playback uses that entry.
  The 2026-09-06 L1 pacing change was initially attached to a CPU-grid
  diagnostic while resident playback used the GPU L2-to-L1 bridge. Its
  apparent paced/unpaced and batch-size timing differences were uncontrolled
  variation, not effects of those edits. Verify the actual Scene call path.
- A blocking GPU wait on another thread can block drawing: pinned wgpu holds
  a fence read lock across that wait and submission needs its write lock.
  This applies in both directions, including a benchmark's draw-completion
  wait preventing worker submission. Nonblocking callback polling avoids
  holding the lock throughout GPU execution; it still has polling overhead.
- Use descriptor `pitch[]`/`offset[]` verbatim. Chroma pitch is
  `align(width, 512)`: at 3840-wide that is 4096 != 3840, and computed
  strides shear chroma on real footage while passing on 1920/2560 tests.
  The M0 spike saw exactly this on X4 Air footage, plus the other half of
  the trap: luma pitch is 3840, NOT padded. Padding is per-plane, so no
  single computed rule is right for both. (Chroma offset 14745600,
  modifier `0x200000010401b04`.)
- radeonsi exports ONE fd; later planes reference object 0. `dup()` per
  plane use; caller closes every fd. Spike saw `nb_objects` 1 with both
  layers on `object_index` 0.
- Pre-flight the format modifier via
  `vkGetPhysicalDeviceImageFormatProperties2` before image creation;
  an unsupported modifier is UB, not a clean error.
- Do NOT use ffmpeg's `hwcontext_vulkan` (AVVkFrame) route: handing an
  imported frame to a second Vulkan consumer produced
  `VK_ERROR_DEVICE_LOST` on the target box, twice. DRM_PRIME only.
- `av_hwframe_map` with `MAP_READ` calls `vaSyncSurface` first (ffmpeg
  6.1 `hwcontext_vaapi.c:1337`): the map waits for the decode to finish.
  The spike's 7.64 ms "deliver" is mostly this wait with one frame in
  flight; keep 2-3 frames in flight to hide it. Measured on the player
  (`kjerag-spike --bin playback`): mapping the oldest queued frame rather
  than the newest takes dual-stream decode from 2.19x realtime at depth 0
  to 2.46x at depth 2, and 2.47x at depth 4. `Reader::lookahead` is that
  depth and the engine sets it to 2.
- Do not interpret `Reader::pool_size() == Some(0)` as exhaustion or a
  fixed zero-surface pool. Current FFmpeg7.1 VA-API uses dynamic allocation;
  both lanes on both owner cameras report0 after decoding (2026-09-11).
  The earlier20-surface reading is historical, not a current ceiling.
  `extra_hw_frames` adds capacity only when FFmpeg selects a positive fixed
  pool; modern VA-API grows on demand. The context is created at hardware
  format negotiation, not necessarily at `avcodec_open2`. Every held frame,
  mapped or not, still prevents reuse of its surface. The engine holds at
  most9 per stream (2 lookahead, 2
  queued pairs, the one on screen, the one peeked, and 3 retained on the GPU)
  on the generic route. Nothing checks that generic-route count at runtime;
  selected resident playback instead owns pending, installed and bounded
  retired sources until their callbacks prove completion.
- On the generic and legacy diagnostic routes, an imported texture aliases the decoder's surface, so dropping the
  `Frames` while the GPU is still reading hands live memory back to the
  decoder. `ScenePipeline` keeps the last 3 pairs alive behind the one it
  binds; iced submits after `prepare` returns and presents later still, so
  "the draw call was recorded" is not "the GPU is done".
- The selected resident ONE X2 path enforces the same rule with a sealed
  transition rather than a retention convention. `ImportedOneXsPicture` owns
  the exact two imported plane pairs, GPU context and
  decoder `Frames`. Its consuming resident-front operation derives the exact
  frame identity and luma textures internally, refuses a foreign context
  before reservation or encoding, appends parent, geometry and belt work to
  one command stream, and moves that same aggregate into the submission
  lease. The crate-visible admission accepts the concrete aggregate rather
  than a `SourceTextures`/owner pair. Its opaque result retains the only
  module-private consuming motion continuation. Scene reaches it only through
  the capture facade and renderer attachment. No raw wgpu or dmabuf resource
  crosses that boundary. Bounded callback retirement, rather than the generic
  three-pair convention, proves when each selected source may be released.
- Reference import code: `ez-ffmpeg` 0.17 `wgpu_filter/hw_interop.rs`,
  `iroh-live` `rusty-codecs/src/render/dmabuf_import.rs`, `bevy-dmabuf`.
- GStreamer was evaluated and rejected: no wgpu or dmabuf-to-Vulkan sink.
- The base Pop!_OS runtime was FFmpeg6.1; this project now builds against
  FFmpeg7.1, with libavcodec.so.61 and libavutil.so.59 verified on the host.
  Bindings must match the linked runtime. Do not assume8.x APIs from research
  notes are available; AGENTS.md records the development-package setup.
- wgpu-hal 28 enables `VK_KHR_external_memory_fd` and
  `VK_EXT_external_memory_dma_buf` whenever the adapter has them, but never
  `VK_EXT_image_drm_format_modifier`, and `iced_wgpu` builds its device from
  a fixed `DeviceDescriptor` with no hook. The `[patch.crates-io]` entry in
  the workspace root manifest is what turns the third one on, for iced's
  device and ours alike; the spike additionally forces it through wgpu-hal's
  `open_with_callback` because it builds its own device. `dmabuf::import`
  still checks `enabled_device_extensions()` and refuses: creating an image
  with a disabled extension's structures is UB, not an error, so the day
  someone drops the patch entry the failure must be loud.
- libcosmic's content container insets the app's view by `border_padding`
  on the right and, because `nav_bar.active` defaults to true even with no
  nav model, by nothing on the left (`app/mod.rs`, `main_content_padding`).
  Measured at scale 1.25: 1 physical px of border left, 10 right. The app
  sets `core.window.border_padding = Some(0)` and keeps the container
  (issue #93): the same `[0, 0, 0, 0]` around the video, and the window
  background comes with it. That background is painted only on the
  container branch (`app/mod.rs:856-874`,
  `background(theme.transparent).base`), so an app that turns the container
  off to reach the window edges is transparent behind its own content, and
  under the desktop's blur that reads as blur with no window over it.
- `iced_renderer` silently drops shader primitives when the tiny-skia
  fallback is chosen (`fallback.rs`: a `log::warn!` and nothing drawn), so
  a blank widget can mean "wrong renderer", not "wrong shader". libcosmic's
  `wgpu` feature is not on by default.
- iced's surface is sRGB when it gamma-corrects, while the spike's offscreen
  target is `Rgba8Unorm`. The same WGSL writes different numbers to the two:
  gamma-encoded video has to be linearised before an sRGB target re-encodes
  it. `TextureFormat::is_srgb()` decides at runtime. A screenshot therefore
  renders into a texture of the *surface's* format rather than a format of
  its own, which is what makes the bytes it reads back the bytes the
  compositor was handed. Measured both ways on real footage: the two agree
  to within one code on 5% of channels and exactly on the rest, which is the
  8-bit rounding of the round trip and nothing else.

## Projection

The generic camera geometry below is shared, but its crossover, adaptive band
and research calibration knobs are not the selected ONE X2 handover. A
supported ONE X2 instead consumes the recovered native type-2 map and alpha
through the direct route described under Playback. That map owns its handover;
the legacy band cannot modify it.

Insta360 stores a full Mei/UCM camera model per lens in the trailer
(`offset_v3`): xi, fx/fy, cx/cy, k1-k3, p1/p2, per-lens extrinsics
(~33 mm baseline). The X4 Air fixture: xi = 2.31494. The forward map is
~20 lines of WGSL, written from the Mei/OpenCV-omnidir description in
docs/research/insv-format.md 5.1; Gyroflow's
`distortion_models/insta360.wgsl` (GPL-3.0, AGPL-compatible) remains an
available reference for anything the description does not cover, and a file
that takes it carries an SPDX header. Nothing does today. Static calibrated
warp with a smooth blend; no optical flow (measured to not help). A reframed
view centered near a lens axis contains no seam at all.

**There are two camera models, chosen per lens.** A DJI Osmo 360 `.OSV`
keeps its calibration in the file's own telemetry track rather than a
trailer, and its model is the Kannala-Brandt fisheye,
`r = fx * theta * (1 + k1 t^2 + ... + k5 t^10)`, with all five of the
coefficients the file carries. Both families are one function of a unit ray
in the lens's own frame and nothing else, so the model is a branch inside
`lens_pixel` and the rest of the pass - the caps, the crossover, the
handover, the readout - does not know which one it is running. Both take
five coefficients, which is why `LensBlock` carries five slots and not ten.
The GPU twin guard runs its whole comparison once per model, because a
branch is only guarded at a fixture that takes it. docs/research/osv-format.md
is that format's reference chapter, including what is still unknown about it.

`kjerag-meta` turns that string into a `CalibrationSet` whose pixel numbers
are already in delivered-frame coordinates (3840x3840 per lens), not the
15360x7680 side-by-side calibration canvas the file writes them on. The
shader consumes them as they come; nothing downstream rescales.

The rotation is `Rz(roll - 90 deg) * Ry(yaw) * Rx(pitch)`, in a frame whose
axes are the delivered frame's own (x right, y down, z along the optical
axis), times a half turn about the body's vertical for lens 1. The IMU is
bolted to the sensor rather than to the picture and wants the same three
angles **without** the quarter-turn datum (`Pose::sensor_from_body`), which
is how issue #8 settled where the datum comes from. That
quarter-turn datum was measured against rendered frames from two cameras,
not assumed: applying roll as the file writes it puts the world on its
side. So was the half turn, which the file does not contain at all (lens
1's recorded yaw is 0.039 degrees, not 180) and which the pictures of the
two lenses have to agree across the seam to settle.
docs/research/insv-format.md 4.8 and 4.9 have the frames, the method and
the tables.

Every ray is weighed against both lenses (issue #7), and the weights sum
to 1 wherever anything has it. Each lens's claim is its **share of the
crossover** times its **coverage depth**, zero where the ray is not in that
lens's picture at all. The crossover is **8 degrees wide on an X4-class file**,
and centred where the two lenses are equally far off their own axes (issue
#48), which is the seam wherever the calibration puts it; the coverage depth is
a distance transform from the lens's own validity boundary, so a lens fades out
as it runs out of picture and the rim of the image circle, where vignetting
lands and where the distortion polynomial is least trustworthy, is
down-weighted for nothing.

**The width is the camera's, not the build's** (2026-08-05). `CROSSOVER_DEG` is
what the picture asks for; each file's own two lenses clamp it, because the
handover reaches half its width off the seam plus the whole bend it carries and
that has to stay inside the ring both lenses have a picture of
(`band::affordable`). Six X4 Air captures afford 9.36 to 9.82 degrees and take
the 8; a ONE X2 overlaps by 9.19 and hands over across **4.18**. That clamp is
a **floor** and not a ceiling: since the blend curve landed on 2026-08-08 the
adaptive term may open to 4.33 (`band::WIDEST_DEG`), so a camera affording less
than that opens past its own overlap at the near field, and exactly one in the
corpus does (docs/research/seam-temporal.md 9.6). The width
travels in the uniform block rather than being written into the shader source,
because the shader is compiled once before any file is open, and both halves of
the map read it from there. The app says which width a file drew, on the
`blend:` line at open.

The width is the one number here a person chose, and it is a trade with
measurements on both sides (docs/research/insv-format.md 6.8): a wider band
draws whatever the two lenses disagree about twice, and a narrower one folds
the picture where they disagree at all. Narrowing it could only happen after
the calibration was corrected. Widening it back out was a percept and not a
measurement: every instrument with an opinion is monotone in the width, and the
owner chose 8 over 2 label-blind (docs/ROADMAP.md, 2026-08-05).

SUPERSEDED, kept for the shape of the trade rather than for its numbers: at the
2 degrees issue #48 shipped, this paragraph read that the crossover takes the
doubled band on real footage from 10.6 degrees to 1.5 while the band's own
sharpness goes from 0.723 of one lens's to 1.074. Through the shipped path at 8
the doubled band is about 4.8 degrees and the sharpness about 12 percent under
what 2 kept.

A file with **one** lens stream takes no crossover at all: it has no seam to
hand over at, and its picture runs to the edge of its own coverage, 7 degrees
past where a seam would have been.

### Factory seam and explicit research correction

The production app fits and stores no seam calibration. It uses the factory
calibration as the parity base, and supported ONE X2 playback replaces the
generic handover with its recovered type-2 map. The former per-file fallback,
saved per-camera pool and calibration action are removed.

The renderer retains `Scene::use_seam` for explicit research correction only.
Headless instruments may name all five knobs, but no omitted argument can
select a content-fitted pose. The historical fit method and its measured
transfer results remain in docs/research/insv-format.md 6.8; they are evidence
about the legacy generic route, not selected product behavior.

Nothing is shown from neither lens: the two 97.4-degree caps overlap by
about 14 degrees, which is checked over the whole sphere by `cargo test`
and over a 40-view sweep of real footage by counting the pixels the shader
painted grey. Where one lens carries the ray alone its weight is written
rather than divided out, because a GPU `x / x` is a reciprocal multiply
and lands an ulp short: on RADV that ulp reached the picture as one code
on 6 pixels of a million, which is enough to stop a one-stream file from
rendering the bytes it used to.

Exposure is corrected, but not from the shutter records (issue #103, stage
3). The trailer carries both lenses' per-frame shutter (records 4 and 12,
parsed by `kjerag-meta` and kept apart) and it is **not** a brightness ratio:
the two lenses trade shutter against sensor gain to reach the same picture
brightness, and applying the symmetric split that ratio implies makes the step
across the seam four to twenty times worse (6.3).

What is corrected is what the band **measures**. The same compute pass that
lines the two lenses up on the same content reads how much brighter one of
them drew it, which is a question only that alignment makes answerable: the
correlation that finds the shift is invariant to a brightness change, so the
shift is not moved by the exposure and the exposure read at that shift is not
moved by the shift. One workgroup then pools the ring into a single gain and
`Tone::split` halves it between the two lenses, so neither hemisphere carries
the whole change.

Three things about it are measured rather than chosen, and
docs/research/insv-format.md 6.10 has the tables: it is pooled from the **far
field only**, at the band's own knee, because a near-field direction's
photometry reads the alignment rather than the exposure; it is a **least
squares in codes**, which is the only one of three poolings that lowers the
step on all nine captures tried; and it is smoothed at the constant the far
field already uses, so nothing was added for it. A file with one lens stream,
and every frame before the first reading, take a gain of exactly zero and draw
byte-identical pictures.

The forward map exists twice, in `crates/render/src/projection.rs`: once in
WGSL for the GPU and once in Rust so `cargo test` can check known angles
with no GPU and no footage. Both read one `Reframe` uniform block, and the
bind group's `min_binding_size` makes wgpu reject a pipeline whose two
definitions have drifted apart.

**Two definitions of the same arithmetic drift, and `min_binding_size` only
catches the layout.** A review on 2026-08-09 planted a bend inside the WGSL half
alone: the whole workspace stayed green while the rendered picture changed, and
the only instrument that noticed needed real footage and a second build of the
tree. `crates/render/src/twin.rs` is the cheap guard - it compiles the shipped
`projection::wgsl()` with a compute probe after it, runs `blend` on a few
thousand rays on this box's own device, and compares every weight and landing
against the Rust mirror. It needs a GPU, so CI skips it and
`KJERAG_REQUIRE_GPU=1` turns that skip into a failure; `scripts/uitest.sh` runs
it that way, which is the same seat the harness itself sits in.

**A guard is only a guard at a fixture that reaches the code**, which the same
review proved a second time on 2026-08-09: the probe was built on a pose with
no rolling shutter in it, so `row_axis` was zero, so the readout half of
`project` was behind a false test on both halves and ran on neither, and a
WGSL-only change to `readout_share` passed 226 of 226 tests while the picture
moved. The fixture rolls now and the test asserts that it does.

Reframing, stabilization, and rolling-shutter correction fuse into ONE
backward mapping per output pixel. No intermediate equirect, ever.

## The output projection: flat, then bent, then a ball (issue #47)

The map above answers "which pixel is this ray"; something else has to say
which ray a point of the **output** is, and until issue #47 that was one line
of rectilinear projection with a hard 110-degree cap on it. Past there a flat
window stops being one: it stretches the corner by `1 / cos` of the angle out
to it, 3.1x at the corners of a 110-degree 16:9 view, and runs away to
infinity at 180. So the frame bends instead, and keeps bending until the whole
sphere is a ball with room around it.

One family does all of it (`projection::Screen`). A plane radius `r` from the
middle of the frame is the direction `theta` off the view axis with

```
r = tan(shrink * theta) / shrink
```

`shrink` of 1 is `tan(theta)`, the flat window, arithmetic for arithmetic what
it always was. 1/2 is `2 tan(theta / 2)`, which **is** stereographic: the
**tiny planet**, what Insta360 calls it and what the owner means by the blue
ball, arrived at rather than special-cased. Below that the
sphere closes into a disc of finite radius and the frame reaches past it.
`shrink` is `110 degrees / fov`, held at 1 until the view is wider than that,
so past the threshold `shrink * fov / 2` is constant: **the frame keeps the
half angle of the widest flat view and the world is shrunk into it**, which is
what keeps zooming out zooming out. `the_picture_only_ever_shrinks` is that
claim over the whole range and every point of the frame, and it is the one a
different-looking schedule fails: the widening and the bend pull opposite ways
and an unbalanced pair hands back a scroll that reverses in the middle.

The field of view keeps meaning what it meant, past a full turn included: 360
degrees is the frame's own edges half a turn out, so the sphere is exactly as
wide as the frame, and wider still is the frame reaching past the sphere. That
is where the room around the ball comes from, and it is why the far end
(`fov_ceiling`) depends on the **window shape**: the ball is round and fills
0.8 of the frame's shorter side there, which is 406 degrees on a square window
and 605 on a 16:9 one. Outside the ball there is no ray at all, and since
issue #100 the pass writes nothing there: transparent black, through a
premultiplied blend, so **what fills the room is whatever the shell put behind
the video** (`app::backdrop`). In a window that is libcosmic's own pane, which
is a translucent copy of the background colour over the compositor's blur
while the theme is frosted and the same colour opaque when it is not; in
fullscreen it is black; and a still, which runs the same pass into a texture
cleared black, comes out black with no alpha left in it.

The drag needed no new mathematics, which is the finding rather than luck:
`Camera::look` and `Camera::aim` were already written against `view_ray`
rather than against a `tan`, so they invert whatever map the view is in. Two
things did change. A press on the room around the ball takes hold of nothing,
because there is nothing there (`look` answers `Option`), and a cursor exactly
a quarter turn out along the frame's own horizontal axis has a ray no pitch
can move, which is zero over zero in the height solve and would have left a
NaN camera that never comes back.

Measured on the X4 Air at 2560x1440 (`kjerag-spike --bin ball`):

- **No pop.** One scroll from 20 degrees to the ball, a notch at a time,
  rendered: the largest single step is 64.3 codes at fov 402, and the largest
  a step grows against the step before it is 1.32x at fov 173. Inside the flat
  range, which this change did not touch, that same number is 1.15x. Nothing
  stands out at the threshold or at stereographic; at a quarter of a notch the
  largest growth anywhere is 1.12x. The geometric statement is in `cargo test`
  (`the_bend_starts_without_a_step`): halving the scroll across the threshold
  halves the angle the picture moves, four halvings running, and a jump would
  not shrink at all.
- **It costs a tenth of a millisecond at the far end and nothing at all in
  the range that already existed.** Interleaved across the range, three runs
  back to back agreeing within 0.01 ms a cell: 0.71 ms/redraw at the default
  view, 0.63 at the threshold, 0.67 at 150 degrees, 0.71 at stereographic,
  0.83 at 320 and **0.81 at the ball**, against a 33 ms frame. The trig is not
  what the far end costs, and the table says so itself: 320 degrees runs the
  same bent map over a frame that is all picture and costs the same 0.83. (All
  six rows move together with the box's clock state -- an earlier session sat
  near 2.2 ms a cell with the same shape -- so this table is read across its
  own rows rather than against another day's.) The flat range is unchanged,
  measured the way it was first measured: `--bin zoom`'s cost table off this
  branch against the same binary built off main, alternated on one box, agrees
  cell for cell within 0.04 ms in both directions (fov 90: 0.56 / 0.73 / 0.98
  main against 0.58 / 0.71 / 1.00). It runs the two multiplies it always ran,
  and the `length`, the `atan` and the `sin_cos` the bend needs are all behind
  the one uniform test that says the frame is not flat.
- **Playback holds at the ball.** 20 s of real footage at the far end of the
  zoom, 2560x1440, with three 3840 px captures taken during it: 600 redraws,
  29.97 fps presented, **0 dropped and 0 starved**, 2.65 ms per redraw in the
  pass under live decode. The captures are the ball: a 3840 px still through
  `Scene::capture` at the ball is byte for byte the same picture as the same
  view drawn straight into a 3840 px target, which is issue #15's own check
  run at a field of view it was never written for.
- **Minification, not magnification.** Out wide an output pixel covers 7.6
  delivered texels at the middle of the ball and 5.3 at its rim, so issue
  #11's Catmull-Rom kernel reads 0.00 engagement everywhere past 110 degrees
  and the pass is plain bilinear, which is what it should be. What that costs
  is aliasing rather than softness: against the same view supersampled 4x4 and
  box averaged, the ball is 4.1 codes out over the pixels that have picture
  and 107 at worst, against 1.1 codes and 11 at the default view. A moving
  picture will shimmer on high-contrast edges out there. The fix is a
  prefilter, not a sharper kernel, and it is not built: the imported dmabuf
  textures have one mip level and no room to generate more in place, so it
  would be a downsample pass per frame per lens for a view the player is in
  for a few seconds at a time. Issue #47's comment thread has the numbers.

## Sampling a magnified picture (issue #11)

Zoomed in far enough, an output pixel sits inside one source texel and the
hardware's bilinear tent is what the eye sees rather than the picture. Where
that is true the pass reads a **Catmull-Rom kernel** instead, engaged
smoothly as magnification passes 1:1 and exactly off at or below it
(`crates/render/src/sampling.rs`). Sixteen texels, taken as **nine bilinear
fetches**: each axis's middle pair of weights is positive, so it is one fetch
placed between its two texels; the outer two are negative, which is where the
resolving comes from, and a sampler cannot weigh a fetch by less than
nothing. Measured against the same kernel as sixteen `textureLoad`s on the
highest-contrast view in this footage, the nine agree to 0.14 codes RMS and
one code at worst, which is the sampler's own filter-weight precision.

**How magnified is decided per fragment, off the map's Jacobian**, because
nothing about it is uniform. The fisheye's angular density varies across its
own picture (1106 texels per radian down the X4 Air's axis, 948 radially at
the rim) and a rectilinear output's rises towards its corners, so at the
widest view the player offers a 2560 px window is past 1:1 in the middle
(1.23 texels to the pixel) and two thirds inside it at the corners (0.74).
The shader reads it as `max(length(dpdx(landing)), length(dpdy(landing)))`,
the hardware's own quad derivative of the landing the model just computed,
which is the Jacobian by finite difference with the distortion, the mounting
and the readout already in it. It is taken in the entry point rather than in
`blend`, because a derivative needs uniform control flow and the blend is
branches; `Reframe::texels_per_pixel` is the Rust mirror, checked in
`cargo test` against the paraxial focal length `fx / (1 + xi)` over the whole
zoom range.

Reading the step off the quad rather than off a resolution in the uniform
block is also what makes a **still** right without being told: the capture
draws this same pipeline into a target of its own size (issue #15), and a
quad of that target steps a smaller share of the picture by itself. A 3840 px
still off a 2560 px window is byte for byte a 3840 px render of the same
view, which `kjerag-spike --bin zoom` checks rather than assumes.

**The chroma plane is not upgraded**, and that is the measurement rather than
an omission. NV12's two planes are two grids: chroma is half the size, so one
output pixel covers half as many of its texels and it reaches 1:1 an octave
of zoom before luma does. On this camera at a 2560 px window that means
chroma is magnified at **every** field of view the player offers, so
upgrading it is not a cost paid at high zoom but a cost paid always, and it
is the larger half of the bill. What it buys on 8-bit 4:2:0 chroma that HEVC
has already smoothed is 0.41 codes on 40% of pixels and no measurable change
in detail at all. `Sampling::Sharp` keeps it renderable, one line from
shipping, for the footage that would change the answer.

## Rolling shutter (issue #9): fused, measured, and on

A frame does not leave the sensor at an instant. `rolling_shutter_time` is
15.883 ms on the X4 Air, so the row a ray lands on was read up to 8 ms from
the frame's nominal time, and the orientation that ray should be carried
through is the one at **that row's** instant. That is circular, because the
row decides the instant and the instant moves the row, so the landing is
solved for rather than computed: `Reframe::solve`, from the frame's own
instant outwards. **One round**, which at the hardest instant of a 30-minute
capture (551 deg/s) leaves 4.5 px of a 112 px correction and at the median
rate leaves a hundredth of a pixel; a second round would cost another pass
through the model per lens per pixel for a quarter of a pixel.

It is one rotation per frame and a multiplication per pixel, not a lookup per
row: `OrientationTrack::turn` over the readout window is a rotation vector,
and a row's share of the readout scales it. Measured against the track's own
orientation looked up per row, that straight line is 0.019 to 0.068 degrees
out at the median and 0.64 at the worst of two captures, and what is left is
vibration inside one readout that a 200 Hz track does not resolve either.

The correction is **not under the horizon toggle**: it is the camera's own
motion during the frame, not the display's, so a view that rides the body has
the same skew in it as one that does not. With no IMU record it disables
itself and the pass is what it was before, down to the bits.

**Which way the sensor reads is not in the file**, and it decides everything:
applied backwards, a correction does not fail to remove the skew, it doubles
it. So it is measured per camera in `readout_sweep`, and an X4 reads **down
the delivered frame**, 1.00 +-0.12 of a whole frame in the trailer's own
15.883 ms, against 0.02 +-0.07 across it. Both lenses read down their own
pictures, which is the same world direction, so it cancels at the seam and a
seam measurement is blind to it: that is why issue #42 shipped
`Sweep::Unknown`, and it is also why switching this on cannot put any of the
1.9 degrees of misalignment that an across-frame sweep would have put into
the band #7 blends.

A camera nobody has measured keeps `Sweep::Unknown`, which is a zero axis and
therefore no correction at all rather than a guess. docs/research/insv-format.md
6.7 has the three instruments, the injected controls on each axis of the fit,
and why a still capture cannot answer this question.

## Horizon lock (issue #8)

The trailer's IMU record is read at open, integrated once, and the result is
a `world_from_body` quaternion every 5 ms of the file
(`kjerag_meta::OrientationTrack`). The pass composes its inverse between the
lens mounting and the camera:

```
view_to_lens = lens_from_body * body_from_world * camera_rotation
```

Identity in the middle is the toggle off, and then the pass is bit for bit
what it was. The drag needed no change at all, which is the finding rather
than luck: `Camera::look` answers in whatever frame `camera_rotation` lands
in, so with the lock on that frame is the world, the anchor a press stores is
a world direction, and the solve puts a world direction back under the
cursor while the picture turns underneath it.

The filter is complementary and about sixty lines: integrate the gyroscope,
turn the estimate towards the accelerometer with a 20 s time constant, and
believe the accelerometer only while its magnitude is near 1 g, because a
banked turn is not gravity. Roll, pitch and yaw are then **all locked**: the
view is pointed at a direction in the world and the aircraft turns underneath
it, which is what Insta360 Studio does (owner ruling, 2026-08-06). The three
finite constants are measured and the tables are in
docs/research/insv-format.md 8.5; the yaw one is infinite and is a ruling
rather than a measurement. What the lock cannot hold is the gyroscope's own
yaw drift, because gravity does not observe heading and no capture here carries
a byte of the trailer's magnetic record. It is **not a steady creep**: the
locked frame turns at `bias . up_in_body`, so a camera tens of degrees off
vertical brings its horizontal bias in, and `kjerag-spike --bin drift` walks
that through the July 14 file at -36 degrees by minute 3, +87 by minute 8 and
+149 by minute 19, about 185 degrees peak to peak against a signed mean of
2.08 deg/min.

**The world frame's yaw zero is the heading at the file's first IMU sample**,
which is a couple of seconds before its first video frame: the body has turned
18.71 degrees by the time the picture starts on that capture, so `Ctrl+0` is
that direction and not the aircraft's nose. It is a convention, and every
stored `lock=1` view line means what it means only while the convention holds
(docs/UI.md, the view line; docs/research/reference-views.md).

Verification without a Studio export: physics in the footage itself.
`kjerag-spike --bin horizon` renders runs of frames through the app's own
pass and measures the angle of the horizon in each. Residual sway is 0.23
degrees peak to peak over 120 frames of calm flight and 2.86 through a
61 deg/s roll, against a picture whose horizon leaves the frame entirely with
the lock off. The same instrument's 24-way sweep of axis conventions is the
negative control: the string telemetry-parser falls through to for this
camera reads 54 to 65 degrees of standard deviation against 0.04 to 0.68 for
the right one.

The orientation track is also **issue #9's input**, and it needed one method
rather than one call per row: `OrientationTrack::turn` reads the two ends of a
frame's readout window and answers the rotation between them as a vector, so a
row's share of the readout is a multiplication in the shader instead of a
lookup. The section above is what came of it.

## Clock domains (the correctness minefield)

Video PTS, `first_frame_timestamp`, gyro timestamps, `gyro_timestamp`
offset when `is_has_gyro_timestamp`, per-lens exposure timestamps, and
rolling-shutter row time (15.9 ms on the X4 Air). Failure mode is a
swimming horizon, not a crash. Two of them are now nailed down and measured
(below and in issue #8's entry above); the harness that measures them is
`kjerag-spike --bin horizon`, and a Studio export drops into it as one more
row.

One of those is now nailed down. **The trailer's tick is not always a
microsecond**: `is_raw_gyro` selects it, and `first_frame_timestamp` is in
whichever tick the file uses. The X4 Air sets the flag and writes
microseconds; the ONE X2 does not and writes milliseconds, including in
`first_frame_timestamp`. That is the "divide by 1000 twice" of the format
study read as what it is, and it is measured against both cameras'
exposure tracks in `kjerag_meta::ExposureTrack`. The gyro track reads on the
same `Clock`, and then takes `gyro_timestamp` off it as milliseconds (1.6 ms
on the X4 Air).

## Open questions

- Vignetting coefficients are not in the metadata; the seam band may show
  rolloff. The weight field down-weights the rim it lands on, which may be
  enough; flat-field calibration if it is not.
- The **order** of `yaw`, `pitch` and `roll` within the lens pose. Their
  composition is settled (above); the order is not, and no known camera can
  distinguish it, because every one of them records sub-degree yaw and
  pitch (docs/research/insv-format.md 4.8).
- What remains on the legacy generic seam. Historical fitting isolated a
  repeatable calibration component and a content-dependent component on
  flights. The production app no longer fits or stores that five-knob
  correction. docs/research/insv-format.md 6.8 retains the measurements and
  transfer table; the selected ONE X2 route is the recovered type-2 map
  described above rather than a continuation of that fitting design.
- The following legacy-seam exposure measurement predates the selected resident
  path's automatic photometric matching; that older path did not correct lens
  exposure (6.3). Exact Studio parity for the selected ONE X2/X4 matching remains
  open in issue #185. When the legacy crossover narrowed from 10 degrees to 2,
  there was less band to hide a brightness step in. Measured then on the flattest,
  brightest content in this footage, it did not become one: the luma profile across the seam is the same
  ramp either way, 147.8 to 153.8 codes over 70 px before and 147.1 to 154.4
  after, because what differs between the two lenses there is vignetting
  inside each lens's own picture rather than a step at the handover. One view;
  it identified vignetting as a diagnostic lead, not proof about the current
  photometric result. That crossover became 8 degrees wide on 2026-08-05,
  a band this reading was not taken over.
