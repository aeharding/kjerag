# GPU pipeline research: dual-fisheye `.insv` playback on Linux/AMD

**Status:** research complete, pre-implementation. **Date:** 2026-07-30.
**Scope:** feasibility and stack selection for a native COSMIC-desktop, high-performance
Insta360 `.insv` player: simultaneous decode of two 3840x3840 HEVC streams, GPU
reprojection of dual fisheye into a user-controlled perspective view, frame-accurate
pause/seek, screenshot, and clip export.

Everything marked **[measured]** was run on the target hardware described in §1. Claims
sourced from code or documentation carry a URL. Anything unverified is labelled
**UNKNOWN** rather than guessed.

---

## TL;DR

**Recommended stack**

> `ffmpeg-next` / `ffmpeg-sys-next` 8.1 demuxing one MP4 into two VA-API HEVC decoders,
> frames delivered via `av_hwframe_map` -> `AV_PIX_FMT_DRM_PRIME` -> per-plane
> `texture_from_dmabuf_fd` -> `create_texture_from_hal`, reprojected by a custom WGSL
> fragment shader implementing the MEI lens model read from the `.insv` trailer, hosted
> in either libcosmic + `iced::widget::shader` or winit + wgpu 30.

**Decode is not the problem.** Two 3840x3840 HEVC streams decode concurrently at
**2.40x realtime, sustained, at 17% of one CPU core**. There is 2.4x headroom.

**The problem is frame delivery into wgpu**, and it is a known-recipe problem with three
reference implementations, not open research.

**Effort:** ~22-36 person-days for an MVP (open file, play, drag-reframe, zoom,
screenshot).

---

## 1. Test bench

### Hardware

AMD Ryzen 5 7640U / **Radeon 760M** (`Phoenix1 [1002:15bf]`, gfx1103, GFX11/RDNA3,
**8 CU**), **VCN 4.0**, LPDDR5 unified memory.

> Note: this is a 760M, **not** the 780M often assumed for "Phoenix". 8 CUs, not 12.
> Shader headroom estimates should use the smaller part.

Vulkan heaps as reported by RADV: heap0 5402 MB device-local, heap1 10 GB host-visible.

### Software

| component | version |
|---|---|
| OS | Pop!_OS 24.04 (noble), `XDG_CURRENT_DESKTOP=COSMIC` |
| kernel | 7.0.11-76070011-generic |
| Mesa | 25.2.8 (radeonsi, LLVM 20.1.2), DRM 3.64 |
| libva | 2.20.0 (`vainfo`/libva-utils **not installed**) |
| FFmpeg | 6.1.1 (system) |
| Vulkan driver | RADV PHOENIX, API 1.4.318 |

FFmpeg `-hwaccels`: `vdpau cuda vaapi qsv drm opencl vulkan`.
VA-API encoders present: `av1_vaapi h264_vaapi hevc_vaapi mjpeg_vaapi vp8_vaapi vp9_vaapi`.

### Test asset

A 30-minute Insta360 X-series `.insv` recording, ~37.9 GB, used unmodified. `ffprobe`:

| stream | codec | resolution | pix_fmt | rate | bitrate | notes |
|---|---|---|---|---|---|---|
| 0 | HEVC Main, L6.1 (`level=183`) | 3840x3840 | `yuvj420p` (**full range**) | 30000/1001 | 89.9 Mbps | `has_b_frames=0`, `refs=1`, tag `hvc1` |
| 1 | HEVC Main, L6.1 | 3840x3840 | `yuvj420p` | 30000/1001 | 78.2 Mbps | same |
| 2 | AAC-LC | - | - | 48 kHz stereo | 189 kbps | - |

Container total **168.6 Mbps**, duration 1799.798 s, 53940 frames per video stream.

Two properties matter enormously and are worth stating up front:

- **`has_b_frames=0`, `refs=1`** - IPPP with a single reference. Best possible case for
  decode latency and decoder memory. Also means **no frame is skippable** during
  seek-forward (see §7).
- **`yuvj420p`** - full-range, not limited-range. Any YUV->RGB matrix hardcoded for
  limited range (as `iced_video_player` does) will crush blacks and clip whites.

---

## 2. Decode feasibility: **yes, 2.4x headroom, measured**

All runs against the real asset, `-f null -`, 60 s of video unless noted.

| test | speed | fps | CPU | wall |
|---|---|---|---|---|
| SW decode, 1 stream, `-threads 0` | **0.896x** | 27 | 248% | 67.1 s |
| VA-API, stream 0, GPU-resident | **4.62x** | 139 | 17% | 13.2 s |
| VA-API, stream 1, GPU-resident | **4.86x** | 146 | 17% | 12.5 s |
| **VA-API, both streams, one process, GPU-resident** | **2.39-2.41x** | 72+72 | **17%** | 25.3 s |
| VA-API, both + `hwdownload,format=nv12` | **2.09x** | 63+63 | 99% | 28.9 s |
| VA-API, both, **sustained 300 s** | **2.40x** | 72+72 | 15% | 125.0 s |
| Two **separate processes**, 1 stream each | 2.39x / 2.40x | 72 each | - | - |

Command shape for the dual-stream case:

```sh
ffmpeg -hwaccel vaapi -hwaccel_device /dev/dri/renderD128 \
       -hwaccel_output_format vaapi -t 60 -i CLIP.insv \
       -map 0:0 -f null - -map 0:1 -f null -
```

**Findings**

1. **Software decode is not viable.** One stream alone runs at 0.896x realtime across all
   cores. Hardware decode is a requirement, not an optimization.
2. Aggregate VCN throughput is **~142 frames/s at 3840x3840 (~2.1 Gpixel/s)**. The
   requirement is 59.94 frames/s aggregate, so **2.37x headroom**.
3. Single-stream (139/146 fps) and dual-stream (72+72 = 144 fps) totals are identical.
   This is direct evidence of **one VCN engine, cleanly time-shared** - consistent with
   Phoenix being a single-VCN part.
4. Two separate *processes* achieve the same aggregate as one process with two decoders.
   Kernel-side arbitration is fair and lossless, so the app may structure decoding as one
   or two decoder instances freely.
5. **No thermal or clock throttling over 300 s** (2.40x at t=300 s, identical to t=60 s).
   This is a 15 W laptop APU and it held.
6. HW decode costs **17% of one core** for both streams. Decode is effectively free on the
   CPU side; everything that follows is about moving pixels, not producing them.

**Confidence: very high.** Direct measurement on target hardware with the target asset.

### GPU shader headroom

Proxy measurement, since the real reprojection shader does not exist yet: decode ->
dmabuf -> Vulkan -> libplacebo scale to 2560x1440 -> readback -> null, one stream:
**2.49x realtime, 75 fps** **[measured]**.

libplacebo's default chain (full colour management plus a polar resampler) is
substantially heavier than a reprojection shader, which is ~50 ALU ops and two bilinear
fetches per output pixel. At 2560x1440x60 that is 221 Mpixel/s of gather from a 22 MB
texture - bandwidth-bound rather than ALU-bound, and well inside an LPDDR5 APU's budget.

**Confidence: medium-high.** The proxy is a genuine GPU shader pass on the real frames,
but it is not the actual shader. Re-measure once the real one exists.

---

## 3. Frame delivery into wgpu

This is the crux of the project. Three routes were investigated. One is recommended, one
is rejected with evidence, one is a proven fallback.

### 3.1 Recommended: VA-API -> DRM_PRIME -> per-plane `texture_from_dmabuf_fd`

```
VAAPI decode
  -> av_hwframe_map(dst.format = AV_PIX_FMT_DRM_PRIME, MAP_READ|MAP_DIRECT)
  -> AVDRMFrameDescriptor: per-layer { fd, offset, pitch, format_modifier }
  -> texture_from_dmabuf_fd(fd,  R8Unorm,  modifier, pitch[0], offset[0])   // luma
  -> texture_from_dmabuf_fd(dup, Rg8Unorm, modifier, pitch[1], offset[1])   // chroma
  -> create_texture_from_hal::<Vulkan>(.., initial_state)
  -> acquire from VK_QUEUE_FAMILY_EXTERNAL (raw ash; no-op on RADV)
  -> YUV -> RGB in WGSL
```

Why this one:

- It is what mpv's `gpu-next` and libplacebo both do - the only VAAPI->Vulkan path with
  real production mileage on AMD Mesa.
- It needs **nothing unreleased**. `ffmpeg-sys-next` 8.1 already binds `hwcontext_drm.h`,
  giving `AVDRMFrameDescriptor`, `av_hwframe_map` and `AV_PIX_FMT_DRM_PRIME`.
- FFmpeg exports `SEPARATE_LAYERS`, so NV12 arrives as exactly two single-plane layers -
  precisely the shape wgpu's single-plane import wants.
- It never materialises an `AVVkFrame`, so the fragile joint documented in §3.2 does not
  exist.

The VAAPI->dmabuf->Vulkan import **is verified working on this exact machine**
**[measured]**. `ffmpeg -vf hwmap=derive_device=vulkan` logs, for every frame at
3840x3840:

```
[AVHWFramesContext] Mapped DRM object to Vulkan!
```

The full chain (decode -> dmabuf -> VkImage -> readback -> null) ran at **3.25x realtime,
97 fps** for one stream. Extensions FFmpeg enabled to do it, all present on RADV PHOENIX:
`VK_EXT_image_drm_format_modifier`, `VK_EXT_physical_device_drm`,
`VK_KHR_external_memory_fd`, `VK_KHR_external_semaphore_fd`.

#### The "single-plane only" limitation is a non-issue

wgpu's `texture_from_dmabuf_fd` is documented single-plane, and
[wgpu#9801](https://github.com/gfx-rs/wgpu/issues/9801) (open, 2026-07-03) tracks
multi-plane support. **This does not block us.** Call the function **twice**, once per
plane, each with its own offset and pitch and the shared modifier. #9801 only matters if
you want a single NV12 wgpu texture, which we do not - two textures (`R8Unorm` +
`Rg8Unorm`) is exactly what a YUV->RGB fragment shader wants anyway.

#### Synchronisation

libva exposes **no fence or sync-fd export** - only `vaSyncSurface` / `vaSyncSurface2` /
`vaSyncBuffer`, all CPU waits. Two options:

1. Rely on `vaSyncSurface()` before export. This is what FFmpeg's own
   `vulkan_map_from_vaapi()` does, so it is the well-trodden path.
2. Export the dmabuf's implicit fence via `DMA_BUF_IOCTL_EXPORT_SYNC_FILE`, import as a
   binary `VK_KHR_external_semaphore_fd`, and feed it to
   `Queue::add_wait_semaphore(sem, None, stage)` (wgpu 30, PR
   [#9461](https://github.com/gfx-rs/wgpu/pull/9461)) to avoid a CPU stall.

Start with (1); move to (2) only if profiling shows the sync costing real time.

### 3.2 Rejected: FFmpeg `hwcontext_vulkan` / `AVVkFrame`

The intuitively attractive route is to hand wgpu's own `VkDevice` to FFmpeg via
`AVVulkanDeviceContext` and consume `AVVkFrame`s directly, with no import at all. FFmpeg
does support this - the header states *"All of these can be set before init to change what
the context uses"*, and user-settable fields include `inst`, `phys_dev`, `act_dev`,
`device_features`, `enabled_{inst,dev}_extensions`, and `qf[64]`/`nb_qf`
([hwcontext_vulkan.h](https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavutil/hwcontext_vulkan.h)).
mpv does exactly this for its `--hwdec=vulkan` (Vulkan Video) path.

**We reject it anyway.** Four blockers, in order of severity:

1. **Queue sharing is unsolved.** FFmpeg submits on its own queue guarded by
   `lock_queue`/`unlock_queue`. **wgpu exposes no queue-lock hook**, so the mutex cannot
   be shared. The workaround - create the device yourself with >=2 queues, let FFmpeg take
   index 0 and give wgpu index 1 via `device_from_raw` - is **UNVERIFIED** end to end.
2. **Silent segfault on misconfiguration.** `vulkan_frames_init()` dereferences
   `p->compute_qf->num` and `p->transfer_qf->num` with **no NULL check**
   (`hwcontext_vulkan.c:3083`). If `qf[]` omits a compute or transfer family you get a
   crash, not an error.
3. **No released Rust bindings.** `hwcontext_vulkan.h` is bound by no published crate.
   `ffmpeg-sys-next` added it on master 2026-07-01 (commit `369976d94`), version bumped to
   9.0.0, **unreleased**.
4. **Measured device loss [measured].** Feeding an FFmpeg-imported `AVVkFrame` to a second
   Vulkan consumer kills the GPU on this box:

   ```
   -vf 'hwmap=derive_device=vulkan,format=vulkan,libplacebo=...,hwdownload,format=nv12'
     [libplacebo] vkQueueSubmit2: VK_ERROR_DEVICE_LOST (../src/vulkan/command.c:504)
   ```

   Reproduced twice. The same chain ending in `hwdownload` instead of libplacebo: 60/60
   frames clean. libplacebo fed the VAAPI frame *directly* via its own dmabuf import:
   clean. Stack was FFmpeg 6.1.1 / libplacebo 6.338.2 / RADV PHOENIX.
   **UNVERIFIED on FFmpeg 8.1 + libplacebo 7.360.** Plausible cause is
   `queue_family[i] = VK_QUEUE_FAMILY_EXTERNAL` left on imported frames versus
   libplacebo's `VK_QUEUE_FAMILY_IGNORED` acquire - roughly what FFmpeg commit `2e19e74a2`
   addressed - but **the link is unconfirmed**.

**Corroboration:** Jellyfin independently abandoned the direct hwmap. Its
`EncodingHelper.cs:5383` now labels `hwmap=derive_device=vulkan` from VAAPI as
`// legacy va-vk mapping that works only in jellyfin-ffmpeg6` and for >=7.0.1 routes
explicitly through DRM: `hwmap=derive_device=drm, format=drm_prime, hwmap=derive_device=vulkan, format=vulkan`.

**Reading of the evidence:** the *import* is solid on AMD Mesa. The fragile joint is
handing an imported `AVVkFrame` to a **second** Vulkan consumer. The DRM_PRIME route
never creates one, which is the main reason it is preferred.

Also noted: FFmpeg 8.0 is the version floor for the import-my-device case regardless -
commit `bd75fad85` (2025-06-26) fixed `p->mprops` never being populated for externally
created devices, so allocations failed on 7.1.

### 3.3 VA-API dmabuf export: the details that bite

#### Export API

`vaExportSurfaceHandle(VADisplay, VASurfaceID, mem_type, flags, descriptor)`.
The mem_type/struct pairing is commonly gotten backwards:

| constant | value | descriptor struct |
|---|---|---|
| `..._DRM_PRIME` (legacy) | `0x20000000` | `VASurfaceAttribExternalBuffers` |
| `..._DRM_PRIME_2` | `0x40000000` | **`VADRMPRIMESurfaceDescriptor`** |
| `..._DRM_PRIME_3` | `0x08000000` | `VADRMPRIME3SurfaceDescriptor` |

**Legacy PRIME does not work on Mesa at all** - `vlVaExportSurfaceHandle` rejects anything
that is not PRIME_2/PRIME_3 and returns `VA_STATUS_ERROR_UNSUPPORTED_MEMORY_TYPE` (36)
**[measured]**. Legacy also has no modifier field; FFmpeg's legacy path comments
`// There is no way to get the format modifier with this API` and hardcodes
`DRM_FORMAT_MOD_INVALID`.

PRIME_2 landed in libva 2.1.0 / VA-API 1.1.0 (2018-02-12), so the floor is ancient and
irrelevant in practice.

#### SEPARATE vs COMPOSED layers

- **SEPARATE** (`0x0004`) -> `num_layers == 2`: layer 0 `DRM_FORMAT_R8` (1 plane),
  layer 1 `DRM_FORMAT_GR88` (1 plane). Note **GR88, not RG88** - the other byte order is
  not what you get.
- **COMPOSED** (`0x0008`) -> `num_layers == 1`, `drm_format = DRM_FORMAT_NV12`,
  `num_planes = 2`.
- Mesa only special-cases COMPOSED; **no flag set falls through to SEPARATE**.

FFmpeg and Chromium always use SEPARATE. mpv uses COMPOSED for Wayland interop and
SEPARATE for GL/libplacebo. We want SEPARATE, and FFmpeg gives it to us by default.

#### The 3840-wide chroma pitch trap - **applies directly to this project**

**[measured]** on gfx1103, requesting NV12 surfaces:

| request | object size | Y pitch | UV pitch | UV offset |
|---|---|---|---|---|
| **3840x2160** | 13565952 | **3840** | **4096** | 8847360 |
| 4096x2160 | 14155776 | 4096 | 4096 | 9437184 |
| 2560x1440 | 5898240 | 2560 | 2560 | 3932160 |
| 1920x1080 | 3932160 | 2048 | 2048 | 2621440 |
| 1280x720 | 1572864 | 1280 | 1536 | 983040 |
| 3840x2160 P010 | 25559040 | 7680 (R16) | 7680 (GR1616) | 16711680 |

The rule on GFX11 `64K_R_X`: **Y pitch = `align(width, 256)` bytes, UV pitch =
`align(width, 512)` bytes** (chroma is 2 bytes/element and aligns to 256 elements).

Our streams are **3840 wide**. `align(3840, 256) = 3840` but `align(3840, 512) = 4096`.

> **The trap:** at 3840, `pitch == width` "looks right" for luma and is **wrong for
> chroma**. Code that derives chroma pitch from luma pitch, or computes
> `uv_offset = y_pitch * height`, **shears chroma at 3840 while working perfectly at 1920
> and 2560**. It is a bug that only appears on real footage.
>
> Note also that UV offset is **not** `y_pitch * height`: at 3840x2160 it is
> 3840 x 2304, with height padded 2160 -> 2304.
>
> **Rule: never compute pitch or offset. Always use `layers[].pitch[]` and
> `layers[].offset[]` verbatim.**

Height padding for 3840x**3840** specifically was **not measured** - the table above is
3840x2160. Read the descriptor; do not extrapolate.

#### One object, not two

**[measured]** radeonsi always exports **1 object (one fd)**, never 2.
`vl_video_buffer_create_as_resource()` sets `contiguous_planes = true` unconditionally,
and later planes point `object_index` back at object 0. Consequences:

- You must **`dup()` the fd** for per-plane use.
- The caller **owns and must `close()`** every returned fd. Each export call returns a
  fresh fd.
- Closing fds does not destroy the surface; the dmabuf holds a BO reference, so the memory
  outlives `vaDestroySurfaces`.

#### Modifiers and DCC

**[measured]** GFX11 decode surface modifier: `0x0200000010401b04` = `AMD_FMT_MOD`,
`TILE_VERSION=4` (GFX11), `TILE=27` (`AMD_FMT_MOD_TILE_GFX9_64K_R_X`), **`DCC=0`**,
`PIPE_XOR_BITS=2`, `PACKERS=2`. Tiled, no DCC.

Generation gating (`ac_modifier_supports_video()` in `ac_surface.c`):

| generation | video modifier support |
|---|---|
| GFX6-GFX8 (Polaris/Fiji) | **`DRM_FORMAT_MOD_INVALID`** - unsupported ([mesa#11074](https://gitlab.freedesktop.org/mesa/mesa/-/issues/11074)); a GFX6-8 patch was posted May 2026 but is **not merged** |
| VCN 1.0 (Raven/Picasso) | **LINEAR only** |
| VCN 2.0/2.2 (Navi1x, Renoir) | tiled, but `GFX9_64K_S` only |
| VCN 3.0+ (RDNA2/3, GFX10.3/GFX11) | `_R_X` modifiers allowed |
| DCC on video output | **no** on GFX9-GFX11; **yes** on GFX12/RDNA4 (Mesa 25.1+) |

The VCN 2.2 restriction was a real bug fix -
[mesa#14032](https://gitlab.freedesktop.org/mesa/mesa/-/issues/14032) ("some video files
are not shown in mpv when using vaapi hw decoding on amd apu"), fixed 2025-10-15. Blank
video is the symptom of the engine writing a swizzle the modifier lied about.

Escape hatches: `AMD_DEBUG=novideotiling` forces linear; passing
`VASurfaceAttribDRMFormatModifiers = DRM_FORMAT_MOD_LINEAR` at `vaCreateSurfaces()` yields
exactly-packed linear (3840x2160 -> Y pitch 3840 offset 0, UV pitch 3840 offset 8294400,
zero height padding) **[measured]**.

#### Other gotchas, each with a source

- **Sync ordering.** `va.h` is explicit that export performs no synchronisation; call
  `vaSyncSurface()` first if you will read. Mesa also flips submission mode the first time
  any handle escapes (`vlVaSurfaceFlush` uses
  `drv->has_external_handles ? 0 : PIPE_FLUSH_ASYNC`, commit `7ed38749961c`).
- **Surface recycling - the classic AMD corruption.**
  [mesa#8996](https://gitlab.freedesktop.org/mesa/mesa/-/issues/8996): *"VA-API video
  output is corrupted if decoded surfaces are exported by vaExportSurfaceHandle and then
  quickly returned to ffmpeg/va-api decoder and reused... If vaExportSurfaceHandle() is
  not called the bug doesn't occur."* Fixed in **Mesa 23.1.1**; Firefox still hard-blocks
  VA-API on AMD below that version. We are on 25.2.8, so this is historical - but the
  durable lesson stands: **a VA surface is a pool slot and the exported dmabuf aliases
  it.** Hold the frame/surface reference for the entire lifetime of the imported image.
- **Interlaced surfaces cannot be exported** (`VA_STATUS_ERROR_INVALID_SURFACE`).
  Irrelevant here, and interlaced support was removed from radeonsi in Mesa 25.3 anyway.
- **`objects[].size` is `uint32_t`.** Current radeonsi sets it correctly and it is nonzero
  **[measured]**, but mpv and GStreamer both carry an `lseek(fd, 0, SEEK_END)` fallback
  taking the max. Worth keeping for robustness on older stacks.
- radeonsi marks all **YUV** modifiers `external_only`, so a GL importer of a COMPOSED
  NV12 needs `GL_TEXTURE_EXTERNAL_OES`. The SEPARATE R8/GR88 layers are *not*
  external-only. Irrelevant on the Vulkan path.

### 3.4 wgpu 28 vs 30 - the sharpest decision in the project

Three APIs this pipeline wants all landed in **wgpu 30.0.0 (2026-07-01)**:

| API | PR | what it does |
|---|---|---|
| `vulkan::Device::texture_from_dmabuf_fd` | [#9366](https://github.com/gfx-rs/wgpu/pull/9366) (merged 2026-04-09) | the import itself |
| `create_texture_from_hal(.., initial_state)` | [#9496](https://github.com/gfx-rs/wgpu/pull/9496) | declare the incoming image layout |
| `Queue::add_wait_semaphore(sem, Option<u64>, stage)` | [#9461](https://github.com/gfx-rs/wgpu/pull/9461) | wait on an external producer without a CPU block |

> Changelog erratum: the CHANGELOG attributes the dmabuf import to #9412, which is
> actually the SHADER_I16 PR. **Cite #9366.**

New feature bits in `wgpu-types`: `VULKAN_EXTERNAL_MEMORY_FD` (1<<35) and
`VULKAN_EXTERNAL_MEMORY_DMA_BUF` (1<<63), the latter set iff the adapter supports all of
`VK_KHR_external_memory_fd`, `VK_EXT_external_memory_dma_buf` and
`VK_EXT_image_drm_format_modifier`. Critically, **wgpu-hal 30 enables all three
unconditionally when supported** - the feature bits are for reporting and gating only, so
a plain `request_device` yields a usable device.

**This is new in 30.** Before it, `VK_EXT_image_drm_format_modifier` was *not* enabled,
which is why `ez-ffmpeg` (targeting wgpu 26) must go through
`hal_adapter.open_with_callback(|args| args.extensions.extend_from_slice(&EXTENSIONS))` +
`create_device_from_hal`.

**libcosmic pins wgpu 28.** So:

| | libcosmic (wgpu 28) | winit + wgpu 30 |
|---|---|---|
| import | hand-rolled raw `ash`, ~120 lines unsafe, plus force-enabling `VK_EXT_image_drm_format_modifier` via `open_with_callback` | two calls to `texture_from_dmabuf_fd` |
| `initial_state` on `create_texture_from_hal` | **UNKNOWN** whether present on 28 | present |
| external semaphore wait | **UNKNOWN** on 28 | `add_wait_semaphore` |
| window chrome | native COSMIC | Adwaita titlebar (see §5) |

**Resolve this in week 1.** It trades UI nativeness against roughly 3 days of unsafe
interop work and a real correctness surface.

#### The layout-discard hazard, and why it is benign here

PR #9496 exists because wgpu records `TextureUses::UNINITIALIZED` for hal-imported
textures, emitting `vkCmdPipelineBarrier(oldLayout = UNDEFINED)` on first use. Per spec,
`UNDEFINED` permits the driver to discard contents - and for images with vendor
compression (AFBC on Mali, UBWC on Adreno, DCC on AMD) the driver resets compression
metadata and **sampling reads garbage**.

**On RADV pre-GFX12 this cannot bite us**, for two independently sufficient reasons:

1. RADV **blocks DCC for multi-planar formats below GFX12**
   (`radv_get_modifier_flags`: *"We don't enable DCC for multi-planar formats before
   GFX12"*), and we measured `DCC=0` on the actual decode surface.
2. RADV **no-ops the acquire barrier from `VK_QUEUE_FAMILY_EXTERNAL`** entirely
   (`radv_cmd_buffer.c:15622` early-returns when
   `src_family_index == VK_QUEUE_FAMILY_EXTERNAL`). Zero decompression or retile work.

On GFX12/RDNA4 with DCC video modifiers this becomes live: **UNKNOWN**, untested, no
hardware available.

`TextureUses` -> layout mapping: `RESOURCE` -> `SHADER_READ_ONLY_OPTIMAL`,
`UNINITIALIZED` -> `UNDEFINED`, anything composite -> `GENERAL`. So pass a composite
(e.g. `COPY_SRC | RESOURCE`) to declare `GENERAL`, then perform the real
`VK_QUEUE_FAMILY_EXTERNAL` acquire yourself. Until PR
[#9668](https://github.com/gfx-rs/wgpu/pull/9668) (queue-family ownership transfer in
`hal::TextureBarrier`, **unmerged** as of 2026-06-13) lands, that acquire is raw ash via
`CommandEncoder::as_hal`.

#### Why not RADV's ycbcr conversion?

wgpu **deliberately never enables `samplerYcbcrConversion`** - the enable is commented out
in `wgpu-hal/src/vulkan/adapter.rs:420-427`. The extension's presence is used only to gate
reporting of `Features::TEXTURE_FORMAT_NV12` / `TEXTURE_FORMAT_P010`. You therefore cannot
create a `VkSamplerYcbcrConversion` on a wgpu device. Sampling an NV12 wgpu texture
directly panics outright (`wgpu-core/src/validation.rs:982`).

`Features::EXTERNAL_TEXTURE` (WebGPU `GPUExternalTexture`, WGSL `texture_external`, added
in wgpu 27) would be the ideal API and is even plane-shaped internally - but it is
**DX12 and Metal only**; `wgpu-hal/src/vulkan/conv.rs:848` is literally
`wgt::BindingType::ExternalTexture => unimplemented!()`, with no open PR for Vulkan.
`importExternalTexture` is browser-only.

This is why **every** real implementation does per-plane import plus WGSL colour
conversion. It is not a workaround; it is the supported path.

#### RADV also rules out the disjoint-image design

`radv_formats.c:662`: `/* Unconditionally disable DISJOINT support for modifiers for now */`.
Multi-object (one BO per plane) import is not supported on RADV. Harmless for us -
radeonsi always exports one BO - but it kills that design outright.

### 3.5 Fallback: the CPU copy path (measured, and better than it looks)

**The catastrophic write-combined readback scenario cannot occur on radeonsi decode
surfaces.** Mesa structurally prevents it by interposing a GPU blit into *cached* GTT.

`vlVaDeriveImage` succeeds on radeonsi - it fails only for interlaced surfaces and for
multi-plane without contiguous-planes support, and radeonsi reports contiguous planes
always. **[measured]** FFmpeg logs `Direct mapping possible.` The staging path is
`PIPE_USAGE_STAGING` -> `RADEON_DOMAIN_GTT` **without** `RADEON_FLAG_GTT_WC`, i.e.
**cached, snooped system memory**, per `si_buffer.c:39`. `si_texture.c:2147` states the
intent verbatim: *"Reading from VRAM or GTT WC is slow, always use the staging texture in
this case."*

**[measured] proof it is cached:** reading the derived mapping cost 4.8 ms/frame at 4K
versus 3.0 ms/frame for a cache-warm normal `AVFrame` - a ratio of 1.6x, exactly
cold-DRAM versus L3-warm. Write-combined would show 20-50x.

Historical floor: **Mesa >= 23.3**. Before commit `c638e61e` (2023-11-03) derived decode
surfaces were mapped write-only and reads returned garbage.

#### FFmpeg takes the slow path by default

`hwcontext_vaapi.c:875` gates `vaDeriveImage` behind
`(flags & AV_HWFRAME_MAP_DIRECT) || !(flags & AV_HWFRAME_MAP_READ)`, with the comment
explaining it is an **Intel Gen7-Gen9 heuristic** (memory mappable but uncached there).
`hwdownload` passes only `AV_HWFRAME_MAP_READ`, so it **always takes `vaGetImage`**. On
AMD this costs you. Opt in with `-vf hwmap=mode=read+direct`, or call `vaDeriveImage`
yourself.

> Our own dual-stream `hwdownload` measurement of **2.09x** in §2 was therefore on the
> **slow** path. The derive path is faster.

#### Measured costs, 360 frames, 3-run medians

**4K (3840x2160 NV12, 12.44 MB/frame):**

| config | wall | fps |
|---|---|---|
| decode only | 0.920 s | 391 |
| + `hwmap=mode=read+direct` (derive, no pixel read) | 1.11 s | 324 |
| + derive **+ full pixel read** | 1.27 s | 283 |
| + `hwdownload` (vaGetImage) | 1.85 s | 195 |
| + `hwdownload` **+ full pixel read** | 2.00 s | 180 |

**1080p:** decode only 0.295 s / 1220 fps; + `hwdownload` 0.50 s / 720 fps.

Marginal download cost: **4K 2.58 ms/frame, 1080p 0.57 ms/frame** (~4.8-5.4 GB/s
effective). **Derive saves ~2.0 ms/frame at 4K over getImage.**

#### Per-frame budget at 60 fps (16.67 ms)

| | 1080p60 | 4K60 |
|---|---|---|
| VA decode | 0.82 ms | 2.56 ms |
| VA -> CPU, **derive** | ~0.15 ms | **0.53 ms** |
| VA -> CPU, getImage | 0.57 ms | 2.58 ms |
| CPU -> wgpu staging (1 memcpy) | ~0.2 ms | ~0.75 ms |
| `copy_buffer_to_texture` (UMA) | small, UNMEASURED | small, UNMEASURED |
| **total, derive** | **~1.2 ms** | **~3.8 ms** |
| **total, getImage** | **~1.6 ms** | **~5.9 ms** |

Three caveats that matter more than the numbers:

1. **The map is a synchronous GPU stall.** `amdgpu_bo_map` flushes the command stream and
   waits `OS_TIMEOUT_INFINITE`. Pipeline 2-3 frames deep or the rate collapses toward the
   *sum* of stages rather than the max.
2. **Use derive, not getImage.**
3. **Do not use naive `write_texture`.** It creates a brand-new `hal::Buffer`, maps it,
   memcpys, and releases it after the next submit - an allocation plus map **per call**,
   so 120/s at 60 fps if Y and UV go separately. Use
   **`Queue::write_buffer_with()`** (documented to *"skip one allocation and one copy"* -
   write straight from the VA mapped pointer) or **`util::StagingBelt`**.

Alignment on RADV is a non-issue (`optimalBufferCopyOffsetAlignment = 1`,
`optimalBufferCopyRowPitchAlignment = 1`), **but** the VA-API pitch is padded, so passing
it verbatim drops you into a row-by-row loop. See the pitch trap in §3.3.

RADV on an APU exposes multiple GB of `DEVICE_LOCAL | HOST_VISIBLE | HOST_COHERENT`
(a 2/3 : 1/3 gtt/visible-vram split), write-combined - ideal for streaming uploads,
terrible for reads. The "256 MB on APU" figure describes Windows/AMDVLK, **not RADV on
Linux**.

**Verdict: the copy path is a legitimate bring-up strategy.** Ship it first, it works, it
costs ~1 core; convert to zero-copy as an optimization with a known-good reference to diff
against.

### 3.6 Reference implementations

| project | approach | URL |
|---|---|---|
| **`ez-ffmpeg`** 0.17.0 (2026-07-29, on crates.io) | **Closest to what we want.** Two single-plane `VkImage`s (`R8_UNORM`, `R8G8_UNORM`) over one dmabuf, fd `dup`'d, per-plane layouts. Mandatory cached `vkGetPhysicalDeviceImageFormatProperties2` pre-flight. Graceful `av_hwframe_transfer_data` fallback. ~600 lines, directly liftable. | https://github.com/YeautyYE/ez-ffmpeg |
| **`iroh-live`** | Most complete. Non-disjoint multi-plane import then `vkCmdCopyImage` per plane. Does the `VK_QUEUE_FAMILY_EXTERNAL` acquire properly. Has a VAAPI VPP re-tile fallback for non-importable modifiers - nobody else does. | https://github.com/n0-computer/iroh-live |
| **`bevy-dmabuf`** 0.2.0 | Minimal worked `create_texture_from_hal::<Vulkan>` example, including building a VkDevice with extra extensions and handing it to wgpu. | https://github.com/Schmarni-Dev/bevy-dmabuf |
| **Firefox** | `ExternalTextureDMABuf`: raw ash + explicit modifier layouts + exportable OPAQUE_FD semaphores. Shipping. | `gfx/wgpu_bindings/src/server.rs` |
| **mpv** | `dmabuf_interop_pl.c` (138 lines) - exported fd straight to `pl_tex_create(PL_HANDLE_DMA_BUF)`. The production VAAPI->Vulkan proof on AMD. | https://github.com/mpv-player/mpv |
| `ffgpu` | Weakest - imports each fd as a `VkBuffer` then copies. Its README's rationale (*"wgpu does not have any way to request `VK_EXT_image_drm_format_modifier`"*) is **now out of date** as of wgpu 30. | https://github.com/jazzfool/ffgpu |

**No crate packages VA-API decode -> wgpu texture end to end.** `ez-ffmpeg` is the closest
thing, and it buries the capability inside a filter framework.

`gpu-video` 0.4.0 (ex-`vk-video`, Software Mansion, part of
https://github.com/software-mansion/smelter) is the only *library* in this space and is
**ruled out**: it is Vulkan Video, **H.264 decode only** (HEVC "planned"). Our streams are
HEVC. It also insists on creating the `VkDevice` itself.

---

## 4. Rust decode API selection

### Versions (crates.io, 2026-07-30)

| crate | max stable | updated | downloads |
|---|---|---|---|
| `ffmpeg-next` / `ffmpeg-sys-next` | **8.1.0** | 2026-03-18 | 6.07M / 6.35M |
| `gstreamer` / `gstreamer-video` | 0.25.3 | 2026-06-29 | 9.26M / 6.78M |
| `wgpu` | **30.0.0** | 2026-07-02 | 29.6M |
| `iced` | 0.14.0 | 2025-12-07 | 2.42M |
| `ash` | 0.38.0+1.3.281 | 2024-04-01 | 30.6M |
| `libmpv2` | 6.0.0 | 2026-05-12 | 58.8k |
| `rsmpeg` | 0.18.0+ffmpeg.8.0 | 2025-08-24 | 155k |
| `iced_video_player` | 0.6.0 | 2025-12-14 | 13.4k |
| `cros-libva` | 0.0.13 | 2024-12-06 | 779k |
| `drm-fourcc` | 2.2.0 | **2021-09-05** | 7.2M |
| `libcosmic` | *(git only, not on crates.io)* | - | - |

### Choice: `ffmpeg-next` / `ffmpeg-sys-next`

1. **Two video streams from one demuxer with shared timestamps** is a natural
   `AVFormatContext` plus two `AVCodecContext`s. GStreamer would need `qtdemux` with two
   src pads into two decoders plus manual appsink sync - more machinery, less clock
   control.
2. **Frame-accurate pause, step and seek** is exactly what `av_seek_frame` plus manual
   `avcodec_send_packet` / `avcodec_receive_frame` gives. GStreamer's playbin fights you
   on frame accuracy.
3. `ffmpeg-sys-next` 8.1 **already binds `hwcontext_drm.h`**, which is all the recommended
   route needs.

Practical notes:

- The safe `ffmpeg-next` wrapper exposes **zero** hw-frame API; drop to
  `ffmpeg_next::sys::*` for `av_hwframe_map` and `AVDRMFrameDescriptor`.
- **`rsmpeg` cannot do this without patching** - `hwcontext_drm.h` is commented out of its
  header whitelist and it has no `av_hwframe_map`.
- **Build friction:** `ffmpeg-next` 8.1 wants FFmpeg 8.1 headers; Pop!_OS 24.04 ships
  6.1.1. Expect to vendor or build FFmpeg. Budget it on day one.
- `drm-fourcc`'s **published** version predates `AMD_FMT_MOD` and AMD's parameterised
  modifiers are un-enumerable anyway (always `Unrecognized(u64)`). Use it for fourcc names
  only; pass the raw `u64` modifier to Vulkan.

### Rejected: GStreamer

Three verified negatives:

1. **No `gst-plugin-wgpu` and no wgpu sink anywhere in gst-plugins-rs.** `wgpu` appears in
   its `Cargo.lock` only transitively via `cubecl-wgpu`.
2. **`vulkanupload` does not accept VAMemory or DMABuf.** Its `upload_methods[]`
   (`ext/vulkan/vkupload.c:949`) is buffer/raw -> buffer/raw -> image/buffer -> image plus
   Android AHB. `VK_EXT_external_memory_dma_buf` appears exactly once in the whole
   GStreamer repo, in an unrelated AMF header. **`vapostproc ! vulkanupload` round-trips
   through system memory.**
3. **`gstreamer-vulkan` 0.25.2 has zero dmabuf/DRM surface**, and no `gstreamer-va` crate
   exists.

GStreamer's only real zero-copy story is `gtk4paintablesink` (dmabuf direct on GTK >= 4.14) -
**GTK-only**, so useless for a COSMIC/iced or winit app.

### Rejected: libmpv

libmpv is a *player*, not a frame source. Its render API hands you a rendered FBO, not
decoded planes. That is the wrong shape for a custom reprojection shader - you would be
fighting it to get two raw fisheye planes out per lens.

---

## 5. UI shell

### cosmic-player: proof COSMIC-native video ships, but the wrong frame path to copy

Repo: https://github.com/pop-os/cosmic-player (GPL-3.0, a submodule of
https://github.com/pop-os/cosmic-epoch). COSMIC is **1.x released**, not alpha.

- **Decode stack:** GStreamer, via a fork of `iced_video_player`. The pipeline literal:

  ```
  playbin uri="{}" video-sink="videoscale ! videoconvert ! videoflip method=automatic !
    appsink name=iced_video drop=true caps=video/x-raw,format=NV12,pixel-aspect-ratio=1/1"
  ```

  `video/x-raw` is **system memory**. No `(memory:DMABuf)`, no `glupload`.
- **Frame delivery:** worker thread `try_pull_sample()` -> `map_readable()` -> **two
  `queue.write_texture` calls per frame** (`R8Unorm` luma + `Rg8Unorm` chroma), YUV->RGB
  in WGSL with hardcoded BT.709 **limited-range** coefficients. A full CPU round trip
  every frame.
- **VA-API is never explicitly wired.** No hits for `vaapi|hwdec|glupload|dmabuf` in its
  sources; it relies on `playbin` autoplugging, and the `video/x-raw` caps force a
  VA->system download anyway.

**Verdict: do not build on `iced_video_player`.** Two 3840x3840 streams through
`write_texture` is ~2.7 GB/s of pure overhead, plus the limited-range matrix is wrong for
our full-range `yuvj420p` source. Its own issue tracker asks for exactly what we are
building (an ffmpeg zero-copy backend "for high-performance video applications"), and
notes that 3840-wide video blows the 2048 texture limit on downlevel GPUs.

### iced does give you the GPU

`iced::widget::shader` (feature-gated on `wgpu`) exposes the raw device and queue through
the `Primitive` trait:

```rust
fn prepare(&self, pipeline: &mut Self::Pipeline, device: &Device, queue: &Queue,
           bounds: &Rectangle, viewport: &Viewport);
fn render(&self, pipeline: &Self::Pipeline, encoder: &mut CommandEncoder,
          target: &TextureView, clip_bounds: &Rectangle<u32>);
```

`Program::update` can request a redraw, so continuous playback is supported.
Example: https://github.com/iced-rs/iced/tree/master/examples/custom_shader

Caveats: libcosmic vendors iced as a git submodule (`pop-os/iced`) pinned to **wgpu 28**
(see §3.4), and **`wgpu` is not a default libcosmic feature** - the default is tiny-skia
software rendering, so you must opt in as cosmic-player does. **No COSMIC app is known to
ship the shader widget**, so its behaviour under cosmic-comp is **UNKNOWN** and worth a
one-day spike.

### winit + wgpu directly

winit's Wayland backend defaults to `sctk-adwaita` client-side decorations, so you get an
**Adwaita/GNOME titlebar on COSMIC** - the visible "not native" failure. cosmic-comp does
implement `zxdg_toplevel_decoration_v1`, but advertises `ServerSide` only when the window
`is_stack()`; otherwise `ClientSide`. Whether a plain winit window explicitly requesting
SSD gets COSMIC-styled chrome is **UNKNOWN** - the code path exists, no confirming report
found. There is **no standalone COSMIC titlebar crate**: COSMIC chrome means libcosmic or
reimplementation.

### Recommendation

Prefer **libcosmic + `iced::widget::shader`** for native chrome, contingent on the week-1
wgpu-28 spike (§3.4). Fall back to **winit + wgpu 30** if either the shader widget fails
under cosmic-comp or the wgpu-28 interop proves too costly - the clean two-call
`texture_from_dmabuf_fd` path is worth real consideration on its own.

---

## 6. Reprojection, capture, and export

### Prior art: none in Rust/wgpu

GitHub API searches across `equirectangular language:rust`, `fisheye language:rust`,
`vaapi wgpu`, `dmabuf wgpu`, `vulkan video rust wgpu player` return only toys (12, 3, 1
stars). **No Rust/wgpu 360 video viewer exists. No "vaapi to wgpu" crate exists.** This is
greenfield.

The math direction is the easy one, though: for each output pixel, compute the view ray,
rotate by yaw/pitch, select the hemisphere, project to fisheye UV, sample. **No model
inversion is required.**

### Lens model: MEI, and the calibration is in the file

`insv-stitch` (https://github.com/BenjaminHenriksson/insv-stitch, Python, CPU-only)
documents the actual model: **MEI (Mei-Rives) projection with xi = 2.0, 13 distortion
coefficients per lens**, plus per-lens intrinsics and extrinsics, stored as protobuf. It
reports PSNR 22.5-22.9 dB against Insta360 Studio at 7680x3840 - stitching fidelity is
genuinely hard, but for a *viewer* that mostly pans within one lens, seam quality only
matters near the seam.

**The trailer is confirmed present in real footage [measured].** The final 64 bytes decode
as `[u32 trailer_size][u32 version = 3][magic "8db42d694ccc418790edff439fe026bf"]`; in the
30-minute sample the trailer was ~77 MB, consistent with ~1 kHz IMU data over 1800 s.

Frame-type table, from `insvdump` (https://github.com/ke4ukz/insvdump, Python + protobuf,
Apache-2.0; derived from https://github.com/alex-plekhanov/insvtools, Java, Apache-2.0):

| code | name | contents |
|---|---|---|
| 0 | INDEX | frame index/offset table |
| 1 | **INFO** | camera metadata + **calibration** (protobuf) |
| 3 | GYRO | gyroscope |
| 4 | EXPOSURE | exposure / shutter |
| 6 | TIMELAPSE | timestamp mapping |
| 7 | GPS | location |
| 13 | MAGNETIC | magnetometer |
| 14 | EULER | orientation quaternions |
| 23 | POS | drone telemetry |

Metadata lives at end-of-file and is **read backwards**; both indexed and non-indexed
variants exist. **No Rust crate parses this** - budget a port of `insvdump`'s reader plus
the INFO `.proto`. GYRO and EULER additionally give free horizon-levelling later.

### Screenshot: trivial, two gotchas

`copy_texture_to_buffer` -> `map_async` -> PNG. The two standard traps:

1. **`bytes_per_row` must be a multiple of 256** (`COPY_BYTES_PER_ROW_ALIGNMENT`). For
   2560-wide RGBA8, 10240 is already aligned; for arbitrary window widths you must pad the
   buffer and strip padding row-by-row when writing the PNG.
2. **sRGB.** Read back a `*UnormSrgb` target and the bytes are already sRGB-encoded -
   write straight to PNG. Read back a linear `Rgba8Unorm` target and you must encode
   manually or the PNG comes out dark.

Also request `Limits::default()` explicitly: `max_texture_dimension_2d` is 8192 there
(fine for 3840), but `downlevel_defaults()` is 2048 and would fail.

### Clip export: both paths measured

**Lossless passthrough remux** (keep the raw dual-fisheye; output stays `.insv`-like).
30 s range, both video streams plus audio, `-c copy`: **0.54 s wall, 596 MB output**
**[measured]**. Effectively instant and bit-exact, and it preserves the ability to reframe
later.

Caveats: cuts land on keyframe boundaries (1 s granularity - fine, see §7), and **you must
copy the Insta360 trailer across yourself** or the clip loses calibration and IMU and
stops being reframable. That trailer surgery is the only real work in this path.

**Re-encode the reframed view** (bake pan/zoom into a normal flat video). Decode ->
`scale_vaapi 2560x1440` -> `hevc_vaapi` at 40 Mbps, 30 s: **3.29x realtime, 99 fps, 9.3 s
wall, 148 MB** **[measured]**. A 1-minute reframed export takes ~18 s.

In the real app the scale step is replaced by the reprojection shader writing into a
VA-API-importable surface; the encode side is unchanged and has ample headroom. AV1
encode is also available on VCN 4.0. **Offer both paths.**

---

## 7. Seeking a 37.9 GB file: a non-problem

**[measured]**

- **GOP = exactly 30 frames / 1.001 s.** Keyframes at 0.000, 1.001, 2.002, 3.003, ...
  (120 keyframes in 3597 packets over 120 s). I-frame average **1.12 MB**, max **1.79 MB**.
- Cold seek plus decode of one frame, **including full ffmpeg process startup and VA-API
  device creation**: 0.52 s at t=60 s, **0.33 s at t=900 s, 0.42 s at t=1500 s, 0.36 s at
  t=1790 s**.

**Seek cost is position-independent.** The MP4 index (`stco`/`stss`) is parsed up front,
so there is no linear scan and file size is irrelevant. In a long-lived app with the
demuxer open and decoders warm, subtract essentially all of that startup time.

**Worst-case scrub:** seek to previous keyframe, decode forward <=29 frames x 2 streams =
58 frames / 142 fps = **~0.41 s worst case, ~0.2 s average**.

**You cannot skip frames on the way.** `refs=1` with no B-frames means every P-frame is a
reference; there is no `AVDISCARD_NONREF` shortcut.

**Strategy for responsive scrubbing:** display the **keyframe immediately** (always <=1 s
away, a single decode), then decode forward to the exact frame and swap. Scrubbing feels
instant while staying frame-accurate on release. An offline thumbnail strip is a
nice-to-have, not a necessity - with a 1 s GOP, `-skip_frame nokey` extraction is very
fast.

**Confidence: very high.**

---

## 8. Effort estimate

MVP scope: open file, play, drag to reframe, scroll to zoom, screenshot.
One experienced Rust/graphics developer.

| area | days |
|---|---|
| FFmpeg build/pin, demux, dual VA-API decoders, playback clock, pause, seek | 6-9 |
| Frame -> GPU, **copy path first** (`hwmap=mode=read+direct` + `StagingBelt`) | 1-2 |
| Zero-copy conversion (DRM_PRIME, per-plane import, acquire barrier) | 3-6 |
| `.insv` trailer + protobuf calibration parsing (port from `insvdump`) | 2-3 |
| MEI reprojection shader, yaw/pitch/FOV controls, dual-lens blend | 4-6 |
| UI shell + shader widget wiring | 3-5 |
| Screenshot | 0.5 |
| Integration, debug, polish | 3-5 |
| **total** | **~22-36** |

Realistic split: **~20-25 days** to a usable MVP on the copy path with minimal chrome;
**~30-36** for zero-copy plus COSMIC-native chrome. Add ~3 days if libcosmic is chosen and
the ash import must be hand-rolled on wgpu 28.

Order of work matters: **build the copy path first**. It is measured to work, it unblocks
the shader and UI immediately, and it becomes the reference implementation to diff against
when zero-copy misbehaves.

---

## 9. Risks, confidences, unknowns

### Biggest performance risk

**Not decode** - that is settled with 2.4x measured, sustained headroom at 17% CPU.

The risk is **frame delivery into wgpu**, but it is a known-recipe problem with three
reference implementations rather than open research. The residual sharp edges:

1. **The 3840 chroma-pitch trap** (§3.3). Silent visual corruption that appears only on
   real footage and only at this width. Highest-value thing to get right first.
2. **The wgpu 28 vs 30 fork** (§3.4). Decides both the UI framework and ~3 days of unsafe
   work.
3. **The synchronous map stall** on the copy path (§3.5). Requires 2-3 frames of
   pipelining or throughput collapses toward the sum of stages.

### Confidence summary

| claim | confidence | basis |
|---|---|---|
| Dual 3840x3840 HEVC decodes in realtime with headroom | **very high** | measured, sustained 300 s, target hardware and asset |
| Seek is fast and position-independent | **very high** | measured at four offsets |
| Clip export both ways is fast | **very high** | measured |
| VAAPI -> dmabuf -> Vulkan import works here | **high** | measured per-frame on this machine |
| Copy fallback is realtime | **high** | measured, both slow and fast variants |
| Per-plane `texture_from_dmabuf_fd` is the right route | **high** | wgpu 30 source + three reference impls |
| `AVVkFrame` route is fragile | **medium-high** | reproduced device-lost twice; root cause unconfirmed |
| Reprojection shader fits the GPU budget | **medium-high** | libplacebo proxy, not the real shader |
| GStreamer cannot do zero-copy to wgpu | **high** | three independent source-level negatives |

### Unknowns to close early (cheap, high-leverage)

1. **Does `create_texture_from_hal` (and `initial_state`) exist on libcosmic's pinned wgpu
   28?** ~1 day. Decides the shell.
2. **Does `iced::widget::shader` render correctly under cosmic-comp?** No COSMIC app ships
   it. ~1 day.
3. **Height padding for 3840x3840 specifically.** Measured data is 3840x2160. Read the
   descriptor; never extrapolate. Hours.
4. **Does a winit window requesting server-side decorations get COSMIC chrome?** ~1 hour,
   only needed if route B is taken.

### Unknowns accepted (not blocking)

- Whether the libplacebo device-lost has been fixed in FFmpeg 8.1 + libplacebo 7.360. We
  avoid the path entirely.
- GFX12/RDNA4 behaviour with DCC video modifiers (the layout-discard hazard becomes live).
  No hardware available; not our target.
- Cost of `copy_buffer_to_texture` on UMA. Unmeasured, believed small.
- Discrete-AMD-GPU viability of the copy path at 4K60. Bandwidth is fine (1.49 GB/s,
  ~9.5% of PCIe 3.0 x16); latency is the constraint and is unmeasured. Not our target.

---

## Appendix: reproducing the decode benchmarks

```sh
D=/dev/dri/renderD128
CLIP=your-clip.insv

# software decode, one stream
ffmpeg -threads 0 -t 60 -i "$CLIP" -map 0:0 -f null -

# VA-API, one stream, frames stay on the GPU
ffmpeg -hwaccel vaapi -hwaccel_device $D -hwaccel_output_format vaapi \
       -t 60 -i "$CLIP" -map 0:0 -f null -

# VA-API, BOTH streams concurrently (the real workload)
ffmpeg -hwaccel vaapi -hwaccel_device $D -hwaccel_output_format vaapi \
       -t 60 -i "$CLIP" -map 0:0 -f null - -map 0:1 -f null -

# copy path, fast variant (derive, not getImage)
ffmpeg -hwaccel vaapi -hwaccel_device $D -hwaccel_output_format vaapi \
       -t 60 -i "$CLIP" -map 0:0 -vf 'hwmap=mode=read+direct,hwdownload,format=nv12' -f null -

# verify the dmabuf -> Vulkan import (look for "Mapped DRM object to Vulkan!")
ffmpeg -v debug -init_hw_device vaapi=va:$D -init_hw_device vulkan=vk@va -filter_hw_device vk \
       -hwaccel vaapi -hwaccel_device va -hwaccel_output_format vaapi \
       -t 20 -i "$CLIP" -map 0:0 -vf 'hwmap=derive_device=vulkan,hwdownload,format=nv12' -f null -

# GOP structure
ffprobe -v error -select_streams v:0 -show_entries packet=pts_time,size,flags \
        -read_intervals "%+120" -of csv=p=0 "$CLIP" | awk -F, '$3 ~ /K/ {print $1, $2}'
```
