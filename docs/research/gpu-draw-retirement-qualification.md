# ONE X2 exact draw-retirement qualification

This receipt qualifies only the private, unselected ownership primitive that
binds retirement proof to iced's exact render pass. It does not wire Scene,
publish resident history, select resident arithmetic, or prove dmabuf playback
safety, parity or performance.

## Exact code and dependency line

- Code head: `c81a9c3e3e6c35069e951b79d5df17ffe43278ff`
- Tree: `b2b8383cba88e37656dc4f3fe1130d9c1b9c1608`
- Parent: `303d69f68a2e6d41cac601a0478efd2b3c07f08f`
- Integration merge: `4c2b095066cfc9f15af4f44a01bc76772487e09d`
- Exact integration parent: `94159d98c6cd4750a0ef06f26ed4b35dfc13c5d6`
- Accepted fail-closed queue ancestor: `bbab0c7231957c25e6bdb23c93c15f85a1a599d8`
- Author and committer: `Alex Harding <noreply@harding.dev>`
- Author and commit timestamp: `2026-09-01T07:50:59-05:00`

`git status --porcelain=v1` was empty before the fresh build, before the run
and after the run. The ignored raw log is
`scratch/gpu-draw-retirement/radv-draw-retirement.log`.

Relevant SHA-256 values:

```text
958b6a134fb7d2a86dbd7da4b482c54360ce3becd493cb2c6bb9523506cfa283  crates/render/src/draw_retirement.rs
f39a30853d12e6696095803c1d7bc0a0f3f7f59e4e3e5ed42f2adee34bac4540  Cargo.lock
71d3f8cc266e7e722f8148feee87b0a2d84183cb4220a514d966808c2f94af7f  crates/render/Cargo.toml
```

## Fresh build and exact executable

Generated render artifacts were removed with `cargo clean -p kjerag-render`.
The exact committed tree was then built with:

```sh
env MESA_VK_DEVICE_SELECT=1002:15bf! \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  WGPU_BACKEND=vulkan KJERAG_REQUIRE_GPU=1 \
  scripts/quiet.sh cargo test -p kjerag-render \
  draw_retirement::tests --no-run
```

The build exited zero and produced
`target/debug/deps/kjerag_render-292a94dc388251e5`. The executable was
518,567,760 bytes, created at `2026-09-01 07:51:33.421661574 -0500` and last
modified at `2026-09-01 07:51:33.867657823 -0500`. Its SHA-256 was
`b05c474f76250cae3c97b9a0563273b1e0937bdfe280d657c50a2af7e0032a68`
immediately before the run and remained identical afterward.

## Forced Phoenix RADV execution

The run started and ended at `2026-09-01 07:51:49-05:00`:

```sh
env MESA_VK_DEVICE_SELECT=1002:15bf! \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  WGPU_BACKEND=vulkan KJERAG_REQUIRE_GPU=1 \
  scripts/quiet.sh target/debug/deps/kjerag_render-292a94dc388251e5 \
  draw_retirement::tests --nocapture --test-threads=1
```

Exit code `0`: **8 passed, 0 failed, 676 filtered out**. Every real-pass test
named `AMD Radeon 760M Graphics (RADV PHOENIX)`. The raw typescript SHA-256 is
`0be112d1f597181b11251130ee7cdb9c3691564185ff80003e7c1c7e5470138d`.
The visible registration, poll and guarded-draw panic lines are deliberate
caught injections; their tests pass only after fail-closed quarantine.

The focused cases cover reservation before candidate installation, full
backpressure without moving an offered owner, undrawn candidate release,
ordered and out-of-order exact-once proof, repeated polling, bounded signal
reuse and stale-generation ABA refusal, initial draw plus repeat redraw without
history recommit, successor replacement while an old draw is pending, exact
offscreen render-pass completion, a never-submitted pass retaining
backpressure and source ownership, poll error/panic, callback-registration
panic with prior and offered owners, retained-permit accounting after terminal
failure, and guarded-draw panic quarantine.

## Device and driver binding

- PCI device: AMD Phoenix1 `[1002:15bf]`, revision `cb`, at `c1:00.0`
- Subsystem: Framework Computer Inc. `[f111:0006]`
- Kernel driver: `amdgpu`
- Adapter: `AMD Radeon 760M Graphics (RADV PHOENIX)`
- Kernel: `Linux 7.0.11-76070011-generic x86_64`
- Mesa Vulkan driver: `mesa-vulkan-drivers
  26.1.6-1pop0~1787580452~24.04~a5619ea`
- Vulkan loader: `libvulkan1
  1.3.280.0-1pop1~1722439676~24.04~a41a7d6`
- Radeon ICD: `/usr/share/vulkan/icd.d/radeon_icd.json`, SHA-256
  `bca32a660ca32a42b598d53cdc63b3e3e917ea40c0b3b1e00fdbd3115280b51d`
- RADV library: `/usr/lib/x86_64-linux-gnu/libvulkan_radeon.so`, SHA-256
  `7bf31db67cbf803c7cbd8bbe9280f2ba0cb77d8840529af640dc5978a14e589d`

The forced PCI selector includes `!`, Vulkan was restricted to the Radeon ICD,
the test was forbidden to skip for lack of a GPU, and the Vulkan backend was
explicit. The repository quiet wrapper routed any incidental audio to its null
sink.

## Non-hardware gates

The same code head passed:

```sh
cargo fmt --all --check
git diff --check
cargo check -p kjerag-render
cargo clippy --workspace --all-targets -- -D warnings
WGPU_BACKEND=vulkan cargo test -p kjerag-render \
  draw_retirement::tests -- --nocapture --test-threads=1
```

The focused host run also reported 8 passed. Hardware acceptance still awaits
independent audit; this receipt is not that acceptance.
