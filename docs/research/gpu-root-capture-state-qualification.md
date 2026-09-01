# GPU resident root-capture qualification

Date: 2026-09-01

Scope: private, unselected ONE X2 resident capture-root reservation and motion
handoff. This checkpoint does not wire Scene, final-map installation or a
production ready-draw publication path.

## Qualified code

- Commit: `7f0af2c91d1f842e4ed200eac6b513b228147448`
- Tree: `487de81c0fcda9148754e22ede951aa3bd3a494e`
- Parent: `72048d2e1057140da81a06811002269ebd3d62d4`
- Author and committer: `Alex Harding <noreply@harding.dev>`
- Dedicated target was absent at preflight and created only after the code
  commit under
  `scratch/gpu-root-capture-state/audit-7f0af2c-target`.
- Retained evidence is under
  `.worktrees/gpu-root-capture-state-recovery/scratch/gpu-root-capture-state/audit-7f0af2c/`
  in the owner's repository.

## Environment and commands

- Rust: `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- Cargo: `cargo 1.97.1 (c980f4866 2026-06-30)`
- ICD: `/usr/share/vulkan/icd.d/radeon_icd.json`, SHA-256
  `bca32a660ca32a42b598d53cdc63b3e3e917ea40c0b3b1e00fdbd3115280b51d`
- Mesa Vulkan package:
  `mesa-vulkan-drivers:amd64 26.1.6-1pop0~1787580452~24.04~a5619ea`
- Forced GPU environment:
  `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json` and
  `KJERAG_REQUIRE_GPU=1`
- Adapter reported by the tests: AMD Radeon 760M Graphics (RADV PHOENIX),
  driver `radv`.

The clean-build gate ran:

```text
cargo fmt --all --check
cargo check -p kjerag-render --lib
cargo clippy -p kjerag-render --all-targets -- -D warnings
```

The post-commit forced-RADV gate ran:

```text
cargo test -p kjerag-render --lib flow::one_xs::one_xs_belt_gpu:: -- --nocapture --test-threads=1
```

Result: 41 passed, 0 failed, 0 ignored, 648 filtered. The set includes the
root reservation tests, early front-half admission tests, exact cold/warm
motion tests, foreign-context refusals and the lease-before-root-rollback
witness for transaction drop, transaction abort, transaction unwind, frame
drop and frame abort. Expected caught panics are printed by poison, unwind and
submission-lease panic-safety tests; each corresponding test passed.

## Evidence hashes

- `preflight.log`:
  `479f55f3530a55f9d0d90bbafe3bc7a0f9b694dd25142179e1b8d83069eceda1`
- `gates.log`:
  `e9db4fcb0941ae8fa9a343613ee02c14ff75e531a1a9a1a8d70cf0968f585151`
- `forced-radv.log`:
  `0b16471b6aae9ecf6f0c61a0b9ea5249dde4b2fe4f301e38b4bc1ae451027f8c`
- `postflight.log`:
  `75b124b86e4f1fb4d284b431bc2ba4f4050002e7ad33aea74cc71269ae2278bc`
- Exact test executable:
  `df5481d9f769aa98079743ef7d8bfeedcf72c7180051be9dfd4b199ce29ade0c`

The preflight and postflight both recorded a clean tracked status at the code
commit. Scratch evidence and its dedicated target are ignored, not production
artifacts.
