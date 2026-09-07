# Local iced_wgpu patch

Source: `iced/wgpu` in libcosmic commit
`dc1cf9f00cbe2902a52166492654bb9fee8a73d1`, including its iced MIT license.
The local changes are in `src/window/compositor.rs`, `src/engine.rs`,
`src/lib.rs` and `src/primitive.rs`.
The manifest expands that workspace's inherited values and keeps sibling
packages at the exact same git revision. It disables publishing.

The compositor now requests the adapter's supported storage-buffer count
instead of leaving shader widgets at wgpu's default limit of eight.
Other limits, fallback device attempts and features are unchanged. There is
no new hardware requirement for the UI. Kjerag's selected
ONE X2 path checks its own requirement before constructing GPU pipelines and
reports an ordinary playback error on a device below that requirement.

This fixes a real-window startup validation panic: the prepared-source stage
declares eleven storage buffers, and warm post-L1 declares fifteen. Headless
instruments requested adapter limits already and concealed the UI mismatch.
There is no shader, arithmetic, cadence or stitched-pixel change in this patch.

Why local: libcosmic does not expose device-limit configuration. Keeping only
this renderer crate avoids forking both libcosmic and its iced submodule or
changing the GPU pipeline's qualified buffer layout just to fit a UI default.
Remove this patch when the pinned UI renderer supports requesting these limits.
Do not send it or an issue to an outside project.

The current branch additionally evaluates native video readiness. Custom
primitives default to presentable; Kjerag can decline a frame when its two
draw-retirement slots are occupied. Window preparation checks this before
acquiring a swapchain image. Custom primitives are prepared first; a refusal
trims only their pipelines, without preparing built-in UI batches or submitting
an iced command encoder. The compositor skips physical presentation, preserving
its previous complete buffer. A ready window keeps one combined UI
preparation/render submission. Offscreen APIs remain ungated, but also use
shader-first preparation rather than interleaving it with built-ins. The sole
production custom primitive is Kjerag's Scene, which cannot access iced's
built-in batch or staging state and has no such ordering dependency.

The renderer always requests one immediate replacement redraw on the first
refusal because the widget's presentation tick ran before preparation learned
that retirement was full. A primitive may then opt into scheduling later
retries itself. The default is false, so an unavailable primitive without that
guarantee retains iced's redraw request on every attempt. Scene opts in only
when its post-prepare pending-refresh flag proves its next tick will wake; a
draw-retirement Full changes that otherwise-immediate refresh to a 1 ms timer.
Empty and target-mismatch states do not opt in and therefore retain the default
fallback. A successful aggregate preparation resets the refusal episode, and a
transition from a default retry owner to the self-scheduled Scene gets a new
bridge. This local contract supports one independent self-scheduled widget per
window, which is the application's one Scene. It deliberately does not add a
generic identity registry for multiple independent timer owners.

This prevents committing a cleared window on video backpressure while
retaining the previous complete UI and video, and bounds the Full-only retry
storm without changing draw-slot capacity or source ownership. The 1 ms value
is a Kjerag host scheduling policy, not a recovered Studio timing. It is still
under native performance qualification, not a shipping or 240 Hz claim.

The env-gated `preflight_tests` renderer regression uses a real GPU and checks
refusal with zero UI preparations/submissions, once-only custom preparation,
the first bridge and self-scheduled suppression, default fallback and mixed
owner transitions, ready recovery with an actual submit, clipping and ungated
offscreen rendering.
Run it with `KJERAG_WGPU_PREFLIGHT_TEST=1 cargo test -p iced_wgpu --lib preflight_tests`.
Skipping the environment-gated test is not GPU verification.

Checks: `cargo test -p iced_wgpu --lib device_limits`, plus the actual player
through `scripts/uitest.sh` on the selected ONE X2 capture. The root lock file
and Flatpak source list must change together; this directory ships as part of
the application source, not as a downloaded Flatpak crate.
