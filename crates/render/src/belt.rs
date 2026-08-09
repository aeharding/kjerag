//! **The belt: Studio's Optical Flow tier, one strip of the seam matched
//! densely every frame and consumed as a source-UV correction.**
//!
//! What the two lenses still disagree about at the seam is measured today by
//! [`super::band`] and reaches no pixel (docs/research/studio-parity.md 3.1).
//! The band reads 128 directions and one number each. The belt reads the whole
//! overlap as a picture: both lenses' shared ring rectified into ONE strip, a
//! DIS-class dense match over it, and a per-pixel field that displaces what
//! each lens is sampled at **before** the crossfade mixes them. Content the
//! two lenses see in different places is brought into one place; what is left
//! over ghosts, exactly as it does today.
//!
//! **The clock came first and it is why this shape and no other**
//! (docs/research/belt-rung0.md, measured on this box before a line of this
//! file existed):
//!
//! - the strip is [`COLUMNS`] x [`ROWS`], because the native belt is 48:1 and
//!   the candidate sizes all oversampled across the seam relative to along it.
//!   4096x128 holds 11.38 px/deg along and 9.05 across, which is 0.36 and 0.55
//!   of the source's own sampling, and cost **4.91 ms a frame with the film
//!   playing** against 9.47 for 4096x256;
//! - the search is seeded from the previous frame and runs **every frame**.
//!   Half rate was measured and refused: the frame that computes gets *more*
//!   expensive, not less, because a third of the duty cycle is a third of the
//!   reason for the governor to boost (rung-0 3.4);
//! - the workgroup is one RDNA wave with two patch pixels a lane, which
//!   measured 1.3 to 1.4 times faster than the obvious 64 lanes at one pixel
//!   each, because the barriers in the 64-lane version cross two waves
//!   (rung-0 1.2);
//! - the seed is **correctness and not an optimization**. A cold search at one
//!   level recovers 1.5 to 1.9 strip pixels of a planted shift and no more, at
//!   every candidate size, which is a Lucas-Kanade patch's linearization
//!   radius. The capture window the design asks for is ten times that, so the
//!   ladder and the hint are what cover it between them, and the ladder runs
//!   on the first frame of a file, after a seek, and on the frame the arm is
//!   switched on.
//!
//! **There is one shape and there is no ladder of them.** Owner, 2026-08-09:
//! *"I wouldn't necessarily make the params configurable for a lower end
//! device though - lets try to get studio parity first."* Every number here is
//! a constant. rung-0 7.2 is a NO-GO for a UHD 620 at any size and says so;
//! this file is the Phoenix-class answer and nothing about it is dialled down
//! for a part nobody has measured.
//!
//! **Where the halves live.** This module owns the strip's PRODUCTION: the
//! rectification map, the pyramid, the search, the densification and the gate.
//! [`super::projection`] owns its CONSUMPTION - `belt_ray` and its Rust twin -
//! because that is map arithmetic, it sits inside `blend`, and the GPU twin
//! guard (`super::twin`) already compiles exactly that string.

use std::time::{Duration, Instant};

use kjerag_media::Size;

use super::projection::Reframe;

/// How many texels round the seam ring the strip holds.
///
/// rung-0 6: 11.38 px/deg along, 0.36 of the source's own 31.44, and the size
/// whose field measured cleanest of those that fit the clock. The whole strip
/// is 14.8 MB resident.
pub const COLUMNS: u32 = 4096;

/// How many texels across the seam the strip holds. 9.05 px/deg, 0.55 of the
/// source's 16.55: the seam is twice as finely sampled along the ring as
/// across it, so a strip that spends pixels evenly spends them where the
/// source cannot supply them.
pub const ROWS: u32 = 128;

/// Patch side, in strip pixels. DIS's `theta_ps`.
const PATCH: u32 = 8;

/// How far apart patch origins sit: DIS's overlap parameter as a stride.
const STRIDE: u32 = 3;

/// Threads per search workgroup, and patch pixels per thread.
///
/// One RDNA wave and two pixels each, which rung-0 1.2 measured 1.3 to 1.4
/// times faster than 64 lanes at one pixel: the barriers in the wider version
/// cross two waves. `LANES * PER` is one patch.
const LANES: u32 = 32;
const PER: u32 = 2;

/// The shaders index a lane with `& 7` and `>> 3`, reduce over 64 entries and
/// multiply a patch coordinate by a literal 3. This is what says so.
const _: () = assert!(PATCH == 8 && STRIDE == 3 && LANES * PER == 64);

/// How many levels the coarse-to-fine ladder has. Level 2 is a quarter of the
/// strip each way.
const LEVELS: u32 = 3;

/// The residual a patch has to come back under to count, in units of the 0..1
/// luma the strip holds. The belt's version of the band's `KEEP`.
const GATE_RMS: f32 = 0.06;

/// How much of a column's patches have to come back under [`GATE_RMS`] for
/// that along-seam segment to be trusted.
///
/// rung-0 4 measured 79.9 to 81.4 percent of patches under the gate on the
/// owner's own May-01 flight at every candidate size, so a segment that keeps
/// half of its patches is a segment where the content, and not the estimator,
/// has stopped answering.
const KEEP: f32 = 0.50;

/// What an untrusted along-seam segment keeps of the coarse pyramid's answer
/// instead of its own.
///
/// **This is fail-upward and it is the anti-jump design.** Studio blends 98/2
/// toward the coarse estimate on an untrusted belt row and never snaps back to
/// calibration (docs/research/seam-temporal.md 2.4, measured). A gate that
/// zeroes is a correction switching on and off underneath a picture that is
/// standing still, which is the exact defect #172 was built to kill and which
/// the owner named as "jump".
const FAIL_UPWARD: f32 = 0.98;

/// Iterations of the inverse search per level on the cold ladder, and on the
/// seeded rung.
///
/// rung-0 3.1 priced both: at 4096x128 the finest level costs about 0.24 ms an
/// iteration over a fixed 0.26 ms of setup, so the seeded rung is where the
/// per-frame budget lives and the ladder is what is afforded once.
const COLD_ITERS: u32 = 8;
const SEED_ITERS: u32 = 3;

/// How much of the flow each lens carries, as lens 0's share.
///
/// **Antisymmetric, and it is a decision the record left open.** #171's
/// application form was one-sided - lens 1's whole picture displaced across
/// the seam (stage9 10.2) - and this file splits it instead, because the
/// correction cannot be carried all the way out to the far field by either
/// lens and whatever is not carried has to be given up over some run of
/// picture. Giving it up is a shear. At 0.5 each lens gives up half as much
/// over the same run, so the worst gradient in the picture is halved; the area
/// that carries one doubles, and peak gradient is what an eye finds. Measured
/// both ways in the report this branch carries.
pub const SPLIT: f32 = 0.5;

/// The outer share of the strip's half-span that the correction is faded out
/// over, per lens, on that lens's own side.
///
/// **Their `ExtendFlowKel` analog, and the honest cost of the belt.** A
/// displacement that aligns the two lenses inside the handover cannot also be
/// right in the far field, where there is no second lens to align to and the
/// content is at infinity. So it is taken back out, and taking it out over a
/// finite run of picture is a shear - the one thing this project's own
/// principle says a correction must not leave behind (seam-temporal 1). What
/// makes it affordable is where it happens: the fade lives wholly OUTSIDE the
/// handover, so no doubled content is inside it, and [`SPLIT`] halves its
/// gradient.
///
/// 0.35 of a 7.07 degree half-span puts the fade between 4.60 and 7.07 degrees
/// off the seam. The handover reaches 4.00 degrees off the geometric seam and
/// the correction is full over all of it.
pub(crate) const FADE_SHARE: f32 = 0.35;

/// How much wider than the true shared picture `Reframe::within` answers:
/// `projection::CAP_MARGIN_DEG` widens each lens's coverage cap by half a
/// degree, so the interval where both answer true is a whole degree wider than
/// the interval where both really have the picture. A strip built on the
/// generous figure samples off the end of a fisheye circle somewhere on the
/// ring.
const CAP_MARGIN_BOTH_DEG: f64 = 1.0;

/// How far apart two pass blocks sit in the shared uniform buffer:
/// `wgpu::Limits::min_uniform_buffer_offset_alignment`'s own default, which is
/// what every binding into it has to be a multiple of.
const SLOT: u64 = 256;

/// How many blocks one frame of the belt needs: two rectifications, two
/// pyramid levels, three search rungs at most, and the gate and densification
/// which share one.
const SLOTS: usize = 8;

/// Which slot each pass reads. The two rectifications share one block, the two
/// pyramid levels have one each, the ladder has one per rung, and the gate and
/// the densification read the same one.
const SLOT_RECTIFY: usize = 0;
const SLOT_DOWN: usize = 1;
const SLOT_SEARCH: usize = 3;
const SLOT_GATE: usize = 7;

/// The luma the strip holds, and the two answer formats.
const LUMA: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const MAPPED: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const FIELD: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// How many patches fit at one pyramid level.
///
/// The strip wraps along the ring, so a patch may start at any column; it does
/// not wrap across the seam, so the last row of origins is the last one whose
/// patch fits.
pub fn patches(level: u32) -> (u32, u32) {
    grid(level)
}

fn grid(level: u32) -> (u32, u32) {
    let width = COLUMNS >> level;
    let height = ROWS >> level;
    (width / STRIDE, height.saturating_sub(PATCH) / STRIDE + 1)
}

/// The strip's across-seam span for one camera, in radians: the narrowest
/// azimuth's shared picture, which is as wide as a strip may be.
///
/// Walked out from the seam in hundredths of a degree through
/// `Reframe::within`, which is the same test the picture takes, with
/// [`CAP_MARGIN_BOTH_DEG`] taken back off. rung-0 2 read 14.140 degrees at the
/// narrowest of 128 azimuths on the owner's own X4 Air.
///
/// **The axis is the body's own elevation and not [`Ring::epi`], and that is a
/// deviation this file argues rather than hides.** rung-0's probe rectified
/// onto the ring's epipolar axis, which is where parallax is. The two are 0.6
/// to 3.5 degrees apart (`Ring::epi`'s own doc), so 99.8 percent of any
/// parallax displacement still lands on the across axis and the remainder
/// lands on the along axis, where the search reads it too. What elevation buys
/// is an **exact closed-form inverse**: the consuming half runs per output
/// pixel and has to answer "where on the strip is this ray" with no iteration,
/// and `body = centre(phi) cos a + epi(phi) sin a` cannot be inverted in
/// closed form because `epi` is itself a function of `phi`.
pub fn span(map: &Reframe, azimuths: usize) -> f64 {
    const PROBE_DEG: f64 = 0.01;
    let mut narrowest = f64::MAX;
    for index in 0..azimuths {
        let phi = index as f64 / azimuths as f64 * std::f64::consts::TAU;
        let mut both = 0.0;
        for side in [-1.0f64, 1.0] {
            let mut reach = 0.0;
            loop {
                let next = reach + PROBE_DEG;
                let ray = map.view_ray_from_body(direction(phi, side * next.to_radians()));
                if !map.within(0, ray) || !map.within(1, ray) {
                    break;
                }
                reach = next;
                if reach > 20.0 {
                    break;
                }
            }
            both += reach;
        }
        narrowest = narrowest.min(both);
    }
    // `both` is already the full width at this azimuth, and the margin is the
    // whole degree the cap test adds across the pair.
    (narrowest - CAP_MARGIN_BOTH_DEG).max(1.0).to_radians()
}

/// One strip texel's direction in the camera body's frame.
///
/// The WGSL twin is `belt_body` in `super::projection`, and the pair is the
/// whole of the strip's parameterization: azimuth round the seam ring, and
/// elevation off the seam plane.
pub fn direction(phi: f64, elevation: f64) -> [f32; 3] {
    let (sin_e, cos_e) = elevation.sin_cos();
    let (sin_p, cos_p) = phi.sin_cos();
    [(cos_e * cos_p) as f32, (cos_e * sin_p) as f32, sin_e as f32]
}

/// The rectification map: where each strip texel comes from in each lens, in
/// that lens's delivered-frame pixels.
///
/// **A body-frame object, and that is a design finding rather than a
/// shortcut** (rung-0 1). Its texel-to-source-pixel map depends on the
/// calibration and on nothing the view does, so it is built once when a file
/// opens and read every frame. What it leaves out is the rolling shutter,
/// which moves with the body; at the seam an X4 reads the same world direction
/// down both lenses and that term is worth 0.000 degrees there
/// (docs/research/insv-format.md 6.7), so the map is built at a pose with no
/// readout in it and gives nothing up on this camera.
pub struct Map {
    /// The across-seam span the map was built at, in radians.
    pub span: f64,
    /// Which lens's PICTURE each plane of the map is coordinates into.
    ///
    /// `[0, 1]` for a real map. `[0, 0]` for a planted one, and that is not a
    /// detail: a plane of lens 0's coordinates bound to lens 1's picture is
    /// noise, and it is the exact bug rung-0's own probe shipped with until its
    /// control caught it (`Map::planted`). The pairing is carried by the map
    /// rather than assumed by the binding, so the two cannot disagree.
    pub sources: [usize; 2],
    /// Two f32 per texel per lens.
    texels: [Vec<f32>; 2],
    /// How long it took, which the report quotes against the owner's own two
    /// second bar for a file opening.
    pub built: Duration,
}

impl Map {
    /// **The control this file would be worthless without.**
    ///
    /// A search kernel that wanders and reports the wander looks exactly like a
    /// search kernel that works, until something moves the content by a known
    /// amount and asks what it reads. This builds a map whose second plane is
    /// LENS 0 AGAIN, rectified with the across-seam angle displaced by
    /// `plant_deg`, so the strip pair differs by a shift nobody has to measure
    /// to know. The field then has to come back at minus that many strip
    /// pixels across and zero along.
    ///
    /// The pattern is `docs/research/belt-rung0.md` 1.1's, and so is the
    /// warning attached to it: the first version of that probe bound lens 1's
    /// picture to lens 0's planted coordinates, the control strip was noise,
    /// and the plant read +0.135 px where it had to read -2.716. A control that
    /// cannot fail is not a control.
    pub fn planted(map: &Reframe, span: f64, plant_deg: f64) -> Self {
        Self::rectified(map, span, Some(plant_deg))
    }

    pub fn build(map: &Reframe, span: f64) -> Self {
        Self::rectified(map, span, None)
    }

    fn rectified(map: &Reframe, span: f64, plant_deg: Option<f64>) -> Self {
        let began = Instant::now();
        let count = COLUMNS as usize * ROWS as usize;
        let mut texels: [Vec<f32>; 2] = std::array::from_fn(|_| vec![0.0f32; count * 2]);
        for row in 0..ROWS {
            let elevation = (f64::from(row) + 0.5) / f64::from(ROWS) - 0.5;
            let elevation = elevation * span;
            for column in 0..COLUMNS {
                let phi = (f64::from(column) + 0.5) / f64::from(COLUMNS) * std::f64::consts::TAU;
                let view = map.view_ray_from_body(direction(phi, elevation));
                let at = (row as usize * COLUMNS as usize + column as usize) * 2;
                for (lens, plane) in texels.iter_mut().enumerate() {
                    let (source, ray) = match (lens, plant_deg) {
                        // The plant: the second plane is lens 0 again, moved.
                        (1, Some(plant)) => (
                            0,
                            map.view_ray_from_body(direction(phi, elevation + plant.to_radians())),
                        ),
                        _ => (lens, view),
                    };
                    let landing = map.project(source, ray);
                    plane[at] = landing.pixel[0];
                    plane[at + 1] = landing.pixel[1];
                }
            }
        }
        Self {
            span,
            sources: match plant_deg {
                None => [0, 1],
                Some(_) => [0, 0],
            },
            texels,
            built: began.elapsed(),
        }
    }
}

/// Which rung of the ladder one dispatch of the search is.
#[derive(Clone, Copy)]
struct Rung {
    level: u32,
    iters: u32,
    /// Where the starting guess comes from: `None` is a standing start.
    seed: Option<Seed>,
    /// Which answer slot it writes.
    output: usize,
}

#[derive(Clone, Copy)]
enum Seed {
    /// The level above, doubled: the coarse-to-fine ladder's own rung.
    Coarser,
    /// The finest level's answer from the previous frame, at gain one. The
    /// temporal hint.
    Previous,
}

/// Everything the belt needs on the GPU for one file.
pub struct Belt {
    rectify: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    search: wgpu::ComputePipeline,
    gate: wgpu::ComputePipeline,
    densify: wgpu::RenderPipeline,

    rectify_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    search_layout: wgpu::BindGroupLayout,
    gate_layout: wgpu::BindGroupLayout,
    densify_layout: wgpu::BindGroupLayout,

    /// One R8Unorm strip per lens per pyramid level. Kept because a texture
    /// has to outlive the views taken of it.
    _strips: [[wgpu::Texture; LEVELS as usize]; 2],
    strip_views: [[wgpu::TextureView; LEVELS as usize]; 2],
    /// The rectification map, one Rg32Float texture per lens.
    _lut: [wgpu::Texture; 2],
    lut_views: [wgpu::TextureView; 2],
    /// One `vec4` per patch per level, plus the finest level's previous frame.
    answers: [wgpu::Buffer; LEVELS as usize],
    previous: wgpu::Buffer,
    /// One `vec4` per along-seam segment: how much of it came back trusted.
    trust: wgpu::Buffer,
    /// The densified per-pixel field the picture reads. Kept for the view's
    /// sake, as the strips are.
    _field: wgpu::Texture,
    field_view: wgpu::TextureView,
    /// A zero buffer, bound where a rung has no seed.
    nothing: wgpu::Buffer,

    sampler: wgpu::Sampler,
    /// One [`Pass`] block per pass of the frame, in one buffer.
    ///
    /// **Slots rather than one block rewritten, and that is what makes the
    /// belt one submit.** A `write_buffer` lands at the next submit on the
    /// queue, so a single block shared by eleven passes forces eleven submits,
    /// and eleven queue flushes a frame cost more than the arithmetic between
    /// them: measured on this box, splitting them read 29.4 fps against the
    /// control's 29.9, and one submit reads what section 5 of the report says.
    uniforms: wgpu::Buffer,

    /// The bind groups for the two rectification passes, rebuilt whenever the
    /// bound frame pair changes. `None` until a pair has been imported.
    sources: Option<[wgpu::BindGroup; 2]>,
    /// Whether the map has been uploaded for the open file.
    loaded: bool,
    /// The span the loaded map was built at, in radians.
    span: f32,
    /// Which lens's picture each strip is rectified from ([`Map::sources`]).
    sources_of: [usize; 2],
    /// Whether the next frame runs the whole ladder rather than the hint.
    cold: bool,
    /// Instrument only: a range of strip columns flattened after
    /// rectification, so an along-seam segment with no content in it can be
    /// planted and the fail-upward gate measured against one.
    blank: Option<(u32, u32)>,
    /// Whether a field has ever been written, so the picture is not handed a
    /// texture of zeros as though it were a measurement.
    ready: bool,
}

/// The uniform every belt pass reads, one 80 byte block written once per
/// dispatch. Every pass takes the same block so there is one layout to keep in
/// step rather than five.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Pass {
    cols: u32,
    rows: u32,
    iters: u32,
    seeded: u32,
    width: f32,
    height: f32,
    seed_cols: u32,
    seed_rows: u32,
    seed_gain: f32,
    seed_step: u32,
    frame_width: f32,
    frame_height: f32,
    lo: [f32; 2],
    hi: [f32; 2],
    source: [f32; 2],
    dest: [f32; 2],
    wide: f32,
    keep: f32,
    upward: f32,
    gate_rms: f32,
}

impl Pass {
    fn bytes(&self) -> &[u8] {
        // `repr(C)`, all four-byte scalars, no padding and no invalid pattern:
        // the same cast `Reframe::bytes` makes for the block itself.
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// How far a search at one level may wander from its guess, in strip pixels of
/// that level.
///
/// Along the ring it is the wider of the design's +-1.5 degrees and
/// `band::PERP_DEG`'s +-0.90; across the seam it is `band::FAR_DEG..NEAR_DEG`
/// of -1.2..+2.6, which contains the design's +-0.7. Taking the union on each
/// axis is the only reading that refuses nothing either document asks for
/// (rung-0 2.1), and without a clamp an unconstrained Lucas-Kanade wanders
/// about a patch width on decimated content and reports the wander as a
/// reading.
fn capture(level: u32, span_deg: f64) -> ([f32; 2], [f32; 2]) {
    let along = f64::from(COLUMNS >> level) / 360.0;
    let across = f64::from(ROWS >> level) / span_deg;
    (
        [(-1.5 * along) as f32, (-1.2 * across) as f32],
        [(1.5 * along) as f32, (2.6 * across) as f32],
    )
}

impl Belt {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = |label: &str, source: String| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };

        let uniform = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Pass>() as u64),
            },
            count: None,
        };
        let texture = |binding, visibility, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let storage = |binding, visibility, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let fragment = wgpu::ShaderStages::FRAGMENT;
        let compute = wgpu::ShaderStages::COMPUTE;

        let rectify_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt rectify"),
            entries: &[
                uniform(fragment),
                texture(1, fragment, false),
                texture(2, fragment, true),
                sampler_entry(3, fragment),
            ],
        });
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt down"),
            entries: &[
                uniform(fragment),
                texture(1, fragment, true),
                sampler_entry(2, fragment),
            ],
        });
        let search_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt search"),
            entries: &[
                uniform(compute),
                texture(1, compute, true),
                texture(2, compute, true),
                sampler_entry(3, compute),
                storage(4, compute, true),
                storage(5, compute, false),
            ],
        });
        let gate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt gate"),
            entries: &[
                uniform(compute),
                storage(1, compute, true),
                storage(2, compute, false),
            ],
        });
        let densify_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt densify"),
            entries: &[
                uniform(fragment),
                storage(1, fragment, true),
                storage(2, fragment, true),
                storage(3, fragment, true),
            ],
        });

        let render = |label: &str,
                      layout: &wgpu::BindGroupLayout,
                      source: String,
                      format: wgpu::TextureFormat| {
            let module = module(label, source);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[layout],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let compute_pipeline =
            |label: &str, layout: &wgpu::BindGroupLayout, source: String, entry: &str| {
                let module = module(label, source);
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[layout],
                        immediate_size: 0,
                    });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };

        let rectify = render(
            "belt rectify",
            &rectify_layout,
            format!("{FULLSCREEN}\n{RECTIFY}"),
            LUMA,
        );
        let down = render(
            "belt down",
            &down_layout,
            format!("{FULLSCREEN}\n{DOWN}"),
            LUMA,
        );
        let densify = render(
            "belt densify",
            &densify_layout,
            format!("{FULLSCREEN}\n{}", densify_wgsl()),
            FIELD,
        );
        let search = compute_pipeline("belt search", &search_layout, search_wgsl(), "search");
        let gate = compute_pipeline("belt gate", &gate_layout, gate_wgsl(), "gate");

        let strip = |level: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("belt strip"),
                size: wgpu::Extent3d {
                    width: COLUMNS >> level,
                    height: ROWS >> level,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: LUMA,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    // Instrument only ([`Belt::blank`]): a segment with no
                    // content in it is what the fail-upward gate is for, and
                    // the only way to plant one is to write it.
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let strips: [[wgpu::Texture; LEVELS as usize]; 2] =
            std::array::from_fn(|_lens| std::array::from_fn(|level| strip(level as u32)));
        let strip_views = std::array::from_fn(|lens: usize| {
            std::array::from_fn(|level: usize| strips[lens][level].create_view(&Default::default()))
        });

        let lut: [wgpu::Texture; 2] = std::array::from_fn(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("belt map"),
                size: wgpu::Extent3d {
                    width: COLUMNS,
                    height: ROWS,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MAPPED,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        });
        let lut_views =
            std::array::from_fn(|lens: usize| lut[lens].create_view(&Default::default()));

        let answers: [wgpu::Buffer; LEVELS as usize] = std::array::from_fn(|level| {
            let (cols, rows) = grid(level as u32);
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("belt answers"),
                size: u64::from(cols) * u64::from(rows) * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        });
        let (fine_cols, fine_rows) = grid(0);
        let previous = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt hint"),
            size: u64::from(fine_cols) * u64::from(fine_rows) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let trust = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt trust"),
            size: u64::from(fine_cols) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let nothing = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt nothing"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let field = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("belt field"),
            size: wgpu::Extent3d {
                width: COLUMNS,
                height: ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FIELD,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let field_view = field.create_view(&Default::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            // The strip wraps along the ring and does not wrap across the
            // seam, and both halves of that are here as well as in `tap`.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt pass"),
            size: SLOTS as u64 * SLOT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            rectify,
            down,
            search,
            gate,
            densify,
            rectify_layout,
            down_layout,
            search_layout,
            gate_layout,
            densify_layout,
            _strips: strips,
            strip_views,
            _lut: lut,
            lut_views,
            answers,
            previous,
            trust,
            _field: field,
            field_view,
            nothing,
            sampler,
            uniforms,
            sources: None,
            loaded: false,
            span: 0.0,
            sources_of: [0, 1],
            cold: true,
            blank: None,
            ready: false,
        }
    }

    /// Instrument only: flatten a range of strip columns after rectification.
    ///
    /// **Narrow on purpose.** A band this wide is textureless at the finest
    /// level - a whole 8 px patch fits inside it with nothing in it - and is
    /// still only a quarter as wide two levels up, where a patch straddling it
    /// keeps most of its content. That is exactly the situation the fail-
    /// upward gate exists for, and it is why the plant says something: the
    /// coarse answer in the planted band is real, so a field that reads it
    /// there is failing upward and a field that reads zero is snapping.
    pub fn blank(&mut self, columns: Option<(u32, u32)>) {
        self.blank = columns;
    }

    /// Instrument only: the finest level's patch answers, read back.
    ///
    /// A mapping stall, which is why no shipped path takes it: the field lives
    /// on the GPU for its whole life and the hot path never reads one back.
    pub fn answers(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<[f32; 4]> {
        self.read(device, queue, &self.answers[0])
    }

    /// The same for the coarsest level, which is what an untrusted segment
    /// fails upward to.
    pub fn coarse(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<[f32; 4]> {
        self.read(device, queue, &self.answers[(LEVELS - 1) as usize])
    }

    /// And the per-segment trust: how much of each along-seam segment came
    /// back under the gate, and how much of the coarse answer it is taking.
    pub fn trust(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<[f32; 4]> {
        self.read(device, queue, &self.trust)
    }

    fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        from: &wgpu::Buffer,
    ) -> Vec<[f32; 4]> {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt readback"),
            size: from.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(from, 0, &readback, 0, from.size());
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the belt readback did not finish");
        let mapped = readback.slice(..).get_mapped_range();
        let out = mapped
            .chunks_exact(16)
            .map(|lane| {
                std::array::from_fn(|word| {
                    f32::from_le_bytes(lane[word * 4..word * 4 + 4].try_into().unwrap())
                })
            })
            .collect();
        drop(mapped);
        readback.unmap();
        out
    }

    /// The field the picture reads. Bound every redraw whether the belt is
    /// running or not, because a bind group has to satisfy every entry its
    /// layout declares; what decides whether a pixel reads it is the block's
    /// own `belt` word.
    pub fn field(&self) -> &wgpu::TextureView {
        &self.field_view
    }

    /// The strip's across-seam span, in radians, or zero before a map has been
    /// built. The block carries it so the consuming half can turn a row into
    /// an angle.
    pub fn span(&self) -> f32 {
        self.span
    }

    /// Whether a field has been computed since the file opened. Until it has,
    /// the picture is handed a zero and told not to read it, which is not the
    /// same thing as reading a zero.
    pub fn ready(&self) -> bool {
        self.ready && self.loaded
    }

    /// Forget the hint. A seek and a hard turn both invalidate it, and rung-0
    /// 6 is emphatic that a search cannot recover a field more than about 1.5
    /// strip pixels wrong and that nothing downstream would know.
    pub fn chill(&mut self) {
        self.cold = true;
    }

    /// A new file: the map has to be rebuilt before anything else runs.
    pub fn unload(&mut self) {
        self.loaded = false;
        self.ready = false;
        self.cold = true;
        self.sources = None;
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Upload one file's rectification map.
    pub fn load(&mut self, queue: &wgpu::Queue, map: &Map) {
        for (lens, plane) in map.texels.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self._lut[lens],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes_of(plane),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(COLUMNS * 8),
                    rows_per_image: Some(ROWS),
                },
                wgpu::Extent3d {
                    width: COLUMNS,
                    height: ROWS,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.span = map.span as f32;
        self.sources_of = map.sources;
        self.loaded = true;
        self.ready = false;
        self.cold = true;
    }

    /// Point the rectification at a newly imported frame pair.
    pub fn rebind(&mut self, device: &wgpu::Device, luma: [&wgpu::TextureView; 2], wide: bool) {
        let _ = wide;
        self.sources = Some(std::array::from_fn(|lens| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("belt rectify"),
                layout: &self.rectify_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.slot(SLOT_RECTIFY),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.lut_views[lens]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        // The map says which lens's picture this plane's
                        // coordinates are into, and a planted map says lens 0
                        // for both. Reading it here rather than assuming
                        // `lens` is what stops a control from being noise.
                        resource: wgpu::BindingResource::TextureView(luma[self.sources_of[lens]]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        }));
    }

    /// One frame of the belt: rectify, pyramid, search, gate, densify.
    ///
    /// Encoded into the caller's encoder and submitted with it, from
    /// `ScenePipeline::prepare` and never from `draw`, which is handed a
    /// render pass it cannot leave. No readback anywhere: the field lives on
    /// the GPU for its whole life.
    pub fn run(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: Size, wide: bool) {
        let Some(sources) = &self.sources else {
            return;
        };
        if !self.loaded {
            return;
        }
        let span_deg = f64::from(self.span).to_degrees();

        // Which rungs of the ladder this frame runs. The coarsest runs every
        // frame and is what an untrusted along-seam segment fails upward to;
        // the two finer cold rungs run only on a frame with no usable hint,
        // which is the first of a file, the one after a seek, and the one the
        // arm was switched on.
        let mut ladder = vec![Rung {
            level: LEVELS - 1,
            iters: COLD_ITERS,
            seed: None,
            output: (LEVELS - 1) as usize,
        }];
        match self.cold {
            true => {
                for level in (0..LEVELS - 1).rev() {
                    ladder.push(Rung {
                        level,
                        iters: COLD_ITERS,
                        seed: Some(Seed::Coarser),
                        output: level as usize,
                    });
                }
            }
            false => ladder.push(Rung {
                level: 0,
                iters: SEED_ITERS,
                seed: Some(Seed::Previous),
                output: 0,
            }),
        }

        // **Every block for the frame, written once.** A `write_buffer` lands
        // at the next submit, so a block rewritten between passes is a submit
        // between passes; slots are what let the whole belt be one.
        let (cols, rows) = grid(0);
        let mut blocks = vec![Pass::default(); SLOTS];
        blocks[SLOT_RECTIFY] = Pass {
            frame_width: frame.width as f32,
            frame_height: frame.height as f32,
            wide: f32::from(u8::from(wide)),
            ..Pass::default()
        };
        for level in 1..LEVELS {
            blocks[SLOT_DOWN + level as usize - 1] = Pass {
                source: [
                    (COLUMNS >> (level - 1)) as f32,
                    (ROWS >> (level - 1)) as f32,
                ],
                dest: [(COLUMNS >> level) as f32, (ROWS >> level) as f32],
                ..Pass::default()
            };
        }
        for (index, rung) in ladder.iter().enumerate() {
            blocks[SLOT_SEARCH + index] = self.block_for(*rung, span_deg);
        }
        blocks[SLOT_GATE] = Pass {
            cols,
            rows,
            width: COLUMNS as f32,
            height: ROWS as f32,
            keep: KEEP,
            upward: FAIL_UPWARD,
            gate_rms: GATE_RMS,
            seed_cols: grid(LEVELS - 1).0,
            seed_rows: grid(LEVELS - 1).1,
            seed_step: 1 << (LEVELS - 1),
            seed_gain: (1 << (LEVELS - 1)) as f32,
            ..Pass::default()
        };
        let mut staged = vec![0u8; SLOTS * SLOT as usize];
        for (index, block) in blocks.iter().enumerate() {
            let at = index * SLOT as usize;
            staged[at..at + std::mem::size_of::<Pass>()].copy_from_slice(block.bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &staged);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("belt"),
        });

        // 1. Both lenses' overlap into one strip each, at the measured shape.
        for (lens, source) in sources.iter().enumerate() {
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("belt rectify"),
                color_attachments: &[Some(attachment(&self.strip_views[lens][0]))],
                ..Default::default()
            });
            render.set_pipeline(&self.rectify);
            render.set_bind_group(0, source, &[]);
            render.draw(0..3, 0..1);
        }

        // The planted textureless segment, if an instrument asked for one. It
        // lands between the rectification and the pyramid, so the coarse
        // levels are built from a strip that already has it - and it is a
        // submit of its own, which no shipped run reaches.
        if let Some((from, to)) = self.blank {
            queue.submit([encoder.finish()]);
            self.flatten(queue, from, to);
            encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("belt"),
            });
        }

        // 2. The pyramid: half each way, four taps, which is the support a
        //    coarse-to-fine search wants rather than the single tap a mip
        //    chain gives it.
        for level in 1..LEVELS {
            for lens in 0..2 {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("belt down"),
                    layout: &self.down_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.slot(SLOT_DOWN + level as usize - 1),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &self.strip_views[lens][level as usize - 1],
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("belt down"),
                    color_attachments: &[Some(attachment(&self.strip_views[lens][level as usize]))],
                    ..Default::default()
                });
                render.set_pipeline(&self.down);
                render.set_bind_group(0, &group, &[]);
                render.draw(0..3, 0..1);
            }
        }

        // 3. The search, rung by rung. Each is its own compute pass, because
        //    a rung reads the answer the rung above wrote and two dispatches
        //    inside one pass have no barrier between them.
        for (index, rung) in ladder.iter().enumerate() {
            let group = self.search_group(device, *rung, SLOT_SEARCH + index);
            let (rung_cols, rung_rows) = grid(rung.level);
            let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("belt search"),
                timestamp_writes: None,
            });
            compute.set_pipeline(&self.search);
            compute.set_bind_group(0, &group, &[]);
            compute.dispatch_workgroups(rung_cols, rung_rows, 1);
        }

        // 4. The gate, and 5. the densification, in that order: an untrusted
        //    segment is decided before the field it shapes is written.
        let gate_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("belt gate"),
            layout: &self.gate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.slot(SLOT_GATE),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.answers[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.trust.as_entire_binding(),
                },
            ],
        });
        let densify_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("belt densify"),
            layout: &self.densify_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.slot(SLOT_GATE),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.answers[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.answers[(LEVELS - 1) as usize].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.trust.as_entire_binding(),
                },
            ],
        });
        {
            let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("belt gate"),
                timestamp_writes: None,
            });
            compute.set_pipeline(&self.gate);
            compute.set_bind_group(0, &gate_group, &[]);
            compute.dispatch_workgroups(cols, 1, 1);
        }
        {
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("belt densify"),
                color_attachments: &[Some(attachment(&self.field_view))],
                ..Default::default()
            });
            render.set_pipeline(&self.densify);
            render.set_bind_group(0, &densify_group, &[]);
            render.draw(0..3, 0..1);
        }
        // The finest level's answer becomes the next frame's hint. A copy
        // rather than a swap, because the search reads its seed and writes its
        // answer in one dispatch and a buffer cannot be both.
        encoder.copy_buffer_to_buffer(&self.answers[0], 0, &self.previous, 0, self.previous.size());
        queue.submit([encoder.finish()]);

        self.cold = false;
        self.ready = true;
    }

    /// Instrument only: the planted textureless segment.
    fn flatten(&self, queue: &wgpu::Queue, from: u32, to: u32) {
        let width = to.saturating_sub(from).min(COLUMNS.saturating_sub(from));
        if width == 0 {
            return;
        }
        let flat = vec![128u8; width as usize * ROWS as usize];
        for lens in 0..2 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self._strips[lens][0],
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: from,
                        y: 0,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &flat,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width),
                    rows_per_image: Some(ROWS),
                },
                wgpu::Extent3d {
                    width,
                    height: ROWS,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn block_for(&self, rung: Rung, span_deg: f64) -> Pass {
        let (cols, rows) = grid(rung.level);
        let (lo, hi) = capture(rung.level, span_deg);
        let (seed_cols, seed_rows, seed_gain, seed_step) = match rung.seed {
            None => (0, 0, 1.0, 1),
            Some(Seed::Coarser) => {
                let (c, r) = grid(rung.level + 1);
                (c, r, 2.0, 2)
            }
            Some(Seed::Previous) => (cols, rows, 1.0, 1),
        };
        Pass {
            cols,
            rows,
            iters: rung.iters,
            seeded: u32::from(rung.seed.is_some()),
            width: (COLUMNS >> rung.level) as f32,
            height: (ROWS >> rung.level) as f32,
            seed_cols,
            seed_rows,
            seed_gain,
            seed_step,
            lo,
            hi,
            gate_rms: GATE_RMS,
            ..Pass::default()
        }
    }

    fn search_group(&self, device: &wgpu::Device, rung: Rung, slot: usize) -> wgpu::BindGroup {
        let seed = match rung.seed {
            None => &self.nothing,
            Some(Seed::Coarser) => &self.answers[rung.level as usize + 1],
            Some(Seed::Previous) => &self.previous,
        };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("belt search"),
            layout: &self.search_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.slot(slot),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &self.strip_views[0][rung.level as usize],
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &self.strip_views[1][rung.level as usize],
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: seed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.answers[rung.output].as_entire_binding(),
                },
            ],
        })
    }

    fn slot(&self, index: usize) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.uniforms,
            offset: index as u64 * SLOT,
            size: wgpu::BufferSize::new(std::mem::size_of::<Pass>() as u64),
        })
    }
}

fn attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
        },
    }
}

fn bytes_of(values: &[f32]) -> &[u8] {
    // `f32` has no padding and no invalid pattern.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

// ---------------------------------------------------------------------------
// The shaders.
// ---------------------------------------------------------------------------

/// One triangle over the whole target, so every render pass here is a fragment
/// shader with no vertex buffer.
const FULLSCREEN: &str = r#"
struct Pass {
  cols: u32, rows: u32, iters: u32, seeded: u32,
  width: f32, height: f32, seed_cols: u32, seed_rows: u32,
  seed_gain: f32, seed_step: u32, frame_width: f32, frame_height: f32,
  lo: vec2<f32>, hi: vec2<f32>,
  source: vec2<f32>, dest: vec2<f32>,
  wide: f32, keep: f32, upward: f32, gate_rms: f32,
};
@group(0) @binding(0) var<uniform> p: Pass;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
  let y = f32(index & 2u) * 2.0 - 1.0;
  return vec4<f32>(x, y, 0.0, 1.0);
}
"#;

/// **Stage 1, rectification.** One strip texel reads where it comes from out of
/// the map and takes one bilinear sample of that lens's luma plane.
const RECTIFY: &str = r#"
@group(0) @binding(1) var lut: texture_2d<f32>;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

// P010 is imported two bytes at a time because the device cannot make a 16-bit
// normalized texture (`dmabuf::plane_format`), so a wide plane's luma is a
// little endian word read back out of two 8-bit components. The same
// arithmetic as `plane_word` in the draw's own half; three lines rather than a
// dependency between two pipelines that share no module.
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let cell = vec2<i32>(i32(at.x), i32(at.y));
  let pixel = textureLoad(lut, cell, 0).xy;
  let uv = (pixel + vec2<f32>(0.5)) / vec2<f32>(p.frame_width, p.frame_height);
  let raw = textureSampleLevel(src, samp, uv, 0.0);
  var luma = raw.r;
  if p.wide > 0.5 {
    luma = (raw.r + raw.g * 256.0) * 255.0 / 65472.0;
  }
  return vec4<f32>(luma, 0.0, 0.0, 1.0);
}
"#;

/// **Stage 2, the pyramid.** Half each way, four taps.
const DOWN: &str = r#"
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = at.xy / p.dest;
  let nudge = 0.5 / p.source;
  var sum = 0.0;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>(-nudge.x, -nudge.y), 0.0).r;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>( nudge.x, -nudge.y), 0.0).r;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>(-nudge.x,  nudge.y), 0.0).r;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>( nudge.x,  nudge.y), 0.0).r;
  return vec4<f32>(sum * 0.25, 0.0, 0.0, 1.0);
}
"#;

/// **Stage 3, the inverse search.**
///
/// Inverse-compositional Lucas-Kanade, which is what the "inverse" in DIS's
/// inverse search means: the Hessian is built once from the anchor's own
/// gradients and inverted once, and each iteration costs one warped bilinear
/// fetch per patch pixel plus a two-element reduction. Nothing follows it,
/// because this design has no variational refinement - the record refuses it
/// on the clock and rung-0 priced the alternative.
///
/// One workgroup per patch, [`PER`] patch pixels a lane, the anchor and its
/// two gradients in registers, the update reduced in shared memory. The strip
/// wraps along the ring and does not wrap across the seam, and both halves of
/// that are in `tap` and in the sampler's address modes.
fn search_wgsl() -> String {
    let fold = LANES / 2;
    format!(
        r##"
const WG: u32 = {LANES}u;
const PER: u32 = {PER}u;

struct Pass {{
  cols: u32, rows: u32, iters: u32, seeded: u32,
  width: f32, height: f32, seed_cols: u32, seed_rows: u32,
  seed_gain: f32, seed_step: u32, frame_width: f32, frame_height: f32,
  lo: vec2<f32>, hi: vec2<f32>,
  source: vec2<f32>, dest: vec2<f32>,
  wide: f32, keep: f32, upward: f32, gate_rms: f32,
}};
@group(0) @binding(0) var<uniform> p: Pass;
@group(0) @binding(1) var held: texture_2d<f32>;
@group(0) @binding(2) var moving: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<storage, read> seed: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> answer: array<vec4<f32>>;

var<workgroup> scratch: array<vec4<f32>, {LANES}>;
var<workgroup> flow: vec2<f32>;

fn tap(x: i32, y: i32) -> f32 {{
  let w = i32(p.width);
  let h = i32(p.height);
  let cx = ((x % w) + w) % w;
  let cy = clamp(y, 0, h - 1);
  return textureLoad(held, vec2<i32>(cx, cy), 0).r;
}}

@compute @workgroup_size({LANES})
fn search(@builtin(workgroup_id) group: vec3<u32>,
          @builtin(local_invocation_index) lane: u32) {{
  let px = group.x;
  let py = group.y;
  let ox = i32(px * 3u);
  let oy = i32(py * 3u);

  var anchor: array<f32, {PER}>;
  var gradx: array<f32, {PER}>;
  var grady: array<f32, {PER}>;
  var xs: array<i32, {PER}>;
  var ys: array<i32, {PER}>;
  var hess = vec4<f32>(0.0, 0.0, 0.0, 0.0);
  for (var q = 0u; q < PER; q = q + 1u) {{
    let cell = lane + q * WG;
    let x = ox + i32(cell & 7u);
    let y = oy + i32(cell >> 3u);
    xs[q] = x;
    ys[q] = y;
    anchor[q] = tap(x, y);
    let gx = 0.5 * (tap(x + 1, y) - tap(x - 1, y));
    let gy = 0.5 * (tap(x, y + 1) - tap(x, y - 1));
    gradx[q] = gx;
    grady[q] = gy;
    hess = hess + vec4<f32>(gx * gx, gx * gy, gy * gy, 0.0);
  }}

  scratch[lane] = hess;
  workgroupBarrier();
  var fold = {fold}u;
  loop {{
    if fold == 0u {{ break; }}
    if lane < fold {{ scratch[lane] = scratch[lane] + scratch[lane + fold]; }}
    workgroupBarrier();
    fold = fold >> 1u;
  }}
  let whole = scratch[0];
  workgroupBarrier();

  let det = whole.x * whole.z - whole.y * whole.y;
  let inv = select(0.0, 1.0 / det, abs(det) > 1e-9);
  let i00 =  whole.z * inv;
  let i01 = -whole.y * inv;
  let i11 =  whole.x * inv;

  if lane == 0u {{
    var start = vec2<f32>(0.0, 0.0);
    if p.seeded != 0u && p.seed_cols > 0u {{
      let sx = min(px / p.seed_step, p.seed_cols - 1u);
      let sy = min(py / p.seed_step, p.seed_rows - 1u);
      start = clamp(seed[sy * p.seed_cols + sx].xy * p.seed_gain, p.lo, p.hi);
    }}
    flow = start;
  }}
  workgroupBarrier();

  var residual = 0.0;
  for (var round = 0u; round < p.iters; round = round + 1u) {{
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    for (var q = 0u; q < PER; q = q + 1u) {{
      let uv = (vec2<f32>(f32(xs[q]) + 0.5, f32(ys[q]) + 0.5) + flow)
             / vec2<f32>(p.width, p.height);
      let warped = textureSampleLevel(moving, samp, uv, 0.0).r;
      let miss = warped - anchor[q];
      acc = acc + vec4<f32>(miss * gradx[q], miss * grady[q], miss * miss, 0.0);
    }}
    scratch[lane] = acc;
    workgroupBarrier();
    var k = {fold}u;
    loop {{
      if k == 0u {{ break; }}
      if lane < k {{ scratch[lane] = scratch[lane] + scratch[lane + k]; }}
      workgroupBarrier();
      k = k >> 1u;
    }}
    let b = scratch[0];
    residual = b.z;
    if lane == 0u {{
      // Clamped to the capture range the design declares, in strip pixels of
      // this level. Without it an unconstrained Lucas-Kanade wanders about a
      // patch width on decimated content and reports the wander as a reading.
      flow = clamp(
        flow - vec2<f32>(i00 * b.x + i01 * b.y, i01 * b.x + i11 * b.y),
        p.lo, p.hi);
    }}
    workgroupBarrier();
  }}

  if lane == 0u {{
    answer[py * p.cols + px] = vec4<f32>(
      flow.x, flow.y, sqrt(residual / 64.0), whole.x + whole.z);
  }}
}}
"##
    )
}

/// **Stage 4, the gate.** One trust per along-seam segment: how many of that
/// segment's patches came back under the residual gate.
///
/// A "row of the belt" in Studio's own vocabulary is a segment ALONG the seam,
/// which is a COLUMN of this strip, because the strip's long axis runs round
/// the ring. One workgroup per column.
fn gate_wgsl() -> String {
    String::from(
        r##"
struct Pass {
  cols: u32, rows: u32, iters: u32, seeded: u32,
  width: f32, height: f32, seed_cols: u32, seed_rows: u32,
  seed_gain: f32, seed_step: u32, frame_width: f32, frame_height: f32,
  lo: vec2<f32>, hi: vec2<f32>,
  source: vec2<f32>, dest: vec2<f32>,
  wide: f32, keep: f32, upward: f32, gate_rms: f32,
};
@group(0) @binding(0) var<uniform> p: Pass;
@group(0) @binding(1) var<storage, read> answers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> trust: array<vec4<f32>>;

var<workgroup> tally: array<vec4<f32>, 64>;

@compute @workgroup_size(64)
fn gate(@builtin(workgroup_id) group: vec3<u32>,
        @builtin(local_invocation_index) lane: u32) {
  let col = group.x;
  var kept = 0.0;
  var seen = 0.0;
  var row = lane;
  loop {
    if row >= p.rows { break; }
    let entry = answers[row * p.cols + col];
    seen = seen + 1.0;
    if entry.z < p.gate_rms && entry.w > 1e-5 {
      kept = kept + 1.0;
    }
    row = row + 64u;
  }
  tally[lane] = vec4<f32>(kept, seen, 0.0, 0.0);
  workgroupBarrier();
  var fold = 32u;
  loop {
    if fold == 0u { break; }
    if lane < fold { tally[lane] = tally[lane] + tally[lane + fold]; }
    workgroupBarrier();
    fold = fold >> 1u;
  }
  if lane == 0u {
    let t = tally[0];
    let share = t.x / max(t.y, 1.0);
    // FAIL UPWARD. A segment that keeps less than `keep` of its patches does
    // not snap to zero and does not snap to calibration: it takes `upward` of
    // the coarse pyramid's answer instead, which is Studio's own 98/2 measured
    // behaviour and the reason nothing switches on and off underneath a
    // picture that is standing still.
    let up = select(0.0, p.upward, share < p.keep);
    trust[col] = vec4<f32>(share, t.y, up, 0.0);
  }
}
"##,
    )
}

/// **Stage 5, densification.** DIS's own: every pixel is the residual-weighted
/// mean of the patches that cover it, which at patch 8 stride 3 is a 3x3
/// neighbourhood, with an untrusted segment blended toward the coarse
/// pyramid's own answer at that place.
fn densify_wgsl() -> String {
    String::from(
        r##"
@group(0) @binding(1) var<storage, read> answers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> coarse: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> trust: array<vec4<f32>>;

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let first = vec2<i32>((i32(at.x) - 7) / 3, (i32(at.y) - 7) / 3);
  var sum = vec2<f32>(0.0, 0.0);
  var weight = 0.0;
  for (var dy = 0; dy < 3; dy = dy + 1) {
    for (var dx = 0; dx < 3; dx = dx + 1) {
      let cx = (((first.x + dx) % i32(p.cols)) + i32(p.cols)) % i32(p.cols);
      let cy = clamp(first.y + dy, 0, i32(p.rows) - 1);
      let entry = answers[u32(cy) * p.cols + u32(cx)];
      let w = 1.0 / (entry.z + 0.01);
      sum = sum + entry.xy * w;
      weight = weight + w;
    }
  }
  let fine = sum / max(weight, 1e-6);

  // The coarse pyramid's own answer at this place, scaled back up to the
  // finest level's pixels. This is what an untrusted segment fails upward to.
  let sx = min(u32(max(first.x + 1, 0)) / p.seed_step, p.seed_cols - 1u);
  let sy = min(u32(max(first.y + 1, 0)) / p.seed_step, p.seed_rows - 1u);
  let up = coarse[sy * p.seed_cols + sx].xy * p.seed_gain;

  let cx = (((first.x + 1) % i32(p.cols)) + i32(p.cols)) % i32(p.cols);
  let blend = trust[u32(cx)].z;
  return vec4<f32>(mix(fine, up, blend), 0.0, 1.0);
}
"##,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip's own shape, so a change to either number is a change a
    /// reader has to make on purpose.
    #[test]
    fn the_strip_is_the_shape_the_clock_chose() {
        assert_eq!((COLUMNS, ROWS), (4096, 128));
        // rung-0 2: 31.44 source px per degree along the ring and 16.55
        // across, so the belt is 48:1 at native sampling and this strip is
        // 32:1. Along it holds 0.36 of the source and across 0.55.
        let along = f64::from(COLUMNS) / 360.0;
        assert!((along - 11.378).abs() < 0.01, "{along}");
    }

    /// Every level of the ladder has patches in it, and the finest has the
    /// count the cost was measured at.
    #[test]
    fn every_rung_of_the_ladder_has_patches() {
        for level in 0..LEVELS {
            let (cols, rows) = grid(level);
            assert!(cols > 0 && rows > 0, "level {level} is empty");
        }
        assert_eq!(grid(0), (1365, 41));
        assert_eq!(grid(2), (341, 9));
    }

    /// The capture window is the union the design and `band.rs` ask for
    /// between them, in strip pixels of each level.
    #[test]
    fn the_capture_window_refuses_neither_document() {
        let (lo, hi) = capture(0, 14.14);
        assert!((f64::from(hi[0]) - 1.5 * f64::from(COLUMNS) / 360.0).abs() < 1e-3);
        assert!((f64::from(lo[0]) + 1.5 * f64::from(COLUMNS) / 360.0).abs() < 1e-3);
        // Across, the band's own -1.2..+2.6 rather than a symmetric window:
        // the far end is where a pooled calibration's residual sits and the
        // near end is 0.73 m at this baseline.
        assert!(hi[1] > 0.0 && lo[1] < 0.0 && hi[1] > -lo[1]);
    }

    /// The strip is parameterized so that the consuming half can invert it in
    /// closed form, and this is that inverse run against the forward map.
    #[test]
    fn the_strip_parameterization_inverts_exactly() {
        for &phi in &[0.0, 0.7, 3.0, 6.0] {
            for &elevation in &[-0.12, -0.03, 0.0, 0.05, 0.12] {
                let body = direction(phi, elevation);
                let back_phi = f64::from(body[1]).atan2(f64::from(body[0]));
                let back_e = f64::from(body[2]).asin();
                let wrapped = (back_phi - phi).rem_euclid(std::f64::consts::TAU);
                let wrapped = wrapped.min(std::f64::consts::TAU - wrapped);
                assert!(wrapped < 1e-6, "phi {phi} came back {back_phi}");
                assert!((back_e - elevation).abs() < 1e-6, "e {elevation}");
            }
        }
    }

    /// The fade lives wholly outside the handover, which is the property that
    /// makes it affordable: no doubled content is inside the run of picture
    /// the correction is given up over.
    #[test]
    fn the_fade_starts_outside_the_handover() {
        // The X4 Air's own measured span and the shipped 8 degree handover.
        let half_span = 14.14 / 2.0;
        let starts = half_span * (1.0 - f64::from(FADE_SHARE));
        assert!(
            starts > 4.0,
            "the fade starts at {starts} deg, inside the handover"
        );
        assert!(starts < half_span);
    }
}
