# Resident drawable prerequisite qualification

Date: 2026-09-01

This receipt covers the private prerequisite at code commit
`a2fbc67cb806815a1af20f4259a2885456f9dea9`, tree
`ae6fb9cf46cd231718ac2811bc498241ecb681ff`. That merge contains drawable
implementation commit `551333a0175411c5221a90b0aa904277786d2927` and accepted
asynchronous final-map validity commit
`17c7d8e4e7afc94be4b9bb5af805e19fe6475d0d`.

## Qualified scope

- `IcedDrawRetirements` supplies shared interior mutability around the accepted
  bounded retirement owner while registering the exact render-pass callback
  before invoking the borrowed draw closure.
- Registration, draw and poll panics remain fail-closed. Never-submitted work
  retains its source and consumes capacity exactly as the accepted primitive
  did.
- The direct Type2 shader, render pipeline and map bind-group layout are one
  immutable owner. The selected CPU route separately owns its exact-size
  packed and alpha upload buffers and binding, with unchanged bytes, shader,
  bind order and frame reporting.
- The merged final-map API exposes binding only on `GpuPackedMapFrame`, reached
  through the validated `ValidityPoll::Ready` transition. Its pending type has
  no binding operation.

This receipt makes no playback, Scene-selection, parity or performance claim.
It does not claim an installed resident draw owner. The current dmabuf import
site returns source frames and imported planes separately, so it cannot yet
produce an unforgeable association between the exact `Arc<Frames>` and those
textures. A sealed import-site aggregate is the prerequisite for that owner;
synthetic test ownership is not production proof.

## Commands and results

All retained logs are under the owner's repository at
`scratch/gpu-drawable-install-evidence/`. No retained evidence was written to
`/tmp`. Builds used the dedicated target directory
`scratch/targets/gpu-drawable-install`.

The forced-GPU commands set
`MESA_VK_DEVICE_SELECT=1002:15bf!`, `WGPU_BACKEND=vulkan` and
`KJERAG_REQUIRE_GPU=1`. Focused retirement tests reported
`AMD Radeon 760M Graphics (RADV PHOENIX)` and passed 8 of 8, including exact
callback release, never-submit backpressure, registration panic, draw panic,
poll panic, stale generation and repeated redraw cases. Focused direct Type2
tests passed 2 of 2, including WGSL validation and the dense CPU twin across
ordinary and polar views.

The complete forced-RADV `cargo test --workspace --quiet --
--test-threads=1` gate passed. The remaining required gates also passed:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
scripts/name-check.sh
scripts/cargo-sources.sh --check
```

## Evidence hashes

```text
512690ff0f796a8e27ab330b3ccb2fd344be8a32eb3b3b38a3500a32d2e56f92  cargo-sources.log
83160dca247dea8c7a80d67d66c4dcc5e91f29ce744918e25c20c6fa2c25c1e8  clippy.log
b5ecd977ae65519b5062cf5e8710b0432756c7b9be3fa7e95a5f73b45f3bf4d1  direct-type2-radv.log
70825c6743990f5d840b9768c32763aaa663b0210117be4f732ccfd9c113a83e  draw-retirement-radv.log
cc44e11aaf50aa8fe3760a34f51cf10528bee5a51e8a3c14a69252e7ef27ac21  fmt.log
44360d9dd9b75cb61253073ab7c1b8c5aaf1df0c3f40b213febc77e59a0e2c39  identity.log
b6a90f9f414d439e8bba2d41f2d7453325ab6e9bd11bf5663b02b4fb26e42430  name-check.log
75151237356ba8e5246e014abee8b762ae447c7b791748b4b2e481d88bf4cfef  workspace-tests-radv.log
```

The focused render test executable hashes to
`58c28d1510c7481dd2f2489d113457f0a12fd6b98615ca83d12c9c5989405da4`.
The workspace render test executable hashes to
`0a40ad98afa279cd8d13f9479241b2b66ce2d100a689ac170a1ada862ecfeb07`.
