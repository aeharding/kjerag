//! The one shader pass the player draws.
//!
//! With no file it is an animated gradient, which proves only that a custom
//! wgpu pass runs inside libcosmic. With a file it reprojects one lens of a
//! real VA-API frame imported by [`super::dmabuf`]: for every output pixel,
//! a view ray through the camera's yaw, pitch and field of view, rotated
//! into the lens frame and pushed through the Mei/UCM model in
//! [`super::projection`], sampled from the NV12 planes and converted to RGB.
//! One pass, no intermediate target.
//!
//! The frames move now (issue #4). A [`Player`] decodes both lenses on its
//! own thread and this file asks it, on every redraw, which pair belongs on
//! screen; [`ScenePipeline::prepare`] imports that pair and binds it. Both
//! lenses are imported. Only lens 0 is sampled, because the shader has one
//! lens in it: issue #27 adds the second binding and the per-ray choice, and
//! the frames are already on the GPU when it does.
//!
//! [`Scene::pump`] takes `&self` and keeps the clock behind a [`RefCell`],
//! which is not how a player would be written on its own. It is how iced's
//! `shader::Program` is shaped: `update` and `draw` both borrow the program
//! immutably, and the pump has to happen inside the redraw pass, before the
//! draw, or the picture is always one refresh behind the clock. The cell is
//! touched from the UI thread only; the decode thread never sees it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kyerag_media::{Cue, Frames, Player, Reader, Stats};
use kyerag_meta::{CalibrationSet, Lens};

use super::projection::{self, Reframe};
use super::{Camera, Extent, Fallible, Nudge, Planes, Size, dmabuf};

/// The lens the shader samples. Issue #27 makes this a per-ray choice.
const LENS: usize = 0;

/// Frames kept alive behind the one being drawn.
///
/// An imported texture aliases the decoder's surface: dropping the
/// [`Frames`] hands that surface back to the decoder, which will write the
/// next picture into it. The GPU may still be reading it, because iced
/// submits after `prepare` returns and presents later still, so a frame is
/// released only once this many newer ones have been bound.
const RETAINED: usize = 3;

/// When the widget should come back, which is the whole of frame pacing:
/// the shell sleeps until the instant the next frame is due rather than
/// polling, so 29.97 fps content costs 29.97 redraws a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Whenever the compositor will take a frame. The gradient animates on
    /// every refresh, and so does playback that is still waiting for its
    /// first decoded frame.
    Refresh,
    /// At this instant, when the frame after the one just taken is due.
    At(Instant),
    /// Nothing changes by itself: paused, ended, or a still frame.
    Never,
}

/// The widget's state, owned by the shell.
pub struct Scene {
    show: Option<Show>,
    /// Wall-clock origin for the no-file gradient.
    started: Instant,
    /// Set while the shell has hidden the controls, which is when the pointer
    /// goes with them: `mouse_interaction` answers `Hidden` instead of `Grab`
    /// (docs/UI.md, "The cursor"). One bit of shell state, and the only one
    /// this crate carries.
    cursor_hidden: bool,
    /// A [`Nudge`] the `View` menu left for the widget, which is where iced
    /// keeps the camera. Read once, by the next redraw.
    nudge: Cell<Option<Nudge>>,
}

/// A file on screen: its calibration, and where its frames come from.
struct Show {
    lens: Arc<Lens>,
    /// The clock and the frame it is showing. See the module docs for why
    /// this is a cell.
    playing: RefCell<Playing>,
}

struct Playing {
    frames: Option<Arc<Frames>>,
    source: Source,
}

enum Source {
    /// Playing, or paused mid-play: a decode thread and a clock. Boxed
    /// because the other arm carries nothing.
    Live(Box<Player>),
    /// One frame, no thread. What the headless instruments use.
    Still,
}

impl Scene {
    /// No file: the animated gradient.
    pub fn blank() -> Self {
        Self {
            show: None,
            started: Instant::now(),
            cursor_hidden: false,
            nudge: Cell::new(None),
        }
    }

    /// Opens a file and starts playing it. Returns as soon as the container
    /// is parsed; the first frames arrive on the decode thread.
    pub fn open(path: &Path) -> Fallible<Self> {
        let mut player = Player::open(path)?;
        let lens = calibrated(path, player.size())?;
        println!(
            "media:  {}, {}x{}, {:.3} fps, {} frames, {:.1} s",
            // The older cameras write one lens per file, so this is 1 as
            // often as it is 2.
            match player.lenses() {
                1 => "1 lens stream".to_owned(),
                n => format!("{n} lens streams"),
            },
            player.size().width,
            player.size().height,
            player.timing().fps(),
            player.timing().frames,
            player.timing().duration().as_secs_f64(),
        );
        // Opening a file plays it, which is what every player does. Space
        // and the control row's button pause it (issue #16).
        player.play();
        Ok(Self {
            show: Some(Show::new(lens, None, Source::Live(Box::new(player)))),
            ..Self::blank()
        })
    }

    /// One frame of a file, decoded on this thread. The headless
    /// instruments render with this, and it takes a [`Cue`] rather than
    /// always giving frame 0 because #8's Studio-diff harness needs to name
    /// the frame it is checking.
    pub fn still(path: &Path, at: Cue) -> Fallible<Self> {
        let mut reader = Reader::open(path)?;
        let lens = calibrated(path, reader.size())?;
        let frames = reader.frame(at)?;
        println!(
            "frame:  {} at {:.3} s",
            frames.index,
            frames.timestamp.as_secs_f64()
        );
        println!("drm:    {}", frames.lenses[LENS].describe());
        Ok(Self {
            show: Some(Show::new(lens, Some(Arc::new(frames)), Source::Still)),
            ..Self::blank()
        })
    }

    /// Takes whichever frame belongs on screen at `now`, and says when to
    /// come back. Call it on every redraw: this is the presentation clock's
    /// only tick.
    pub fn pump(&self, now: Instant) -> Next {
        let Some(show) = &self.show else {
            return Next::Refresh;
        };
        // Out of the cell in one step: the borrow checker splits the fields
        // of a `&mut Playing`, but not those of a `RefMut`.
        let Playing { frames, source } = &mut *show.playing.borrow_mut();
        let Source::Live(player) = source else {
            return Next::Never;
        };
        match player.pump(now) {
            Ok(None) => {}
            Ok(Some(taken)) => *frames = Some(taken),
            Err(e) => {
                eprintln!("kyerag: playback stopped: {e}");
                player.pause(now);
                return Next::Never;
            }
        }
        // The end of the file stops the clock rather than leaving it running
        // against frames that will never arrive.
        if player.is_ended() {
            player.pause(now);
            return Next::Never;
        }
        match (player.is_playing(), player.next_due()) {
            (false, _) => Next::Never,
            (true, Some(due)) => Next::At(due),
            // Playing, but the clock has nothing to measure from yet: the
            // first frame is still being decoded.
            (true, None) => Next::Refresh,
        }
    }

    pub fn toggle_play(&mut self, now: Instant) {
        if let Some(player) = self.player_mut() {
            player.toggle(now);
        }
    }

    pub fn is_playing(&self) -> bool {
        self.player(Player::is_playing).unwrap_or(false)
    }

    pub fn position(&self, now: Instant) -> Duration {
        self.player(|player| player.position(now))
            .unwrap_or_default()
    }

    /// How long the file runs, from the container: the frame count and the
    /// rational frame rate, divided.
    pub fn duration(&self) -> Duration {
        self.player(|player| player.timing().duration())
            .unwrap_or_default()
    }

    /// Hide the pointer along with the controls, or bring both back.
    pub fn hide_cursor(&mut self, hidden: bool) {
        self.cursor_hidden = hidden;
    }

    pub fn is_cursor_hidden(&self) -> bool {
        self.cursor_hidden
    }

    /// Leave a view change for the widget to apply on its next redraw.
    pub fn nudge(&self, nudge: Nudge) {
        self.nudge.set(Some(nudge));
    }

    pub(crate) fn take_nudge(&self) -> Option<Nudge> {
        self.nudge.take()
    }

    pub fn stats(&self) -> Option<Stats> {
        self.player(Player::stats)
    }

    /// Reading the player needs the cell, so this hands it to a closure
    /// rather than handing out a reference into it.
    fn player<T>(&self, read: impl FnOnce(&Player) -> T) -> Option<T> {
        let show = self.show.as_ref()?;
        let playing = show.playing.borrow();
        match &playing.source {
            Source::Live(player) => Some(read(player)),
            Source::Still => None,
        }
    }

    fn player_mut(&mut self) -> Option<&mut Player> {
        match &mut self.show.as_mut()?.playing.get_mut().source {
            Source::Live(player) => Some(player),
            Source::Still => None,
        }
    }

    pub fn primitive(&self, camera: Camera) -> ScenePrimitive {
        ScenePrimitive {
            elapsed: self.started.elapsed().as_secs_f32(),
            camera,
            view: self.show.as_ref().and_then(Show::view),
        }
    }
}

impl Show {
    fn new(lens: Arc<Lens>, frames: Option<Arc<Frames>>, source: Source) -> Self {
        Self {
            lens,
            playing: RefCell::new(Playing { frames, source }),
        }
    }

    fn view(&self) -> Option<View> {
        Some(View {
            lens: self.lens.clone(),
            frames: self.playing.borrow().frames.clone()?,
        })
    }
}

/// The calibration for the lens the shader samples, checked against the
/// stream it will be sampled from.
fn calibrated(path: &Path, size: Size) -> Fallible<Arc<Lens>> {
    let calibration = CalibrationSet::from_insv(path)?;
    // The calibration's pixel numbers are already in delivered-frame
    // coordinates, so they describe this texture only if the stream is the
    // size the trailer says it is. A mismatch reprojects at the wrong scale,
    // which reads as a mild lens error rather than a bug.
    if (calibration.dimension.width, calibration.dimension.height) != (size.width, size.height) {
        return Err(format!(
            "trailer says lens frames are {}x{} but the stream decodes {}x{}",
            calibration.dimension.width, calibration.dimension.height, size.width, size.height
        )
        .into());
    }
    let lens = calibration
        .lenses
        .get(LENS)
        .ok_or("calibration describes no lens 0")?
        .clone();
    println!(
        "lens:   {} {}, lens {LENS} of {}",
        calibration.camera_model,
        calibration.firmware,
        calibration.lenses.len(),
    );
    Ok(Arc::new(lens))
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    elapsed: f32,
    camera: Camera,
    view: Option<View>,
}

/// A pair of decoded lenses and the calibration that reprojects them. Both
/// halves are shared, so a redraw that changes nothing but the camera costs
/// two atomic increments.
#[derive(Clone, Debug)]
struct View {
    lens: Arc<Lens>,
    frames: Arc<Frames>,
}

/// The GPU state behind the widget. iced builds one of these per primitive
/// type and keeps it for the life of the renderer.
pub struct ScenePipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// The frame the bind group points at, and the ones still in flight
    /// behind it. Newest first.
    live: VecDeque<Live>,
    /// Set when an import fails, so the message is printed once rather than
    /// on every redraw.
    failed: bool,
    linearize: bool,
    reported: bool,
}

/// One frame on the GPU. The mapped frames must outlive the textures
/// imported from them, and both must outlive the passes that read them,
/// which is what [`RETAINED`] is about.
struct Live {
    frames: Arc<Frames>,
    _planes: Vec<Planes>,
}

impl ScenePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{SHADER}", projection::wgsl()).into()),
        });
        let layout = bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
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
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            // iced's own pass has one sample and no depth attachment
            // (`iced_wgpu/src/lib.rs`, "iced_wgpu render pass"); a pipeline
            // that disagrees fails to draw rather than looking wrong.
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: std::mem::size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Views keep their textures alive, so the blank pair needs no field.
        let bind_group = bind(device, &layout, &uniforms, &blank_planes(device), &sampler);

        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            bind_group,
            live: VecDeque::new(),
            failed: false,
            // iced picks an sRGB surface when it gamma-corrects, and the GPU
            // then encodes whatever the shader writes. Video is already
            // gamma-encoded, so it has to be decoded back to linear first or
            // the picture washes out.
            linearize: format.is_srgb(),
            reported: false,
        }
    }

    /// `aspect` is the output's width over its height, which is what decides
    /// the vertical field of view. The widget reads it from its bounds.
    pub fn prepare(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aspect: f32,
    ) {
        if !self.reported {
            self.reported = true;
            println!("device: {}", dmabuf::device_report(device));
        }
        if let Some(view) = &primitive.view {
            self.show(device, view);
        }

        let reframe = match &primitive.view {
            Some(view) if self.is_bound(view) => Reframe::new(
                &view.lens,
                view.frames.size,
                primitive.camera,
                aspect,
                self.linearize,
            ),
            _ => Reframe::gradient(primitive.elapsed, aspect, self.linearize),
        };
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn is_bound(&self, view: &View) -> bool {
        self.live
            .front()
            .is_some_and(|live| Arc::ptr_eq(&live.frames, &view.frames))
    }

    /// Imports a newly delivered pair and points the bind group at it. A
    /// redraw that shows the same pair again does nothing here.
    fn show(&mut self, device: &wgpu::Device, view: &View) {
        if self.failed || self.is_bound(view) {
            return;
        }
        match self.import(device, view) {
            Ok(planes) => {
                self.bind_group = bind(
                    device,
                    &self.layout,
                    &self.uniforms,
                    &planes[LENS],
                    &self.sampler,
                );
                self.live.push_front(Live {
                    frames: view.frames.clone(),
                    _planes: planes,
                });
                self.live.truncate(RETAINED);
            }
            Err(e) => {
                eprintln!("kyerag: frame not shown: {e}");
                self.failed = true;
            }
        }
    }

    /// Every lens of the pair, not just the one the shader samples: issue
    /// #27 needs lens 1 on the GPU, and importing it here is what shows the
    /// second stream's dmabufs arrive whole.
    fn import(&self, device: &wgpu::Device, view: &View) -> Fallible<Vec<Planes>> {
        view.frames
            .lenses
            .iter()
            .map(|frame| dmabuf::import(device, frame.descriptor(), view.frames.size))
            .collect()
    }
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    planes: &Planes,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view(&planes.luma)),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view(&planes.chroma)),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    // The one place the Rust and WGSL definitions of the
                    // uniform block are checked against each other: pipeline
                    // creation fails if the shader's struct wants more bytes
                    // than `Reframe` has.
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<Reframe>() as u64),
                },
                count: None,
            },
            texture(1),
            texture(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// The shader samples its planes unconditionally, so something has to be bound
/// before a file is. One black pixel each.
fn blank_planes(device: &wgpu::Device) -> Planes {
    let make = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank"),
            size: Size::new(1, 1).extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    Planes {
        luma: make(wgpu::TextureFormat::R8Unorm),
        chroma: make(wgpu::TextureFormat::Rg8Unorm),
    }
}

/// The half of the shader that belongs to this file. `projection::WGSL`
/// declares the uniform block, the view ray and the forward map, and is
/// concatenated ahead of this.
const SHADER: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var out: VsOut;
  out.uv = vec2<f32>(x, y);
  out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

@group(0) @binding(1) var luma: texture_2d<f32>;
@group(0) @binding(2) var chroma: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

fn gradient(uv: vec2<f32>, t: f32) -> vec3<f32> {
  let d = length(uv - vec2<f32>(0.5, 0.5));
  let wave = 0.5 + 0.5 * sin(d * 24.0 - t * 3.0);
  return vec3<f32>(wave * uv.x, wave * uv.y, wave);
}

// BT.709 full range: ffprobe reports bt709 and the camera writes yuvj420p.
// DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr.
fn nv12(uv: vec2<f32>) -> vec3<f32> {
  let y = textureSample(luma, samp, uv).r;
  let c = textureSample(chroma, samp, uv).rg - vec2<f32>(0.5, 0.5);
  return vec3<f32>(
    y + 1.5748 * c.g,
    y - 0.1873 * c.r - 0.4681 * c.g,
    y + 1.8556 * c.r,
  );
}

fn linearize(c: vec3<f32>) -> vec3<f32> {
  let lo = c / 12.92;
  let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(lo, hi, c > vec3<f32>(0.04045));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let landing = project(view_ray(in.uv));
  // Every branch is evaluated because `textureSample` needs uniform control
  // flow; picking afterwards is the WGSL-legal way to write this. A ray that
  // missed the lens would otherwise read a clamped edge texel and smear it
  // across the whole invalid region.
  let lens = select(OUTSIDE_GRAY, nv12(frame_uv(landing.pixel)), landing.inside);
  let rgb = select(gradient(in.uv, reframe.elapsed), lens, reframe.has_frame > 0.5);
  return vec4<f32>(select(rgb, linearize(rgb), reframe.linearize > 0.5), 1.0);
}
"#;
