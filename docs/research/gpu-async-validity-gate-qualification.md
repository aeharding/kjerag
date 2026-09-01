# Asynchronous GPU validity-gate qualification receipt

This receipt authenticates the private four-byte asynchronous validity gate
for the unselected resident ONE X2 final-map materializer. It makes no Scene,
playback, performance, Studio-parity or merge-readiness claim.

## Audited source

- Code commit: `71012c1fd97e4bdc59019dd93e0bc865258f9a56`
- Tree: `964f68614b089e3706c4bcb7be1ff5fc4adda64a`
- Parent: `72048d2e1057140da81a06811002269ebd3d62d4`
- Author and committer: `Alex Harding <noreply@harding.dev>`
- `git status --porcelain=v1` was empty immediately before the fresh build,
  before the forced run and after the run.

All hashes below are SHA-256.

- `Cargo.toml`: `d0f0203539c51fa37a8445f719daa388a1ba27f96679b48dd6fccde1dd6f99c1`
- `Cargo.lock`: `f39a30853d12e6696095803c1d7bc0a0f3f7f59e4e3e5ed42f2adee34bac4540`
- `crates/render/src/flow/one_xs/map_patch_gpu.rs`:
  `1eae34051b1222860802e5cd86b440b2d7a119eb96f20707e597a001cf61fae9`
- `crates/render/src/flow/one_xs/map_patch_gpu.wgsl`:
  `e2b7b42c626e2e2e8b301a1f2eb1bbde3c638d6f3ec2aadb39321d758c1590bd`

The unchanged WGSL hash is the accepted final-map arithmetic shader. The code
change adds only the post-dispatch validity copy and pending policy around it.

## Fresh build

The dedicated target did not exist before the build. The exact retained
command was:

```sh
env CARGO_TARGET_DIR=scratch/gpu-async-validity-evidence/target-71012c1 \
  cargo test --locked -p kjerag-render --lib --no-run
```

It ran from `2026-09-01 08:06:34-05:00` through
`2026-09-01 08:08:20-05:00` and exited zero. The complete build transcript
hashes to `362b0eeda981f1a6d0ac362b346bb3cbd26906cbf5c023e7107842f3936462a0`.
It produced exactly one matching executable:

```text
scratch/gpu-async-validity-evidence/target-71012c1/debug/deps/kjerag_render-292a94dc388251e5
```

That 518,988,432-byte executable hashed to
`08a0f94b5e1b268b4db7b3153642c419983e13854434d283fad780e4cfcce7d0`
before and after execution.

## Forced RADV run

The retained executable was invoked directly:

```sh
env KJERAG_REQUIRE_GPU=1 \
  WGPU_BACKEND=vulkan \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  scratch/gpu-async-validity-evidence/target-71012c1/debug/deps/kjerag_render-292a94dc388251e5 \
  map_patch_gpu --nocapture --test-threads=1
```

It ran from `2026-09-01 08:08:35-05:00` through
`2026-09-01 08:08:39-05:00` and exited zero. All six focused tests passed,
with 679 filtered out. They cover bounded nonblocking polling and valid Scene
conversion, exact packed-map arithmetic and alpha binding, invalid encoded
direction/component/patch refusal plus upstream-drop ownership, injected map
error text, frame and structural-context refusal, and all 20 live shader
mutations. The complete run transcript hashes to
`7b26a0fc560e5a9bd965a00000ce7a6c28af14db8eca4b238faa67f6d334819f`.

The test identified `AMD Radeon 760M Graphics (RADV PHOENIX)`, driver `radv`,
Mesa `26.1.6-1pop0~1787580452~24.04~a5619ea`. The selected 138-byte ICD JSON
hashes to `bca32a660ca32a42b598d53cdc63b3e3e917ea40c0b3b1e00fdbd3115280b51d`.
The resolved Radeon Vulkan driver hashes to
`7bf31db67cbf803c7cbd8bbe9280f2ba0cb77d8840529af640dc5978a14e589d`.
The loader package is `libvulkan1 1.3.280.0-1pop1~1722439676~24.04~a41a7d6`.

The host was Linux `7.0.11-76070011-generic`, Rust
`1.97.1 (8bab26f4f 2026-07-14)` and Cargo
`1.97.1 (c980f4866 2026-06-30)`.

## Wider gates and retained evidence

After the focused audit, the same clean code commit passed
`cargo fmt --all --check`, workspace Clippy across all targets with warnings
denied, the complete workspace unit/integration/doc-test suite, rename
checking and offline Cargo-source checking. The render library reported 662
passed, zero failed and 23 ignored tests; ignored tests require detached
private corpora.

The ignored durable build, executable and transcripts remain inside the
owner's repository under `scratch/gpu-async-validity-evidence/` in this
worktree. This documentation-only successor does not alter the qualified code
or its tree.
