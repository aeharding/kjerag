# Flatpak permissions

One line per `finish-args` entry in [`flatpak/dev.harding.Kjerag.yml`](../flatpak/dev.harding.Kjerag.yml).
Longer reasoning lives in the manifest comments and [DISTRIBUTION.md](DISTRIBUTION.md); measurements in the linked issues.

| Permission | Why | Where |
|---|---|---|
| `--socket=wayland` | The window. No X11: the dmabuf frame path has never run under Xwayland, so claiming it would be a guess. | [`crates/render/src/scene.rs`](../crates/render/src/scene.rs) |
| `--device=dri` | The whole pipeline: VA-API decode on the render node, wgpu Vulkan import of the decoded dmabufs. No render device means files are refused, not played slowly. | [`crates/media/src/reader.rs`](../crates/media/src/reader.rs), [`crates/render/src/scene.rs`](../crates/render/src/scene.rs) |
| `--socket=pulseaudio` | Sound. cpal links ALSA; the runtime routes ALSA's `default` to the pulse plugin over this socket. | [`crates/media/src/player.rs`](../crates/media/src/player.rs) |
| `--filesystem=xdg-config/cosmic` `--filesystem=~/.local/state/cosmic` | cosmic-config escapes the sandbox by design: host theme in, remembered state out. Without the state grant every run logs "saved state not saved" (measured). | [`crates/app/src/config.rs`](../crates/app/src/config.rs) |
| `--talk-name=com.system76.CosmicSettingsDaemon` `...Config.*` | libcosmic reads the live system theme and settings from the daemon. | [`crates/app/src/app.rs`](../crates/app/src/app.rs) |
| `--filesystem=xdg-pictures` | Stills save where the pilot will look for them, not into a private sandbox home. | [`crates/app/src/shot.rs`](../crates/app/src/shot.rs) |
| `--filesystem=xdg-videos:ro` | Bare host paths are how most footage arrives: cosmic-files drags offer only `text/uri-list` (no portal registration), and CLI and double-click launches pass plain paths; all of them read fine under `:ro`. The one flow it cannot serve is the file chooser (the portal seals picks into single-file documents unless the grant covers its always-write ask, measured in #123) - v1 answers that with guidance and multi-select; the v2 folder-open design (#134) retires it entirely. | [`crates/app/src/dnd.rs`](../crates/app/src/dnd.rs), [`crates/meta/src/capture.rs`](../crates/meta/src/capture.rs), issues #118 #123 |
| `--filesystem=xdg-run/gvfs:ro` | Footage libraries that outgrew a laptop live on NAS shares the file manager mounts through GVFS; every such path is host-only. Read-only: the player never writes footage. | issue #118 |

Nothing else, on purpose: any file outside these grants arrives through the file
chooser portal or as a document-portal path from a source that registered it.
