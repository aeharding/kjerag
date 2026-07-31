//! The one shader pass the player draws.
//!
//! With no file it is an animated gradient, which proves only that a custom
//! wgpu pass runs inside libcosmic. With a file it reprojects a real VA-API
//! frame imported by [`super::dmabuf`]: for every output pixel, a view ray
//! through the camera's yaw, pitch and field of view, rotated into each
//! lens's frame and pushed through the Mei/UCM model in
//! [`super::projection`], sampled from the NV12 planes of whichever lens
//! wins and converted to RGB. One pass, no intermediate target.
//!
//! The frames move (issue #4): a [`Player`] decodes both lenses on its own
//! thread and this file asks it, on every redraw, which pair belongs on
//! screen; [`ScenePipeline::prepare`] imports that pair and binds it. Both
//! lenses are sampled as well as imported, so the picture is the whole
//! sphere (issue #27), and where the two overlap the pass mixes them by the
//! weight field in [`super::projection`] rather than picking one (issue #7).
//! Outside the overlap that weight is exactly 1 and the fetch is the single
//! fetch it always was.
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

use kyerag_media::{Accuracy, Cue, Frames, Player, Reader, Stats};
use kyerag_meta::{CalibrationSet, Lens};

use super::capture::{self, Order, Pending, Request, Shutter, Stamp};
use super::projection::{self, MAX_LENSES, Reframe};
use super::{Camera, Extent, Fallible, Nudge, Planes, Size, dmabuf};

/// The sampler binding, which sits after every lens's two planes.
const SAMPLER_BINDING: u32 = 1 + 2 * MAX_LENSES as u32;

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
    /// Where a capture waits for the redraw that takes it (issue #15).
    shutter: Shutter,
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
    /// One per decoded stream, in stream order.
    lenses: Arc<[Lens]>,
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
            shutter: Shutter::default(),
            cursor_hidden: false,
            nudge: Cell::new(None),
        }
    }

    /// Opens a file and starts playing it. Returns as soon as the container
    /// is parsed; the first frames arrive on the decode thread.
    pub fn open(path: &Path) -> Fallible<Self> {
        let mut player = Player::open(path)?;
        let lenses = calibrated(path, player.size(), player.lenses())?;
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
            show: Some(Show::new(lenses, None, Source::Live(Box::new(player)))),
            ..Self::blank()
        })
    }

    /// One frame of a file, decoded on this thread. The headless
    /// instruments render with this, and it takes a [`Cue`] rather than
    /// always giving frame 0 because #8's Studio-diff harness needs to name
    /// the frame it is checking.
    pub fn still(path: &Path, at: Cue) -> Fallible<Self> {
        let mut reader = Reader::open(path)?;
        let lenses = calibrated(path, reader.size(), reader.lenses())?;
        let frames = reader.frame(at)?;
        println!(
            "frame:  {} at {:.3} s",
            frames.index,
            frames.timestamp.as_secs_f64()
        );
        for frame in &frames.lenses {
            println!("drm:    {}", frame.describe());
        }
        Ok(Self {
            show: Some(Show::new(lenses, Some(Arc::new(frames)), Source::Still)),
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
            // A seek is outstanding: the picture is about to change even
            // though no clock is running towards it, so keep asking until it
            // does. This is what makes a scrub visible while paused, and
            // dragging the scrubber pauses.
            (false, _) if player.is_seeking() => Next::Refresh,
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

    pub fn play(&mut self) {
        if let Some(player) = self.player_mut() {
            player.play();
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(player) = self.player_mut() {
            player.pause(now);
        }
    }

    /// Move the picture, to a keyframe while a drag is still going and to the
    /// frame itself when it ends (issue #5).
    pub fn seek(&mut self, to: Duration, accuracy: Accuracy) {
        if let Some(player) = self.player_mut() {
            player.seek(Cue::Time(to), accuracy);
        }
    }

    /// One frame forward or back.
    pub fn step(&mut self, now: Instant, frames: i64) {
        if let Some(player) = self.player_mut() {
            player.step(now, frames);
        }
    }

    /// A seek has been asked for and has not landed. The shell keeps the
    /// picture redrawing while this is true.
    pub fn is_seeking(&self) -> bool {
        self.player(Player::is_seeking).unwrap_or(false)
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

    /// Asks for a still of whatever the next redraw draws, at the size the
    /// request names. The pixels come back on a worker thread, through the
    /// request's own `then`; nothing here waits.
    pub fn capture(&self, request: Request) {
        self.shutter.arm(request);
    }

    pub fn primitive(&self, camera: Camera) -> ScenePrimitive {
        ScenePrimitive {
            elapsed: self.started.elapsed().as_secs_f32(),
            camera,
            view: self.show.as_ref().and_then(Show::view),
            shutter: self.shutter.clone(),
        }
    }
}

impl Show {
    fn new(lenses: Arc<[Lens]>, frames: Option<Arc<Frames>>, source: Source) -> Self {
        Self {
            lenses,
            playing: RefCell::new(Playing { frames, source }),
        }
    }

    fn view(&self) -> Option<View> {
        Some(View {
            lenses: self.lenses.clone(),
            frames: self.playing.borrow().frames.clone()?,
        })
    }
}

/// The calibration for the lenses the shader samples, checked against the
/// streams they will be sampled from.
///
/// One entry per decoded stream, and in the same order: the trailer writes
/// its lens blocks in the order the container carries the streams. A camera
/// that writes one lens per file (the ONE X2 and older) calibrates two
/// lenses in a file that decodes one, and then this is lens 0 alone and the
/// picture is one hemisphere.
fn calibrated(path: &Path, size: Size, streams: usize) -> Fallible<Arc<[Lens]>> {
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
    let sampled = streams.min(MAX_LENSES);
    let lenses = calibration
        .lenses
        .get(..sampled)
        .ok_or_else(|| {
            format!(
                "file decodes {streams} lens streams but the trailer calibrates {}",
                calibration.lenses.len()
            )
        })?
        .to_vec();
    println!(
        "lens:   {} {}, sampling {sampled} of {} calibrated",
        calibration.camera_model,
        calibration.firmware,
        calibration.lenses.len(),
    );
    Ok(lenses.into())
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    elapsed: f32,
    camera: Camera,
    view: Option<View>,
    /// A handle on the [`Scene`]'s shutter, not a copy of it: the request
    /// is taken by whichever redraw reaches [`ScenePipeline::prepare`]
    /// first, and one that never does is still armed for the next.
    shutter: Shutter,
}

/// A pair of decoded lenses and the calibration that reprojects them. Both
/// halves are shared, so a redraw that changes nothing but the camera costs
/// two atomic increments.
#[derive(Clone, Debug)]
struct View {
    lenses: Arc<[Lens]>,
    frames: Arc<Frames>,
}

/// The GPU state behind the widget. iced builds one of these per primitive
/// type and keeps it for the life of the renderer.
pub struct ScenePipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// One black pixel, bound wherever a lens has no stream: before a file is
    /// open, and in the second slot of a file that carries one lens.
    blank: Planes,
    bind_group: wgpu::BindGroup,
    /// The frame the bind group points at, and the ones still in flight
    /// behind it. Newest first.
    live: VecDeque<Live>,
    /// Set when an import fails, so the message is printed once rather than
    /// on every redraw.
    failed: bool,
    /// The target this pass was built for, which is iced's surface format.
    /// A capture renders into a texture of the same format, so that what it
    /// reads back is what the compositor would have been handed.
    format: wgpu::TextureFormat,
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
        let blank = blank_planes(device);
        let bind_group = bind(device, &layout, &uniforms, [&blank; MAX_LENSES], &sampler);

        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            blank,
            bind_group,
            live: VecDeque::new(),
            failed: false,
            format,
            reported: false,
        }
    }

    /// iced picks an sRGB surface when it gamma-corrects, and the GPU then
    /// encodes whatever the shader writes. Video is already gamma-encoded,
    /// so it has to be decoded back to linear first or the picture washes
    /// out.
    fn linearize(&self) -> bool {
        self.format.is_srgb()
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
                &view.lenses,
                view.frames.size,
                primitive.camera,
                aspect,
                self.linearize(),
            ),
            _ => Reframe::gradient(primitive.elapsed, aspect, self.linearize()),
        };
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());

        // After the uniform write, and only after it: the write lands at the
        // next submit on this queue, and the capture's own submit is that
        // one. Taken here rather than in `draw` because this is the call
        // that has a device to render with.
        if let Some(request) = primitive.shutter.take() {
            self.shoot(device, queue, request, aspect, primitive.view.as_ref());
        }
    }

    /// Draws the view a second time, offscreen, at the size the capture
    /// asked for. Everything after the submit is the worker thread's
    /// (`super::capture`).
    fn shoot(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: Request,
        aspect: f32,
        view: Option<&View>,
    ) {
        let at = view.map_or_else(Stamp::default, |view| Stamp {
            index: view.frames.index,
            time: view.frames.timestamp,
        });
        capture::deliver(
            self.expose(device, queue, request.width, aspect, at),
            request.then,
        );
    }

    /// The render-thread half: a target, one pass into it, and the copy that
    /// will be read back. The frame it samples is the one the bind group
    /// already points at, and `RETAINED` is what keeps that frame's decoder
    /// surfaces alive long enough for this pass to finish: three frames of
    /// slack against a pass that costs a few milliseconds.
    fn expose(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        aspect: f32,
        at: Stamp,
    ) -> Fallible<Pending> {
        let order = Order::of(self.format)?;
        let size = capture::fitted(width, aspect)?;
        let stride = capture::stride(size.width);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: u64::from(stride) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size.height),
                },
            },
            size.extent(),
        );

        Ok(Pending {
            device: device.clone(),
            _texture: texture,
            readback,
            submission: queue.submit([encoder.finish()]),
            size,
            stride,
            order,
            at,
        })
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
                    // A file with one lens stream leaves the second slot on
                    // the blank pixel. Nothing samples it: `Reframe`'s lens
                    // count says one, and a bind group still needs an entry
                    // for every binding the layout declares.
                    std::array::from_fn(|lens| planes.get(lens).unwrap_or(&self.blank)),
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

    /// Every lens of the pair: both are sampled, one per output pixel.
    fn import(&self, device: &wgpu::Device, view: &View) -> Fallible<Vec<Planes>> {
        view.frames
            .lenses
            .iter()
            .map(|frame| dmabuf::import(device, frame.descriptor(), view.frames.size))
            .collect()
    }
}

/// The uniform block, then each lens's luma and chroma planes in lens order,
/// then the sampler they share. The shader names those textures rather than
/// indexing them, so the count is [`MAX_LENSES`] on both sides.
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    lenses: [&Planes; MAX_LENSES],
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    // The views have to outlive the descriptor that borrows them, so they are
    // built before it rather than inside it.
    let views: Vec<wgpu::TextureView> = lenses
        .iter()
        .flat_map(|planes| [&planes.luma, &planes.chroma])
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    entries.extend(
        views
            .iter()
            .enumerate()
            .map(|(plane, view)| wgpu::BindGroupEntry {
                binding: 1 + plane as u32,
                resource: wgpu::BindingResource::TextureView(view),
            }),
    );
    entries.push(wgpu::BindGroupEntry {
        binding: SAMPLER_BINDING,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &entries,
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
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            // The one place the Rust and WGSL definitions of the uniform
            // block are checked against each other: pipeline creation fails
            // if the shader's struct wants more bytes than `Reframe` has.
            min_binding_size: NonZeroU64::new(std::mem::size_of::<Reframe>() as u64),
        },
        count: None,
    }];
    entries.extend((1..SAMPLER_BINDING).map(texture));
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: SAMPLER_BINDING,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene"),
        entries: &entries,
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

// Two planes per lens, in lens order, then the sampler they share. WGSL has
// no texture array to index here, so `picture` branches on the lens the ray
// picked; `SAMPLER_BINDING` is the Rust half of these numbers.
@group(0) @binding(1) var luma0: texture_2d<f32>;
@group(0) @binding(2) var chroma0: texture_2d<f32>;
@group(0) @binding(3) var luma1: texture_2d<f32>;
@group(0) @binding(4) var chroma1: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;

fn gradient(uv: vec2<f32>, t: f32) -> vec3<f32> {
  let d = length(uv - vec2<f32>(0.5, 0.5));
  let wave = 0.5 + 0.5 * sin(d * 24.0 - t * 3.0);
  return vec3<f32>(wave * uv.x, wave * uv.y, wave);
}

// Each lens's picture at that ray, mixed by its weight, or grey where no lens
// has the ray. A lens weighted zero is not sampled at all, so outside the
// overlap this is the single fetch the hard pick took before issue #7, and
// the second fetch is what the blend band costs.
//
// WGSL has no texture array to index here, so the lenses are named rather
// than looped. The explicit mip level is what makes that legal: a
// `textureSample` computes its own level from derivatives and needs uniform
// control flow to do it, and every one of these textures has a single level
// anyway.
fn picture(mix: Blend) -> vec3<f32> {
  var rgb = vec3<f32>(0.0);
  var total = 0.0;
  if mix.weights[0] > 0.0 {
    rgb += mix.weights[0] * nv12(luma0, chroma0, frame_uv(mix.landings[0].pixel));
    total += mix.weights[0];
  }
  if mix.weights[1] > 0.0 {
    rgb += mix.weights[1] * nv12(luma1, chroma1, frame_uv(mix.landings[1].pixel));
    total += mix.weights[1];
  }
  return select(OUTSIDE_GRAY, rgb, total > 0.0);
}

// BT.709 full range: ffprobe reports bt709 and the camera writes yuvj420p.
// DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr.
fn nv12(luma: texture_2d<f32>, chroma: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
  let y = textureSampleLevel(luma, samp, uv, 0.0).r;
  let c = textureSampleLevel(chroma, samp, uv, 0.0).rg - vec2<f32>(0.5, 0.5);
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
  let lens = picture(blend(view_ray(in.uv)));
  let rgb = select(gradient(in.uv, reframe.elapsed), lens, reframe.has_frame > 0.5);
  return vec4<f32>(select(rgb, linearize(rgb), reframe.linearize > 0.5), 1.0);
}
"#;
