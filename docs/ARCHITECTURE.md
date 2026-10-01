# Architecture

This page is the current implementation map: layer ownership, selected frame
paths, lifetime boundaries, and traps. It is not an experiment or release log.

Derivations, rejected candidates, measurements, and earlier qualification
states are preserved verbatim in
[`ARCHITECTURE-HISTORY-20260912.md`](ARCHITECTURE-HISTORY-20260912.md).
Current product status and open work belong in [`ROADMAP.md`](ROADMAP.md).
Delivery identity, tradeoffs, and qualification evidence belong in
[`MERGE_READINESS.md`](MERGE_READINESS.md).

The owner accepted the installed player built from source `48e1d741`: "looks
good. not perfect but pretty damn good." That acceptance includes the
source-owned world-coordinate temporal correction. Release 0.3.1 is published
and functionally qualified on the named X4 Air and ONE X2 fixtures. This is not
exact Studio parity, all-camera coverage, or a performance guarantee. X4 source
cadence and frame-time spikes remain unresolved; see the delivery authorities.

## Layers

Kjerag is a cargo workspace with one crate per layer:

```text
crates/app      kjerag         libcosmic shell, window, controls, alerts
crates/render   kjerag-render  wgpu import, stitching, temporal correction,
                               projection, screenshots, shader widget
crates/media    kjerag-media   ffmpeg demux/decode, paired frames, audio,
                               presentation clock, seeking
crates/meta     kjerag-meta    trailer, calibration, gyro, exposure, pairing
crates/spike    kjerag-spike   headless instruments and reference runs
```

The dependency direction is `app -> render -> media -> meta`, with
`render -> meta` for calibration consumed on the GPU. `meta` has no ffmpeg,
wgpu, or UI dependency and must remain independently testable.

The main boundaries are [`app.rs`](../crates/app/src/app.rs) for shell state,
[`scene.rs`](../crates/render/src/scene.rs) for renderer preparation and
publication, [`player.rs`](../crates/media/src/player.rs) for media time and
presentation policy, [`reader.rs`](../crates/media/src/reader.rs) for ordered
decode delivery, and `kjerag-meta`'s
[`calibration.rs`](../crates/meta/src/calibration.rs),
[`orientation.rs`](../crates/meta/src/orientation.rs), and
[`pair.rs`](../crates/meta/src/pair.rs) for file-derived facts.

`render` mentions libcosmic only in
[`widget.rs`](../crates/render/src/widget.rs), where Rust coherence requires
the shader widget traits to live beside renderer-owned types. Engine code does
not report directly to the pilot.

## Capture admission and camera selection

The shell prepares file opens on one background worker, with one replaceable
queued choice. `Scene::prepare_with` reads the same complete calibration and
motion track into a Send-only `PreparedScene`, retaining the inspected Reader.
Packet input remains idle during that preparation. UI initialization returns
without waiting for capture IO. While the first file is preparing, the window
shows its normal transparent pane, not a black, welcome or loading screen.
With an existing video, that picture remains until the replacement is ready.
The UI starts Player/sound and constructs Scene only when the
still-current result arrives. No Scene or UI cells cross that thread boundary.
Close and a newer file choice revoke publication without joining a blocked
read. Obsolete results are released without waiting for their UI messages.
Failed opens preserve the old picture and view. CLI/pasted framing travels
with the request and is applied only after that same file opens successfully.
The clean `6dbbf16b` transparent-pane test Flatpak is installed. Both camera UI
suites and all six actual-installed blocked-open checks pass; the owner's
startup retest remains pending. Headless verification checks the ordinary pane,
not the desktop compositor's blur. This does not qualify instant first-picture
preparation or the remaining presentation/capacity requirements. The owner also
reports persistent intermittent stuttering after pause/resume or seeking,
cleared by another pause/resume or seek; its cause is not yet established.

An eligible selected capture constructs one immutable `ResidentCameraProfile`
from the parsed calibration before GPU state or sequential playback is selected.
Selection requires usable orientation, delivery of all calibrated lenses, and
supported camera data. The profile
owns resolved camera identity, source dimensions, parent-map inputs, static
maps, and camera support. Seeks reuse the profile but create a fresh temporal
epoch.

It is implemented in
[`flow/one_xs_belt_gpu.rs`](../crates/render/src/flow/one_xs_belt_gpu.rs),
with camera selection in
[`stitch_camera.rs`](../crates/render/src/stitch_camera.rs).
The selected shared resident stitcher currently admits the qualified ONE X2
and model-6 X4 Air families. Camera-specific calibration and coordinate laws
remain inputs to one GPU engine, not separate player architectures.

Unselected camera families, missing orientation, incomplete lens delivery, and
X4-like pairs without model-6 data stay on generic projection. Once the camera
classifier selects a resident family, profile-construction failures propagate
as errors rather than silently falling back. Selection is automatic.
There is no setup ritual or quality toggle, and the legacy optical flow control
cannot select a second solver for an admitted resident capture.

ONE X2 retains its recovered mounting and housing law; X4 Air uses model-6
parent geometry with explicit stream-order and body-chart conversions. The
downstream solver is shared. X4's image-circle/alpha support still comes from
the established v3 geometry, so peripheral model-6 coverage is not fully proven.

Container color matrix, range, and bit depth travel with decoded `Samples`, not
the camera profile. Conversion is shared by the ordinary and resident paths in
[`projection.rs`](../crates/render/src/projection.rs) and
[`direct_type2.rs`](../crates/render/src/direct_type2.rs).

## The frame path

There are three live paths. They share media delivery and calibrated projection
types, but their processing and resource lifetimes differ. Do not generalize an
invariant from one path to the other two.

### Selected shared filtered path

This is the production path for qualified ONE X2 and X4 Air captures:

```text
paired VA-API decoded Frames
  -> DRM_PRIME import of both lenses
  -> capture-owned resident source transaction
  -> parent map, geometry, belt, sparse PIS, and dense map on the GPU
  -> source-aligned photometric matching and immutable ratio pair
  -> one source-preparation command buffer producing:
       raw lenses + map + colour -> world panorama for temporal input
       raw lenses -> prefiltered R8/RG8 display snapshots owned by wgpu
       source map -> native endpoint cache for curved view drawing
  -> reduced periodic temporal correction over seven real sources
  -> CorrectedFrame pairing the exact source, map, colour, matrix, current
     low-resolution control, and filtered low-resolution result
  -> one final view-projection pass to the surface or screenshot target
```

The final view is one render pass, but source preparation is not. Source-rate
GPU work precedes publication, and its completed result is reused across view
redraws. A 29.970 fps source does not imply 240 distinct source pictures.

The unqualified `refactor/filtered-work-admission` branch changes admission and
GPU continuation scheduling, not picture sampling. Its RGB and coordinate-cube
experiments failed actual-player qualification and are removed. Source-stamped
native endpoint caching and the existing full-resolution display path remain.

The worker and facade live in
[`filtered_capture.rs`](../crates/render/src/flow/one_xs/filtered_capture.rs),
[`resident_worker.rs`](../crates/render/src/flow/one_xs/resident_worker.rs),
and [`temporal_worker.rs`](../crates/render/src/flow/one_xs/temporal_worker.rs).
Completion cannot publish a frame. Only the UI-side playback event transaction
may install the completed result matching `Player`'s opaque `FrameStamp`.
That transaction is independent of surface redraw callbacks. A redraw consumes
the latest due completed picture, not the source-processing queues.

The filtered stitch executor owns a capture actor with a bounded ordered work
queue, rather than one shared-executor channel message per source. Executing
stitch, queued stitch and temporal work share a four-source CPU admission
limit and the four-output reservation bound. GPU lifetime slots are separately
bounded at two and acquired by the worker only when a source begins execution.
Queued decoded frames do not consume these slots. An admitted successor continues
without another shell handoff. Actor exit and admission use the same short
state lock, so the successor either belongs to the running actor or kicks its
replacement. Queue payloads hold no back-reference to their capture owner.
Source admission never polls the graphics device or reserves GPU lifetimes.
The worker drives nonblocking retirement polls and checks epoch cancellation
while waiting for a slot, with no Scene/state lock held.
The shell checks that CPU admission capacity before constructing another source
projection; the final admission still rechecks it under the same state lock.
Seek and terminal failure discard unexecuted queue payloads, while submitted
work keeps its existing GPU-retirement ownership.

The resident map path remains GPU-owned. After capture-session construction,
ordinary live processing maps the four-byte final validity word and a private
four-byte temporal-output completion marker. The marker's contents are never
read: its buffer mapping proves the final output submission's completion.
It does not read back solver belts, sparse terminals, full maps, source images,
or temporal output. The first lazy resident-session construction does run
target-device arithmetic probes with bulk readbacks and waits. Tests and
instruments may also read resources; neither describes steady-state playback.

### Source, map, and temporal ownership

Imported dmabufs alias VA-API surfaces and stay in the source transaction until
GPU completion. A completed corrected frame does not retain those aliases.

[`source_snapshot.rs`](../crates/render/src/direct_type2/source_snapshot.rs)
evaluates the selected source prefilter into ordinary GPU-owned textures:
`R8Unorm` luma and `Rg8Unorm` chroma at each original plane's dimensions.
Chroma has half the luma width and height. These are rendered copies, not VA
aliases, and retain exact frame and device identity. This snapshot path requires
8-bit input; do not infer higher-bit-depth support from the generic importer.

The installed `MapSnapshot` in
[`map_patch_gpu.rs`](../crates/render/src/flow/one_xs/map_patch_gpu.rs)
retains its packed map, alpha, and immutable photometric ratio binding. Newer
computation cannot replace bindings owned by the installed draw.
Its diagnostic handles reference those same allocations, not the production
carrier or decoder leases. Explicit Scene map inspection follows the retained
filtered owner when that path is selected; it must not consult the separate
spatial facade. Map readback remains diagnostic-only, never a playback step.

The filtered path also evaluates its 51 by 101 native map endpoints once in
the same source-preparation command buffer. Each source owns a 164,832-byte
GPU cache containing endpoint positions and packed map samples. Curved-view
fragments reuse these values instead of recomputing them for each screen pixel
and redraw. Native cell search, watertight triangle admission, barycentrics,
alpha, fusion and full-resolution source sampling remain in the final draw.
The cache names the same opaque frame and graphics context as the map and
source snapshots; their completed owner retains its binding until retirement.
It introduces no readback, CPU wait, extra queue submission or source-cadence
change. Temporal-input panorama evaluation is unchanged. The uncached draw
remains a test-only same-owner reference. Separate GPU compilation can change
floating-point rounding, so this optimization requires rendered comparison,
not an assumed pixel-identity claim.

[`panorama.rs`](../crates/render/src/direct_type2/panorama.rs) distinguishes:

- `RgbPanorama`, a coordinate-neutral source-stamped gamma-RGB texture;
- `WorldPanorama`, that texture inseparably paired with the exact
  `body_from_world` columns used to materialize its world chart;
- body-fixed panorama types retained for references and diagnostics.

The temporal image stream consumes only the coordinate-neutral panorama.
Coordinate ownership stays beside the matching high-resolution display source.
[`corrected.rs`](../crates/render/src/flow/one_xs/corrected.rs) enforces that
`CorrectionInput` names the same `FrameStamp` for panorama, source, and map.
Its view endpoint cache must name that same frame and graphics context too.

[`correction_stream.rs`](../crates/render/src/temporal_fusion/correction_stream.rs)
owns two low-resolution textures per emitted source:

- `current`, the unfiltered RGB/NV12/RGB control for that source;
- `filtered`, the completed temporal result for that same source.

`CorrectionSequence` retains the matching high-resolution source/map owners
until the temporal stream emits their center. `CorrectedFrame` then moves the
exact display source, map, colour binding, immutable source-world matrix, low
current, and low filtered result into one typed owner. Final correction samples
the displayed body ray in that source's world chart. Mouse direction, window
size, and the horizon display toggle cannot steer the temporal field.

The corrected draw evaluates `clamp(rgb8(high) + (filtered - current), 0, 1)`
before the target's output transfer. The high term samples source-rate
prefiltered planes; both low terms share the same RGB/NV12/RGB conversion.
The panorama producer samples the raw lenses through the reference box, not
the prefiltered display snapshots. Horizontal temporal processing wraps the
panorama cut; vertical boundaries keep their selected rules. The reduced field
and prefilter can change noise and detail, as disclosed in the accepted tradeoffs.

History contains seven real contiguous sources. Startup and EOF clip reference
intervals rather than fabricate frames. Radius zero copies current but can be a
later reference. A discontinuous seek resets temporal, ISO, and color history;
an adjacent forward step may retain warm state.
Near-EOF seeks prepare the last seven real sources without showing pre-target
outputs. A whole capture shorter than seven sources reports unavailable
filtered output rather than inventing padding.

Terminal worker failure cancels unpublished temporal history without discarding
the independently installed picture. Cancellation never waits for the stream
mutex: an idle stream drops immediately, and executing work rechecks cancellation
after releasing its stream guard, including when cancellation wins after its
last in-operation check. The capture-state lock is released before cancellation.
A poisoned filtered-state mutex is made readable only after normalization to a
sticky terminal failure; this preserves the first raw error and stopped-picture
access, not permission to resume processing. Restart rechecks failure under its
final retirement lock, so replacement construction cannot hide an earlier error.

There is no coefficient EMA, invented gradual color transition, skipped source
refresh, correction interpolation, or reduced seam refresh cadence.

### Resident spatial path

A capture whose `ResidentCameraProfile` is supported can still lack an
authenticated temporal settings route. An explicitly unsupported selector is
reported at open and retains the shared resident spatial stitcher through
`ResidentCaptureFacade`; it does not silently guess ISO/filter settings and it
does not fall all the way back to generic projection.
A missing or unordered ISO track, or another provider failure on a supported
selector, is an error, not this spatial fallback. Individual zero/low ISO
readings follow the recovered repair law in
[`iso.rs`](../crates/render/src/temporal_fusion/iso.rs); they are not all errors.

The spatial path keeps resident parent mapping, PIS, dense map construction,
photometric matching, exact source/map ownership, direct type-2 drawing, and
completion retirement. It does not construct `FilteredCaptureFacade`, a
seven-source correction history, `WorldPanorama`, or a `CorrectedFrame`.
This boundary is selected in
[`scene.rs`](../crates/render/src/scene.rs) and implemented by
[`flow/one_xs_belt_gpu.rs`](../crates/render/src/flow/one_xs_belt_gpu.rs).

### Generic projection fallback

Unsupported cameras, captures without usable orientation, and explicit legacy
diagnostics use the generic route:

```text
VA-API decode
  -> DRM_PRIME import
  -> calibrated per-lens projection and generic seam blend
  -> one fragment pass at display resolution
```

This is the path described by
[`projection.rs`](../crates/render/src/projection.rs),
[`band.rs`](../crates/render/src/band.rs), and
[`seam.rs`](../crates/render/src/seam.rs). Its imported textures remain draw
sources, so decoder-frame retention covers GPU completion. It is not either
shared resident route and must not be used as their performance or ownership
model.

The generic route uses factory calibration and stores no fitted seam. Explicit
research correction remains available to instruments only. Historical seam
fitting and crossover measurements belong in research and architecture history.

## Playback (issue #4)

### Scheduling and publication

Qualified captures use `PresentationPolicy::SequentialRealtime` in
[`player.rs`](../crates/media/src/player.rs). Media PTS controls presentation
deadlines. The capture worker advances source computation in order while the
UI retains publication authority. For the selected filtered path,
`Scene::progress` handles source admission, completed-output installation and
clock transitions on decoder/worker notifications and absolute media deadlines.
It consumes sources in order even when the compositor withholds redraws.
On the unqualified branch the app dispatches source progression only from
`SceneReady`, which represents these notifications/deadlines. Mouse movement
and unrelated UI messages no longer rerun the source scheduler. Play, pause,
seek and step publish their own coalesced wake.

Source processing never skips camera inputs. With the owner's approval,
obsolete completed screen updates may be omitted while the playback owner
catches up with the audio clock. A render preparation samples the latest due
complete output. `Shown` separately retains the exact corrected-frame owner
used by the most recent draw preparation; screenshots, copied views, map
inspection and terminal-picture recovery follow it rather than a newer logical
installation. Neither logical progression nor draw submission proves physical
scanout. Filtered playback reports source advances and progress pumps, not
physical presentation rates.

Admission and ready queues are bounded. Backpressure may block a worker handoff,
never the UI thread. Submitted outputs may enter the bounded ready queue, but
the exact FIFO-front completion proof gates installation and acknowledgement.
Only completed pictures become displayable. The currently
shown frame remains independently drawable while a successor computes, a seek
lands, a renderer retries, or an older epoch drains.

The filtered route admits at most four CPU source jobs within an epoch, keeps
at most two source GPU lifetimes in flight, and
reserves up to four ready corrected frames plus one installed frame. Player
may prepare six real successors, including while paused for startup or seek.
One temporal executor and its capacity-one channel are shared across restarts;
seeking does not create another worker thread. Superseded epochs cancel and
release unpublished history when their executing work permits, while the old
shown owner remains independent. Retired resources release only on proven
completion; uncertain completion is quarantined, not recycled or retried as
fresh source work.

Draw-retirement teardown checks the exact callback generations already
published, without driving or waiting for the device. It quarantines every
unresolved entry before releasing completion-proven payloads, so a completed
payload's destructor panic cannot release an unresolved source. Already proven
draws must not be retained for process life merely because no later redraw
collected their callbacks.

Startup and exact seek landing hold picture and audio time until the requested
resident result is installed and acknowledged. Pausing during startup cancels
autoplay intent. EOF waits for admitted real inputs to drain before flushing
the temporal tail.

Coalesced progress notifications also cover startup operations that produce no
temporal output, shared executor capacity across seek epochs, and ready-FIFO
space released by logical installation. A preparation-specific decoder wait
supports paused startup and detects delivery racing registration. GPU lifetime
backpressure is worker-owned and adds no UI poll deadline; a paused, complete
pipeline has no periodic playback timer. Renderer attachment supplies
the authenticated GPU context once, but renderer preparation no longer submits
filtered sources or installs completed outputs. Other paths retain their
existing redraw-driven behavior.

The worker uses completion callbacks plus nonblocking device polls. On the
unqualified branch, resident L1 keeps its six command chunks but no longer
waits for the shared queue prefix between them. Same-queue ordering and dispatch
barriers preserve dependencies; the exact lease advances to the last submission
and retains its source until final validity proves completion. Renderer
retirement also polls without blocking. A blocking GPU wait in steady-state
playback can hold pinned wgpu's fence read lock while another thread needs
the write lock to submit work. Constructor arithmetic qualification remains
the explicit startup exception described above.

Temporal filtering likewise records the next ordered source without waiting
for each output's queue prefix. Each output owns a unique completion marker,
cleared in its final encoder and mapped after that submission. Since the marker
is never reused, unrelated later submissions cannot extend its last-use proof.
The sole temporal executor polls pending outputs even when its input channel is
idle; completion wakes Scene but never installs a picture. At four pending
markers the executor stops receiving jobs until GPU progress, and a startup
batch can produce four, bounding this monitor to seven markers across seeks.
Monitor entries have weak capture owners, not retained epoch histories.
Device/map errors retain the underlying error and use terminal worker cleanup.
This scheduling change is unqualified, not an established playback fix.

The source snapshot encoder now uses the same private submission-marker proof
for imported-source retirement. Pinned wgpu attaches a render-pass work-done
callback to the newest queue submission when deferred callbacks are registered;
a concurrent display submit can conservatively extend that boundary. The
snapshot marker's last use instead belongs only to its compound command buffer,
including its later lens copies. Retention still precedes source sampling;
mapping errors quarantine uncertain owners and preserve the underlying error.
Other generic draw paths retain their conservative callback generations.
The real seven-source decoder/GPU regression completes without a shell pump.
Short local2256x1504 player runs now maintain full source cadence during idle
playback,60Hz pan and uncapped pan. Network-backed pan still fails, and capacity
is155completed redraws/s with32.78ms maximum callback interval, not240/4.17ms.
This is a scoped local result, not a general playback or release verdict.

The six-face source-picture cache was retried after removing shared-prefix
CPU waits. It still failed NAS60Hz playback:20.58 source advances/s,9.63s
accumulating lateness and94.75ms maximum draw interval, with memory pressure.
It is removed, not a selectable production path. The restored candidate draws
the corrected source directly using its lens samples, map and temporal field.
The failed cache source and binaries remain in gitignored recovery artifacts;
no quantization/resampling tradeoff from that cache was accepted or installed.

The sphere broad-phase attempt also failed playback, despite its corrected
31-source images differing by at most3RGB8 codes from the reference. It is
replaced by an unqualified native-triangle rasterization candidate for finite
curved screens. Four subdivisions per native edge
follow the original triangle diagonal, positions and packed lens coordinates.
Hardware interpolation replaces per-pixel ray intersection on those views.
This approximates curved projection between subdivision vertices and can alter
subpixel detail/temporal coordinates. The frozen native preview has owner
acceptance. The clean committed-source package passes both camera UI suites;
network playback and the capacity requirement remain unresolved.
Original lens planes remain full resolution, with the same source history,
alpha map, photometric matching and temporal residual law. Ball views and
diagnostic uncached draws retain the original complete ray renderer. The static
index buffer is pipeline-owned, not a per-source picture cache.

The initial front-hemisphere guard passed the 16:9 diagnostic but disabled the
fast path in the actual 2256x1504 player after controls hid. At166.23deg its
full-window corners look slightly rearward. That NAS run still failed; it did
not qualify the intended renderer. The replacement projects a finite curved
view with a hidden-cell clipping rim halfway between the visible corner and
the projection singularity. Admission leaves a whole native cell outside the
visible cone before that rim; narrow-margin/ball views retain the ray path.
The real-source moving comparison now uses the actual full-window1.5aspect,
not a16:9 screenshot that misses this selection boundary. The owner subsequently
accepted the frozen native preview as "Good enough" after reporting improved
performance. The preview contains parked periodic-color edits excluded from
the clean SDK package. Qualification and identities are in MERGE_READINESS.

The corrected full-window mesh captures31ordered sources with no removed
coverage and at most8RGB8codes difference from the ray reference. It still fails
real-player runs before the source-specific retirement correction: NAS and local
storage both accumulate large video lateness. The later local results above
do not erase the unresolved network failure or establish the240capacity target.

The local iced renderer prepares the Scene before surface acquisition. If no
exact resident draw can be reserved, it keeps the previous complete surface
instead of submitting a cleared or UI-only frame. A coalescing worker wake and
bounded retry policy preserve progress without speculative idle redraws. The
patched named-child reconciliation preserves the Scene when controls return.
Patch provenance and removal conditions live in
[`iced_wgpu/KJERAG.md`](../vendor/iced_wgpu/KJERAG.md) and
[`iced_core/KJERAG.md`](../vendor/iced_core/KJERAG.md).

Generic realtime playback can also wait on that Scene subscription when its
next deadline is overdue and the decode channel is empty. The media layer owns
the empty-queue generation and a one-shot standard-library waker, not shell
subscription types. Successful note delivery or decoder exit wakes an armed
consumer; delivery racing registration prevents sleep. Pause and seek cancel
the old wait. Future deadlines, sequential stitch scheduling, and instruments
without a subscription retain their existing behavior. This changes idle
scheduling, not media timestamps, frame-drop policy, or image arithmetic.

### Decode, audio, and clocks

[`decode.rs`](../crates/media/src/decode.rs) owns VA-API decode.
[`reader.rs`](../crates/media/src/reader.rs) delivers one `Frames` value with
both lenses at the same instant. ONE X2 uses one demuxer per file and pairs the
two files by frame index, not by unrelated container start times. A lens frame
without a partner is dropped.

The system-memory [`Walk`](../crates/media/src/walk.rs) uses the same frame-index
alignment decision in [`pairing.rs`](../crates/media/src/pairing.rs). Each decoded
timestamp is normalized by its own source clock before pairing, and the first
lens's actual normalized time accompanies the pair. Its CPU-owned queue state
admits a frame before invoking the GPU-to-CPU transfer, so pre-cue frames cost no
copy. This shares pairing policy, not decoder ownership, Reader lookahead or
presentation scheduling.

Both deliveries use [`capture.rs`](../crates/media/src/capture.rs) to inspect
containers, order named siblings, prefer explicitly picked companions and check
that two files describe matching lenses. A discovered unreadable or mismatched
sibling leaves the named lens usable; a bad explicitly selected pair is an
error. The existing Reader admission and color-metadata rules are the common
policy. Demux seek targets remain unchanged; frame-origin normalization happens
at delivery, after decode.

The single-reader path gives audio its own packet queue and
decoder in [`track.rs`](../crates/media/src/track.rs), and retains the independent
producer in [`audio_worker.rs`](../crates/media/src/audio_worker.rs). Ring capacity paces
refill. Compressed read-ahead, not decoder surfaces, absorbs camera interleave
gaps; reaching its bound still backpressures both consumers. The producer parks at
audio EOF until a seek or shutdown. Video decode waits for audio-seek
acknowledgment, but the UI and device callback never join the producer or wait
for its I/O. Player observes the producer's underlying errors independently
of the video delivery channel and stops it when the capture closes.

Live Reader and Track input passes through
[`packet_input.rs`](../crates/media/src/packet_input.rs). A sole worker owns
each file's libavformat demuxer and routes compressed packets to separate audio
and video consumers. There is no second live audio demuxer or competing file
cursor. Video queues stop at 128 MiB or 512
packets per file; audio stops at 256 KiB or 128 packets. Byte accounting permits
one packet of overshoot because its size is known only after the read. These
are compressed-input limits, not added decoded/GPU frame retention. Walk
instruments retain their synchronous reference input.

Input remains idle until the first read or seek. Audio attaches before reading.
A video seek empties both compressed
queues and invalidates an in-flight read before the same demuxer repositions;
the decoder waits for acknowledgment. Closing signals the reader without
joining a possibly blocked filesystem call. The worker owns the input until
that read returns. Packet bytes, order, stream identity, timestamps, seek
targets, EOF and raw errors remain unchanged. This read-ahead stage absorbs
input bursts; it cannot make sustained slow input or GPU work run at realtime.

The audio target gate is set in that seek transaction, before input resumes.
An earlier video replay discards only pre-target audio packets, leaving source
processing ordered. Audio seek flushes its decoder and adjusts the gate without
repositioning video. Audio epoch invalidation interrupts an empty-queue wait
within 50 ms, without joining backend IO or treating cancellation as AAC EOF.

The follow-on candidate removes the custom file handle, AVIO callbacks and
16 MiB byte cache. The capture opener passes raw filesystem filename bytes,
without the convenience wrapper's UTF-8 unwrap. FFmpeg owns normal container IO from inspection
through close; there is no context substitution or second cursor. Closing
leaves that input alive until its sole reader finishes pending IO. Reader and
offline Walk share capture inspection, while Walk retains synchronous packet
delivery. This simpler input candidate still needs clean-package qualification.
CPU regressions compare packet bytes, timestamps, positions and flags with normal
FFmpeg reads, including routed audio/video consumers with repeated seeks.
Removed cache code/tests remain in Git history and the failed native control's
binary is retained in scratch. CPU AAC output matches the audio-only decoder reference.
That synthetic audio/video fixture uses the `ffmpeg` executable, installed
explicitly in both architecture CI jobs; no footage or sound device is required.

A seek invalidates audio writes under the ring lock before returning to the
UI. That authorization travels with the video seek command and the audio
decoder; an older in-flight packet or superseded seek cannot refill the new
ring. The existing callback, gain fades, resampling and splice arithmetic in
[`audio.rs`](../crates/media/src/audio.rs) remain unchanged.

The presentation clock remains based on container PTS. Its published `Beat`
anchor is extrapolated by the audio callback without needing redraw ticks.
Filtered video promotion and source admission use the playback event owner
described above. Generic and resident-spatial video paths still use the shader
redraw path; the independent audio producer is shared by all live Readers.

The gyro clock is distinct. Frame orientation uses the camera timestamp from
the exposure track, not nominal container PTS. Trailer tick units depend on
`is_raw_gyro`: qualified X4 Air data uses microseconds and ONE X2 uses
milliseconds. `gyro_timestamp` is a separate offset. These conversions live in
`kjerag-meta`, principally
[`exposure.rs`](../crates/meta/src/exposure.rs),
[`gyro.rs`](../crates/meta/src/gyro.rs), and
[`orientation.rs`](../crates/meta/src/orientation.rs).

## Horizon and rolling shutter

Horizon lock and rolling-shutter correction consume the same orientation track
but answer different questions.

Horizon lock changes the display transform. The renderer composes
`lens_from_body * body_from_world * camera_rotation`; disabling the lock makes
the displayed `body_from_world` identity. Camera drag remains in the coordinate
frame produced by that composition.

Rolling-shutter correction compensates camera motion during sensor readout. It
is source geometry, not a display preference, so it remains active when horizon
lock is off. Generic projection applies a source-frame rotation vector scaled
by row share; unknown readout direction disables that generic correction.
The resident parent mapper instead consumes 51 orientation poses across the
sensor readout, with camera-specific readout validation. It does not apply the
generic row-vector model to the native map. See
[`parent_inputs.rs`](../crates/render/src/flow/one_xs/parent_inputs.rs).
Kjerag's orientation provider is an approved substitute, not a recovered
implementation of Studio's internal stabilization cache.

The selected temporal path separately retains the actual source
`body_from_world` even when the displayed horizon is unlocked. That source
matrix fixes the temporal chart and later transforms a displayed body ray back
into the matching source's world coordinates.

The camera, reframe, and projection implementation is in
[`camera.rs`](../crates/render/src/camera.rs),
[`framing.rs`](../crates/render/src/framing.rs), and
[`projection.rs`](../crates/render/src/projection.rs).

## Projection

On the selected filtered path, source processing deliberately materializes a
world-coordinate 2:1 panorama for temporal correction. Final display projection
does not add another intermediate equirectangular image: flat perspective views
sample the completed source/map/correction with the native mesh, while curved
and tiny-planet views use the ray-based projection. The resident spatial path
draws its source and native map directly. The generic path fuses camera
projection, seam blending, and output projection in its single fragment pass.
Screenshots render the exact shown owner through the same surface-format
convention into an offscreen target.

Generic projection can apply the Catmull-Rom luma path in
[`sampling.rs`](../crates/render/src/sampling.rs), selected per fragment from
the mapped footprint; its chroma remains bilinear. Do not infer that generic
sampling policy for the resident spatial or filtered source consumers. Minified
wide generic views need a prefilter rather than a sharper magnification kernel;
no unqualified extra path should be inferred from historical experiments.

## Failures the pilot is told about (issue #124)

Lower layers author errors at the failure site in plain words. The shell shows
the raw underlying message rather than replacing it with a generic summary.
[`crates/app/src/fail.rs`](../crates/app/src/fail.rs) owns the only pilot
alert funnel: `Alert::raise(Failure)` both displays and echoes the failure.

The app may add guidance only where it knows something the lower error cannot:
unsupported camera format, missing decoder, inaccessible sandbox path, or the
instruction to reopen after a terminal playback stop. A stopped capture stays
stopped; reopening constructs a new capture. Capture-write failures remain
toasts because video is still available.

Renderer failures cross the layer boundary as typed stalls from
[`stall.rs`](../crates/render/src/stall.rs). Renderer code does not invent UI
copy or print an alternative diagnosis.

## Trap list

- Use every `AVDRMFrameDescriptor` layer's `pitch` and `offset` verbatim. At
  width 3840, chroma may use pitch 4096 while luma remains 3840.
- radeonsi may export one fd for multiple planes. Duplicate it per imported use
  and close every owned descriptor.
- Preflight DRM modifiers before Vulkan image creation. Unsupported use is
  undefined behavior, not a recoverable import attempt.
- Use FFmpeg `DRM_PRIME` only. `AVVkFrame` caused device loss on target hardware.
- `av_hwframe_map(... MAP_READ | MAP_DIRECT)` synchronizes the VA surface;
  preserve decoder lookahead to overlap the wait.
- FFmpeg VA-API pool size zero means dynamic allocation, not exhaustion. Every
  retained frame still delays surface reuse.
- Imported textures alias decoder memory until GPU completion. Submission is
  not retirement; retain `Frames` or use a sealed completion-tracked transition.
- Completed filtered display owns `SourceSnapshot` R8/RG8 copies, not dmabuf
  aliases, and needs no decoder lease after their production completes.
- Typed owners must keep capture root, decode epoch, `FrameStamp`, source, map,
  ratio pair, temporal output, and world matrix together. Indices cannot cross
  epochs.
- Resident work stays on its authenticated `OneXsGpuContext`. Structural
  device/queue equality uses per-Instance IDs; cross-Instance reuse needs
  explicit identity.
- A resident lease advances only on its retained queue. Never give it a detached
  `SubmissionIndex`.
- Never add blocking waits to steady-state playback, retirement, worker
  scheduling, or concurrency benchmarks; one thread's fence lock can block
  another's submission.
- Steady-state live readback is the four-byte validity gate only. Bulk map,
  image and temporal reads are diagnostic-only; initial constructor arithmetic
  qualification has explicit startup readbacks and waits.
- GPU completion advances computation, not publication. Only the exact due
  frame displays.
- Do not reduce source/seam cadence, interpolate corrections, or invent gradual
  color smoothing.
- A primitive test does not prove Scene uses that entry. Verify the selected
  real path.
- A guard covers only fixtures that execute its camera and rolling-shutter branches.
- The qualified 8-bit formats are `R8Unorm` luma and `Rg8Unorm` chroma. DRM
  `GR88` does not authorize swapping chroma channels.
- Screenshots use the shown surface format because sRGB state changes encoding.
- The iced patch must request the resident pipeline's storage-buffer capacity;
  remove it only after proving upstream configuration and presentation behavior.
- Never share a `CARGO_TARGET_DIR` between worktrees; instruments can otherwise
  run a binary from the wrong checkout.
- Keep the root wgpu patch and its enabled dmabuf/DRM-modifier extensions.
  `dmabuf::import` checks that the required extensions really are enabled.
- The runtime must match FFmpeg 7.1 headers (`libavcodec.so.61`,
  `libavutil.so.59`); installed runtime versions are not interchangeable.
- Preserve decode lookahead: mapping the oldest of 2-3 in-flight frames hides
  synchronization that mapping the newest frame exposes. The reader uses depth 2.
- GStreamer has no selected wgpu/dmabuf-to-Vulkan sink in this design. Its
  historical evaluation is retained in the archive, not a second media backend.
- Do not disable libcosmic's content container to remove video padding: that
  removes its background too. The app instead sets border padding to zero.
- Keep libcosmic's wgpu renderer enabled. The tiny-skia fallback does not draw
  custom shader primitives and can present a blank video widget.

## Current limits

The named ONE X2 and X4 Air routes are visually accepted and functionally
qualified for the published 0.3.1 delivery. The qualification does not extend
to arbitrary cameras, modes, bit depths, GPUs, or aarch64 playback hardware.

The active performance requirement is sustained 240 fps view-rendering capacity
during playback while every source frame is stitched at recorded cadence and
audio stays synchronized. X4 source cadence and frame-time spikes remain open.
Do not turn a paused-view benchmark, repeated source picture, average throughput,
or published release into a capacity verdict.

Exact Studio Image Fusion parity also remains open. The shipped photometric
matching is part of the accepted installed result, not proof of isolated native
coefficient or update-law identity. Do not add an invented gradual update.

Consult ROADMAP for the current work queue and MERGE_READINESS for the exact
release evidence. Update this page only when ownership, selected frame paths,
or hard implementation boundaries change.
