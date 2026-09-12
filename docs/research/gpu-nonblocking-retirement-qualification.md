# ONE X2 nonblocking retirement qualification

**Superseded topology:** this receipt covers a queue-prefix callback. The
current implementation instead attaches proof to iced's exact render pass;
this file remains only as the earlier ownership audit trail.

This file retains both the current target-device receipt and the rejected
receipt it supersedes. It qualifies only the submission-retirement ownership
primitive. The resident arithmetic remains unselected, and this is not a
playback parity or performance claim.

## Current callback-registration remediation

The first audit rejected `9ce5bc1` because callback-registration panic leaked
the newly offered payload but left prior pending entries available for normal
destruction after the queue entered its failed state. Commit
`bbab0c7231957c25e6bdb23c93c15f85a1a599d8` (tree
`241f64d5de55f09942660acb4d26cec9d76f0f77`) fixes that lifetime hole. It puts
the uncertain offered entry into the same bounded queue, forgets every prior
and offered completion, signal and payload, and only then enters the terminal
failed state. Its injected regression test starts with one pending owner,
panics callback registration for a second owner, observes neither drop witness
nor wait, and proves a third lease is refused without mutation.

The commit's author and committer are both
`Alex Harding <noreply@harding.dev>`. `git status --porcelain=v1` was empty
before the authenticated build and after both runs. Relevant source SHA-256:

```text
ba07c9475940aaca21ee3f44f389f0ae58c20dc782f7a27c6e7c48ec1ef860e9  crates/render/src/flow/one_xs_belt_gpu.rs
```

The exact post-commit build verification was:

```sh
MESA_VK_DEVICE_SELECT=1002:15bf! WGPU_BACKEND=vulkan \
  scripts/quiet.sh cargo test -p kjerag-render \
  one_xs_belt_gpu::tests::retirement --no-run
```

It exited zero and named
`target/debug/deps/kjerag_render-292a94dc388251e5`. The exact executable was
516,419,432 bytes, created at `2026-09-01 07:17:20.473661315 -0500` and last
modified at `2026-09-01 07:17:34.107549584 -0500`. Its SHA-256 was
`20381380592b29a41f81e160b01349ea1a5861d86f51bcd7f46463849b27d12d`
immediately before the runs and remained identical after both.

Both runs used the device and driver binding recorded below and the exact
environment shown in their commands. Retirement started at
`2026-09-01 07:18:31-05:00` and ended at `2026-09-01 07:18:32-05:00`:

```sh
env MESA_VK_DEVICE_SELECT=1002:15bf! \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  WGPU_BACKEND=vulkan KJERAG_REQUIRE_GPU=1 \
  scripts/quiet.sh target/debug/deps/kjerag_render-292a94dc388251e5 \
  one_xs_belt_gpu::tests::retirement --nocapture --test-threads=1
```

Exit code `0`: **5 passed, 0 failed, 671 filtered out**. The raw log SHA-256
is `29f847c3cd0c5833cc4211ecf49e6d874535d5b26a714325fb0dfd16d02a85b3`.
The existing poll-panic line and the new callback-registration-panic line are
deliberate caught injections; their test passed only after proving fail-closed
retention.

SubmissionLease paths started at `2026-09-01 07:18:40-05:00` and ended at
`2026-09-01 07:18:41-05:00`:

```sh
env MESA_VK_DEVICE_SELECT=1002:15bf! \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  WGPU_BACKEND=vulkan KJERAG_REQUIRE_GPU=1 \
  scripts/quiet.sh target/debug/deps/kjerag_render-292a94dc388251e5 \
  one_xs_belt_gpu::tests::submission_lease --nocapture --test-threads=1
```

Exit code `0`: **8 passed, 0 failed, 668 filtered out**. The raw log SHA-256
is `0eebd67f35bbe30ccff493020f35ffb077cf9c874494b86760d10862adb0e5cb`.

## Rejected earlier receipt

The following `9ce5bc1` run is retained only as the audit trail. Its tests did
not exercise callback-registration panic with an already-pending owner, and
the audit rejected that code for the lifetime bug described above. It must not
be read as current qualification.

### Exact code

- Code commit: `9ce5bc1c28c581bcb3e7b7f8a9ab0073b30cc00b`
- Tree: `26a679d3d4806aeca14c9562ae5bbd909bac469a`
- Parents: `614e1ee30c8952d36f1443b4189c6d840e4d88ed` and
  integration fix `0fc69fc6c0df478821537448512820ca2487a1d5`
- Author and committer: `Alex Harding <noreply@harding.dev>`
- `git status --porcelain=v1`: empty before the build and after both runs

Relevant source SHA-256 values at that commit:

```text
834eb0e3e8c7d245f9980c7c08cb4726eb62a3e52afcb70013a5bbfaa9de8c49  crates/render/src/flow/one_xs_belt_gpu.rs
155b0603c2bdbf8b2aa52e1725c78a55bea5c71bcc05d7b8a42056f14eab9376  crates/render/src/flow/one_xs/pis_frontend_gpu.rs
94ad0e456515aa9202f06387557fc285b61ccc350d76a004ba60d61301de08e7  crates/render/src/scene.rs
```

The exact build command was:

```sh
MESA_VK_DEVICE_SELECT=1002:15bf! WGPU_BACKEND=vulkan \
  scripts/quiet.sh cargo test -p kjerag-render \
  one_xs_belt_gpu::tests::retirement --no-run
```

It exited zero and produced
`target/debug/deps/kjerag_render-292a94dc388251e5`. Its SHA-256 was
`3fc307810f92c5c82a60cfd87eda18f3e61e184877483366293740921f1a6981`
immediately before the runs and the same immediately after them. The binary
was created at `2026-09-01 07:08:24.891050477 -0500`, last modified at
`2026-09-01 07:08:34.489971812 -0500`, and was 516,470,016 bytes.

## Common device and driver binding

- PCI device: AMD Phoenix1 `[1002:15bf]`, revision `cb`, at `c1:00.0`
- Subsystem: Framework Computer Inc. `[f111:0006]`
- Kernel driver: `/sys/bus/pci/drivers/amdgpu`
- Vulkan implementation: `mesa-vulkan-drivers
  26.1.6-1pop0~1787580452~24.04~a5619ea`
- Vulkan loader: `libvulkan1 1.3.280.0-1pop1~1722439676~24.04~a41a7d6`
- Radeon ICD manifest:
  `/usr/share/vulkan/icd.d/radeon_icd.json`, SHA-256
  `bca32a660ca32a42b598d53cdc63b3e3e917ea40c0b3b1e00fdbd3115280b51d`
- RADV library: `/usr/lib/x86_64-linux-gnu/libvulkan_radeon.so`, SHA-256
  `7bf31db67cbf803c7cbd8bbe9280f2ba0cb77d8840529af640dc5978a14e589d`

Both commands forced `MESA_VK_DEVICE_SELECT=1002:15bf!`, restricted Vulkan to
that Radeon ICD with `VK_DRIVER_FILES`, required a real GPU instead of a skip
with `KJERAG_REQUIRE_GPU=1`, selected wgpu's Vulkan backend, and ran through
the repository's quiet wrapper.

## Rejected earlier exact runs

Retirement, start `2026-09-01 07:09:29-05:00`, end
`2026-09-01 07:09:30-05:00`:

```sh
env MESA_VK_DEVICE_SELECT=1002:15bf! \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  WGPU_BACKEND=vulkan KJERAG_REQUIRE_GPU=1 \
  scripts/quiet.sh target/debug/deps/kjerag_render-292a94dc388251e5 \
  one_xs_belt_gpu::tests::retirement --nocapture --test-threads=1
```

Exit code `0`: **5 passed, 0 failed, 671 filtered out**. This covers cloned
and foreign contexts on real Vulkan, ordered and out-of-order proof, exact-once
release under repeated polling, cancellation and full refusal, poll error and
panic quarantine, and ABA generation reuse. Raw log SHA-256:
`26c7ad62cd594cbf62efa956dcd0dd8f0f5057db999dbdd386bec40aa8265508`.

SubmissionLease paths, start `2026-09-01 07:09:39-05:00`, end
`2026-09-01 07:09:40-05:00`:

```sh
env MESA_VK_DEVICE_SELECT=1002:15bf! \
  VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json \
  WGPU_BACKEND=vulkan KJERAG_REQUIRE_GPU=1 \
  scripts/quiet.sh target/debug/deps/kjerag_render-292a94dc388251e5 \
  one_xs_belt_gpu::tests::submission_lease --nocapture --test-threads=1
```

Exit code `0`: **8 passed, 0 failed, 668 filtered out**. This includes the
module-path-derived isolated double-unwind helper, exact-once success, explicit
and Drop panic quarantine, poll failure quarantine, cancellation wait, and
cloned/foreign pre-encode provenance. Raw log SHA-256:
`36a44db91c7ca1be0798b0f8261b7b475b8f967911928afc964287b3f43ac659`.

The visible panic lines in both raw logs are deliberate injected-failure
cases; their enclosing tests passed after proving quarantine and panic
preservation.
