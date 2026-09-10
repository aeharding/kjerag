use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const Y_SIZE: [u32; 2] = [16, 16];
const UV_SIZE: [u32; 2] = [8, 8];

#[test]
fn gpu_uses_separate_y_and_uv_motion_weights() {
    // Nonpositive confidence contributes exactly zero for either plane,
    // independently of the other plane's positive confidence and sample.
    for inactive_confidence in [0, -1, i16::MIN] {
        assert_separate_y_and_uv_motion_weights(inactive_confidence);
    }
}

fn assert_separate_y_and_uv_motion_weights(inactive_confidence: i16) {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let y = array_texture(
        &device,
        "temporal test Y",
        Y_SIZE,
        3,
        wgpu::TextureFormat::R8Unorm,
    );
    let uv = array_texture(
        &device,
        "temporal test UV",
        UV_SIZE,
        3,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let flow = array_texture(
        &device,
        "temporal test flow",
        [1, 1],
        2,
        wgpu::TextureFormat::Rgba16Sint,
    );
    let luma = array_texture(
        &device,
        "temporal test luma",
        [1, 1],
        1,
        wgpu::TextureFormat::R8Uint,
    );

    write_layer(&queue, &y, 1, [0, 0], Y_SIZE, 16, &vec![64; 16 * 16]);
    let moving_y: Vec<u8> = (0..16).flat_map(|_| (0..16).map(|x| 96 + 2 * x)).collect();
    write_layer(&queue, &y, 2, [0, 0], Y_SIZE, 16, &moving_y);
    write_layer(&queue, &y, 0, [0, 0], Y_SIZE, 16, &vec![240; 16 * 16]);
    write_layer(
        &queue,
        &uv,
        1,
        [0, 0],
        UV_SIZE,
        16,
        &repeated_uv([40, 200], 64),
    );
    write_layer(
        &queue,
        &uv,
        2,
        [0, 0],
        UV_SIZE,
        16,
        &repeated_uv([250, 10], 64),
    );
    write_layer(
        &queue,
        &uv,
        0,
        [0, 0],
        UV_SIZE,
        16,
        &repeated_uv([100, 140], 64),
    );
    // Reference 0 moves one pixel right and contributes only Y. The last
    // output column therefore exercises the source-edge clamp. Reference 1
    // contributes only UV, proving that z/w are independent confidences.
    write_layer(
        &queue,
        &flow,
        0,
        [0, 0],
        [1, 1],
        8,
        &flow_texel(1, 0, 255, inactive_confidence),
    );
    write_layer(
        &queue,
        &flow,
        1,
        [0, 0],
        [1, 1],
        8,
        &flow_texel(0, 0, inactive_confidence, 255),
    );
    write_layer(&queue, &luma, 0, [0, 0], [1, 1], 1, &[73]);

    let parameters = Parameters {
        noise: 1.0,
        limit: 1.0,
        y_limits: [1.0; 256],
        uv_limits: [1.0; 256],
        current_layer: 1,
        reference_layers: vec![2, 0],
    };
    let pipeline = GpuFuse::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = pipeline
        .encode(
            &device,
            &mut encoder,
            Inputs {
                y: &y,
                uv: &uv,
                flow: &flow,
                luma: &luma,
            },
            &parameters,
            [0, 0, 16, 16],
        )
        .unwrap();
    let y_copy = copy_texture(&device, &mut encoder, &output.y, [16, 16], 1);
    let uv_copy = copy_texture(&device, &mut encoder, &output.uv, [8, 8], 2);
    queue.submit([encoder.finish()]);
    let actual_y = read_copy(&device, &y_copy, [16, 16], 1);
    let actual_uv = read_copy(&device, &uv_copy, [8, 8], 2);
    let expected_y: Vec<u8> = (0..16)
        .flat_map(|_| (0..16).map(|x| if x == 15 { 95 } else { 81 + x }))
        .collect();
    assert_eq!(actual_y, expected_y);
    assert_eq!(actual_uv, repeated_uv([70, 170], 64));
}

#[test]
fn gpu_uses_signed_uv_shift_and_independent_component_gates() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let y = array_texture(
        &device,
        "signed UV test Y",
        Y_SIZE,
        2,
        wgpu::TextureFormat::R8Unorm,
    );
    let uv = array_texture(
        &device,
        "signed UV test UV",
        UV_SIZE,
        2,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let flow = array_texture(
        &device,
        "signed UV test flow",
        [1, 1],
        1,
        wgpu::TextureFormat::Rgba16Sint,
    );
    let luma = array_texture(
        &device,
        "signed UV test luma",
        [1, 1],
        1,
        wgpu::TextureFormat::R8Uint,
    );
    for layer in 0..2 {
        write_layer(&queue, &y, layer, [0, 0], Y_SIZE, 16, &vec![64; 16 * 16]);
    }
    write_layer(
        &queue,
        &uv,
        0,
        [0, 0],
        UV_SIZE,
        16,
        &repeated_uv([100, 200], 64),
    );
    let reference_uv: Vec<u8> = (0..8)
        .flat_map(|_| (0..8).flat_map(|x| [110 + 2 * x, 0]))
        .collect();
    write_layer(&queue, &uv, 1, [0, 0], UV_SIZE, 16, &reference_uv);
    // The selected UV law uses an arithmetic signed shift, so -1 >> 1 is -1.
    write_layer(
        &queue,
        &flow,
        0,
        [0, 0],
        [1, 1],
        8,
        &flow_texel(-1, 0, 0, 255),
    );
    write_layer(&queue, &luma, 0, [0, 0], [1, 1], 1, &[73]);
    let mut uv_limits = [1.0; 256];
    uv_limits[73] = 0.5;
    let parameters = Parameters {
        noise: 40.0 / 255.0,
        limit: 16.0 / 255.0,
        y_limits: [1.0; 256],
        uv_limits,
        current_layer: 0,
        reference_layers: vec![1],
    };
    let pipeline = GpuFuse::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = pipeline
        .encode(
            &device,
            &mut encoder,
            Inputs {
                y: &y,
                uv: &uv,
                flow: &flow,
                luma: &luma,
            },
            &parameters,
            [0, 0, 16, 16],
        )
        .unwrap();
    let copy = copy_texture(&device, &mut encoder, &output.uv, UV_SIZE, 2);
    queue.submit([encoder.finish()]);
    let actual = read_copy(&device, &copy, UV_SIZE, 2);
    let expected: Vec<u8> = (0..8)
        .flat_map(|_| {
            (0u8..8).flat_map(|x| {
                let shifted = x.saturating_sub(1);
                [(105 + shifted).min(108), 200]
            })
        })
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn encode_rejects_ambiguous_layers_odd_regions_and_nonfinite_parameters() {
    let Some((device, _queue)) = gpu() else {
        return;
    };
    let y = array_texture(
        &device,
        "temporal validation Y",
        Y_SIZE,
        3,
        wgpu::TextureFormat::R8Unorm,
    );
    let uv = array_texture(
        &device,
        "temporal validation UV",
        UV_SIZE,
        3,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let flow = array_texture(
        &device,
        "temporal validation flow",
        [1, 1],
        2,
        wgpu::TextureFormat::Rgba16Sint,
    );
    let luma = array_texture(
        &device,
        "temporal validation luma",
        [1, 1],
        1,
        wgpu::TextureFormat::R8Uint,
    );
    let inputs = || Inputs {
        y: &y,
        uv: &uv,
        flow: &flow,
        luma: &luma,
    };
    let parameters = |noise, references| Parameters {
        noise,
        limit: 1.0,
        y_limits: [1.0; 256],
        uv_limits: [1.0; 256],
        current_layer: 0,
        reference_layers: references,
    };
    let pipeline = GpuFuse::new(&device);

    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        pipeline
            .encode(
                &device,
                &mut encoder,
                inputs(),
                &parameters(1.0, vec![1, 1]),
                [0, 0, 16, 16],
            )
            .err()
            .expect("duplicate reference layer was accepted")
            .contains("distinct current and reference layers")
    );
    assert!(
        pipeline
            .encode(
                &device,
                &mut encoder,
                inputs(),
                &parameters(1.0, vec![1, 2]),
                [0, 0, 15, 16],
            )
            .err()
            .expect("odd region was accepted")
            .contains("even-aligned region")
    );
    assert!(
        pipeline
            .encode(
                &device,
                &mut encoder,
                inputs(),
                &parameters(f32::NAN, vec![1, 2]),
                [0, 0, 16, 16],
            )
            .err()
            .expect("nonfinite noise was accepted")
            .contains("finite and nonnegative")
    );
}

/// Replays the two authenticated native input packets. This intentionally
/// allocates full-size arrays so captured rectangle origins remain source
/// coordinates. It is a diagnostic of the selected captured arithmetic, not
/// a general Studio-parity or image-quality test.
#[test]
#[ignore]
fn exact_native_captured_packets_are_within_one_storage_code() {
    let fixture = std::env::var_os("KJERAG_DENOISE_FIXTURE_DIR")
        .map(PathBuf::from)
        .expect("set KJERAG_DENOISE_FIXTURE_DIR to denoise-offline-01/root-run");
    let Some((device, queue)) = gpu() else {
        return;
    };
    for (packet, source, expected_sha) in [
        (
            6,
            18_214,
            "a2ddeaeb8e035d3601bf5ed63d234ced3a33a2a28157b43826080d7fd4b5e7e8",
        ),
        (
            7,
            18_215,
            "c50cc19b8bb7de1f6a98ef4ee214279e28746a2ccfcaf4f7fada887528f0edd3",
        ),
    ] {
        replay_fixture(
            &device,
            &queue,
            &fixture.join(format!("denoise-{packet:03}.bin")),
            packet,
            source,
            expected_sha,
        );
    }
}

fn replay_fixture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
    packet: u32,
    source: u32,
    sha: &str,
) {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert_eq!(
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        sha,
        "fixture SHA {}",
        path.display()
    );
    let fixture = Fixture::read(bytes);
    // Both values were written from denoise_source_associated, and the sealed
    // container hash authenticates that event-derived association. Do not
    // infer either value from the other's arithmetic.
    assert_eq!((fixture.source, fixture.packet), (source, packet));
    assert_eq!((fixture.current_y.fw, fixture.current_y.fh), (7680, 3840));
    assert_eq!((fixture.current_uv.fw, fixture.current_uv.fh), (3840, 1920));

    let y = array_texture(
        device,
        "captured temporal Y",
        [7680, 3840],
        7,
        wgpu::TextureFormat::R8Unorm,
    );
    let uv = array_texture(
        device,
        "captured temporal UV",
        [3840, 1920],
        7,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let flow = array_texture(
        device,
        "captured temporal flow",
        [480, 240],
        6,
        wgpu::TextureFormat::Rgba16Sint,
    );
    let luma = array_texture(
        device,
        "captured temporal luma",
        [480, 240],
        1,
        wgpu::TextureFormat::R8Uint,
    );
    upload_rect(queue, &y, 0, &fixture.current_y);
    upload_rect(queue, &uv, 0, &fixture.current_uv);
    upload_rect(queue, &luma, 0, &fixture.luma);
    for (ordinal, reference) in fixture.references.iter().enumerate() {
        upload_rect(queue, &flow, ordinal as u32, &reference.flow);
        upload_rect(queue, &y, ordinal as u32 + 1, &reference.y);
        upload_rect(queue, &uv, ordinal as u32 + 1, &reference.uv);
    }
    let parameters = Parameters {
        noise: fixture.y_parameters.noise,
        limit: fixture.y_parameters.limit,
        y_limits: fixture.y_parameters.table,
        uv_limits: fixture.uv_parameters.table,
        current_layer: 0,
        reference_layers: (1..=6).collect(),
    };
    assert_eq!(
        (fixture.y_parameters.noise, fixture.y_parameters.limit),
        (fixture.uv_parameters.noise, fixture.uv_parameters.limit)
    );
    let roi = [
        fixture.current_y.x,
        fixture.current_y.y,
        fixture.current_y.w,
        fixture.current_y.h,
    ];
    assert_eq!(
        [
            fixture.expected_y.x,
            fixture.expected_y.y,
            fixture.expected_y.w,
            fixture.expected_y.h,
        ],
        roi
    );
    assert_eq!(
        [
            fixture.expected_uv.x,
            fixture.expected_uv.y,
            fixture.expected_uv.w,
            fixture.expected_uv.h,
        ],
        [roi[0] / 2, roi[1] / 2, roi[2] / 2, roi[3] / 2]
    );
    let pipeline = GpuFuse::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = pipeline
        .encode(
            device,
            &mut encoder,
            Inputs {
                y: &y,
                uv: &uv,
                flow: &flow,
                luma: &luma,
            },
            &parameters,
            roi,
        )
        .unwrap();
    let y_copy = copy_texture(device, &mut encoder, &output.y, [roi[2], roi[3]], 1);
    let uv_size = [roi[2] / 2, roi[3] / 2];
    let uv_copy = copy_texture(device, &mut encoder, &output.uv, uv_size, 2);
    queue.submit([encoder.finish()]);
    let actual_y = read_copy(device, &y_copy, [roi[2], roi[3]], 1);
    let actual_uv = read_copy(device, &uv_copy, uv_size, 2);
    let y_diff = differences(&actual_y, &fixture.expected_y.bytes);
    let uv_diff = differences(&actual_uv, &fixture.expected_uv.bytes);
    eprintln!(
        "native denoise fixture packet={} source={} Y differing={} UV-component differing={}",
        fixture.packet, fixture.source, y_diff, uv_diff
    );
    assert!(
        actual_y
            .iter()
            .zip(&fixture.expected_y.bytes)
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    );
    assert!(
        actual_uv
            .iter()
            .zip(&fixture.expected_uv.bytes)
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    );
}

fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).filter(|(a, b)| a != b).count()
}
fn repeated_uv(value: [u8; 2], count: usize) -> Vec<u8> {
    (0..count).flat_map(|_| value).collect()
}
fn flow_texel(x: i16, y: i16, z: i16, w: i16) -> Vec<u8> {
    [x, y, z, w]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect()
}

fn array_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    layers: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
fn write_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    origin: [u32; 2],
    size: [u32; 2],
    row: u32,
    bytes: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin[0],
                y: origin[1],
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
}
fn upload_rect(queue: &wgpu::Queue, texture: &wgpu::Texture, layer: u32, t: &CapturedTexture) {
    write_layer(
        queue,
        texture,
        layer,
        [t.x, t.y],
        [t.w, t.h],
        t.row,
        &t.bytes,
    )
}

pub(super) struct Copy {
    buffer: wgpu::Buffer,
    padded_row: u32,
}
pub(super) fn copy_texture(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    size: [u32; 2],
    bpp: u32,
) -> Copy {
    let packed = size[0] * bpp;
    let padded =
        packed.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("temporal output readback"),
        size: u64::from(padded) * u64::from(size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    Copy {
        buffer,
        padded_row: padded,
    }
}
pub(super) fn read_copy(device: &wgpu::Device, copy: &Copy, size: [u32; 2], bpp: u32) -> Vec<u8> {
    copy.buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = copy.buffer.slice(..).get_mapped_range();
    let packed = (size[0] * bpp) as usize;
    let mut out = Vec::with_capacity(packed * size[1] as usize);
    for row in mapped
        .chunks(copy.padded_row as usize)
        .take(size[1] as usize)
    {
        out.extend_from_slice(&row[..packed])
    }
    drop(mapped);
    copy.buffer.unmap();
    out
}

struct CapturedParameters {
    noise: f32,
    limit: f32,
    table: [f32; 256],
}
struct CapturedTexture {
    fw: u32,
    fh: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    row: u32,
    bytes: Vec<u8>,
}
struct CapturedReference {
    flow: CapturedTexture,
    y: CapturedTexture,
    uv: CapturedTexture,
}
struct Fixture {
    source: u32,
    packet: u32,
    y_parameters: CapturedParameters,
    uv_parameters: CapturedParameters,
    current_y: CapturedTexture,
    current_uv: CapturedTexture,
    luma: CapturedTexture,
    references: Vec<CapturedReference>,
    expected_y: CapturedTexture,
    expected_uv: CapturedTexture,
}
struct Reader {
    bytes: Vec<u8>,
    at: usize,
}
impl Reader {
    fn take(&mut self, n: usize) -> &[u8] {
        let at = self.at;
        self.at += n;
        self.bytes.get(at..at + n).expect("truncated fixture")
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn parameters(&mut self) -> CapturedParameters {
        let noise = self.f32();
        let limit = self.f32();
        let mut table = [0.0; 256];
        for v in &mut table {
            *v = self.f32()
        }
        CapturedParameters {
            noise,
            limit,
            table,
        }
    }
    fn texture(&mut self) -> CapturedTexture {
        let fw = self.u32();
        let fh = self.u32();
        let x = self.u32();
        let y = self.u32();
        let w = self.u32();
        let h = self.u32();
        let row = self.u32();
        let bpp = self.u32();
        let len = self.u32() as usize;
        assert_eq!(len, row as usize * h as usize);
        assert_eq!(row, w * bpp);
        let bytes = self.take(len).to_vec();
        CapturedTexture {
            fw,
            fh,
            x,
            y,
            w,
            h,
            row,
            bytes,
        }
    }
}
impl Fixture {
    fn read(bytes: Vec<u8>) -> Self {
        let mut r = Reader { bytes, at: 0 };
        assert_eq!(r.take(8), b"KDNREF01");
        let source = r.u32();
        let packet = r.u32();
        let y_parameters = r.parameters();
        let uv_parameters = r.parameters();
        let current_y = r.texture();
        let current_uv = r.texture();
        let luma = r.texture();
        let mut references = Vec::with_capacity(6);
        for _ in 0..6 {
            references.push(CapturedReference {
                flow: r.texture(),
                y: r.texture(),
                uv: r.texture(),
            })
        }
        let expected_y = r.texture();
        let expected_uv = r.texture();
        assert_eq!(r.at, r.bytes.len());
        Self {
            source,
            packet,
            y_parameters,
            uv_parameters,
            current_y,
            current_uv,
            luma,
            references,
            expected_y,
            expected_uv,
        }
    }
}

pub(super) fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let (_instance, adapter) = gpu_adapter()?;
    request_test_device(&adapter, "temporal fusion test")
}

pub(super) fn gpu_pair() -> Option<[(wgpu::Device, wgpu::Queue); 2]> {
    let (_instance, adapter) = gpu_adapter()?;
    let first = request_test_device(&adapter, "temporal fusion test device one")?;
    let second = request_test_device(&adapter, "temporal fusion test device two")?;
    Some([first, second])
}

fn gpu_adapter() -> Option<(wgpu::Instance, wgpu::Adapter)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let require_gpu = std::env::var_os("KJERAG_REQUIRE_GPU").is_some();
    let adapters = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    let adapter = adapters
        .iter()
        .find(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu)
        .cloned()
        .or_else(|| {
            (!require_gpu)
                .then(|| adapters.into_iter().next())
                .flatten()
        });
    let Some(adapter) = adapter else {
        assert!(
            !require_gpu,
            "KJERAG_REQUIRE_GPU test has no non-CPU Vulkan adapter"
        );
        eprintln!("skipping temporal fusion GPU test: no Vulkan adapter");
        return None;
    };
    eprintln!("temporal fusion GPU adapter: {:?}", adapter.get_info());
    Some((instance, adapter))
}

fn request_test_device(
    adapter: &wgpu::Adapter,
    label: &'static str,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    match block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    })) {
        Ok(pair) => Some(pair),
        Err(error) => {
            assert!(
                std::env::var_os("KJERAG_REQUIRE_GPU").is_none(),
                "temporal fusion GPU device: {error}"
            );
            eprintln!("skipping temporal fusion GPU test: {error}");
            None
        }
    }
}
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}
