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
//! cargo run --release -p kyerag-spike --bin playback -- <file.insv> [seconds] [hz]
//! ```
//!
//! Nothing is written to disk: this instrument reports, it does not render
//! pictures of real footage.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kyerag_media::{Fallible, Reader};
use kyerag_render::{Camera, Extent, Next, Scene, ScenePipeline, Size, dmabuf};

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

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(
        args.get(1)
            .ok_or("usage: playback <file.insv> [seconds] [hz]")?,
    );
    let seconds: u64 = parse(&args, 2, 60)?;
    let hz: u32 = parse(&args, 3, 60)?;

    for lookahead in [0, 2, 4] {
        println!("{}", drain(&input, lookahead)?);
    }
    println!();
    play(&input, Duration::from_secs(seconds), hz)
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
fn play(input: &Path, run: Duration, hz: u32) -> Fallible<()> {
    let gpu = Gpu::new()?;
    println!("gpu:    {}", gpu.adapter.get_info().name);
    println!("device: {}", dmabuf::device_report(&gpu.device));

    let mut scene = Scene::open(input)?;
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let refresh = Duration::from_secs_f64(1.0 / f64::from(hz));
    println!(
        "pace:   due-time redraws on a {hz} Hz display for {} s, rendering {}x{}",
        run.as_secs(),
        OUTPUT.width,
        OUTPUT.height
    );

    let start = Instant::now();
    let cpu = Cpu::now();
    let (mut redraws, mut render) = (0u64, Duration::ZERO);

    while start.elapsed() < run {
        let now = Instant::now();
        let next = match scene.pump(now) {
            Next::At(due) => due,
            Next::Refresh => now + refresh,
            Next::Never => break,
        };
        let primitive = scene.primitive(Camera::default());
        pipeline.prepare(
            &primitive,
            &gpu.device,
            &gpu.queue,
            OUTPUT.width as f32 / OUTPUT.height as f32,
        );

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
    pause(&mut scene, Duration::from_secs(1));
    Ok(())
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
