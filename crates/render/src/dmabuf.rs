//! Importing VA-API frames into wgpu 28, by hand.
//!
//! wgpu 30 ships `vulkan::Device::texture_from_dmabuf_fd` (gfx-rs/wgpu#9366).
//! libcosmic's iced is pinned to wgpu 28, which does not have it, so this
//! module is that function written against raw Vulkan. It is deleted, not
//! maintained, the day libcosmic reaches wgpu 30.
//!
//! Everything here is one idea: an `AVDRMFrameDescriptor` layer names a
//! dmabuf fd plus a pitch, an offset and a DRM format modifier, and Vulkan
//! will alias that memory as a `VkImage` if you hand it exactly those four
//! numbers. The traps are in docs/ARCHITECTURE.md; each has a comment where
//! it bites.

use std::ffi::{CStr, c_int};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};

use ash::vk;
use ffmpeg_next::ffi::AVDRMFrameDescriptor;
use wgpu::hal::api::Vulkan;

use super::{Extent, Fallible, Planes, Size};

/// NV12 arrives as two single-plane layers, not one two-plane image.
const DRM_FORMAT_R8: u32 = fourcc(b"R8  ");
const DRM_FORMAT_GR88: u32 = fourcc(b"GR88");

const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}

/// The device extensions an import needs. wgpu-hal 28 already enables the
/// first two whenever the adapter has them; it never enables the third, which
/// is the whole reason [`force_extensions`] exists.
pub const REQUIRED: [&CStr; 3] = [
    ash::khr::external_memory_fd::NAME,
    ash::ext::external_memory_dma_buf::NAME,
    ash::ext::image_drm_format_modifier::NAME,
];

/// Adds the missing extension at device creation. Pass this to wgpu-hal's
/// `Adapter::open_with_callback`; there is no wgpu-level equivalent on 28,
/// and an unmodified `request_device` yields a device that cannot import.
pub fn force_extensions(args: wgpu::hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>) {
    for name in REQUIRED {
        if !args.extensions.contains(&name) {
            args.extensions.push(name);
        }
    }
}

/// A device that can import, for the headless binaries in `kyerag-spike`,
/// which build their own instead of using iced's.
///
/// A plain `request_device` yields a device that cannot import a tiled
/// dmabuf, because wgpu-hal 28 never enables
/// `VK_EXT_image_drm_format_modifier`; [`force_extensions`] through
/// `open_with_callback` is the only hook that can add it. The app needs none
/// of this: its device is iced's, and the `[patch.crates-io]` wgpu entry is
/// what puts the extension on that one.
pub fn open_device(adapter: &wgpu::Adapter) -> Fallible<(wgpu::Device, wgpu::Queue)> {
    let opened = unsafe {
        let hal = adapter.as_hal::<Vulkan>().ok_or("not a Vulkan adapter")?;
        hal.open_with_callback(
            wgpu::Features::empty(),
            &wgpu::MemoryHints::default(),
            Some(Box::new(force_extensions)),
        )?
    };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal::<Vulkan>(
            opened,
            &wgpu::DeviceDescriptor {
                label: Some("headless"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            },
        )
    }?;
    Ok((device, queue))
}

/// The first of [`REQUIRED`] this device does not have, if any. Creating an
/// image with a disabled extension's structures is undefined behavior rather
/// than an error, so this guards every import. It should never fire: the
/// `[patch.crates-io]` entry in Cargo.toml exists to keep it quiet.
pub fn missing_extension(device: &wgpu::Device) -> Fallible<Option<&'static CStr>> {
    let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
    let enabled = hal.enabled_device_extensions();
    Ok(REQUIRED.into_iter().find(|name| !enabled.contains(name)))
}

/// One line naming what this device can and cannot do, for the bring-up
/// report. Cheap enough to print on every start.
pub fn device_report(device: &wgpu::Device) -> String {
    match missing_extension(device) {
        Err(e) => format!("dmabuf import: unavailable ({e})"),
        Ok(None) => "dmabuf import: all extensions enabled".to_owned(),
        Ok(Some(name)) => format!(
            "dmabuf import: NOT enabled, missing {}",
            name.to_string_lossy()
        ),
    }
}

/// Import one DRM_PRIME descriptor as two sampled textures.
pub fn import(device: &wgpu::Device, desc: &AVDRMFrameDescriptor, luma: Size) -> Fallible<Planes> {
    if let Some(name) = missing_extension(device)? {
        return Err(format!(
            "device is missing {}; importing anyway is undefined behavior. The \
             [patch.crates-io] wgpu entry in Cargo.toml is what enables it.",
            name.to_string_lossy()
        )
        .into());
    }
    if desc.nb_layers != 2 {
        return Err(format!("expected 2 NV12 layers, got {}", desc.nb_layers).into());
    }
    Ok(Planes {
        luma: import_layer(device, desc, 0, luma)?,
        chroma: import_layer(device, desc, 1, luma.halved())?,
    })
}

fn import_layer(
    device: &wgpu::Device,
    desc: &AVDRMFrameDescriptor,
    index: usize,
    size: Size,
) -> Fallible<wgpu::Texture> {
    let layer = &desc.layers[index];
    let plane = &layer.planes[0];
    let object = &desc.objects[plane.object_index as usize];
    let (wgpu_format, vk_format) = plane_format(layer.format)?;
    let modifier = object.format_modifier;

    let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
    let raw = hal.raw_device().clone();
    let instance = hal.shared_instance().raw_instance();
    let physical = hal.raw_physical_device();

    if !modifier_supported(instance, physical, vk_format, modifier) {
        return Err(format!(
            "modifier {modifier:#x} unsupported for {vk_format:?}; importing anyway would be UB"
        )
        .into());
    }

    // Pitch and offset verbatim from the descriptor. At 3840 wide the chroma
    // pitch is 4096 and the luma pitch is 3840: padding is per-plane, so no
    // computed rule is right for both, and a wrong one shears chroma only on
    // real footage.
    let plane_layout = vk::SubresourceLayout::default()
        .offset(u64::from(plane.offset as u32))
        .row_pitch(u64::from(plane.pitch as u32));
    let image = unsafe { create_image(&raw, vk_format, size, modifier, &plane_layout) }?;
    let guard = ImageGuard {
        device: &raw,
        image,
        memory: vk::DeviceMemory::null(),
    };

    // radeonsi exports one object for both layers, so each import needs its
    // own fd. `vkAllocateMemory` takes ownership of the fd on success only.
    let fd = dup_fd(object.fd)?;
    let memory = unsafe { import_memory(instance, &raw, physical, image, fd) }?;
    unsafe { raw.bind_image_memory(image, memory, 0) }?;
    let image = guard.release();

    let hal_desc = wgpu::hal::TextureDescriptor {
        label: Some("plane"),
        size: size.extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::TextureUses::RESOURCE,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: vec![],
    };
    // wgpu owns neither the image nor the memory, so it must be told how to
    // let go of both. `TextureMemory::External` is the "not mine" marker.
    let owned = raw.clone();
    let release: wgpu::hal::DropCallback = Box::new(move || unsafe {
        owned.destroy_image(image, None);
        owned.free_memory(memory, None);
    });
    let hal_texture = unsafe {
        hal.texture_from_raw(
            image,
            &hal_desc,
            Some(release),
            wgpu::hal::vulkan::TextureMemory::External,
        )
    };

    // wgpu 28 has no `initial_state` argument (that is wgpu#9496, new in 30),
    // so the first barrier is `oldLayout = UNDEFINED`, which permits a driver
    // to discard the contents. Benign on RADV before GFX12: DCC is off for
    // multi-planar modifiers and the acquire from VK_QUEUE_FAMILY_EXTERNAL is
    // a no-op. The byte-identical spike PNGs are the evidence.
    Ok(unsafe {
        device.create_texture_from_hal::<Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("plane"),
                size: size.extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        )
    })
}

/// # Safety
/// `layout` must describe the plane the caller is about to bind to `image`.
unsafe fn create_image(
    raw: &ash::Device,
    format: vk::Format,
    size: Size,
    modifier: u64,
    layout: &vk::SubresourceLayout,
) -> Fallible<vk::Image> {
    let mut modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(modifier)
        .plane_layouts(std::slice::from_ref(layout));
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width: size.width,
            height: size.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external)
        .push_next(&mut modifier_info);
    Ok(unsafe { raw.create_image(&info, None) }?)
}

/// # Safety
/// `image` must have been created with `DMA_BUF_EXT` external memory, and
/// `fd` must be a dmabuf that backs it.
unsafe fn import_memory(
    instance: &ash::Instance,
    raw: &ash::Device,
    physical: vk::PhysicalDevice,
    image: vk::Image,
    fd: OwnedFd,
) -> Fallible<vk::DeviceMemory> {
    let loader = ash::khr::external_memory_fd::Device::new(instance, raw);
    let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
    unsafe {
        loader.get_memory_fd_properties(
            vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
            fd.as_raw_fd(),
            &mut fd_properties,
        )
    }?;

    let mut dedicated = vk::MemoryDedicatedRequirements::default();
    let mut requirements = vk::MemoryRequirements2::default().push_next(&mut dedicated);
    unsafe {
        raw.get_image_memory_requirements2(
            &vk::ImageMemoryRequirementsInfo2::default().image(image),
            &mut requirements,
        )
    };
    let requirements = requirements.memory_requirements;

    // The fd narrows the choice; the image narrows it again. Either alone
    // picks a heap the other rejects.
    let allowed = requirements.memory_type_bits & fd_properties.memory_type_bits;
    let type_index = memory_type_index(instance, physical, allowed)
        .ok_or("no memory type accepts both the image and the dmabuf")?;

    let mut dedicated_alloc = vk::MemoryDedicatedAllocateInfo::default().image(image);
    // Ownership of the fd moves into Vulkan when this call succeeds and stays
    // with us when it fails, so the raw fd is only released at the last moment.
    let fd = fd.into_raw_fd();
    let mut import = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        .fd(fd);
    let info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(type_index)
        .push_next(&mut dedicated_alloc)
        .push_next(&mut import);
    match unsafe { raw.allocate_memory(&info, None) } {
        Ok(memory) => Ok(memory),
        Err(e) => {
            drop(unsafe { OwnedFd::from_raw_fd(fd) });
            Err(e.into())
        }
    }
}

fn memory_type_index(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    allowed: u32,
) -> Option<u32> {
    let properties = unsafe { instance.get_physical_device_memory_properties(physical) };
    (0..properties.memory_type_count).find(|i| allowed & (1 << i) != 0)
}

/// `vkGetPhysicalDeviceImageFormatProperties2` pre-flight. Creating an image
/// with a modifier the driver does not support is undefined behavior rather
/// than a clean error, so this runs before every import.
fn modifier_supported(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    format: vk::Format,
    modifier: u64,
) -> bool {
    let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .push_next(&mut external)
        .push_next(&mut drm);
    let mut external_properties = vk::ExternalImageFormatProperties::default();
    let mut properties = vk::ImageFormatProperties2::default().push_next(&mut external_properties);
    unsafe {
        instance
            .get_physical_device_image_format_properties2(physical, &info, &mut properties)
            .is_ok()
    }
}

/// Destroys a half-built image if the memory import fails part way through.
struct ImageGuard<'a> {
    device: &'a ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
}

impl ImageGuard<'_> {
    fn release(self) -> vk::Image {
        let image = self.image;
        std::mem::forget(self);
        image
    }
}

impl Drop for ImageGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_image(self.image, None);
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
}

fn plane_format(drm_format: u32) -> Fallible<(wgpu::TextureFormat, vk::Format)> {
    match drm_format {
        DRM_FORMAT_R8 => Ok((wgpu::TextureFormat::R8Unorm, vk::Format::R8_UNORM)),
        DRM_FORMAT_GR88 => Ok((wgpu::TextureFormat::Rg8Unorm, vk::Format::R8G8_UNORM)),
        other => Err(format!("unexpected DRM plane format {other:#x}").into()),
    }
}

fn dup_fd(fd: c_int) -> Fallible<OwnedFd> {
    let duplicate = unsafe { libc::dup(fd) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Straight from drm_fourcc.h: a typo here would silently pick the wrong
    /// wgpu format for a plane, and the render would still produce an image.
    #[test]
    fn plane_fourccs_match_drm_fourcc_h() {
        assert_eq!(DRM_FORMAT_R8, 0x2020_3852);
        assert_eq!(DRM_FORMAT_GR88, 0x3838_5247);
    }

    /// The third name is the one wgpu-hal 28 never enables on its own; if it
    /// ever drops out of this list, `force_extensions` becomes a no-op and the
    /// import silently turns into undefined behavior.
    #[test]
    fn required_extensions_include_the_modifier_extension() {
        assert!(REQUIRED.contains(&ash::ext::image_drm_format_modifier::NAME));
    }
}
