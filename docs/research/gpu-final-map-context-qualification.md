# Shared-context GPU final-map qualification receipt

This receipt authenticates the unselected shared-context ONE X2 final-map
materializer at code commit
`e620a4cae27f1a51b8998328620594765c277dd7`, tree
`b7ce05854f507824170c1364ac4bddbc54b110c5`. The later commit adding this
receipt changes documentation only.

## Clean source identity

Immediately before the fresh build and again after the focused run,
`git status --porcelain=v1` was empty and `HEAD` still named the code commit
and tree above. Both author and committer are
`Alex Harding <noreply@harding.dev>`.

All hashes below are SHA-256.

- `Cargo.toml`: `d0f0203539c51fa37a8445f719daa388a1ba27f96679b48dd6fccde1dd6f99c1`
- `Cargo.lock`: `f39a30853d12e6696095803c1d7bc0a0f3f7f59e4e3e5ed42f2adee34bac4540`
- `crates/render/src/flow/one_xs.rs`: `9185489e7e88325d71c2ab31ad9d6b06c5a2a949d050aa3a2459fd68db3c25e3`
- `crates/render/src/flow/one_xs/gpu_context.rs`: `7c7842af1091678d38583de0c7fc9e05c60b624786620ec932e15e5b70519476`
- `crates/render/src/flow/one_xs/map_patch.rs`: `2c8614351ca78b6fedc4cecb47a0ec06f2ce82df4650e6044dcc2f9500e293e7`
- `crates/render/src/flow/one_xs/map_patch_gpu.rs`: `9cf1966836db388c1e5c69e12bc48d37aa7bb34a3c32cbfbb007c89ba59f0ea1`
- `crates/render/src/flow/one_xs/map_patch_gpu.wgsl`: `e2b7b42c626e2e2e8b301a1f2eb1bbde3c638d6f3ec2aadb39321d758c1590bd`
- `crates/render/src/direct_type2.rs`: `1bc3ac132a82302acef10c2ada78513415a391246acc3f15d793c04292132506`

The WGSL hash is identical to the accepted standalone final-map shader.

## Fresh build and pre-run seal

The target directory did not exist before this build:

```sh
env CARGO_TARGET_DIR=/home/aeharding/kjerag/scratch/worktrees/gpu-final-map-context/scratch/gpu-final-map-context-evidence/target-e620a4c \
  cargo test --locked -p kjerag-render --lib --no-run
```

The build started at `2026-09-01T11:47:12+00:00`, ended at
`2026-09-01T11:48:33+00:00`, and its retained transcript hashes to
`39e43f36b5cc883f40acf4c21cd61ab2a94197ff70fddc27d5c1ecad6c5cf5bd`.
It produced exactly one matching executable:

```text
/home/aeharding/kjerag/scratch/worktrees/gpu-final-map-context/scratch/gpu-final-map-context-evidence/target-e620a4c/debug/deps/kjerag_render-292a94dc388251e5
```

That executable hashed to
`09a221f59e77b3cee5b159db0a5159ba720f76aa5ed8833eed9372f2347791de`
before the run. Before invoking it, the evidence driver wrote a receipt that
binds the clean code identity, build and run commands, environment, ICD,
executable, and every source hash listed above. That pre-run receipt hashes to
`98be0bc640364941ab06ba3d5a53e07b07e4bea9155ee5f4ab1f717d9b9d9609`.

## Forced RADV run

The exact retained executable was invoked directly:

```sh
env KJERAG_REQUIRE_GPU=1 \
  WGPU_BACKEND=vulkan \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  /home/aeharding/kjerag/scratch/worktrees/gpu-final-map-context/scratch/gpu-final-map-context-evidence/target-e620a4c/debug/deps/kjerag_render-292a94dc388251e5 \
  map_patch_gpu --nocapture
```

The selected ICD file hashes to
`bca32a660ca32a42b598d53cdc63b3e3e917ea40c0b3b1e00fdbd3115280b51d`.
The running test reported `AMD Radeon 760M Graphics (RADV PHOENIX)`, driver
`radv`, Mesa `26.1.6-1pop0~1787580452~24.04~a5619ea`.

The run began at `2026-09-01T11:48:34+00:00`, ended at
`2026-09-01T11:48:35+00:00`, and exited zero. All four focused tests passed:
the CPU coverage fixture, exact resident packed-map twin and alpha binding,
frame/structural-context refusals, and refusal of all 20 live shader
mutations. There were no failures.

The complete run transcript hashes to
`34fe395b5f996a26a4b1e5bbc6943207294c03e723d04a556c41c98216df3c86`.
The executable hash after the run remained
`09a221f59e77b3cee5b159db0a5159ba720f76aa5ed8833eed9372f2347791de`.
The post-run receipt, including exit status, timestamps, adapter and unchanged
binary hash, hashes to
`dc1039b41b68df250f0e9bb10a6e05584c64094b2ab8067d9bf6636eb407f777`.
The retained evidence driver hashes to
`8cd300bb7a97a9a38ecbf02d4670ccbf7e64877898733bdb464dcbc9a7dafbb0`.

The ignored durable evidence remains under
`scratch/gpu-final-map-context-evidence/` in the branch worktree. This receipt
authenticates the focused unselected materializer qualification only. It makes
no Scene, playback, performance or wider parity claim.
