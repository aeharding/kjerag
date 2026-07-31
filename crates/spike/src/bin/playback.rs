//! Headless playback: the player's own frame path, paced and measured.
//!
//! The app's numbers cannot be read off a window, so this runs the same
//! [`Scene`] and [`ScenePipeline`] the shader widget runs, schedules its
//! redraws the way the widget does (sleep until the next frame is due),
//! renders each one offscreen, and reports what playback did: frames
//! presented, frames dropped, redraws that found nothing decoded, and CPU.
//!
//! It also measures the decode side on its own, at several
//! [`Reader::lookahead`] depths, because "keep 2-3 frames in flight to hide
//! the `vaSyncSurface` wait" (docs/ARCHITECTURE.md) is a claim with a number
//! attached, and this is where the number comes from.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin playback -- <file.insv> [seconds] [hz] [shots]
//! ```
//!
//! `shots` is issue #15's measurement: that many screen captures, spread
//! over the run, taken through the same [`Scene::capture`] the `s` key
//! reaches, while the file plays. What has to stay true is the pacing
//! report underneath it, so the number this instrument exists to produce is
//! dropped and starved with a capture burst running.
//!
//! Nothing is written to disk unless `shots` asks for captures, which land
//! in ./scratch/ (gitignored): frames of real footage are personal video
//! and this repo is public.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use kyerag_media::{Fallible, Reader};
use kyerag_render::{
    Camera, Extent, Next, Readout, Request, Scene, ScenePipeline, Shot, Size, Sweep, dmabuf,
};

/// Not sRGB, so the pass writes the video's own numbers: the same choice the
/// `reframe` instrument makes.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A plausible window on this laptop's display. The reprojection pass costs
/// output pixels, so the size is part of the measurement.
const OUTPUT: Size = Size {
    width: 2560,
    height: 1440,
};

/// Pairs each depth of the decode-side benchmark pulls.
const BENCH_PAIRS: usize = 200;

/// What the app asks for (`kyerag::shot::WIDTH`), so the burst costs what a
/// pilot's `s` key costs.
const SHOT_WIDTH: u32 = 3840;

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(
        args.get(1)
            .ok_or("usage: playback <file.insv> [seconds] [hz] [shots] [yaw] [readout] [fov]")?,
    );
    let seconds: u64 = parse(&args, 2, 60)?;
    let hz: u32 = parse(&args, 3, 60)?;
    let shots: u32 = parse(&args, 4, 0)?;
    // Degrees, and the reason it is here is the captures: a view down a
    // lens axis holds one lens, and only a view across the seam proves a
    // still carries both.
    let yaw: f32 = parse(&args, 5, 0.0)?;
    // Issue #9's cost: the file's own readout is what the app uses, and a
    // named sweep forces the correction on over a file whose direction has
    // not been measured, which is what the cost of switching it on is
    // measured through.
    let readout: String = parse(&args, 6, "file".to_owned())?;
    // Degrees. A wide view is the one that puts a large area of both lenses on
    // screen at once, which is where the cost of sampling two of them is
    // largest (issue #10).
    let fov: f32 = parse(&args, 7, Camera::default().fov.to_degrees())?;

    for lookahead in [0, 2, 4] {
        println!("{}", drain(&input, lookahead)?);
    }
    println!();
    let camera = Camera {
        yaw: yaw.to_radians(),
        fov: fov.to_radians(),
        ..Camera::default()
    };
    play(
        &input,
        Duration::from_secs(seconds),
        hz,
        shots,
        camera,
        &readout,
    )
}

fn parse<T: std::str::FromStr>(args: &[String], i: usize, fallback: T) -> Fallible<T>
where
    T::Err: std::fmt::Display,
{
    match args.get(i) {
        None => Ok(fallback),
        Some(raw) => raw
            .parse()
            .map_err(|e| format!("bad argument {i}: {e}").into()),
    }
}

/// Decode as fast as the hardware will go, with `lookahead` frames between
/// a surface being decoded and being mapped. Both lenses, one demuxer, no
/// GPU work: this is the ceiling realtime playback is measured against.
fn drain(input: &Path, lookahead: usize) -> Fallible<String> {
    let mut reader = Reader::open(input)?.lookahead(lookahead);
    let timing = reader.timing();

    let start = Instant::now();
    let mut pairs = 0;
    while pairs < BENCH_PAIRS {
        match reader.next_frames()? {
            Some(_) => pairs += 1,
            None => break,
        }
    }
    let elapsed = start.elapsed();
    let fps = pairs as f64 / elapsed.as_secs_f64();
    // Only now: the frame pool does not exist until a frame has been decoded.
    let pool = reader.pool_size();

    Ok(format!(
        "decode: lookahead {lookahead}: {fps:6.1} pairs/s, {:4.2}x realtime, \
         {:5.2} ms/pair (pool {})",
        fps / timing.fps(),
        elapsed.as_secs_f64() * 1000.0 / pairs as f64,
        pool.map_or_else(|| "?".to_owned(), |n| n.to_string()),
    ))
}

/// The real thing: the same scheduling the shell uses, with every presented
/// pair imported and reprojected.
///
/// The shell sleeps until the instant the scene says the next frame is due
/// (`iced`'s `RedrawRequest::At`, from `kyerag_render`'s widget), so this
/// does too. `hz` is the display's refresh rate, and caps how often a redraw
/// can happen when the scene asks for one as soon as possible.
fn play(
    input: &Path,
    run: Duration,
    hz: u32,
    shots: u32,
    camera: Camera,
    readout: &str,
) -> Fallible<()> {
    let gpu = Gpu::new()?;
    println!("gpu:    {}", gpu.adapter.get_info().name);
    println!("device: {}", dmabuf::device_report(&gpu.device));

    let mut scene = Scene::open(input)?;
    let sweep = match readout {
        "right" => Some(Sweep::Right),
        "left" => Some(Sweep::Left),
        "down" => Some(Sweep::Down),
        "up" => Some(Sweep::Up),
        _ => None,
    };
    if let (Some(sweep), Some(file)) = (sweep, scene.readout()) {
        scene.set_readout(Some(Readout { sweep, ..file }));
    }
    println!("shutter: readout {readout}");
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let refresh = Duration::from_secs_f64(1.0 / f64::from(hz));
    println!(
        "pace:   due-time redraws on a {hz} Hz display for {} s, rendering {}x{} at yaw {:.0}, \
         fov {:.0}",
        run.as_secs(),
        OUTPUT.width,
        OUTPUT.height,
        camera.yaw.to_degrees(),
        camera.fov.to_degrees(),
    );

    let start = Instant::now();
    let cpu = Cpu::now();
    let (mut redraws, mut render) = (0u64, Duration::ZERO);
    let mut burst = Burst::new(shots, run);

    while start.elapsed() < run {
        let now = Instant::now();
        let next = match scene.pump(now) {
            Next::At(due) => due,
            Next::Refresh => now + refresh,
            Next::Never => break,
        };
        let armed = burst.due(start.elapsed());
        if armed {
            scene.capture(burst.request());
        }
        let primitive = scene.primitive(camera);

        let began = Instant::now();
        pipeline.prepare(
            &primitive,
            &gpu.device,
            &gpu.queue,
            OUTPUT.width as f32 / OUTPUT.height as f32,
        );
        burst.prepared(armed, began.elapsed());

        let drawn = Instant::now();
        gpu.render(&pipeline)?;
        render += drawn.elapsed();
        redraws += 1;

        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }

    let elapsed = start.elapsed();
    let stats = scene.stats().ok_or("no player")?;
    println!(
        "play:   {} redraws, {:.2} s played, {}",
        redraws,
        scene.position(Instant::now()).as_secs_f64(),
        stats.report(elapsed),
    );
    println!(
        "cost:   {:.2} ms per redraw in the pass, {:.1}% of one core",
        render.as_secs_f64() * 1000.0 / redraws as f64,
        cpu.percent(elapsed),
    );
    burst.report();
    pause(&mut scene, Duration::from_secs(1));
    Ok(())
}

/// A run of captures during playback, and what they cost the redraw they
/// were armed on.
///
/// The whole of issue #15's performance claim is the difference between the
/// two `prepare` numbers this prints: a capture adds a target, a pass and a
/// copy to one redraw, and everything after the submit belongs to a worker
/// thread. If that difference ever grew to a frame's worth of time the
/// pilot would see the flight stutter as they photographed it.
struct Burst {
    left: u32,
    every: Duration,
    next: Duration,
    written: Sender<String>,
    reports: mpsc::Receiver<String>,
    /// Worst and total `prepare` with a capture armed, and without.
    with: Cost,
    without: Cost,
}

#[derive(Default)]
struct Cost {
    worst: Duration,
    total: Duration,
    count: u32,
}

impl Cost {
    fn add(&mut self, took: Duration) {
        self.worst = self.worst.max(took);
        self.total += took;
        self.count += 1;
    }

    fn mean_ms(&self) -> f64 {
        self.total.as_secs_f64() * 1000.0 / f64::from(self.count.max(1))
    }
}

impl Burst {
    fn new(shots: u32, run: Duration) -> Self {
        let (written, reports) = mpsc::channel();
        // Spread over the run, the first one a beat in so that the file is
        // actually playing when it fires.
        let every = run / shots.max(1);
        Self {
            left: shots,
            every,
            next: every / 2,
            written,
            reports,
            with: Cost::default(),
            without: Cost::default(),
        }
    }

    /// Whether a capture should be armed on the redraw about to happen.
    fn due(&mut self, elapsed: Duration) -> bool {
        if self.left == 0 || elapsed < self.next {
            return false;
        }
        self.left -= 1;
        self.next += self.every;
        true
    }

    fn request(&self) -> Request {
        let written = self.written.clone();
        Request {
            width: SHOT_WIDTH,
            then: Box::new(move |taken| {
                let _ = written.send(match taken.and_then(|shot| write_png(&shot)) {
                    Ok(line) => line,
                    Err(e) => format!("failed: {e}"),
                });
            }),
        }
    }

    fn prepared(&mut self, armed: bool, took: Duration) {
        match armed {
            true => self.with.add(took),
            false => self.without.add(took),
        }
    }

    fn report(&self) {
        if self.with.count == 0 {
            return;
        }
        println!(
            "shots:  {} captures at {SHOT_WIDTH} px, prepare {:.2} ms with one armed \
             against {:.2} ms without (worst {:.2} against {:.2})",
            self.with.count,
            self.with.mean_ms(),
            self.without.mean_ms(),
            self.with.worst.as_secs_f64() * 1000.0,
            self.without.worst.as_secs_f64() * 1000.0,
        );
        // The workers are still developing the last of them, and a capture
        // nobody waited for is not a capture that happened.
        for _ in 0..self.with.count {
            match self.reports.recv() {
                Ok(line) => println!("shot:   {line}"),
                Err(e) => println!("shot:   lost: {e}"),
            }
        }
    }
}

/// Worker thread: the spike's own version of what the app does with a
/// [`Shot`], which is a PNG on disk. No naming policy here; the app owns
/// that (`kyerag::shot`).
fn write_png(shot: &Shot) -> Fallible<String> {
    let began = Instant::now();
    let out = PathBuf::from("scratch").join(format!("playback-frame{}.png", shot.index));
    std::fs::create_dir_all("scratch")?;

    let file = std::io::BufWriter::new(std::fs::File::create(&out)?);
    let mut encoder = png::Encoder::new(file, shot.width, shot.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;

    Ok(format!(
        "{} at {:.3} s, {}x{}, encoded in {:.0} ms",
        out.display(),
        shot.time.as_secs_f64(),
        shot.width,
        shot.height,
        began.elapsed().as_secs_f64() * 1000.0,
    ))
}

/// What the space bar does, without a keyboard: the clock stops where it is
/// and asks for no more redraws, and resuming carries on from there rather
/// than jumping to wall-clock time.
fn pause(scene: &mut Scene, hold: Duration) {
    let now = Instant::now();
    scene.toggle_play(now);
    let held = scene.position(now);

    let until = now + hold;
    let mut asked = 0;
    while Instant::now() < until {
        if scene.pump(Instant::now()) != Next::Never {
            asked += 1;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let woke = Instant::now();
    let drift = scene.position(woke).saturating_sub(held);
    scene.toggle_play(woke);
    scene.pump(Instant::now());
    println!(
        "pause:  held {:.3} s at {:.3} s, clock moved {:.1} ms, {asked} redraws asked for, \
         resumed at {:.3} s",
        hold.as_secs_f64(),
        held.as_secs_f64(),
        drift.as_secs_f64() * 1000.0,
        scene.position(Instant::now()).as_secs_f64(),
    );
}

/// Process CPU time, straight from `/proc/self/stat`: utime and stime are
/// fields 14 and 15, in clock ticks.
struct Cpu(Duration);

impl Cpu {
    fn now() -> Self {
        Self(Self::used())
    }

    fn used() -> Duration {
        let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
            return Duration::ZERO;
        };
        // The second field is the executable name in brackets and may hold
        // spaces, so counting starts after the closing bracket.
        let Some(rest) = stat.rsplit_once(')') else {
            return Duration::ZERO;
        };
        let fields: Vec<&str> = rest.1.split_whitespace().collect();
        let ticks: u64 = [11, 12]
            .iter()
            .filter_map(|i| fields.get(*i)?.parse::<u64>().ok())
            .sum();
        // _SC_CLK_TCK is 100 on every Linux this runs on.
        Duration::from_secs_f64(ticks as f64 / 100.0)
    }

    fn percent(&self, over: Duration) -> f64 {
        (Self::used().saturating_sub(self.0)).as_secs_f64() / over.as_secs_f64() * 100.0
    }
}

/// A device that can import, and one offscreen target to draw into.
struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: wgpu::Texture,
}

impl Gpu {
    fn new() -> Fallible<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        let (device, queue) = dmabuf::open_device(&adapter)?;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("playback"),
            size: OUTPUT.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Ok(Self {
            adapter,
            device,
            queue,
            target,
        })
    }

    /// One pass, waited on. A window would hand this to the compositor
    /// instead of waiting, so this timing is the pessimistic one.
    fn render(&self, pipeline: &ScenePipeline) -> Fallible<()> {
        let view = self.target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("playback"),
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
            pipeline.draw(&mut pass);
        }
        let index = self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })?;
        Ok(())
    }
}
