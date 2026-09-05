# Local iced_wgpu patch

Source: `iced/wgpu` in libcosmic commit
`dc1cf9f00cbe2902a52166492654bb9fee8a73d1`, including its iced MIT license.
All source files are unchanged except `src/window/compositor.rs`.
The manifest expands that workspace's inherited values and keeps sibling
packages at the exact same git revision. It disables publishing.

The compositor now requests the adapter's supported storage-buffer count
instead of leaving shader widgets at wgpu's default limit of eight.
Other limits, fallback device attempts, features and renderer behavior are
unchanged. There is no new hardware requirement for the UI. Kjerag's selected
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

Checks: `cargo test -p iced_wgpu --lib device_limits`, plus the actual player
through `scripts/uitest.sh` on the selected ONE X2 capture. The root lock file
and Flatpak source list must change together; this directory ships as part of
the application source, not as a downloaded Flatpak crate.
