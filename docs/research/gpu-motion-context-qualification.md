# Resident GPU motion qualification receipt

This receipt authenticates the sealed resident motion/reference implementation
after publication was moved from the motion transaction to the future final
capture-owner ready-draw install. It makes no Scene-selection or performance
claim.

## Audited tree

- Commit: `8c25706ea5422760c21bf8c127f33c234e54e72c`
- Tree: `8afa41b99aa04cb695650b29d221e7dd29d68734`
- Parents: `51d4e53f1161c1c4dfc720c2a587508756c671f9` and
  `94159d98c6cd4750a0ef06f26ed4b35dfc13c5d6`
- `git status --short --branch` was clean before the fresh build and after the
  forced-RADV test.
- The dedicated target
  `scratch/gpu-motion-context/audit-8c25706-target` did not exist before the
  audit. Its executable search returned no test binary before the forced test.

The exact gate commands were:

```sh
CARGO_TARGET_DIR=scratch/gpu-motion-context/audit-8c25706-target cargo fmt --all --check
CARGO_TARGET_DIR=scratch/gpu-motion-context/audit-8c25706-target cargo check -p kjerag-render --lib
CARGO_TARGET_DIR=scratch/gpu-motion-context/audit-8c25706-target cargo clippy -p kjerag-render --all-targets -- -D warnings
CARGO_TARGET_DIR=scratch/gpu-motion-context/audit-8c25706-target \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  KJERAG_REQUIRE_GPU=1 \
  cargo test -p kjerag-render --lib flow::one_xs::one_xs_belt_gpu:: \
  -- --nocapture --test-threads=1
```

All format, check and warnings-denied Clippy gates passed. The forced test ran
31 tests with 31 passed, zero failed and 648 filtered. This includes exact
cold final-install publication, warm pending-frame drop without publication,
retry/publication, semantic mutations, foreign-context refusal, geometry,
frontend, final-map, and every submission-lease success, failure, panic and
double-unwind/drop regression.

## Adapter and toolchain

The tests identified the adapter as `AMD Radeon 760M Graphics (RADV PHOENIX)`,
driver `radv`, Mesa
`26.1.6-1pop0~1787580452~24.04~a5619ea`. `vulkaninfo` was not installed, so
the adapter identity is the one printed by the forced production qualification
tests. The system packages were:

- `mesa-vulkan-drivers:amd64` version
  `26.1.6-1pop0~1787580452~24.04~a5619ea`
- `libvulkan1:amd64` version
  `1.3.280.0-1pop1~1722439676~24.04~a41a7d6`
- Rust `1.97.1 (8bab26f4f 2026-07-14)`, Cargo `1.97.1`
- Linux `7.0.11-76070011-generic`

The ICD JSON was 138 bytes, timestamped
`2026-08-24 09:07:32 -0500`, SHA-256
`bca32a660ca32a42b598d53cdc63b3e3e917ea40c0b3b1e00fdbd3115280b51d`.
The resolved `/usr/lib/x86_64-linux-gnu/libvulkan_radeon.so` was 18,508,088
bytes with the same timestamp and SHA-256
`7bf31db67cbf803c7cbd8bbe9280f2ba0cb77d8840529af640dc5978a14e589d`.

Installed-package manifest evidence:

| File | Timestamp | SHA-256 |
| --- | --- | --- |
| `mesa-vulkan-drivers:amd64.list` | `2026-08-26 17:25:53.821254563 -0500` | `a3fc33a18a1e1150c3e6022cbdfa89ebba169bac55602b7e9a5e55e47f081bc7` |
| `mesa-vulkan-drivers:amd64.md5sums` | `2026-08-24 09:07:32 -0500` | `98e4a47fdb956ceec895dea4384885b974bffb658b16ab7ce34981cca3d476be` |
| `libvulkan1:amd64.list` | `2025-10-01 15:05:07 -0500` | `2eb51351d0fe8c10213fbe3f5b8cac853920fae33fdb398987d904a0533efd08` |
| `libvulkan1:amd64.md5sums` | `2024-07-31 10:27:56 -0500` | `3d3e69e85ae9dbc6297fcedfbe866c73414b2be817d525e35486591efa2a537d` |

## Source, binary and durable evidence hashes

The fresh test binary was absent before the forced test. Afterwards it was
518,174,640 bytes, timestamped `2026-09-01 07:35:33.037693790 -0500`, and had
SHA-256 `fc6cfabb434f43fd0bc6c4a801e980eca3be6512b025ef1475b151cf5a506aad`.

| Input | SHA-256 |
| --- | --- |
| `one_xs/temporal_gpu.rs` | `63a29d4e69c39aa93568d6b4415a74be5e774f888e73af7c57392e189a7836b1` |
| `one_xs/geometry_gpu.rs` | `17f487d1cc144a4a18091f8c70d4ac645284bba3abc0eb5fd317e29990112bc1` |
| `one_xs/map_patch_gpu.rs` | `d8fa5913a48405eb9a2f81ce973a8859a46cabddaab659fa6745343454546999` |
| `one_xs/map_patch_gpu.wgsl` | `e2b7b42c626e2e2e8b301a1f2eb1bbde3c638d6f3ec2aadb39321d758c1590bd` |
| `one_xs/pis_frontend_gpu.rs` | `836010ee8c307d529dfaf8a718f1d2166b2c9ed71a7137491d66eea0f5af0578` |
| `one_xs_belt_gpu.rs` | `5c72527c01cc7f95427a9baa62634ec9698d673f53b2352c51accaaf25d1a68f` |

The gitignored durable logs live under
`scratch/gpu-motion-context/audit-8c25706/`:

| Evidence | SHA-256 |
| --- | --- |
| `preflight.log` | `07f3e208b10c225d9c4d53694c81893818d30380ebfb882823c9639c45beb349` |
| `build-gates.log` | `1d4a60ff29741c071ddec8ef42e1fda070f31a3377e561a5e96cf7ff4215a802` |
| `forced-radv.log` | `4dc5ca1f7830f244cb88483fddf0fd04e8619e6b16b3b4bcc2837a9a413f92b0` |
| `postflight.log` | `471492c58413e1b84856da5e95b601a733f5b21ac284058b38d1094ccecad172` |
| `package-evidence.log` | `4ee57d8170b9a01a688af20f2a7af9e6644ad152e6fc7da424f4433b2873f25d` |

The command traces in those logs record the exact environment assignments,
pre/post executable searches, timestamps, hashes, test result, and clean
commit/tree checks. This receipt is documentation only; it changes no render
source or shader from the audited tree.
