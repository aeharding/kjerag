//! **Rung 0 of the belt: what one frame of it would cost, measured before a
//! line of it is written.**
//!
//! The belt is Studio's "Optical Flow" column, and it is the next thing after
//! the flat seam (docs/research/studio-parity.md 7). Per frame it would:
//! rectify the two lenses' overlap into a common seam-belt strip; run a
//! DIS-class dense match on that strip; densify the per-patch answers into a
//! per-pixel field; gate it per direction; and let the sampler read it as a
//! source-UV correction, GPU-resident, with no readback on the hot path.
//!
//! **This binary builds none of that.** It stands up throwaway probes with the
//! same cost shape and times them with GPU timestamp queries, so that the
//! design decision has a clock in front of it. The project's hardest lesson is
//! that a build gets refused on a clock nobody measured first (issue #171:
//! *"the performance of this approach is unworkable"*, said after the
//! mechanism had already been proved). This is that clock, taken at design
//! time. docs/research/belt-rung0.md is what it produced.
//!
//! ```sh
//! # what the strip has to be, off the file's own calibration
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=geometry
//!
//! # per-stage GPU cost at each candidate strip size, on real decoded frames
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=cost
//!
//! # the same probe with the real file playing through the app's own pass
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=live belt=on
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=live belt=off
//! ```
//!
//! **Every stage is timed by a timestamp pair the pass writes itself.** Nothing
//! here is inferred from a wall clock around a submit: the repo has been caught
//! by that before (`ScenePipeline::band_repeats` exists because a redraw's wall
//! time on a loaded box is the pass plus whatever else ran). Nothing in the
//! tree used timestamp queries before this file, so the device it opens is
//! `dmabuf::open_device`'s with `TIMESTAMP_QUERY` added, which is why that
//! function is copied into [`Gpu::open`] rather than called.
//!
//! **What is faithful and what is an approximation**, stated here because a
//! cost probe that flatters its design is worse than no probe:
//!
//! - the rectification map is exact. It is `Reframe::project` run over the
//!   file's own calibration ([`Lut`]), and it is a body-frame object, so it is
//!   built once when a file opens and read every frame;
//! - the search is a real inverse-compositional Lucas-Kanade over 8x8 patches
//!   at stride 3: one workgroup per patch, one lane per patch pixel, the
//!   template and its two gradients in registers, the Hessian built once from
//!   the template and inverted once, and one warped bilinear fetch per pixel
//!   per iteration reduced in shared memory. That is DIS's inner loop and DIS's
//!   memory pattern. There is no variational refinement, because the design
//!   does not have one;
//! - the consuming lookup is measured as a **difference** between two arms of
//!   one pass, so the two lens samples the shipped pass already pays for
//!   cancel and what is left is the belt's own share;
//! - what is NOT faithful: `mode=live` runs the belt over one fixed real frame
//!   while the player decodes and draws every frame of the film. That arm
//!   measures contention, and `mode=cost` measures the answer, on consecutive
//!   real frames.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kjerag_media::{Fallible, Plane, Size, Walk};
use kjerag_meta::CalibrationSet;
use kjerag_render::{
    Camera, Extent, Next, Reframe, Ring, Scene, ScenePipeline, band, dmabuf, seam,
};

/// Patch side, in strip pixels. DIS's `theta_ps`, and the design's own figure.
const PATCH: u32 = 8;

/// How far apart patch origins sit. DIS's overlap parameter written as a
/// stride; the design's own figure.
const STRIDE: u32 = 3;

/// Threads per search workgroup: one per patch pixel.
const LANES: u32 = PATCH * PATCH;

/// The shaders below index a lane with `lane & 7` and `lane >> 3`, reduce over
/// 64 entries, and multiply patch coordinates by a literal 3. Those literals
/// are these constants, and this is what says so.
const _: () = assert!(PATCH == 8 && STRIDE == 3 && LANES == 64);

/// The residual a patch has to come back under to count. In units of the 0..1
/// luma the strip holds; the belt's version of the band's `KEEP`.
const GATE_RMS: f32 = 0.06;

/// What a redraw has to fit inside. The shipped pass measures 8.14 to 8.15 ms
/// per redraw at 2560x1440 under live decode on this box class, and the branch
/// stage 9 was taken on read 8.44, so the belt's per-frame cost is quoted as a
/// fraction of this.
const PASS_MS: f64 = 8.15;

/// A 30 fps frame, which is what every file in the corpus is.
const FRAME_MS: f64 = 1000.0 / 30.0;

/// A plausible window on this laptop, and `--bin playback`'s own, so the
/// consuming lookup is charged over the same picture the 8.15 ms was.
const OUTPUT: Size = Size {
    width: 2560,
    height: 1440,
};

const LUMA: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const MAPPED: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const FIELD: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode.as_str() {
        "geometry" => geometry(&options),
        "cost" => cost(&options),
        "live" => live(&options),
        other => Err(format!("no mode called {other}. {USAGE}").into()),
    }
}

const USAGE: &str = "usage: belt <file.insv> [mode=geometry|cost|live] [at=seconds] \
     [frames=n] [iters=n] [seed=n] [res=WxH,WxH] [seconds=n] [hz=n] [belt=seeded|cold|off] [every=n]";

struct Options {
    input: PathBuf,
    mode: String,
    at: f64,
    frames: usize,
    iters: u32,
    seed_iters: u32,
    sizes: Vec<Size>,
    seconds: u64,
    hz: u32,
    belt: String,
    /// One belt in every this many frames. 1 is our target, 2 and 3 are
    /// Studio's own cadence with a pure hold between.
    every: u64,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            mode: "cost".to_owned(),
            // The owner's own `down1` view sits here on the May-01 file, which
            // is the content the seam work has been judged on all week.
            at: 63.5,
            frames: 6,
            iters: 8,
            seed_iters: 3,
            sizes: vec![
                Size::new(2048, 128),
                Size::new(4096, 256),
                Size::new(8192, 512),
            ],

            seconds: 20,
            hz: 60,
            belt: "seeded".to_owned(),
            every: 1,
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", v)) => options.mode = v.to_owned(),
                Some(("at", v)) => options.at = v.parse()?,
                Some(("frames", v)) => options.frames = v.parse()?,
                Some(("iters", v)) => options.iters = v.parse()?,
                Some(("seed", v)) => options.seed_iters = v.parse()?,
                Some(("seconds", v)) => options.seconds = v.parse()?,
                Some(("hz", v)) => options.hz = v.parse()?,
                Some(("belt", v)) => options.belt = v.to_owned(),
                Some(("every", v)) => options.every = v.parse::<u64>()?.max(1),
                Some(("res", v)) => {
                    options.sizes = v
                        .split(',')
                        .map(|one| {
                            let (w, h) = one.split_once('x').ok_or("res wants WxH")?;
                            Ok(Size::new(w.parse()?, h.parse()?))
                        })
                        .collect::<Fallible<_>>()?;
                }
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        Ok(options)
    }
}

// ---------------------------------------------------------------------------
// 1. The geometry: what the strip has to be, off the file's own calibration.
// ---------------------------------------------------------------------------

/// One azimuth of the seam ring, measured through the real map.
struct Spoke {
    /// Where both lenses still have the picture, in degrees across the seam.
    both: (f64, f64),
    /// Source pixels per degree, per lens, along the ring.
    along: [f64; 2],
    /// The same across the seam.
    across: [f64; 2],
}

/// The seam ring as the belt would have to see it: for every azimuth, how wide
/// the shared picture is, and how many pixels of each lens's own raster a
/// degree of world angle is worth there.
///
/// **The axes are the band's**, not new ones: `Ring::perp` is the seam circle's
/// own tangent (along) and `Ring::epi` is the epipolar axis (across), which is
/// the only axis a subject's distance can displace content along
/// (docs/research/seam-two-axis.md 1). Rectifying onto anything else would put
/// the search's long axis somewhere parallax is not.
///
/// The scale is a central difference of `Reframe::project` at 0.01 degrees,
/// which is the method `kjerag_spike::crossing::source_scale` uses; it is
/// written out here because that function takes a traced contour `Site` and
/// this wants a plain azimuth.
fn spokes(map: &Reframe, baseline: [f32; 3], count: usize) -> Vec<Spoke> {
    const PROBE_DEG: f64 = 0.01;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let phi = index as f64 / count as f64 * std::f64::consts::TAU;
        let ring = Ring::of(phi as f32, baseline);
        let at = |across_deg: f64| {
            let (sin, cos) = across_deg.to_radians().sin_cos();
            let body: [f32; 3] = std::array::from_fn(|axis| {
                ring.centre[axis] * cos as f32 + ring.epi[axis] * sin as f32
            });
            map.view_ray_from_body(unit(body))
        };
        // Walked out from the seam in hundredths of a degree, so the answer is
        // the real boundary of the shared picture and not a cap angle quoted at
        // it.
        let mut hi = 0.0;
        let mut lo = 0.0;
        let mut walk = 0.0;
        while walk < 15.0 {
            walk += 0.01;
            if map.within(0, at(walk)) && map.within(1, at(walk)) {
                hi = walk;
            } else {
                break;
            }
        }
        walk = 0.0;
        while walk > -15.0 {
            walk -= 0.01;
            if map.within(0, at(walk)) && map.within(1, at(walk)) {
                lo = walk;
            } else {
                break;
            }
        }

        let scale = |axis: [f32; 3], lens: usize| -> f64 {
            let probe = PROBE_DEG.to_radians() as f32;
            let side = |sign: f32| {
                let body: [f32; 3] =
                    std::array::from_fn(|k| ring.centre[k] + sign * probe * axis[k]);
                map.project(lens, map.view_ray_from_body(unit(body))).pixel
            };
            let (low, high) = (side(-1.0), side(1.0));
            let dx = f64::from(high[0] - low[0]);
            let dy = f64::from(high[1] - low[1]);
            // px per radian, then per degree.
            (dx * dx + dy * dy).sqrt() / (2.0 * f64::from(probe)) * std::f64::consts::PI / 180.0
        };

        out.push(Spoke {
            both: (lo, hi),
            along: [scale(ring.perp, 0), scale(ring.perp, 1)],
            across: [scale(ring.epi, 0), scale(ring.epi, 1)],
        });
    }
    out
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    match n > 0.0 {
        true => v.map(|c| c / n),
        false => v,
    }
}

/// Smallest, middle, largest.
fn spread(values: &[f64]) -> (f64, f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (
        sorted[0],
        sorted[sorted.len() / 2],
        sorted[sorted.len() - 1],
    )
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

/// How much wider than the true shared picture [`Reframe::within`] answers.
///
/// That test is deliberately conservative: `projection::CAP_MARGIN_DEG` widens
/// each lens's coverage cap by half a degree, so the interval where both
/// answer true is a whole degree wider than the interval where both really
/// have the picture. A strip built on the generous figure would sample off the
/// end of a fisheye circle at every azimuth, so it is taken back off here.
const CAP_MARGIN_BOTH_DEG: f64 = 1.0;

/// The narrowest azimuth's shared picture, which is as wide as a strip may be:
/// anything wider asks a lens for pixels it does not have somewhere on the
/// ring.
fn span_of(spokes: &[Spoke]) -> f64 {
    spokes
        .iter()
        .map(|s| s.both.1 - s.both.0)
        .fold(f64::MAX, f64::min)
        - CAP_MARGIN_BOTH_DEG
}

fn geometry(options: &Options) -> Fallible<()> {
    let calibration = CalibrationSet::from_insv(&options.input)?;
    // `CalibrationSet::dimension` is the trailer layer's own `Size`, which is a
    // different type from the frame layer's; one conversion, at the boundary.
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let map = seam::mapped(&calibration.lenses, frame);
    let baseline = band::baseline(&calibration.lenses);
    let reach =
        (baseline[0] * baseline[0] + baseline[1] * baseline[1] + baseline[2] * baseline[2]).sqrt();
    println!(
        "camera: {} fw {}, delivered frame {}x{}, {} lenses",
        calibration.camera_model,
        calibration.firmware,
        frame.width,
        frame.height,
        calibration.lenses.len(),
    );
    println!(
        "seam:   model overlap {:.3} deg, drawn handover {:.2} deg, baseline {:.2} mm",
        map.overlap().unwrap_or(0.0).to_degrees(),
        map.handover_width().to_degrees(),
        reach * 1000.0,
    );

    let spokes = spokes(&map, baseline, band::AZIMUTHS);
    let widths: Vec<f64> = spokes.iter().map(|s| s.both.1 - s.both.0).collect();
    let highs: Vec<f64> = spokes.iter().map(|s| s.both.1).collect();
    let lows: Vec<f64> = spokes.iter().map(|s| -s.both.0).collect();
    let along: Vec<f64> = spokes
        .iter()
        .flat_map(|s| [s.along[0], s.along[1]])
        .collect();
    let across: Vec<f64> = spokes
        .iter()
        .flat_map(|s| [s.across[0], s.across[1]])
        .collect();
    let (wlo, wmid, whi) = spread(&widths);
    let (alo, amid, ahi) = spread(&along);
    let (clo, cmid, chi) = spread(&across);

    println!(
        "\nshared: the ring where BOTH lenses have the picture, walked out at 0.01 deg over {} \
         azimuths",
        band::AZIMUTHS
    );
    println!(
        "        width          min {wlo:.3}  median {wmid:.3}  max {whi:.3} deg, each of them \
         {CAP_MARGIN_BOTH_DEG:.1} deg generous"
    );
    println!(
        "        lens-0 side    min {:.3}  max {:.3} deg",
        spread(&highs).0,
        spread(&highs).2
    );
    println!(
        "        lens-1 side    min {:.3}  max {:.3} deg",
        spread(&lows).0,
        spread(&lows).2
    );
    println!("\nscale:  source px per degree at the seam, both lenses");
    println!("        along the ring  min {alo:.2}  median {amid:.2}  max {ahi:.2}");
    println!("        across the seam min {clo:.2}  median {cmid:.2}  max {chi:.2}");

    let span = span_of(&spokes);
    println!(
        "\nstrip:  across-seam span taken as {span:.3} deg, the narrowest azimuth's shared \
         picture. A strip wider than that asks a lens for picture it does not have."
    );
    println!(
        "        at native source resolution the whole ring would be {:.0} x {:.0} px.",
        360.0 * amid,
        span * cmid
    );

    println!(
        "\n{:>11} {:>8} {:>8} {:>7} {:>7} {:>10} {:>10} {:>8}",
        "strip", "along", "across", "vs src", "vs src", "1/8 px", "1/8 px", "resident"
    );
    println!(
        "{:>11} {:>8} {:>8} {:>7} {:>7} {:>10} {:>10} {:>8}",
        "W x H", "px/deg", "px/deg", "along", "across", "along deg", "across deg", "MB"
    );
    for size in &options.sizes {
        let a = f64::from(size.width) / 360.0;
        let c = f64::from(size.height) / span;
        println!(
            "{:>4} x {:>4} {a:>8.2} {c:>8.2} {:>7.2} {:>7.2} {:>10.4} {:>10.4} {:>8.1}",
            size.width,
            size.height,
            a / amid,
            c / cmid,
            0.125 / a,
            0.125 / c,
            strip_bytes(*size) as f64 / 1e6,
        );
    }

    println!(
        "\nranges: the design brief's capture ranges are +-1.5 deg along and +-0.7 deg across. \
         The repo's own on-record search window is band.rs PERP_DEG +-0.90 deg along and \
         FAR_DEG..NEAR_DEG -1.2..+2.6 deg across. Both are given, because they disagree and \
         the strip has to hold whichever is larger on each axis."
    );
    for size in &options.sizes {
        let a = f64::from(size.width) / 360.0;
        let c = f64::from(size.height) / span;
        println!(
            "        {:>4} x {:>4}: along +-{:.1} px (brief) / +-{:.1} px (band.rs); \
             across +-{:.1} px (brief) / {:.1}..+{:.1} px (band.rs)",
            size.width,
            size.height,
            1.5 * a,
            0.9 * a,
            0.7 * c,
            -1.2 * c,
            2.6 * c,
        );
    }
    Ok(())
}

/// Every byte one working size costs: two luma strips and their two pyramid
/// levels, the patch buffers, the temporal seed, the densified field and the
/// rectification map.
fn strip_bytes(size: Size) -> u64 {
    let plane = |w: u32, h: u32| u64::from(w) * u64::from(h);
    let strips = 2
        * (plane(size.width, size.height)
            + plane(size.width / 2, size.height / 2)
            + plane(size.width / 4, size.height / 4));
    let patches: u64 = (0..3)
        .map(|level| {
            let (c, r) = grid(size, level);
            u64::from(c) * u64::from(r) * 16
        })
        .sum();
    let (c0, r0) = grid(size, 0);
    let temporal = 2 * u64::from(c0) * u64::from(r0) * 16;
    let field = plane(size.width, size.height) * 4;
    let lut = plane(size.width, size.height) * 16;
    strips + patches + temporal + field + lut
}

/// How many patches fit at one pyramid level. The strip wraps along the ring,
/// so a patch may start at any column; it does not wrap across the seam, so the
/// last row of origins is the last one whose patch fits.
fn grid(size: Size, level: u32) -> (u32, u32) {
    let w = size.width >> level;
    let h = size.height >> level;
    (w / STRIDE, h.saturating_sub(PATCH) / STRIDE + 1)
}

// ---------------------------------------------------------------------------
// 2. A device that can both import a decoder's dmabuf and take a timestamp.
// ---------------------------------------------------------------------------

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    period: f32,
}

impl Gpu {
    /// `dmabuf::open_device` with `TIMESTAMP_QUERY` asked for.
    ///
    /// Copied rather than called because that function hardcodes
    /// `Features::empty()` in both halves and a feature has to be named at
    /// device creation. The extension callback is the shipped one, so this
    /// device can still import a decoded frame, which `mode=live` needs: it
    /// runs the app's own `Scene` on the same device the belt is on, which is
    /// the arrangement the belt would really ship in.
    fn open() -> Fallible<Self> {
        use wgpu::hal::api::Vulkan;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        let wanted = wgpu::Features::TIMESTAMP_QUERY;
        if !adapter.features().contains(wanted) {
            return Err("this adapter has no TIMESTAMP_QUERY, so no stage can be timed".into());
        }
        let opened = unsafe {
            let hal = adapter.as_hal::<Vulkan>().ok_or("not a Vulkan adapter")?;
            hal.open_with_callback(
                wanted,
                &wgpu::MemoryHints::default(),
                Some(Box::new(dmabuf::force_extensions)),
            )?
        };
        let (device, queue) = unsafe {
            adapter.create_device_from_hal::<Vulkan>(
                opened,
                &wgpu::DeviceDescriptor {
                    label: Some("belt"),
                    required_features: wanted,
                    required_limits: adapter.limits(),
                    ..Default::default()
                },
            )
        }?;
        let period = queue.get_timestamp_period();
        Ok(Self {
            name: adapter.get_info().name,
            device,
            queue,
            period,
        })
    }
}

/// The timestamps: one pair per pass, resolved and read back once per frame.
///
/// A pass writes its own beginning and end into the query set, so what comes
/// back is the GPU's account of that pass and of nothing around it.
struct Clock {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    period: f32,
    next: u32,
    names: Vec<String>,
    slots: u32,
}

impl Clock {
    fn new(gpu: &Gpu, slots: u32) -> Self {
        let bytes = u64::from(slots) * 8;
        Self {
            set: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("belt"),
                ty: wgpu::QueryType::Timestamp,
                count: slots,
            }),
            resolve: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("belt resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("belt stamps"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            period: gpu.period,
            next: 0,
            names: Vec::new(),
            slots,
        }
    }

    fn start(&mut self) {
        self.next = 0;
        self.names.clear();
    }

    /// The two query indices one pass writes, under the name it reports as.
    fn pair(&mut self, name: &str) -> (u32, u32) {
        assert!(self.next + 2 <= self.slots, "belt: out of timestamp slots");
        let at = self.next;
        self.next += 2;
        self.names.push(name.to_owned());
        (at, at + 1)
    }

    fn finish(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.next == 0 {
            return;
        }
        encoder.resolve_query_set(&self.set, 0..self.next, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, u64::from(self.next) * 8);
    }

    /// Every pass of the frame just submitted, in milliseconds.
    fn read(&self, gpu: &Gpu) -> Fallible<Vec<(String, f64)>> {
        let slice = self.read.slice(..u64::from(self.next) * 8);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        let mapped = slice.get_mapped_range();
        let ticks: Vec<u64> = mapped
            .chunks_exact(8)
            .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
            .collect();
        drop(mapped);
        self.read.unmap();
        Ok(self
            .names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let span = ticks[index * 2 + 1].saturating_sub(ticks[index * 2]);
                (
                    name.clone(),
                    span as f64 * f64::from(self.period) / 1_000_000.0,
                )
            })
            .collect())
    }
}

fn render_stamps(clock: &Clock, pair: (u32, u32)) -> wgpu::RenderPassTimestampWrites<'_> {
    wgpu::RenderPassTimestampWrites {
        query_set: &clock.set,
        beginning_of_pass_write_index: Some(pair.0),
        end_of_pass_write_index: Some(pair.1),
    }
}

fn compute_stamps(clock: &Clock, pair: (u32, u32)) -> wgpu::ComputePassTimestampWrites<'_> {
    wgpu::ComputePassTimestampWrites {
        query_set: &clock.set,
        beginning_of_pass_write_index: Some(pair.0),
        end_of_pass_write_index: Some(pair.1),
    }
}

/// The same cast `Reframe::bytes` makes for the block itself, for types with no
/// padding and no invalid pattern.
fn bytes_of<T: Copy>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

// ---------------------------------------------------------------------------
// 3. The shaders.
// ---------------------------------------------------------------------------

/// One triangle over the whole target, so every render pass below is a fragment
/// shader with no vertex buffer.
const FULLSCREEN: &str = r#"
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
  let y = f32(index & 2u) * 2.0 - 1.0;
  return vec4<f32>(x, y, 0.0, 1.0);
}
"#;

/// **Stage 1, rectification.** One strip texel reads where it comes from out of
/// the map and takes one bilinear sample of that lens.
///
/// The map is a texture rather than arithmetic, and the reason is geometric
/// rather than an optimization: **the seam belt is a body-frame object.** Its
/// texel-to-source-pixel map depends on the calibration and on nothing the view
/// does, so it is built once when a file opens and read every frame. What that
/// leaves out is the rolling-shutter term, which `Reframe::project` does apply
/// and which moves with the body; at the seam an X4 reads the same world
/// direction down both lenses and that term is worth 0.000 degrees there
/// (docs/research/insv-format.md 6.7), so a static map gives nothing up on this
/// camera. A camera where that is not true would have to rebuild the map every
/// frame, and the memo says what that would cost.
const RECTIFY: &str = r#"
struct Rect { frame: vec2<f32>, strip: vec2<f32> };
@group(0) @binding(0) var<uniform> rect: Rect;
@group(0) @binding(1) var lut: texture_2d<f32>;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let cell = vec2<i32>(i32(at.x), i32(at.y));
  let pixel = textureLoad(lut, cell, 0).xy;
  let luma = textureSampleLevel(src, samp, pixel / rect.frame, 0.0).r;
  return vec4<f32>(luma, 0.0, 0.0, 1.0);
}
"#;

/// **Stage 2, the pyramid.** Half each way, four taps, which is the support a
/// coarse-to-fine search wants rather than the single tap a mip chain gives it.
const DOWN: &str = r#"
struct Down { source: vec2<f32>, dest: vec2<f32> };
@group(0) @binding(0) var<uniform> down: Down;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = at.xy / down.dest;
  let nudge = 0.5 / down.source;
  var sum = 0.0;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>(-nudge.x, -nudge.y), 0.0).r;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>( nudge.x, -nudge.y), 0.0).r;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>(-nudge.x,  nudge.y), 0.0).r;
  sum += textureSampleLevel(src, samp, uv + vec2<f32>( nudge.x,  nudge.y), 0.0).r;
  return vec4<f32>(sum * 0.25, 0.0, 0.0, 1.0);
}
"#;

/// **Stage 3, the inverse search.** One workgroup per patch, `per` patch pixels
/// per lane, the anchor and its two gradients in registers, the update reduced
/// in shared memory.
///
/// This is inverse-compositional Lucas-Kanade, which is what the "inverse" in
/// DIS's inverse search means: the Hessian is built once from the **anchor's**
/// gradients and inverted once, and each iteration costs one warped sample per
/// patch pixel plus a two-element reduction. Nothing follows it, because the
/// design has no variational refinement.
///
/// **The memory pattern is what decides the cost and it is deliberate.** Each
/// iteration is 64 bilinear fetches from the moving picture at a displacement
/// the whole workgroup shares, so a patch's fetches are one 8x8 footprint and
/// neighbouring patches at stride 3 overlap in the cache. That is how a real
/// implementation behaves. A version that held the whole anchor in one lane's
/// private array would spill 768 bytes a lane to scratch and would be measuring
/// the spill.
///
/// **Two shapes of the same kernel are generated and both are measured**, and
/// the reason is that the first version of this probe was reduction-bound
/// rather than texture-bound and a single layout's number would have decided a
/// GO/NO-GO on one guess. `search_wgsl(64, 1)` is the obvious mapping, 64 lanes
/// and one pixel each; `search_wgsl(32, 2)` is the same arithmetic in a
/// workgroup of one RDNA wave, where a driver may drop the barriers entirely.
/// The memo quotes the faster of the two and says which it was.
///
/// The strip wraps along the ring and does not wrap across the seam. Both
/// halves of that are in `tap` and in the sampler's address modes.
fn search_wgsl(width: u32, per: u32) -> String {
    let fold = width / 2;
    format!(
        r##"
const WG: u32 = {width}u;
const PER: u32 = {per}u;

struct Search {{
  cols: u32, rows: u32, iters: u32, seeded: u32,
  width: f32, height: f32, seed_cols: u32, seed_rows: u32,
  seed_gain: f32, seed_step: u32, pad0: u32, pad1: u32,
  lo: vec2<f32>, hi: vec2<f32>,
}};
@group(0) @binding(0) var<uniform> p: Search;
@group(0) @binding(1) var held: texture_2d<f32>;
@group(0) @binding(2) var moving: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<storage, read> seed: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> answer: array<vec4<f32>>;

var<workgroup> scratch: array<vec4<f32>, {width}>;
var<workgroup> flow: vec2<f32>;

fn tap(x: i32, y: i32) -> f32 {{
  let w = i32(p.width);
  let h = i32(p.height);
  let cx = ((x % w) + w) % w;
  let cy = clamp(y, 0, h - 1);
  return textureLoad(held, vec2<i32>(cx, cy), 0).r;
}}

@compute @workgroup_size({width})
fn search(@builtin(workgroup_id) group: vec3<u32>,
          @builtin(local_invocation_index) lane: u32) {{
  let px = group.x;
  let py = group.y;
  let ox = i32(px * 3u);
  let oy = i32(py * 3u);

  var anchor: array<f32, {per}>;
  var gradx: array<f32, {per}>;
  var grady: array<f32, {per}>;
  var xs: array<i32, {per}>;
  var ys: array<i32, {per}>;
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
      // patch width on decimated content and reports the wander as a reading,
      // which is what the first run of this probe did.
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

/// **Stage 4, densification.** DIS's own: every pixel is the residual-weighted
/// mean of the patches that cover it, which at patch 8 stride 3 is a 3x3
/// neighbourhood.
const DENSIFY: &str = r#"
struct Dense { cols: u32, rows: u32, width: f32, height: f32 };
@group(0) @binding(0) var<uniform> d: Dense;
@group(0) @binding(1) var<storage, read> answers: array<vec4<f32>>;

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let first = vec2<i32>((i32(at.x) - 7) / 3, (i32(at.y) - 7) / 3);
  var sum = vec2<f32>(0.0, 0.0);
  var weight = 0.0;
  for (var dy = 0; dy < 3; dy = dy + 1) {
    for (var dx = 0; dx < 3; dx = dx + 1) {
      let cx = (((first.x + dx) % i32(d.cols)) + i32(d.cols)) % i32(d.cols);
      let cy = clamp(first.y + dy, 0, i32(d.rows) - 1);
      let entry = answers[u32(cy) * d.cols + u32(cx)];
      let w = 1.0 / (entry.z + 0.01);
      sum = sum + entry.xy * w;
      weight = weight + w;
    }
  }
  return vec4<f32>(sum / max(weight, 1e-6), 0.0, 1.0);
}
"#;

/// **Stage 5, the gate.** One confidence per direction round the ring: how many
/// of that column's patches came back under the residual gate. The belt's
/// version of the band's `KEEP`, and the thing a row of the field is trusted or
/// refused on.
const GATE: &str = r#"
struct Dense { cols: u32, rows: u32, width: f32, height: f32 };
@group(0) @binding(0) var<uniform> d: Dense;
@group(0) @binding(1) var<storage, read> answers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> conf: array<vec4<f32>>;

var<workgroup> tally: array<vec4<f32>, 64>;

@compute @workgroup_size(64)
fn gate(@builtin(workgroup_id) group: vec3<u32>,
        @builtin(local_invocation_index) lane: u32) {
  let col = group.x;
  var kept = 0.0;
  var seen = 0.0;
  var moved = 0.0;
  var row = lane;
  loop {
    if row >= d.rows { break; }
    let entry = answers[row * d.cols + col];
    seen = seen + 1.0;
    if entry.z < GATE_RMS && entry.w > 1e-5 {
      kept = kept + 1.0;
      moved = moved + abs(entry.y);
    }
    row = row + 64u;
  }
  tally[lane] = vec4<f32>(kept, seen, moved, 0.0);
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
    conf[col] = vec4<f32>(t.x / max(t.y, 1.0), t.y, t.z / max(t.x, 1.0), 0.0);
  }
}
"#;

/// **Stage 6, the consuming lookup.** What the sampler pays to read the field.
///
/// Measured as a **difference** between arms of one pass, because the two lens
/// samples and the projection round them are what the shipped pass already does
/// and are not the belt's to charge for. Every arm samples both lenses; only
/// the belt arms read the field and displace the source UV first. `cover` is
/// the share of the picture inside the handover corridor and therefore the
/// share that reads the field at all: 1.0 is the worst case where the seam
/// fills the view.
const CONSUME: &str = r#"
struct Use { size: vec2<f32>, frame: vec2<f32>, belt: f32, cover: f32, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> u: Use;
@group(0) @binding(1) var field: texture_2d<f32>;
@group(0) @binding(2) var lens0: texture_2d<f32>;
@group(0) @binding(3) var lens1: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = at.xy / u.size;
  let base0 = vec2<f32>(uv.x * 0.6 + 0.2, uv.y * 0.6 + 0.2);
  let base1 = vec2<f32>(0.8 - uv.x * 0.6, uv.y * 0.6 + 0.2);
  var shift = vec2<f32>(0.0, 0.0);
  if u.belt > 0.5 && abs(uv.y - 0.5) < u.cover * 0.5 {
    shift = textureSampleLevel(field, samp, uv, 0.0).xy / u.frame;
  }
  let a = textureSampleLevel(lens0, samp, base0 + shift, 0.0).r;
  let b = textureSampleLevel(lens1, samp, base1 - shift, 0.0).r;
  return vec4<f32>(a, b, 0.0, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// 4. The probe.
// ---------------------------------------------------------------------------

/// The rectification map for one working size: where each strip texel comes
/// from in each lens, in that lens's delivered-frame pixels.
///
/// Built on the CPU through `Reframe::project` against the file's own
/// calibration, which makes it exact rather than a stand-in. It is a one-time
/// cost when a file opens, and the report says how long it took.
struct Lut {
    size: Size,
    /// Two f32 per texel: lens 0, lens 1, then lens 0 again through each of
    /// [`PLANTS`] - the two positive controls.
    texels: [Vec<f32>; 4],
    built: Duration,
}

/// The two planted shifts, in degrees across the seam.
///
/// **The controls this probe would be worthless without.** A search kernel that
/// wanders and reports the wander looks exactly like a search kernel that
/// works, until something moves the content by a known amount and asks what it
/// reads. Each of these rectifies lens 0 a second time with the across-seam
/// angle displaced by that much and matches it against the unplanted lens 0,
/// cold, at the finest level. The answer has to come back at minus that many
/// strip pixels.
///
/// **The pair is the point, and they are meant to answer differently.** The
/// small one is inside a Lucas-Kanade patch's linearization radius at every
/// candidate size, so the kernel has to recover it and its error is this
/// probe's accuracy figure. The large one - a third of the owner's own -0.906
/// degree residual - is 2.7 to 10.9 strip pixels depending on the size, which
/// is past what one level can capture from a standing start. A cold search that
/// misses it is not broken; it is the measurement that says the coarse-to-fine
/// ladder and the temporal seed are load-bearing rather than optimizations.
const PLANTS: [f64; 2] = [0.05, 0.30];

impl Lut {
    fn build(map: &Reframe, baseline: [f32; 3], size: Size, span_deg: f64) -> Self {
        let began = Instant::now();
        let count = size.width as usize * size.height as usize;
        let mut texels: [Vec<f32>; 4] = std::array::from_fn(|_| vec![0.0f32; count * 2]);
        // One `Ring` per column: it is the same for every row of that column.
        let rings: Vec<Ring> = (0..size.width)
            .map(|column| {
                let phi = f64::from(column) / f64::from(size.width) * std::f64::consts::TAU;
                Ring::of(phi as f32, baseline)
            })
            .collect();
        for row in 0..size.height {
            let across = (f64::from(row) / f64::from(size.height - 1) - 0.5) * span_deg;
            let (sin, cos) = across.to_radians().sin_cos();
            for column in 0..size.width {
                let ring = &rings[column as usize];
                let body: [f32; 3] = std::array::from_fn(|axis| {
                    ring.centre[axis] * cos as f32 + ring.epi[axis] * sin as f32
                });
                let view = map.view_ray_from_body(unit(body));
                let at = (row as usize * size.width as usize + column as usize) * 2;
                for (lens, plane) in texels.iter_mut().take(2).enumerate() {
                    let landing = map.project(lens, view);
                    plane[at] = landing.pixel[0];
                    plane[at + 1] = landing.pixel[1];
                }
                for (index, plant) in PLANTS.iter().enumerate() {
                    let (psin, pcos) = (across + plant).to_radians().sin_cos();
                    let moved: [f32; 3] = std::array::from_fn(|axis| {
                        ring.centre[axis] * pcos as f32 + ring.epi[axis] * psin as f32
                    });
                    let landing = map.project(0, map.view_ray_from_body(unit(moved)));
                    texels[2 + index][at] = landing.pixel[0];
                    texels[2 + index][at + 1] = landing.pixel[1];
                }
            }
        }
        Self {
            size,
            texels,
            built: began.elapsed(),
        }
    }
}

#[derive(Clone, Copy)]
enum SeedFrom {
    Zero,
    Level(u32),
    Previous,
}

/// Which pair of strips a dispatch matches.
#[derive(Clone, Copy, PartialEq)]
enum Against {
    /// Lens 0 against lens 1, which is the belt's real question.
    Lenses,
    /// Lens 0 against a second rectification of lens 0 moved by one of
    /// [`PLANTS`], which is the control that can fail.
    Plant(usize),
}

/// One dispatch of the search: which pyramid level, how many iterations, where
/// its starting guess comes from and where its answer goes.
struct SearchPlan {
    level: u32,
    output: usize,
    iters: u32,
    source: SeedFrom,
    against: Against,
    cols: u32,
    rows: u32,
    width: f32,
    height: f32,
    seed_cols: u32,
    seed_rows: u32,
    seed_gain: f32,
    seed_step: u32,
    /// The capture range in strip pixels of this level, along then across.
    lo: [f32; 2],
    hi: [f32; 2],
}

/// The capture range in strip pixels at one level, from the design's declared
/// angles.
///
/// Along the ring it is the wider of the brief's +-1.5 degrees and band.rs's
/// own PERP_DEG of +-0.90; across the seam it is band.rs's FAR_DEG..NEAR_DEG of
/// -1.2..+2.6, which contains the brief's +-0.7. Taking the union on each axis
/// is the only reading that refuses nothing either document asks for.
fn capture(size: Size, level: u32, span_deg: f64) -> ([f32; 2], [f32; 2]) {
    let along = f64::from(size.width >> level) / 360.0;
    let across = f64::from(size.height >> level) / span_deg;
    (
        [(-1.5 * along) as f32, (-1.2 * across) as f32],
        [(1.5 * along) as f32, (2.6 * across) as f32],
    )
}

impl SearchPlan {
    /// One rung of the cold coarse-to-fine ladder. Level 2 starts from nothing;
    /// the finer two start from the level above, doubled.
    fn cold(size: Size, level: u32, iters: u32, span_deg: f64) -> Self {
        let (cols, rows) = grid(size, level);
        let (seed_cols, seed_rows, source) = match level {
            2 => (0, 0, SeedFrom::Zero),
            _ => {
                let (c, r) = grid(size, level + 1);
                (c, r, SeedFrom::Level(level + 1))
            }
        };
        let (lo, hi) = capture(size, level, span_deg);
        Self {
            level,
            output: level as usize,
            iters,
            source,
            against: Against::Lenses,
            cols,
            rows,
            width: (size.width >> level) as f32,
            height: (size.height >> level) as f32,
            seed_cols,
            seed_rows,
            seed_gain: 2.0,
            seed_step: 2,
            lo,
            hi,
        }
    }

    /// The finest level alone, started from the last frame's answer. This is the
    /// temporal hint: Studio recomputes one frame in `frame_interval_of_flow_
    /// calc_` and holds between, and our target is every frame, so the hint is
    /// one frame old rather than three.
    ///
    /// Its own output slot, so the cold arm's answer survives and the two can be
    /// compared on the same frame.
    fn seeded(size: Size, iters: u32, span_deg: f64) -> Self {
        let (cols, rows) = grid(size, 0);
        let (lo, hi) = capture(size, 0, span_deg);
        Self {
            level: 0,
            output: 3,
            iters,
            source: SeedFrom::Previous,
            against: Against::Lenses,
            cols,
            rows,
            width: size.width as f32,
            height: size.height as f32,
            seed_cols: cols,
            seed_rows: rows,
            seed_gain: 1.0,
            seed_step: 1,
            lo,
            hi,
        }
    }

    /// The positive control: lens 0 against lens 0 rectified [`PLANT_DEG`] off,
    /// cold, at the finest level. The answer has to read minus that many strip
    /// pixels across, and if it does not then nothing else this probe prints
    /// about the field means anything.
    fn plant(size: Size, iters: u32, span_deg: f64, which: usize) -> Self {
        let (cols, rows) = grid(size, 0);
        let (lo, hi) = capture(size, 0, span_deg);
        Self {
            level: 0,
            output: 4 + which,
            iters,
            source: SeedFrom::Zero,
            against: Against::Plant(which),
            cols,
            rows,
            width: size.width as f32,
            height: size.height as f32,
            seed_cols: 0,
            seed_rows: 0,
            seed_gain: 1.0,
            seed_step: 1,
            lo,
            hi,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        let seeded = u32::from(!matches!(self.source, SeedFrom::Zero));
        for word in [self.cols, self.rows, self.iters, seeded] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.seed_cols.to_le_bytes());
        out.extend_from_slice(&self.seed_rows.to_le_bytes());
        out.extend_from_slice(&self.seed_gain.to_le_bytes());
        out.extend_from_slice(&self.seed_step.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for value in [self.lo[0], self.lo[1], self.hi[0], self.hi[1]] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

fn plans_for(size: Size, iters: u32, seed_iters: u32, span_deg: f64) -> Vec<SearchPlan> {
    vec![
        SearchPlan::cold(size, 2, iters, span_deg),
        SearchPlan::cold(size, 1, iters, span_deg),
        SearchPlan::cold(size, 0, iters, span_deg),
        SearchPlan::seeded(size, seed_iters, span_deg),
        SearchPlan::plant(size, iters, span_deg, 0),
        SearchPlan::plant(size, iters, span_deg, 1),
    ]
}

/// Every GPU resource one working size needs.
struct Belt {
    size: Size,
    /// Level 0, 1, 2 per lens.
    strip: [[wgpu::Texture; 3]; 2],
    /// Lens 0 rectified again through each of [`PLANTS`], for the controls.
    planted: [wgpu::Texture; 2],
    /// The cold ladder's three outputs at index 0, 1, 2, the seeded arm's at
    /// index 3 and the two planted controls' at 4 and 5.
    answers: [wgpu::Buffer; 6],
    seed: wgpu::Buffer,
    field: wgpu::Texture,
    readback: wgpu::Buffer,
    out: wgpu::Texture,

    rectify: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    /// The same kernel in two workgroup shapes, both measured.
    search: [wgpu::ComputePipeline; 2],
    densify: wgpu::RenderPipeline,
    gate: wgpu::ComputePipeline,
    consume: wgpu::RenderPipeline,

    rectify_group: [wgpu::BindGroup; 4],
    down_group: [[wgpu::BindGroup; 2]; 2],
    search_group: Vec<wgpu::BindGroup>,
    densify_group: wgpu::BindGroup,
    gate_group: wgpu::BindGroup,
    consume_group: [wgpu::BindGroup; 3],

    plans: Vec<SearchPlan>,
    lut_built: Duration,
}

impl Belt {
    #[allow(clippy::too_many_lines)]
    fn new(
        gpu: &Gpu,
        lut: &Lut,
        lenses: [&wgpu::Texture; 2],
        frame: Size,
        iters: u32,
        seed_iters: u32,
        span_deg: f64,
    ) -> Self {
        let device = &gpu.device;
        let size = lut.size;
        let plans = plans_for(size, iters, seed_iters, span_deg);

        let texture = |label: &str, w: u32, h: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: Size::new(w, h).extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;

        let maps: [wgpu::Texture; 4] = std::array::from_fn(|lens| {
            let tex = texture(
                "belt lut",
                size.width,
                size.height,
                MAPPED,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            gpu.queue.write_texture(
                tex.as_image_copy(),
                bytes_of(&lut.texels[lens]),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width * 8),
                    rows_per_image: Some(size.height),
                },
                size.extent(),
            );
            tex
        });

        let strip: [[wgpu::Texture; 3]; 2] = std::array::from_fn(|_| {
            std::array::from_fn(|level| {
                texture(
                    "belt strip",
                    size.width >> level,
                    size.height >> level,
                    LUMA,
                    sampled,
                )
            })
        });
        // The planted controls' own strips: lens 0 again, rectified off by each
        // of PLANTS. Finest level only, because each control is a cold search
        // there.
        let planted: [wgpu::Texture; 2] =
            std::array::from_fn(|_| texture("belt plant", size.width, size.height, LUMA, sampled));

        let (c0, r0) = grid(size, 0);
        let finest = u64::from(c0) * u64::from(r0) * 16;
        let answers: [wgpu::Buffer; 6] = std::array::from_fn(|slot| {
            // Slots 0, 1, 2 are the cold ladder's levels; 3 is the seeded arm's
            // and 4 and 5 the controls', all three at level 0's shape.
            let (c, r) = grid(size, if slot >= 3 { 0 } else { slot as u32 });
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("belt answers"),
                size: u64::from(c) * u64::from(r) * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        });
        let seed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt seed"),
            size: finest,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // A seed of nothing for the coldest rung: a buffer it never reads is
        // still a buffer that has to be bound.
        let zero = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt zero"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let conf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt confidence"),
            size: u64::from(c0) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("belt readback"),
            size: finest,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let field = texture("belt field", size.width, size.height, FIELD, sampled);
        let out = texture(
            "belt consume",
            OUTPUT.width,
            OUTPUT.height,
            COLOUR,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );

        // The strip wraps along the ring and does not across the seam, which is
        // a real property of the geometry rather than a convenience.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("belt"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform = |label: &str, bytes: &[u8]| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            gpu.queue.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let view = |tex: &wgpu::Texture| tex.create_view(&wgpu::TextureViewDescriptor::default());

        let slot = |binding: u32, ty: wgpu::BindingType, vis: wgpu::ShaderStages| {
            wgpu::BindGroupLayoutEntry {
                binding,
                visibility: vis,
                ty,
                count: None,
            }
        };
        let unif = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let store = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let tex = |filterable| wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let samp = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let fs = wgpu::ShaderStages::FRAGMENT;
        let cs = wgpu::ShaderStages::COMPUTE;
        let module = |label: &str, source: String| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };

        // --- rectify -------------------------------------------------------
        let rectify_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt rectify"),
            entries: &[
                slot(0, unif, fs),
                slot(1, tex(false), fs),
                slot(2, tex(true), fs),
                slot(3, samp, fs),
            ],
        });
        let rect_uniform = uniform(
            "belt rect",
            bytes_of(&[
                frame.width as f32,
                frame.height as f32,
                size.width as f32,
                size.height as f32,
            ]),
        );
        // Lens 0, lens 1, and lens 0 again through each planted map.
        let rectify_group: [wgpu::BindGroup; 4] = std::array::from_fn(|lens| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("belt rectify"),
                layout: &rectify_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: rect_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view(&maps[lens])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        // Slot 2 is the planted control, which is LENS 0 read
                        // through a moved map. `lens.min(1)` here would bind
                        // lens 1's picture to lens 0's planted coordinates,
                        // which is a strip of nothing - and it is what the
                        // first run of this control did, which is how the
                        // control earned its keep before it measured anything.
                        resource: wgpu::BindingResource::TextureView(&view(
                            lenses[usize::from(lens == 1)],
                        )),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        });
        let rectify = render_pipeline(
            device,
            "belt rectify",
            &module("belt rectify", format!("{FULLSCREEN}{RECTIFY}")),
            &rectify_layout,
            LUMA,
        );

        // --- pyramid -------------------------------------------------------
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt down"),
            entries: &[slot(0, unif, fs), slot(1, tex(true), fs), slot(2, samp, fs)],
        });
        let down_group: [[wgpu::BindGroup; 2]; 2] = std::array::from_fn(|lens| {
            std::array::from_fn(|step| {
                let (from, to) = (step, step + 1);
                let u = uniform(
                    "belt down",
                    bytes_of(&[
                        (size.width >> from) as f32,
                        (size.height >> from) as f32,
                        (size.width >> to) as f32,
                        (size.height >> to) as f32,
                    ]),
                );
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("belt down"),
                    layout: &down_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: u.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&view(&strip[lens][from])),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                })
            })
        });
        let down = render_pipeline(
            device,
            "belt down",
            &module("belt down", format!("{FULLSCREEN}{DOWN}")),
            &down_layout,
            LUMA,
        );

        // --- search --------------------------------------------------------
        let search_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt search"),
            entries: &[
                slot(0, unif, cs),
                slot(1, tex(true), cs),
                slot(2, tex(true), cs),
                slot(3, samp, cs),
                slot(4, store(true), cs),
                slot(5, store(false), cs),
            ],
        });
        let mut search_group = Vec::new();
        for plan in &plans {
            let u = uniform("belt search", &plan.bytes());
            let from: &wgpu::Buffer = match plan.source {
                SeedFrom::Zero => &zero,
                SeedFrom::Level(level) => &answers[level as usize],
                SeedFrom::Previous => &seed,
            };
            search_group.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("belt search"),
                layout: &search_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: u.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view(
                            &strip[0][plan.level as usize],
                        )),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&match plan.against {
                            Against::Lenses => view(&strip[1][plan.level as usize]),
                            Against::Plant(which) => view(&planted[which]),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: from.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: answers[plan.output].as_entire_binding(),
                    },
                ],
            }));
        }
        let search_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("belt search"),
                bind_group_layouts: &[&search_layout],
                immediate_size: 0,
            });
        let search: [wgpu::ComputePipeline; 2] = [(64u32, 1u32), (32, 2)].map(|(wide, per)| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("belt search"),
                layout: Some(&search_pipeline_layout),
                module: &module("belt search", search_wgsl(wide, per)),
                entry_point: Some("search"),
                compilation_options: Default::default(),
                cache: None,
            })
        });

        // --- densify and gate ----------------------------------------------
        let mut dense_bytes = Vec::with_capacity(16);
        dense_bytes.extend_from_slice(&c0.to_le_bytes());
        dense_bytes.extend_from_slice(&r0.to_le_bytes());
        dense_bytes.extend_from_slice(&(size.width as f32).to_le_bytes());
        dense_bytes.extend_from_slice(&(size.height as f32).to_le_bytes());
        let dense_uniform = uniform("belt dense", &dense_bytes);

        let densify_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt densify"),
            entries: &[slot(0, unif, fs), slot(1, store(true), fs)],
        });
        let densify_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("belt densify"),
            layout: &densify_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: dense_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: answers[0].as_entire_binding(),
                },
            ],
        });
        let densify = render_pipeline(
            device,
            "belt densify",
            &module("belt densify", format!("{FULLSCREEN}{DENSIFY}")),
            &densify_layout,
            FIELD,
        );

        let gate_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt gate"),
            entries: &[
                slot(0, unif, cs),
                slot(1, store(true), cs),
                slot(2, store(false), cs),
            ],
        });
        let gate_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("belt gate"),
            layout: &gate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: dense_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: answers[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: conf.as_entire_binding(),
                },
            ],
        });
        let gate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("belt gate"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("belt gate"),
                    bind_group_layouts: &[&gate_layout],
                    immediate_size: 0,
                }),
            ),
            module: &module(
                "belt gate",
                format!("const GATE_RMS = {GATE_RMS:?};\n{GATE}"),
            ),
            entry_point: Some("gate"),
            compilation_options: Default::default(),
            cache: None,
        });

        // --- consume -------------------------------------------------------
        let consume_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("belt consume"),
            entries: &[
                slot(0, unif, fs),
                slot(1, tex(true), fs),
                slot(2, tex(true), fs),
                slot(3, tex(true), fs),
                slot(4, samp, fs),
            ],
        });
        // Three arms: no belt at all, the belt over the whole picture, and the
        // belt over the share of it the 8 degree handover covers at the 60
        // degree view `--bin playback` renders.
        let arms = [(0.0f32, 1.0f32), (1.0, 1.0), (1.0, 8.0 / 60.0)];
        let consume_group: [wgpu::BindGroup; 3] = std::array::from_fn(|arm| {
            let (belt, cover) = arms[arm];
            let u = uniform(
                "belt use",
                bytes_of(&[
                    OUTPUT.width as f32,
                    OUTPUT.height as f32,
                    frame.width as f32,
                    frame.height as f32,
                    belt,
                    cover,
                    0.0,
                    0.0,
                ]),
            );
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("belt consume"),
                layout: &consume_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: u.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view(&field)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view(lenses[0])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&view(lenses[1])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        });
        let consume = render_pipeline(
            device,
            "belt consume",
            &module("belt consume", format!("{FULLSCREEN}{CONSUME}")),
            &consume_layout,
            COLOUR,
        );

        Self {
            size,
            strip,
            planted,
            answers,
            seed,
            field,
            readback,
            out,
            rectify,
            down,
            search,
            densify,
            gate,
            consume,
            rectify_group,
            down_group,
            search_group,
            densify_group,
            gate_group,
            consume_group,
            plans,
            lut_built: lut.built,
        }
    }
}

fn render_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[layout],
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
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
}

/// One target, cleared and drawn into, with a timestamp pair round it.
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    clock: &Clock,
    pair: (u32, u32),
    into: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("belt"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: into,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        timestamp_writes: Some(render_stamps(clock, pair)),
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

/// One frame of the belt, encoded with a timestamp pair round every pass.
///
/// The cold ladder always runs; `seeded` says whether the temporal arm does,
/// which it cannot on the first frame. One frame therefore reports both, so the
/// seeded-against-cold comparison is between two arms of one frame rather than
/// two runs of the binary.
/// What one encoded frame does.
#[derive(Clone, Copy, PartialEq)]
enum Work {
    /// Every arm and both controls: what `mode=cost` runs, so one frame answers
    /// the whole table. The flag is whether this frame has a temporal hint to
    /// start the seeded arm from, which the first frame of a run does not: it
    /// runs the ladder and hands its answer on, which is what a real first
    /// frame does.
    Survey { seeded: bool },
    /// The cold coarse-to-fine ladder alone, which is what a first frame or a
    /// seek has to run.
    Cold,
    /// The finest level alone from last frame's answer, which is what every
    /// frame after the first runs. **This is the belt as it would ship**, and
    /// `mode=live` runs this and nothing else, because a contention arm that
    /// dispatched three extra kernels would be measuring a load nobody has.
    Seeded,
}

fn encode_frame(belt: &Belt, clock: &mut Clock, encoder: &mut wgpu::CommandEncoder, work: Work) {
    let survey = matches!(work, Work::Survey { .. });
    let seeded = match work {
        Work::Survey { seeded } => seeded,
        Work::Cold => false,
        Work::Seeded => true,
    };
    for lens in 0..2 {
        let pair = clock.pair(&format!("rectify.{lens}"));
        let into = belt.strip[lens][0].create_view(&Default::default());
        draw(
            encoder,
            clock,
            pair,
            &into,
            &belt.rectify,
            &belt.rectify_group[lens],
        );
    }
    // The controls' own strips. Timed under names the stage totals do not
    // match, because they are not part of the belt.
    if survey {
        for which in 0..2 {
            let pair = clock.pair(&format!("control.rectify.{which}"));
            let into = belt.planted[which].create_view(&Default::default());
            draw(
                encoder,
                clock,
                pair,
                &into,
                &belt.rectify,
                &belt.rectify_group[2 + which],
            );
        }
    }
    for lens in 0..2 {
        for step in 0..2 {
            let pair = clock.pair(&format!("pyramid.{lens}.{}", step + 1));
            let into = belt.strip[lens][step + 1].create_view(&Default::default());
            draw(
                encoder,
                clock,
                pair,
                &into,
                &belt.down,
                &belt.down_group[lens][step],
            );
        }
    }

    // Both workgroup shapes of the same kernel, so the search's cost is not
    // taken on one layout's word. The `wave` arm writes the same buffers a
    // second time with the same arithmetic, so what it changes is the clock and
    // not the answer.
    let shapes: &[(usize, &str)] = match survey {
        true => &[(0, "lanes"), (1, "wave")],
        // One belt and not two: `mode=live` is a contention measurement and a
        // frame that ran the kernel twice would be measuring the wrong load.
        false => &[(1, "wave")],
    };
    for &(shape, tag) in shapes {
        for (index, plan) in belt.plans.iter().enumerate().take(3) {
            if work == Work::Seeded {
                break;
            }
            let pair = clock.pair(&format!("search.{tag}.cold.L{}", plan.level));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("belt search"),
                timestamp_writes: Some(compute_stamps(clock, pair)),
            });
            pass.set_pipeline(&belt.search[shape]);
            pass.set_bind_group(0, &belt.search_group[index], &[]);
            pass.dispatch_workgroups(plan.cols, plan.rows, 1);
        }
        if seeded {
            let plan = &belt.plans[3];
            let pair = clock.pair(&format!("search.{tag}.seeded.L0"));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("belt search seeded"),
                timestamp_writes: Some(compute_stamps(clock, pair)),
            });
            pass.set_pipeline(&belt.search[shape]);
            pass.set_bind_group(0, &belt.search_group[3], &[]);
            pass.dispatch_workgroups(plan.cols, plan.rows, 1);
        }
    }
    // The planted controls, cold at the finest level, on one shape only: they
    // are a check on the answer and not on the clock.
    if survey {
        for which in 0..2 {
            let plan = &belt.plans[4 + which];
            let pair = clock.pair(&format!("control.search.{which}"));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("belt control"),
                timestamp_writes: Some(compute_stamps(clock, pair)),
            });
            pass.set_pipeline(&belt.search[1]);
            pass.set_bind_group(0, &belt.search_group[4 + which], &[]);
            pass.dispatch_workgroups(plan.cols, plan.rows, 1);
        }
    }

    {
        let pair = clock.pair("densify");
        let into = belt.field.create_view(&Default::default());
        draw(
            encoder,
            clock,
            pair,
            &into,
            &belt.densify,
            &belt.densify_group,
        );
    }
    {
        let (cols, _) = grid(belt.size, 0);
        let pair = clock.pair("gate");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("belt gate"),
            timestamp_writes: Some(compute_stamps(clock, pair)),
        });
        pass.set_pipeline(&belt.gate);
        pass.set_bind_group(0, &belt.gate_group, &[]);
        pass.dispatch_workgroups(cols, 1, 1);
    }

    for (arm, name) in ["consume.none", "consume.whole", "consume.corridor"]
        .iter()
        .enumerate()
    {
        let pair = clock.pair(name);
        let into = belt.out.create_view(&Default::default());
        draw(
            encoder,
            clock,
            pair,
            &into,
            &belt.consume,
            &belt.consume_group[arm],
        );
    }

    // The next frame's temporal hint is this frame's answer: the seeded arm's
    // own, once there is one, so the chain is a real one.
    let source = match seeded {
        true => &belt.answers[3],
        false => &belt.answers[0],
    };
    encoder.copy_buffer_to_buffer(source, 0, &belt.seed, 0, belt.seed.size());
}

/// One lens's luma, uploaded as a texture the probes can sample.
fn upload(gpu: &Gpu, plane: &Plane, into: &wgpu::Texture) {
    gpu.queue.write_texture(
        into.as_image_copy(),
        &plane.luma,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(plane.stride as u32),
            rows_per_image: Some(plane.size.height),
        },
        plane.size.extent(),
    );
}

fn lens_texture(gpu: &Gpu, size: Size) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("belt lens"),
        size: size.extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: LUMA,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

// ---------------------------------------------------------------------------
// 5. mode=cost
// ---------------------------------------------------------------------------

fn cost(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!(
        "gpu:    {} (timestamp period {:.4} ns)",
        gpu.name, gpu.period
    );
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let map = seam::mapped(&calibration.lenses, frame);
    let baseline = band::baseline(&calibration.lenses);
    let ring = spokes(&map, baseline, 64);
    let span = span_of(&ring);
    println!(
        "file:   {} at {:.2} s, per-lens {}x{}",
        options.input.display(),
        options.at,
        frame.width,
        frame.height
    );
    println!(
        "strip:  across-seam span {span:.3} deg, patch {PATCH} px, stride {STRIDE}, \
         {} iterations cold, {} seeded",
        options.iters, options.seed_iters
    );

    let mut walk = Walk::open(&options.input, options.at, frame)?;
    let mut planes: Vec<[Plane; 2]> = Vec::new();
    while planes.len() < options.frames {
        let Some(mut pair) = walk.next_pair()? else {
            break;
        };
        if pair.lenses.len() < 2 {
            return Err("this capture has one lens stream; the belt needs two".into());
        }
        let second = pair.lenses.remove(1);
        let first = pair.lenses.remove(0);
        planes.push([first, second]);
    }
    if planes.len() < 2 {
        return Err("fewer than two frames decoded; the seeded arm needs a previous one".into());
    }
    println!(
        "decode: {} real consecutive frame pairs in hand\n",
        planes.len()
    );

    let lenses = [lens_texture(&gpu, frame), lens_texture(&gpu, frame)];
    for size in &options.sizes {
        let lut = Lut::build(&map, baseline, *size, span);
        let belt = Belt::new(
            &gpu,
            &lut,
            [&lenses[0], &lenses[1]],
            frame,
            options.iters,
            options.seed_iters,
            span,
        );
        let mut clock = Clock::new(&gpu, 96);
        let mut runs: Vec<Vec<(String, f64)>> = Vec::new();
        for (index, pair) in planes.iter().enumerate() {
            upload(&gpu, &pair[0], &lenses[0]);
            upload(&gpu, &pair[1], &lenses[1]);
            clock.start();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encode_frame(
                &belt,
                &mut clock,
                &mut encoder,
                Work::Survey { seeded: index > 0 },
            );
            clock.finish(&mut encoder);
            let submission = gpu.queue.submit([encoder.finish()]);
            gpu.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })?;
            let stamps = clock.read(&gpu)?;
            // The first frame has no temporal hint and warms every pipeline. It
            // is discarded rather than averaged in.
            if index > 0 {
                runs.push(stamps);
            }
        }
        report(
            &gpu,
            &belt,
            &runs,
            *size,
            span,
            options.iters,
            options.seed_iters,
        )?;
    }
    Ok(())
}

/// Add up every pass whose name starts with `stage`, per frame, and take the
/// median frame.
fn stage(runs: &[Vec<(String, f64)>], name: &str) -> f64 {
    let mut totals: Vec<f64> = runs
        .iter()
        .map(|frame| {
            frame
                .iter()
                .filter(|(pass, _)| pass.starts_with(name))
                .map(|(_, ms)| ms)
                .sum()
        })
        .collect();
    median(&mut totals)
}

/// What one working size costs, and what the field it produced looks like.
#[allow(clippy::too_many_arguments)]
fn report(
    gpu: &Gpu,
    belt: &Belt,
    runs: &[Vec<(String, f64)>],
    size: Size,
    span: f64,
    iters: u32,
    seed_iters: u32,
) -> Fallible<()> {
    let rectify = stage(runs, "rectify");
    let pyramid = stage(runs, "pyramid");
    let densify = stage(runs, "densify");
    let gate = stage(runs, "gate");
    let none = stage(runs, "consume.none");
    let whole = stage(runs, "consume.whole");
    let corridor = stage(runs, "consume.corridor");
    let lookup = (corridor - none).max(0.0);
    let rest = rectify + pyramid + densify + gate + lookup;

    // Which workgroup shape to quote. Chosen by the clock, on the cold ladder,
    // and named in the report rather than assumed.
    let shapes = [
        ("lanes", "64 lanes, 1 px each"),
        ("wave", "32 lanes, 2 px each"),
    ];
    let colds: [f64; 2] = shapes.map(|(tag, _)| stage(runs, &format!("search.{tag}.cold")));
    let picked = usize::from(colds[1] < colds[0]);
    let (tag, shape_name) = shapes[picked];
    let cold = colds[picked];
    let cold_l0 = stage(runs, &format!("search.{tag}.cold.L0"));
    let seeded = stage(runs, &format!("search.{tag}.seeded"));

    println!(
        "=== {} x {} strip: {:.2} px/deg along, {:.2} px/deg across, {:.1} MB resident, map \
         built in {:.2} s ===",
        size.width,
        size.height,
        f64::from(size.width) / 360.0,
        f64::from(size.height) / span,
        strip_bytes(size) as f64 / 1e6,
        belt.lut_built.as_secs_f64(),
    );
    println!(
        "{:<28} {:>9}  {:>8}  {:>8}",
        "stage", "ms/frame", "of pass", "of 33 ms"
    );
    let line = |name: &str, ms: f64| {
        println!(
            "{name:<28} {ms:>9.3}  {:>7.1}%  {:>7.1}%",
            100.0 * ms / PASS_MS,
            100.0 * ms / FRAME_MS
        );
    };
    line("1 rectify, 2 lenses", rectify);
    line("2 pyramid, 2 levels x 2", pyramid);
    line("3 search, cold ladder", cold);
    line("    of which finest level", cold_l0);
    line("3 search, seeded L0 only", seeded);
    line("   [other shape, cold]", colds[1 - picked]);
    line("4 densify", densify);
    line("5 gate", gate);
    line("6 lookup, whole picture", whole - none);
    line("6 lookup, 8 deg corridor", lookup);
    println!("{:-<28} {:->9}  {:->8}  {:->8}", "", "", "", "");
    line("BELT, cold every frame", rest + cold);
    line("BELT, seeded every frame", rest + seeded);
    println!(
        "shape:   {shape_name} is the faster of the two and is what the table above quotes; the \
         other reads {:.3} ms on the cold ladder. Per iteration of the finest level, from the \
         cold and seeded arms of the same grid: {:.3} ms, over a fixed setup of {:.3} ms.",
        colds[1 - picked],
        (cold_l0 - seeded) / f64::from(iters - seed_iters).max(1.0),
        cold_l0 - f64::from(iters) * (cold_l0 - seeded) / f64::from(iters - seed_iters).max(1.0),
    );
    println!(
        "control: the lookup pass with no belt in it reads {none:.3} ms, and the belt's share is \
         the difference, so the two lens samples the shipped pass already pays for are not \
         charged here."
    );

    // What the search found, so a reader can tell a converged field from a
    // kernel that ran and returned nothing.
    let (cols, rows) = grid(size, 0);
    for (slot, arm) in [
        (0usize, "cold".to_owned()),
        (3, "seeded".to_owned()),
        (4, format!("PLANT {:.2}", PLANTS[0])),
        (5, format!("PLANT {:.2}", PLANTS[1])),
    ] {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            &belt.answers[slot],
            0,
            &belt.readback,
            0,
            belt.readback.size(),
        );
        let submission = gpu.queue.submit([encoder.finish()]);
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        let slice = belt.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        let mapped = slice.get_mapped_range();
        let values: Vec<f32> = mapped
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect();
        drop(mapped);
        belt.readback.unmap();

        let mut kept = 0usize;
        let mut along = Vec::new();
        let mut across = Vec::new();
        let mut residual = Vec::new();
        let total = cols as usize * rows as usize;
        for patch in 0..total {
            let e = &values[patch * 4..patch * 4 + 4];
            if e[2] < GATE_RMS && e[3] > 1e-5 {
                kept += 1;
                along.push(f64::from(e[0]));
                across.push(f64::from(e[1]));
                residual.push(f64::from(e[2]));
            }
        }
        let per_deg_along = f64::from(size.width) / 360.0;
        let per_deg_across = f64::from(size.height) / span;
        let mid_along = median(&mut along.clone());
        let mid_across = median(&mut across.clone());
        println!(
            "field {arm:<6} {kept} of {total} patches under the gate ({:.1}%), median along \
             {mid_along:+.3} px = {:+.4} deg, median across {mid_across:+.3} px = {:+.4} deg, \
             median residual {:.4}",
            100.0 * kept as f64 / total as f64,
            mid_along / per_deg_along,
            mid_across / per_deg_across,
            median(&mut residual.clone()),
        );
        if slot >= 4 {
            let plant = PLANTS[slot - 4];
            let expected = -plant * per_deg_across;
            println!(
                "         the plant moved lens 0 by {plant:+.2} deg across the seam, so this row \
                 has to read {expected:+.3} px across. It reads {mid_across:+.3}, which is \
                 {:+.4} deg of error, and {mid_along:+.3} px along where it has to read 0.",
                (mid_across - expected) / per_deg_across,
            );
        }
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// 6. mode=live: the same probe with a real file playing through the app's pass.
// ---------------------------------------------------------------------------

fn live(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    println!("device: {}", dmabuf::device_report(&gpu.device));
    let running = options.belt != "off";
    println!("belt:   {}, one frame in {}", options.belt, options.every);

    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let map = seam::mapped(&calibration.lenses, frame);
    let baseline = band::baseline(&calibration.lenses);
    let span = span_of(&spokes(&map, baseline, 64));
    let size = *options.sizes.first().ok_or("live mode wants one res=")?;

    // The belt reads one real decoded frame while the player decodes and draws
    // every frame of the film. **This arm measures contention** - the belt on
    // the same queue as a live dual-4K decode and the app's own pass - and not
    // the belt's answer, which mode=cost measures on real motion.
    let lenses = [lens_texture(&gpu, frame), lens_texture(&gpu, frame)];
    {
        let mut walk = Walk::open(&options.input, options.at, frame)?;
        let pair = walk.next_pair()?.ok_or("no frame decoded for the belt")?;
        for (lens, into) in lenses.iter().enumerate() {
            upload(&gpu, &pair.lenses[lens], into);
        }
    }
    let lut = Lut::build(&map, baseline, size, span);
    let belt = Belt::new(
        &gpu,
        &lut,
        [&lenses[0], &lenses[1]],
        frame,
        options.iters,
        options.seed_iters,
        span,
    );
    let mut clock = Clock::new(&gpu, 96);

    let scene = Scene::open(&options.input)?;
    scene.fit_seam(true);
    let mut pipeline = ScenePipeline::new(&gpu.device, COLOUR);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("belt live"),
        size: OUTPUT.extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOUR,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let camera = Camera::default();
    let refresh = Duration::from_secs_f64(1.0 / f64::from(options.hz));
    let run = Duration::from_secs(options.seconds);
    println!(
        "pace:   due-time redraws on a {} Hz display for {} s, drawing {}x{}, belt strip {}x{}",
        options.hz, options.seconds, OUTPUT.width, OUTPUT.height, size.width, size.height
    );

    let start = Instant::now();
    let mut redraws = 0u64;
    let mut wall = Duration::ZERO;
    let mut cold_ms: Vec<f64> = Vec::new();
    let mut seeded_ms: Vec<f64> = Vec::new();
    let mut belt_ms: Vec<f64> = Vec::new();

    while start.elapsed() < run {
        let now = Instant::now();
        let next = match scene.pump(now) {
            Next::At(due) => due,
            Next::Refresh => now + refresh,
            Next::Never => break,
            Next::Stopped(stall) => {
                eprintln!("play:   stopped: {stall}");
                break;
            }
        };
        let primitive = scene.primitive(camera);
        pipeline.prepare(
            &primitive,
            &gpu.device,
            &gpu.queue,
            OUTPUT.width as f32 / OUTPUT.height as f32,
        );

        let began = Instant::now();
        let view = target.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("belt live"),
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
        clock.start();
        // Studio's cadence, when it is asked for: the belt runs one frame in
        // `every` and the picture holds the last field between, which is what
        // `frame_interval_of_flow_calc_` does.
        let due = running && redraws.is_multiple_of(options.every);
        if due {
            encode_frame(
                &belt,
                &mut clock,
                &mut encoder,
                match (redraws == 0, options.belt.as_str()) {
                    // A first frame has no hint, so it pays the ladder whatever
                    // arm this run is; after that the arm decides.
                    (true, _) | (_, "cold") => Work::Cold,
                    _ => Work::Seeded,
                },
            );
            clock.finish(&mut encoder);
        }
        let submission = gpu.queue.submit([encoder.finish()]);
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        wall += began.elapsed();
        if due && redraws > 0 {
            let stamps = clock.read(&gpu)?;
            let sum = |name: &str| -> f64 {
                stamps
                    .iter()
                    .filter(|(pass, _)| pass.starts_with(name))
                    .map(|(_, ms)| ms)
                    .sum()
            };
            let rest = sum("rectify")
                + sum("pyramid")
                + sum("densify")
                + sum("gate")
                + (sum("consume.corridor") - sum("consume.none")).max(0.0);
            cold_ms.push(sum("search.wave.cold"));
            seeded_ms.push(sum("search.wave.seeded"));
            belt_ms.push(rest + sum("search.wave"));
        }
        redraws += 1;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }

    let elapsed = start.elapsed();
    let stats = scene.stats().ok_or("no player")?;
    println!(
        "play:   {redraws} redraws, {:.2} s played, {}",
        scene.position(Instant::now()).as_secs_f64(),
        stats.report(elapsed)
    );
    println!(
        "cost:   {:.2} ms per redraw, wall, the app's pass and the belt together, over every \
         redraw whether the belt ran on it or not",
        wall.as_secs_f64() * 1000.0 / redraws.max(1) as f64
    );
    if running && !belt_ms.is_empty() {
        let mut sorted = belt_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "belt:   {:.3} ms/frame, median of {} frames; search cold {:.3}, \
             search seeded {:.3}",
            median(&mut belt_ms.clone()),
            belt_ms.len(),
            median(&mut cold_ms.clone()),
            median(&mut seeded_ms.clone()),
        );
        println!(
            "belt:   best {:.3}, p90 {:.3}, worst {:.3} ms",
            sorted[0],
            sorted[sorted.len() * 9 / 10],
            sorted[sorted.len() - 1],
        );
    }
    Ok(())
}
