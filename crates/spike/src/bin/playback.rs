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
//! cargo run --release -p kjerag-spike --bin playback -- <file.insv> [seconds] [hz] [shots]
//! ```
//!
//! `shots` is issue #15's measurement: that many screen captures, spread
//! over the run, taken through the same [`Scene::capture`] the `s` key
//! reaches, while the file plays. What has to stay true is the pacing
//! report underneath it, so the number this instrument exists to produce is
//! dropped and starved with a capture burst running.
//!
//! `range=START:COUNT out-dir=NEW_DIRECTORY` is the exact consecutive-frame
//! diagnostic. It runs the selected production player causally from frame
//! zero, consumes every frame, and captures each displayed frame in the
//! requested range at [`SHOT_WIDTH`]. It publishes the PNGs and a receipt
//! together only after the complete range has passed its frame and run checks.
//!
//! `measure=START:COUNT pace=off receipt=NEW_FILE bench=0` is narrower: every
//! warm-up and measured transaction must expose the exact selected ONE X2
//! direct-map stamp for its current delivery. That is the production route's
//! public proof that preparation selected the direct native type-2 draw rather
//! than a generic camera route. A missing or mismatched stamp refuses the
//! benchmark.
//!
//! Timed `shots` land in ./scratch/ (gitignored). Exact target and range
//! modes write only to their named outputs. Frames of real footage are
//! personal video and this repo is public.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use kjerag_media::{Fallible, FrameStamp, Reader};
use kjerag_render::{
    Camera, Extent, Horizon, Next, OneXsMapFrame, PisBackend, Readout, Request, Sampling, Scene,
    ScenePipeline, Shot, Size, Sweep, dmabuf,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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

/// What the app asks for (`kjerag::shot::WIDTH`), so the burst costs what a
/// pilot's `s` key costs.
const SHOT_WIDTH: u32 = 3840;

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let options = Options::parse(&args)?;
    let evidence = options
        .evidence_out
        .as_deref()
        .map(|out| EvidenceRun::authenticate(&options.input, out))
        .transpose()?;
    if let Some(receipt) = options.receipt.as_deref() {
        ensure_measure_destination(receipt)?;
    }
    if let (Some(_), Some(out)) = (options.range, options.out_dir.as_deref()) {
        ensure_range_destination(out)?;
    }
    let range_sources = options
        .range
        .map(|_| AuthenticatedPair::open(&options.input))
        .transpose()?;
    let measure_sources = options
        .measure
        .map(|_| AuthenticatedPair::open(&options.input))
        .transpose()?;
    let range_provenance = options
        .range
        .map(|_| RangeProvenance::authenticate())
        .transpose()?;
    let measure_provenance = options
        .measure
        .map(|_| RangeProvenance::authenticate())
        .transpose()?;

    if options.bench {
        for lookahead in [0, 2, 4] {
            println!("{}", drain(&options.input, lookahead)?);
        }
        println!();
    }
    let camera = Camera {
        yaw: options.yaw.to_radians(),
        pitch: options.pitch.to_radians(),
        fov: options.fov.to_radians(),
    };
    play(
        &options.input,
        Run::new(&options),
        options.hz,
        options.shots,
        Drawn {
            camera,
            horizon: if options.lock {
                Horizon::Locked
            } else {
                Horizon::Free
            },
            readout: &options.readout,
            sampling: match options.sample.as_str() {
                "bilinear" => Sampling::Bilinear,
                "luma" => Sampling::Luma,
                _ => Sampling::Sharp,
            },
            band: options.band != "noband",
            tone: options.band != "notone" && options.band != "noband",
        },
        RunBindings {
            evidence,
            range_sources,
            range_provenance,
            measure_sources,
            measure_provenance,
        },
    )
}

const USAGE: &str = "usage: playback <file.insv> [seconds] [hz] [shots] [yaw] \
     [file|off|right|left|down|up] [fov] [bilinear|luma|sharp] [band|noband] \
     [target=N] [bench=0|1] [yaw=deg] [pitch=deg] [fov=deg] [lock=0|1] [out=PNG] \
     [evidence-out=NEW-DIRECTORY] [range=START:COUNT out-dir=NEW-DIRECTORY] \
     [measure=START:COUNT pace=off receipt=NEW-FILE]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RangeSpec {
    start: u64,
    count: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MeasureSpec {
    start: u64,
    count: u64,
    end: u64,
}

impl MeasureSpec {
    fn parse(raw: &str) -> Fallible<Self> {
        let (start, count) = raw.split_once(':').ok_or("measure= must be START:COUNT")?;
        if start.is_empty() || count.is_empty() || count.contains(':') {
            return Err("measure= must be START:COUNT".into());
        }
        let start = start
            .parse::<u64>()
            .map_err(|error| format!("bad measure start: {error}"))?;
        let count = count
            .parse::<u64>()
            .map_err(|error| format!("bad measure count: {error}"))?;
        if start == 0 {
            return Err("measure start must leave at least one warm-up frame".into());
        }
        if count == 0 {
            return Err("measure count must be greater than zero".into());
        }
        let end = start
            .checked_add(count)
            .and_then(|exclusive| exclusive.checked_sub(1))
            .ok_or("measure end cannot be counted")?;
        Ok(Self { start, count, end })
    }
}

impl RangeSpec {
    fn parse(raw: &str) -> Fallible<Self> {
        let (start, count) = raw.split_once(':').ok_or("range= must be START:COUNT")?;
        if start.is_empty() || count.is_empty() || count.contains(':') {
            return Err("range= must be START:COUNT".into());
        }
        let start = start
            .parse::<u64>()
            .map_err(|error| format!("bad range start: {error}"))?;
        let count = count
            .parse::<u64>()
            .map_err(|error| format!("bad range count: {error}"))?;
        if count == 0 {
            return Err("range count must be greater than zero".into());
        }
        let exclusive = start
            .checked_add(count)
            .ok_or("range end cannot be counted by presentation statistics")?;
        let end = exclusive - 1;
        Ok(Self { start, count, end })
    }
}

struct Options {
    input: PathBuf,
    seconds: u64,
    hz: u32,
    shots: u32,
    yaw: f32,
    pitch: f32,
    readout: String,
    fov: f32,
    sample: String,
    band: String,
    target: Option<u64>,
    bench: bool,
    lock: bool,
    out: Option<PathBuf>,
    evidence_out: Option<PathBuf>,
    range: Option<RangeSpec>,
    out_dir: Option<PathBuf>,
    measure: Option<MeasureSpec>,
    receipt: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[String]) -> Fallible<Self> {
        let input = PathBuf::from(args.get(1).ok_or(USAGE)?);
        let mut positional = Vec::new();
        let mut named = std::collections::HashMap::new();
        for argument in args.iter().skip(2) {
            let Some((name, value)) = argument.split_once('=') else {
                positional.push(argument.as_str());
                continue;
            };
            if value.is_empty() {
                return Err(format!("{name}= needs a value").into());
            }
            if named.insert(name, value).is_some() {
                return Err(format!("{name}= was named more than once").into());
            }
        }
        if positional.len() > 8 {
            return Err(USAGE.into());
        }
        for name in named.keys() {
            if !matches!(
                *name,
                "target"
                    | "bench"
                    | "yaw"
                    | "pitch"
                    | "fov"
                    | "lock"
                    | "out"
                    | "evidence-out"
                    | "range"
                    | "out-dir"
                    | "measure"
                    | "pace"
                    | "receipt"
            ) {
                return Err(format!("unknown playback option {name}=").into());
            }
        }

        let value = |index: usize| positional.get(index).copied();
        let seconds = parsed(value(0), 60, "seconds")?;
        let hz = parsed(value(1), 60, "hz")?;
        let shots = parsed(value(2), 0, "shots")?;
        let yaw = parsed(named.get("yaw").copied().or_else(|| value(3)), 0.0, "yaw")?;
        let pitch = parsed(named.get("pitch").copied(), 0.0, "pitch")?;
        let readout = value(4).unwrap_or("file").to_owned();
        let fov = parsed(
            named.get("fov").copied().or_else(|| value(5)),
            Camera::default().fov.to_degrees(),
            "fov",
        )?;
        let sample = value(6).unwrap_or("sharp").to_owned();
        let band = value(7).unwrap_or("band").to_owned();
        if !matches!(
            readout.as_str(),
            "file" | "off" | "right" | "left" | "down" | "up"
        ) {
            return Err("readout must be file, off, right, left, down or up".into());
        }
        if !matches!(sample.as_str(), "bilinear" | "luma" | "sharp") {
            return Err("sampling must be bilinear, luma or sharp".into());
        }
        if !matches!(band.as_str(), "band" | "notone" | "noband") {
            return Err("band mode must be band, notone or noband".into());
        }
        let target = named
            .get("target")
            .map(|value| {
                value
                    .parse()
                    .map_err(|error| format!("bad target: {error}"))
            })
            .transpose()?;
        let bench = bit(named.get("bench").copied(), true, "bench")?;
        let lock = bit(named.get("lock").copied(), true, "lock")?;
        let out = named.get("out").map(PathBuf::from);
        let evidence_out = named.get("evidence-out").map(PathBuf::from);
        let range = named
            .get("range")
            .map(|value| RangeSpec::parse(value))
            .transpose()?;
        let out_dir = named.get("out-dir").map(PathBuf::from);
        let measure = named
            .get("measure")
            .map(|value| MeasureSpec::parse(value))
            .transpose()?;
        let pace_off = match named.get("pace").copied() {
            None => false,
            Some("off") => true,
            Some(_) => return Err("pace= must be off".into()),
        };
        let receipt = named.get("receipt").map(PathBuf::from);

        if target.is_none() && (out.is_some() || evidence_out.is_some()) {
            return Err("out= and evidence-out= are only valid with target=".into());
        }
        if target.is_some() && range.is_some() {
            return Err("target= and range= are mutually exclusive".into());
        }
        if measure.is_some() && (target.is_some() || range.is_some()) {
            return Err("measure= cannot be combined with target= or range=".into());
        }
        if target.is_some() && shots != 0 {
            return Err("target= owns its exact capture; positional shots must be 0".into());
        }
        if range.is_some() && shots != 0 {
            return Err("range= owns its exact captures; positional shots must be 0".into());
        }
        if range.is_some() && evidence_out.is_some() {
            return Err("range= and evidence-out= are mutually exclusive".into());
        }
        if range.is_some() != out_dir.is_some() {
            return Err("range= and out-dir= must be supplied together".into());
        }
        if measure.is_some() != receipt.is_some() || measure.is_some() != pace_off {
            return Err("measure=, pace=off and receipt= must be supplied together".into());
        }
        if measure.is_some() && (shots != 0 || out.is_some() || evidence_out.is_some()) {
            return Err("measure= cannot capture pictures or production evidence".into());
        }
        if measure.is_some() && bench {
            return Err("measure= requires bench=0 so no decode benchmark precedes it".into());
        }
        if evidence_out.is_some() && bench {
            return Err(
                "evidence-out= requires bench=0 so only descriptor-bound playback decodes".into(),
            );
        }
        if hz == 0 {
            return Err("hz must be greater than zero".into());
        }

        Ok(Self {
            input,
            seconds,
            hz,
            shots,
            yaw,
            pitch,
            readout,
            fov,
            sample,
            band,
            target,
            bench,
            lock,
            out,
            evidence_out,
            range,
            out_dir,
            measure,
            receipt,
        })
    }
}

fn parsed<T: std::str::FromStr>(raw: Option<&str>, fallback: T, name: &str) -> Fallible<T>
where
    T::Err: std::fmt::Display,
{
    match raw {
        None => Ok(fallback),
        Some(raw) => raw
            .parse()
            .map_err(|error| format!("bad {name}: {error}").into()),
    }
}

fn bit(raw: Option<&str>, fallback: bool, name: &str) -> Fallible<bool> {
    match raw {
        None => Ok(fallback),
        Some("0") => Ok(false),
        Some("1") => Ok(true),
        Some(_) => Err(format!("{name}= must be 0 or 1").into()),
    }
}

/// What the readout argument does to the file's own (issue #9): a direction
/// forces that one, `off` is a readout of no length at all, which is the pass
/// as it was before the correction existed, and anything else leaves the file
/// alone.
fn forced(readout: &str) -> Option<fn(Readout) -> Readout> {
    match readout {
        "right" => Some(|file| Readout {
            sweep: Sweep::Right,
            ..file
        }),
        "left" => Some(|file| Readout {
            sweep: Sweep::Left,
            ..file
        }),
        "down" => Some(|file| Readout {
            sweep: Sweep::Down,
            ..file
        }),
        "up" => Some(|file| Readout {
            sweep: Sweep::Up,
            ..file
        }),
        "off" => Some(|file| Readout {
            seconds: 0.0,
            ..file
        }),
        _ => None,
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
/// (`iced`'s `RedrawRequest::At`, from `kjerag_render`'s widget), so this
/// does too. `hz` is the display's refresh rate, and caps how often a redraw
/// can happen when the scene asks for one as soon as possible.
/// What one run is asked to draw, as against how long for: the arguments that
/// describe the picture rather than the measurement.
struct Drawn<'a> {
    camera: Camera,
    horizon: Horizon,
    readout: &'a str,
    sampling: Sampling,
    /// Whether the per-frame seam band measures (issue #103). Off is the pass
    /// as it was before it, which is what its cost is measured against.
    band: bool,
    /// Whether the ring's readings are pooled into an exposure (stage 3). Off
    /// with `band` on is the pass as stage 2 left it, which is what stage 3's
    /// own share of a redraw is measured against.
    tone: bool,
}

enum Run {
    Timed(Duration),
    Target(Target),
    Range(RangeRun),
    Measure(MeasureRun),
}

struct Target {
    index: u64,
    out: PathBuf,
    evidence_out: Option<PathBuf>,
}

struct RangeRun {
    spec: RangeSpec,
    out_dir: PathBuf,
}

struct MeasureRun {
    spec: MeasureSpec,
    receipt: PathBuf,
}

struct RunBindings {
    evidence: Option<EvidenceRun>,
    range_sources: Option<AuthenticatedPair>,
    range_provenance: Option<RangeProvenance>,
    measure_sources: Option<AuthenticatedPair>,
    measure_provenance: Option<RangeProvenance>,
}

impl Run {
    fn new(options: &Options) -> Self {
        match (options.target, options.range, options.measure) {
            (Some(index), None, None) => Self::Target(Target {
                index,
                out: options.out.clone().unwrap_or_else(|| {
                    PathBuf::from("scratch").join(format!("playback-frame{index}.png"))
                }),
                evidence_out: options.evidence_out.clone(),
            }),
            (None, Some(spec), None) => Self::Range(RangeRun {
                spec,
                out_dir: options
                    .out_dir
                    .clone()
                    .expect("parsed range mode has an output directory"),
            }),
            (None, None, Some(spec)) => Self::Measure(MeasureRun {
                spec,
                receipt: options
                    .receipt
                    .clone()
                    .expect("parsed measure mode has a receipt"),
            }),
            (None, None, None) => Self::Timed(Duration::from_secs(options.seconds)),
            _ => unreachable!("parser rejects competing run modes"),
        }
    }

    fn duration(&self) -> Option<Duration> {
        match self {
            Self::Timed(duration) => Some(*duration),
            Self::Target(_) | Self::Range(_) | Self::Measure(_) => None,
        }
    }
}

fn play(
    input: &Path,
    run: Run,
    hz: u32,
    shots: u32,
    drawn: Drawn<'_>,
    bindings: RunBindings,
) -> Fallible<()> {
    let RunBindings {
        evidence,
        range_sources,
        range_provenance,
        measure_sources,
        measure_provenance,
    } = bindings;
    let Drawn {
        camera,
        horizon,
        readout,
        sampling,
        band,
        tone,
    } = drawn;
    let mut range_output = match &run {
        Run::Range(range) => Some(RangeOutput::begin(&range.out_dir, range.spec)?),
        _ => None,
    };
    let gpu = Gpu::new()?;
    println!("gpu:    {}", gpu.adapter.get_info().name);
    println!("device: {}", dmabuf::device_report(&gpu.device));

    // An instrument has no stored calibration to read, and this is not the
    // app. It draws the factory calibration, the parity base: the per-capture
    // seam fit was the non-parity mechanism and is gone (issue #48, 2026-08-15).
    let mut scene = match (&evidence, &range_sources, &measure_sources) {
        (Some(evidence), None, None) => {
            let [first, second] = evidence.sources.descriptor_paths()?;
            Scene::open_pair(&first, &second)?
        }
        (None, Some(sources), None) | (None, None, Some(sources)) => {
            let [first, second] = sources.descriptor_paths()?;
            Scene::open_pair(&first, &second)?
        }
        (None, None, None) => Scene::open(input)?,
        _ => unreachable!("parser makes authenticated modes mutually exclusive"),
    };
    if let Some(sources) = &range_sources {
        let actual = scene
            .source_paths()
            .ok_or("range scene has no admitted source paths")?;
        sources.require_descriptor_order(&actual)?;
    }
    if let Some(sources) = &measure_sources {
        let actual = scene
            .source_paths()
            .ok_or("measure scene has no admitted source paths")?;
        sources.require_descriptor_order(&actual)?;
    }
    scene.set_horizon(horizon);
    if let (Some(forced), Some(file)) = (forced(readout), scene.readout()) {
        scene.set_readout(Some(forced(file)));
    }
    println!("shutter: readout {readout}");
    scene.set_sampling(sampling);
    println!("sample: {sampling:?}");
    let mut pipeline = ScenePipeline::new(&gpu.device, &gpu.queue, FORMAT);
    pipeline.hold_band(!band);
    pipeline.hold_tone(!tone);
    println!("seam:   band {band}, exposure {tone}");
    let refresh = Duration::from_secs_f64(1.0 / f64::from(hz));
    match &run {
        Run::Timed(duration) => println!(
            "pace:   due-time redraws on a {hz} Hz display for {} s, rendering {}x{} at yaw \
             {:.0}, fov {:.0}",
            duration.as_secs(),
            OUTPUT.width,
            OUTPUT.height,
            camera.yaw.to_degrees(),
            camera.fov.to_degrees(),
        ),
        Run::Target(target) => println!(
            "pace:   every live frame from 0 through {}, rendering {}x{} at yaw {:.2}, pitch \
             {:.2}, fov {:.2}, lock {}; no seek",
            target.index,
            OUTPUT.width,
            OUTPUT.height,
            camera.yaw.to_degrees(),
            camera.pitch.to_degrees(),
            camera.fov.to_degrees(),
            u8::from(horizon == Horizon::Locked),
        ),
        Run::Range(range) => println!(
            "pace:   every live frame from 0 through {}, capturing {}..={} at {} px, yaw \
             {:.2}, pitch {:.2}, fov {:.2}, lock {}; no seek",
            range.spec.end,
            range.spec.start,
            range.spec.end,
            SHOT_WIDTH,
            camera.yaw.to_degrees(),
            camera.pitch.to_degrees(),
            camera.fov.to_degrees(),
            u8::from(horizon == Horizon::Locked),
        ),
        Run::Measure(measure) => println!(
            "pace:   causal frames 0 through {} without pacing; warm-up 0..={}, measure \
             {}..={} at {}x{}, yaw {:.2}, pitch {:.2}, fov {:.2}, lock {}",
            measure.spec.end,
            measure.spec.start - 1,
            measure.spec.start,
            measure.spec.end,
            OUTPUT.width,
            OUTPUT.height,
            camera.yaw.to_degrees(),
            camera.pitch.to_degrees(),
            camera.fov.to_degrees(),
            u8::from(horizon == Horizon::Locked),
        ),
    }

    if let Run::Measure(measure) = &run {
        return measure_playback(
            &mut scene,
            &mut pipeline,
            &gpu,
            MeasureView {
                camera,
                horizon,
                readout,
                sampling,
                band,
                tone,
            },
            measure,
            measure_sources
                .as_ref()
                .ok_or("measure sources were not authenticated")?,
            measure_provenance
                .as_ref()
                .ok_or("measure build provenance was not authenticated")?,
        );
    }

    let start = Instant::now();
    let cpu = Cpu::now();
    let (mut redraws, mut render) = (0u64, Duration::ZERO);
    let mut burst = Burst::new(shots, run.duration().unwrap_or_default());
    let (exact_written, exact_report) = mpsc::channel();
    let mut last_progress = None;

    while run
        .duration()
        .is_none_or(|duration| start.elapsed() < duration)
    {
        let now = Instant::now();
        let next = match scene.pump(now) {
            Next::At(due) => due,
            Next::Refresh => now + refresh,
            Next::Never => break,
            // The window puts this in an alert (issue #124). An instrument
            // has a terminal and a run to end, and a run whose picture died
            // part way through must not be reported as a clean one.
            Next::Stopped(stall) => {
                match &run {
                    Run::Target(_) => {
                        return Err(format!("target playback stopped: {stall}").into());
                    }
                    Run::Range(_) => {
                        return Err(format!("range playback stopped: {stall}").into());
                    }
                    Run::Measure(_) => unreachable!("measure mode has its own loop"),
                    Run::Timed(_) => {}
                }
                eprintln!("play:   stopped: {stall}");
                break;
            }
        };
        let offered = scene.frame();
        let target_hit = match (&run, offered) {
            (Run::Target(target), Some((index, _))) => index == target.index,
            _ => false,
        };
        let range_hit = match (&run, offered, range_output.as_ref()) {
            (Run::Range(range), Some((index, _)), Some(output)) => {
                if output.next_index() <= range.spec.end && index > output.next_index() {
                    return Err(format!(
                        "range skipped displayed frame {}; next required frame is {}",
                        index,
                        output.next_index()
                    )
                    .into());
                }
                index == output.next_index() && index <= range.spec.end
            }
            _ => false,
        };
        if let (Run::Target(target), Some((index, timestamp))) = (&run, offered)
            && last_progress != Some(index)
            && (index == 0 || index % 100 == 0 || index == target.index)
        {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = (index + 1) as f64 / elapsed.max(f64::EPSILON);
            let left = target.index.saturating_sub(index) as f64 / rate;
            println!(
                "frame:  {index}/{} at {:.6} s, {:.2} frames/s, ETA {:.0} s",
                target.index,
                timestamp.as_secs_f64(),
                rate,
                left,
            );
            last_progress = Some(index);
        }
        if let (Run::Range(range), Some((index, timestamp))) = (&run, offered)
            && last_progress != Some(index)
            && (index == 0 || index % 100 == 0 || index == range.spec.end)
        {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = (index + 1) as f64 / elapsed.max(f64::EPSILON);
            let left = range.spec.end.saturating_sub(index) as f64 / rate;
            println!(
                "frame:  {index}/{} at {:.6} s, {:.2} frames/s, ETA {:.0} s",
                range.spec.end,
                timestamp.as_secs_f64(),
                rate,
                left,
            );
            last_progress = Some(index);
        }

        let armed = !target_hit && !range_hit && burst.due(start.elapsed());
        if armed {
            scene.capture(burst.request());
        }
        if target_hit || range_hit {
            let written = exact_written.clone();
            scene.capture(Request {
                width: SHOT_WIDTH,
                then: Box::new(move |shot| {
                    let _ = written.send(shot);
                }),
            });
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

        if let Some(output) = range_output.as_mut()
            && let Some(map) = scene.diagnostic_one_xs_map()?
        {
            output.observe_transaction(&map)?;
        }

        if target_hit || range_hit {
            let capture_label = if range_hit { "range" } else { "target" };
            let (expected_index, expected_timestamp) =
                offered.expect("exact capture hit has a source frame");
            let shot = exact_report
                .recv_timeout(Duration::from_secs(30))
                .map_err(|error| {
                    format!("{capture_label} capture did not finish within 30 s: {error}")
                })??;
            if shot.index != expected_index {
                return Err(format!(
                    "{capture_label} capture holds frame {} but frame {expected_index} was prepared",
                    shot.index
                )
                .into());
            }
            if shot.time != expected_timestamp {
                return Err(format!(
                    "{capture_label} capture time {:.9} differs from prepared frame time {:.9}",
                    shot.time.as_secs_f64(),
                    expected_timestamp.as_secs_f64()
                )
                .into());
            }

            if range_hit {
                if scene.displayed_frame() != Some((expected_index, expected_timestamp)) {
                    return Err(format!(
                        "range frame {expected_index} was captured without that exact frame being displayed"
                    )
                    .into());
                }
                let current_stamp = scene
                    .frame_stamp()
                    .ok_or("range scene lost its exact delivered frame stamp")?;
                if current_stamp.index() != expected_index
                    || current_stamp.timestamp() != expected_timestamp
                {
                    return Err("range scene stamp changed after the picture was displayed".into());
                }
                let map = scene.diagnostic_one_xs_map()?.ok_or(
                    "range scene has no shown selected ONE X2 map for its current delivery",
                )?;
                if map.frame() != &current_stamp {
                    return Err(
                        "range ONE X2 map differs from the scene's exact shown delivery".into(),
                    );
                }
                let output = range_output
                    .as_mut()
                    .expect("range mode created its staged output");
                output.save(&shot, &map)?;
                let range = match &run {
                    Run::Range(range) => range,
                    _ => unreachable!("range hit belongs to range mode"),
                };
                if expected_index == range.spec.end {
                    let stats = scene.stats().ok_or("no player")?;
                    let expected_presented = range
                        .spec
                        .end
                        .checked_add(1)
                        .ok_or("range end cannot be counted")?;
                    if stats.presented != expected_presented
                        || stats.dropped != 0
                        || stats.starved != 0
                    {
                        return Err(format!(
                            "range ended after {} presented, {} dropped and {} starved; expected \
                             {expected_presented} presented, 0 dropped and 0 starved",
                            stats.presented, stats.dropped, stats.starved
                        )
                        .into());
                    }
                    let sources = range_sources
                        .as_ref()
                        .ok_or("range source pair was not authenticated before playback")?;
                    sources.verify()?;
                    let provenance = range_provenance
                        .as_ref()
                        .ok_or("range build provenance was not authenticated before playback")?;
                    let build = provenance.verify()?;
                    output.publish(RangeReceipt {
                        camera,
                        horizon,
                        output: OUTPUT,
                        stats,
                        redraws,
                        elapsed: start.elapsed(),
                        sources: sources.receipt(),
                        readout: readout.to_owned(),
                        sampling: format!("{sampling:?}"),
                        band,
                        tone,
                        build,
                    })?;
                    println!(
                        "range:  frames {}..={} at {} px, {} presented, 0 dropped, 0 starved, {}",
                        range.spec.start,
                        range.spec.end,
                        SHOT_WIDTH,
                        stats.presented,
                        range.out_dir.display(),
                    );
                    return Ok(());
                }
                if let Some(wait) = next.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
                continue;
            }

            let target = match &run {
                Run::Target(target) => target,
                Run::Timed(_) | Run::Range(_) | Run::Measure(_) => {
                    unreachable!("target hit belongs to target mode")
                }
            };
            if shot.index != target.index {
                return Err(format!(
                    "target capture holds frame {} but frame {expected_index} was prepared",
                    shot.index
                )
                .into());
            }
            write_png_to(&shot, &target.out)?;
            let stats = scene.stats().ok_or("no player")?;
            let expected_presented = target
                .index
                .checked_add(1)
                .ok_or("target frame cannot be counted")?;
            if stats.presented != expected_presented || stats.dropped != 0 {
                return Err(format!(
                    "target frame {} rendered after {} presented and {} dropped; expected {} \
                     presented and 0 dropped",
                    target.index, stats.presented, stats.dropped, expected_presented
                )
                .into());
            }
            if let Some(out) = &target.evidence_out {
                let evidence = evidence
                    .as_ref()
                    .ok_or("target evidence was not authenticated before playback")?;
                let current_stamp = scene
                    .frame_stamp()
                    .ok_or("target scene lost its exact delivered frame stamp")?;
                let map = scene.diagnostic_one_xs_map()?.ok_or(
                    "target scene has no shown selected ONE X2 map for its current delivery",
                )?;
                if map.frame() != &current_stamp {
                    return Err(
                        "diagnostic ONE X2 map differs from the scene's exact current delivery"
                            .into(),
                    );
                }
                if current_stamp.index() != expected_index
                    || current_stamp.timestamp() != expected_timestamp
                {
                    return Err("target scene stamp changed after the picture was prepared".into());
                }
                let sources = scene
                    .source_paths()
                    .ok_or("target scene has no admitted source paths")?;
                evidence.sources.require_descriptor_order(&sources)?;
                persist_target_evidence(TargetEvidence {
                    out,
                    evidence,
                    png: &target.out,
                    map: &map,
                    expected: (expected_index, expected_timestamp),
                    stats,
                })?;
            }
            println!(
                "target: frame {} at {:.9} s, {} presented, 0 dropped, {}",
                shot.index,
                shot.time.as_secs_f64(),
                stats.presented,
                target.out.display(),
            );
            return Ok(());
        }

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
    match run {
        Run::Timed(_) => Ok(()),
        Run::Target(target) => Err(format!(
            "file ended before target frame {} was rendered",
            target.index
        )
        .into()),
        Run::Range(range) => Err(format!(
            "file ended before complete range {}..={} was displayed",
            range.spec.start, range.spec.end
        )
        .into()),
        Run::Measure(_) => unreachable!("measure mode returns from its own loop"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MeasuredFrame {
    index: u64,
    timestamp: Duration,
    route: MeasuredRoute,
    source_ns: u64,
    primitive_ns: u64,
    prepare_ns: u64,
    draw_ns: u64,
    transaction_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeasuredRoute {
    OneX2DirectType2,
}

impl MeasuredRoute {
    const fn receipt_name(self) -> &'static str {
        match self {
            Self::OneX2DirectType2 => "one-x2-direct-type-2",
        }
    }
}

struct MeasureSamples {
    frames: Vec<MeasuredFrame>,
    transaction_ns: Vec<u64>,
    source_ns: Vec<u64>,
    primitive_ns: Vec<u64>,
    prepare_ns: Vec<u64>,
    draw_ns: Vec<u64>,
}

impl MeasureSamples {
    fn with_capacity(count: usize) -> Self {
        Self {
            frames: Vec::with_capacity(count),
            transaction_ns: Vec::with_capacity(count),
            source_ns: Vec::with_capacity(count),
            primitive_ns: Vec::with_capacity(count),
            prepare_ns: Vec::with_capacity(count),
            draw_ns: Vec::with_capacity(count),
        }
    }

    fn push(&mut self, frame: MeasuredFrame) {
        self.transaction_ns.push(frame.transaction_ns);
        self.source_ns.push(frame.source_ns);
        self.primitive_ns.push(frame.primitive_ns);
        self.prepare_ns.push(frame.prepare_ns);
        self.draw_ns.push(frame.draw_ns);
        self.frames.push(frame);
    }

    fn len(&self) -> usize {
        self.frames.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AdapterIdentity {
    name: String,
    vendor: u32,
    device: u32,
    device_type: String,
    pci_bus_id: String,
    driver: String,
    driver_info: String,
    backend: String,
}

impl AdapterIdentity {
    fn read(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();
        Self {
            name: info.name,
            vendor: info.vendor,
            device: info.device,
            device_type: format!("{:?}", info.device_type),
            pci_bus_id: info.device_pci_bus_id,
            driver: info.driver,
            driver_info: info.driver_info,
            backend: format!("{:?}", info.backend),
        }
    }

    fn verify(&self, adapter: &wgpu::Adapter) -> Fallible<()> {
        require_adapter_identity(self, &Self::read(adapter))
    }
}

fn require_adapter_identity(bound: &AdapterIdentity, current: &AdapterIdentity) -> Fallible<()> {
    if bound != current {
        return Err("GPU adapter identity changed during measurement".into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct MeasureView<'a> {
    camera: Camera,
    horizon: Horizon,
    readout: &'a str,
    sampling: Sampling,
    band: bool,
    tone: bool,
}

fn measure_playback(
    scene: &mut Scene,
    pipeline: &mut ScenePipeline,
    gpu: &Gpu,
    view: MeasureView<'_>,
    run: &MeasureRun,
    sources: &AuthenticatedPair,
    provenance: &RangeProvenance,
) -> Fallible<()> {
    let adapter = AdapterIdentity::read(&gpu.adapter);
    let mut drive_now = Instant::now();
    for expected in 0..run.spec.start {
        draw_measured_frame(scene, pipeline, gpu, view.camera, expected, &mut drive_now)?;
    }

    // This reading is deliberately after frame START-1's waited GPU submit.
    // It is the boundary between the causal warm-up and the measured window.
    let before = scene.stats().ok_or("measure scene has no player")?;
    let count = usize::try_from(run.spec.count).map_err(|_| "measure count does not fit memory")?;
    // Allocate every timing series before the clock starts. Pushing exactly
    // `count` samples cannot grow any of these vectors inside the interval.
    let mut samples = MeasureSamples::with_capacity(count);
    let interval_started = Instant::now();
    let mut interval_ended = None;
    for expected in run.spec.start..=run.spec.end {
        let frame =
            draw_measured_frame(scene, pipeline, gpu, view.camera, expected, &mut drive_now)?;
        if expected == run.spec.end {
            // Stop immediately after the final transaction. Recording that
            // last sample and all receipt work remain outside elapsed_ns.
            interval_ended = Some(Instant::now());
        }
        samples.push(frame);
    }
    let interval = interval_ended
        .ok_or("measure interval completed without an end timestamp")?
        .duration_since(interval_started);
    adapter.verify(&gpu.adapter)?;
    let stats = scene
        .stats()
        .ok_or("measure scene has no player")?
        .since(before);
    if samples.len() as u64 != run.spec.count
        || stats.presented != run.spec.count
        || stats.dropped != 0
    {
        return Err(format!(
            "measure window completed {} transactions after {} presented and {} dropped; expected {} transactions, {} presented and 0 dropped",
            samples.len(),
            stats.presented,
            stats.dropped,
            run.spec.count,
            run.spec.count,
        )
        .into());
    }

    sources.verify()?;
    let build = provenance.verify()?;
    let receipt = measure_receipt(MeasureReceipt {
        run,
        camera: view.camera,
        horizon: view.horizon,
        readout: view.readout,
        sampling: view.sampling,
        band: view.band,
        tone: view.tone,
        interval,
        stats,
        samples: &samples,
        sources: sources.receipt(),
        build,
        adapter: &adapter,
    })?;
    publish_measure_receipt(&run.receipt, &receipt)?;
    println!(
        "measure: frames {}..={} after {} warm-up frames, {:.3} frames/s, {}",
        run.spec.start,
        run.spec.end,
        run.spec.start,
        run.spec.count as f64 / interval.as_secs_f64().max(f64::EPSILON),
        run.receipt.display(),
    );
    Ok(())
}

fn draw_measured_frame(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    gpu: &Gpu,
    camera: Camera,
    expected: u64,
    drive_now: &mut Instant,
) -> Fallible<MeasuredFrame> {
    let transaction_started = Instant::now();
    let timestamp = loop {
        let next = scene.pump(*drive_now);
        match next {
            Next::At(due) => *drive_now = due,
            Next::Refresh => std::thread::yield_now(),
            Next::Never => {
                return Err(format!("file ended before measure frame {expected}").into());
            }
            Next::Stopped(stall) => {
                return Err(format!("measure playback stopped: {stall}").into());
            }
        }
        match scene.frame() {
            Some((index, timestamp)) if index == expected => break timestamp,
            Some((index, _)) if index > expected => {
                return Err(format!(
                    "measure skipped source frame {expected} and offered frame {index}"
                )
                .into());
            }
            _ => {}
        }
    };
    let source_done = Instant::now();
    let primitive = scene.primitive(camera);
    let primitive_done = Instant::now();
    pipeline.prepare(
        &primitive,
        &gpu.device,
        &gpu.queue,
        OUTPUT.width as f32 / OUTPUT.height as f32,
    );
    // The pipeline returns a stamp only when DirectOneXs is selected, its
    // direct type-2 resource is bound and that resource matches the pipeline's
    // complete display transaction. The scene identities below then bind that
    // allocation-free route proof to this exact offered and displayed pair.
    let current = scene.frame_stamp();
    let direct_map = pipeline.diagnostic_one_xs_direct_frame();
    let route = require_measured_route(
        expected,
        timestamp,
        scene.displayed_frame(),
        current.as_ref(),
        direct_map,
    )?;
    let prepare_done = Instant::now();
    gpu.render(pipeline)?;
    let draw_done = Instant::now();
    if scene.displayed_frame() != Some((expected, timestamp)) {
        return Err(format!(
            "measure frame {expected} completed without that exact frame being displayed"
        )
        .into());
    }
    Ok(MeasuredFrame {
        index: expected,
        timestamp,
        route,
        source_ns: duration_ns(source_done.duration_since(transaction_started))?,
        primitive_ns: duration_ns(primitive_done.duration_since(source_done))?,
        prepare_ns: duration_ns(prepare_done.duration_since(primitive_done))?,
        draw_ns: duration_ns(draw_done.duration_since(prepare_done))?,
        transaction_ns: duration_ns(draw_done.duration_since(transaction_started))?,
    })
}

fn require_measured_route<T: Eq>(
    expected: u64,
    timestamp: Duration,
    displayed: Option<(u64, Duration)>,
    current: Option<&T>,
    direct_map: Option<&T>,
) -> Fallible<MeasuredRoute> {
    if displayed != Some((expected, timestamp)) {
        return Err(format!(
            "measure frame {expected} was not the exact selected ONE X2 display transaction"
        )
        .into());
    }
    let current = current
        .ok_or_else(|| format!("measure frame {expected} has no current aligned-pair identity"))?;
    let direct_map = direct_map.ok_or_else(|| {
        format!("measure frame {expected} did not select the ONE X2 direct native type-2 route")
    })?;
    if direct_map != current {
        return Err(format!(
            "measure frame {expected} selected a ONE X2 map for a different delivery"
        )
        .into());
    }
    Ok(MeasuredRoute::OneX2DirectType2)
}

fn duration_ns(duration: Duration) -> Fallible<u64> {
    u64::try_from(duration.as_nanos())
        .map_err(|_| "measured duration exceeds u64 nanoseconds".into())
}

struct MeasureReceipt<'a> {
    run: &'a MeasureRun,
    camera: Camera,
    horizon: Horizon,
    readout: &'a str,
    sampling: Sampling,
    band: bool,
    tone: bool,
    interval: Duration,
    stats: kjerag_media::Stats,
    samples: &'a MeasureSamples,
    sources: Vec<Value>,
    build: Value,
    adapter: &'a AdapterIdentity,
}

fn measure_receipt(run: MeasureReceipt<'_>) -> Fallible<Vec<u8>> {
    let frame_values = run
        .samples
        .frames
        .iter()
        .map(|frame| {
            json!({
                "index": frame.index,
                "timestamp_seconds": frame.timestamp.as_secs(),
                "timestamp_nanoseconds": frame.timestamp.subsec_nanos(),
                "route": frame.route.receipt_name(),
                "source_ns": frame.source_ns,
                "primitive_ns": frame.primitive_ns,
                "prepare_ns": frame.prepare_ns,
                "draw_ns": frame.draw_ns,
                "transaction_ns": frame.transaction_ns
            })
        })
        .collect::<Vec<_>>();
    let elapsed_ns = duration_ns(run.interval)?;
    let receipt = json!({
        "schema": "kjerag.playback-transaction-benchmark.v2",
        "claim": "unpaced waited selected ONE X2 direct native type-2 transactions after a causal frame-zero warm-up",
        "limitations": {
            "studio_parity_claimed": false,
            "realtime_playback_claimed": false,
            "audio_measured": false,
            "capture_or_png_in_interval": false
        },
        "request": {
            "warmup_start": 0,
            "warmup_end_inclusive": run.run.spec.start - 1,
            "start": run.run.spec.start,
            "count": run.run.spec.count,
            "end_inclusive": run.run.spec.end,
            "pace": "off",
            "no_seek": true,
            "every_source_frame_consumed": true
        },
        "view": {
            "yaw_radians": run.camera.yaw,
            "pitch_radians": run.camera.pitch,
            "fov_radians": run.camera.fov,
            "yaw_degrees": run.camera.yaw.to_degrees(),
            "pitch_degrees": run.camera.pitch.to_degrees(),
            "fov_degrees": run.camera.fov.to_degrees(),
            "horizon_locked": run.horizon == Horizon::Locked,
            "readout": run.readout,
            "sampling": format!("{:?}", run.sampling),
            "seam_band": run.band,
            "exposure_tone": run.tone,
            "render_format": "rgba8unorm",
            "render_width": OUTPUT.width,
            "render_height": OUTPUT.height
        },
        "camera_identity": {
            "product": "Insta360 ONE X2",
            "selector": "selected two-lens route with first calibrated lens_type 0x29 (LensTypeOneXS)",
            "lens_type_hex": "0x29",
            "lens_type_decimal": 41
        },
        "route": {
            "name": MeasuredRoute::OneX2DirectType2.receipt_name(),
            "map": "native 200x100 packed type-2 map with copied-pole alpha",
            "draw": "DirectOneXs",
            "authentication_scope": "every warm-up and measured transaction required the exact selected map for its current aligned pair after production preparation; failure refused the run",
            "authentication_cost": "included in prepare_ns and elapsed_ns; allocation-free stamp comparisons only, with no map payload copied"
        },
        "source": run.sources,
        "build": run.build,
        "gpu": {
            "name": run.adapter.name,
            "vendor_id": run.adapter.vendor,
            "device_id": run.adapter.device,
            "backend": run.adapter.backend,
            "device_type": run.adapter.device_type,
            "pci_bus_id": run.adapter.pci_bus_id,
            "driver": run.adapter.driver,
            "driver_info": run.adapter.driver_info,
            "identity_scope": "AdapterInfo read after device creation before warm-up and reverified immediately after the measured interval"
        },
        "run": {
            "elapsed_ns": elapsed_ns,
            "elapsed_scope": "wall time immediately before the first measured transaction through immediately after the last; includes only sample recording and loop bookkeeping between transactions",
            "transaction_scope": "source wait, Scene primitive construction, selected ONE X2 production map preparation plus exact-route authentication, and waited direct native type-2 GPU draw for one exact frame; excludes sample recording",
            "throughput_frames_per_second": run.run.spec.count as f64
                / run.interval.as_secs_f64().max(f64::EPSILON),
            "presented": run.stats.presented,
            "dropped": run.stats.dropped,
            "starved": run.stats.starved,
            "scene_redraws": run.stats.redraws,
            "instrument_redraws": run.samples.len(),
            "transaction_ns": distribution(&run.samples.transaction_ns)?,
            "source_ns": distribution(&run.samples.source_ns)?,
            "primitive_ns": distribution(&run.samples.primitive_ns)?,
            "prepare_ns": distribution(&run.samples.prepare_ns)?,
            "draw_ns": distribution(&run.samples.draw_ns)?
        },
        "frames": frame_values
    });
    let mut encoded = serde_json::to_vec_pretty(&receipt)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn distribution(values: &[u64]) -> Fallible<Value> {
    if values.is_empty() {
        return Err("cannot summarize an empty measurement".into());
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Ok(json!({
        "median": nearest_rank(&sorted, 50),
        "p95": nearest_rank(&sorted, 95),
        "p99": nearest_rank(&sorted, 99),
        "max": sorted[sorted.len() - 1]
    }))
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    let rank = percentile.saturating_mul(sorted.len()).div_ceil(100);
    sorted[rank.max(1) - 1]
}

fn ensure_measure_destination(out: &Path) -> Fallible<()> {
    if out.exists() {
        return Err(format!("receipt file {} already exists", out.display()).into());
    }
    let parent = out.parent().unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(format!("receipt parent {} is not a directory", parent.display()).into());
    }
    let stage = measure_stage(out)?;
    if stage.exists() {
        return Err(format!("measure staging file {} must be new", stage.display()).into());
    }
    Ok(())
}

fn measure_stage(out: &Path) -> Fallible<PathBuf> {
    let parent = out.parent().unwrap_or(Path::new("."));
    let name = out
        .file_name()
        .ok_or("receipt= must name a file")?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.measure-tmp-{}", std::process::id())))
}

fn publish_measure_receipt(out: &Path, encoded: &[u8]) -> Fallible<()> {
    ensure_measure_destination(out)?;
    let stage = measure_stage(out)?;
    let result = (|| -> Fallible<()> {
        write_new(&stage, encoded)?;
        rename_noreplace(&stage, out)?;
        sync_directory(out.parent().unwrap_or(Path::new(".")))?;
        Ok(())
    })();
    if result.is_err() && stage.exists() {
        let _ = fs::remove_file(&stage);
    }
    result
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
/// that (`kjerag::shot`).
fn write_png(shot: &Shot) -> Fallible<String> {
    let began = Instant::now();
    let out = PathBuf::from("scratch").join(format!("playback-frame{}.png", shot.index));
    write_png_to(shot, &out)?;

    Ok(format!(
        "{} at {:.3} s, {}x{}, encoded in {:.0} ms",
        out.display(),
        shot.time.as_secs_f64(),
        shot.width,
        shot.height,
        began.elapsed().as_secs_f64() * 1000.0,
    ))
}

fn write_png_to(shot: &Shot, out: &Path) -> Fallible<()> {
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut encoder = png::Encoder::new(file, shot.width, shot.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}

const RANGE_RECEIPT: &str = "range-receipt.json";

struct RangeOutput {
    out: PathBuf,
    stage: Option<PathBuf>,
    spec: RangeSpec,
    next: u64,
    frames: Vec<Value>,
    capture_height: Option<u32>,
    next_transaction: u64,
    last_transaction: Option<(FrameStamp, PisBackend)>,
    gpu_pis_transactions: u64,
    cpu_pis_transactions: u64,
}

struct RangeReceipt {
    camera: Camera,
    horizon: Horizon,
    output: Size,
    stats: kjerag_render::Stats,
    redraws: u64,
    elapsed: Duration,
    sources: Vec<Value>,
    readout: String,
    sampling: String,
    band: bool,
    tone: bool,
    build: Value,
}

impl RangeOutput {
    fn begin(out: &Path, spec: RangeSpec) -> Fallible<Self> {
        ensure_range_destination(out)?;
        let stage = range_stage(out)?;
        if let Some(parent) = out.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&stage).map_err(|error| {
            format!(
                "range staging directory {} must be new: {error}",
                stage.display()
            )
        })?;
        Ok(Self {
            out: out.to_owned(),
            stage: Some(stage),
            spec,
            next: spec.start,
            frames: Vec::new(),
            capture_height: None,
            next_transaction: 0,
            last_transaction: None,
            gpu_pis_transactions: 0,
            cpu_pis_transactions: 0,
        })
    }

    fn observe_transaction(&mut self, map: &OneXsMapFrame) -> Fallible<()> {
        self.observe_transaction_identity(map.frame(), map.pis_backend())
    }

    fn observe_transaction_identity(
        &mut self,
        frame: &FrameStamp,
        backend: PisBackend,
    ) -> Fallible<()> {
        let index = frame.index();
        if index.checked_add(1) == Some(self.next_transaction) {
            return match self.last_transaction.as_ref() {
                Some((last_frame, last_backend))
                    if last_frame == frame && *last_backend == backend =>
                {
                    Ok(())
                }
                _ => Err(format!(
                    "range backend provenance redraw changed frame {index} identity or PIS backend"
                )
                .into()),
            };
        }
        if index != self.next_transaction {
            return Err(format!(
                "range backend provenance has frame {index}, expected transaction {}",
                self.next_transaction
            )
            .into());
        }
        match backend {
            PisBackend::Gpu => self.gpu_pis_transactions += 1,
            PisBackend::Cpu => self.cpu_pis_transactions += 1,
        }
        self.next_transaction = self
            .next_transaction
            .checked_add(1)
            .ok_or("range backend transaction count overflows")?;
        self.last_transaction = Some((frame.clone(), backend));
        Ok(())
    }

    fn next_index(&self) -> u64 {
        self.next
    }

    fn save(&mut self, shot: &Shot, map: &OneXsMapFrame) -> Fallible<()> {
        if shot.index != self.next {
            return Err(format!(
                "range capture returned frame {} while frame {} was required",
                shot.index, self.next
            )
            .into());
        }
        if shot.width != SHOT_WIDTH {
            return Err(format!(
                "range frame {} has capture width {}, expected {SHOT_WIDTH}",
                shot.index, shot.width
            )
            .into());
        }
        match self.capture_height {
            Some(height) if height != shot.height => {
                return Err(format!(
                    "range frame {} has capture height {}, expected {height}",
                    shot.index, shot.height
                )
                .into());
            }
            None => self.capture_height = Some(shot.height),
            Some(_) => {}
        }
        if map.frame().index() != shot.index || map.frame().timestamp() != shot.time {
            return Err(format!(
                "range frame {} picture and production map identify different deliveries",
                shot.index
            )
            .into());
        }
        let stage = self
            .stage
            .as_deref()
            .ok_or("range output was already published")?;
        let stem = format!("frame-{:010}", shot.index);
        let name = format!("{stem}.png");
        let packed_name = format!("{stem}.packed-f32le.bin");
        let alpha_name = format!("{stem}.alpha-f32le.bin");
        let encoded = encode_png(shot)?;
        let image_sha256 = digest_hex(Sha256::digest(&encoded));
        let packed = map.packed().bytes();
        let alpha = map.alpha().bytes();
        write_new(&stage.join(&name), &encoded)?;
        write_new(&stage.join(&packed_name), packed)?;
        write_new(&stage.join(&alpha_name), alpha)?;
        self.frames.push(json!({
            "index": shot.index,
            "timestamp_seconds": shot.time.as_secs(),
            "timestamp_nanoseconds": shot.time.subsec_nanos(),
            "image": {
                "file": name,
                "width": shot.width,
                "height": shot.height,
                "bytes": encoded.len(),
                "sha256": image_sha256
            },
            "production_map": {
                "pis_backend": map.pis_backend().as_str(),
                "packed": {
                    "file": packed_name,
                    "bytes": packed.len(),
                    "sha256": digest_hex(Sha256::digest(packed))
                },
                "alpha": {
                    "file": alpha_name,
                    "bytes": alpha.len(),
                    "sha256": digest_hex(Sha256::digest(alpha))
                }
            }
        }));
        self.next = self
            .next
            .checked_add(1)
            .ok_or("captured range cannot advance past the frame index limit")?;
        Ok(())
    }

    fn publish(&mut self, run: RangeReceipt) -> Fallible<()> {
        if self.frames.len() as u64 != self.spec.count || self.next != self.spec.end + 1 {
            return Err(format!(
                "range output is incomplete: captured {} of {} requested frames",
                self.frames.len(),
                self.spec.count
            )
            .into());
        }
        let capture_height = self
            .capture_height
            .ok_or("complete range output has no capture height")?;
        if self.next_transaction != run.stats.presented
            || self.gpu_pis_transactions != run.stats.presented
            || self.cpu_pis_transactions != 0
        {
            return Err(format!(
                "range backend provenance records {} GPU and {} CPU transactions through frame {}, expected {} GPU and 0 CPU",
                self.gpu_pis_transactions,
                self.cpu_pis_transactions,
                self.next_transaction.saturating_sub(1),
                run.stats.presented
            )
            .into());
        }
        let receipt = json!({
            "schema": "kjerag.playback-consecutive-range.v2",
            "claim": "exact consecutive displayed production frames from one causal frame-zero run",
            "limitations": {
                "studio_parity_claimed": false,
                "temporal_parity_claimed": false,
                "dense_seam_trace_included": false
            },
            "request": {
                "start": self.spec.start,
                "count": self.spec.count,
                "end_inclusive": self.spec.end,
                "no_seek": true,
                "cold_start_at_range_boundary": false,
                "every_source_frame_consumed": true,
                "captured_map_substitution": false
            },
            "view": {
                "yaw_radians": run.camera.yaw,
                "pitch_radians": run.camera.pitch,
                "fov_radians": run.camera.fov,
                "yaw_degrees": run.camera.yaw.to_degrees(),
                "pitch_degrees": run.camera.pitch.to_degrees(),
                "fov_degrees": run.camera.fov.to_degrees(),
                "horizon_locked": run.horizon == Horizon::Locked,
                "readout": run.readout,
                "sampling": run.sampling,
                "seam_band": run.band,
                "exposure_tone": run.tone,
                "render_format": "rgba8unorm",
                "render_width": run.output.width,
                "render_height": run.output.height,
                "capture_width": SHOT_WIDTH,
                "capture_height": capture_height
            },
            "source": run.sources,
            "build": run.build,
            "frames": &self.frames,
            "run": {
                "presented": run.stats.presented,
                "dropped": run.stats.dropped,
                "starved": run.stats.starved,
                "scene_redraws": run.stats.redraws,
                "instrument_redraws": run.redraws,
                "gpu_pis_transactions": self.gpu_pis_transactions,
                "cpu_pis_transactions": self.cpu_pis_transactions,
                "elapsed_seconds": run.elapsed.as_secs(),
                "elapsed_nanoseconds": run.elapsed.subsec_nanos()
            }
        });
        let mut encoded = serde_json::to_vec_pretty(&receipt)?;
        encoded.push(b'\n');
        let stage = self
            .stage
            .as_deref()
            .ok_or("range output was already published")?;
        // The receipt is deliberately the last leaf. The directory is not
        // visible under its requested name until every PNG and this complete
        // receipt are durable.
        write_new(&stage.join(RANGE_RECEIPT), &encoded)?;
        sync_directory(stage)?;
        rename_noreplace(stage, &self.out)?;
        sync_directory(self.out.parent().unwrap_or(Path::new(".")))?;
        self.stage = None;
        Ok(())
    }
}

impl Drop for RangeOutput {
    fn drop(&mut self) {
        if let Some(stage) = &self.stage {
            let _ = fs::remove_dir_all(stage);
        }
    }
}

fn range_stage(out: &Path) -> Fallible<PathBuf> {
    let parent = out.parent().unwrap_or(Path::new("."));
    let name = out
        .file_name()
        .ok_or("out-dir must name a directory")?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.range-tmp-{}", std::process::id())))
}

fn ensure_range_destination(out: &Path) -> Fallible<()> {
    if out.exists() {
        return Err(format!("out-dir directory {} already exists", out.display()).into());
    }
    let stage = range_stage(out)?;
    if stage.exists() {
        return Err(format!("range staging directory {} must be new", stage.display()).into());
    }
    Ok(())
}

fn encode_png(shot: &Shot) -> Fallible<Vec<u8>> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, shot.width, shot.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&shot.rgba)?;
    }
    Ok(encoded)
}

const PACKED_EVIDENCE: &str = "production-packed-f32le.bin";
const ALPHA_EVIDENCE: &str = "production-alpha-f32le.bin";
const RECEIPT_EVIDENCE: &str = "production-map-receipt.json";
const BUILD_GIT_HEAD: &str = env!("KJERAG_BUILD_GIT_HEAD");
const BUILD_GIT_TREE: &str = env!("KJERAG_BUILD_GIT_TREE");
const BUILD_GIT_DIRTY: &str = env!("KJERAG_BUILD_GIT_DIRTY");

/// Seal the exact selected map after target playback has already proved the
/// source/map/display association in memory. The opaque `FrameStamp` cannot
/// be serialized; the receipt records that its exact equality check passed
/// and then records only readable report fields and content identities.
struct TargetEvidence<'a> {
    out: &'a Path,
    evidence: &'a EvidenceRun,
    png: &'a Path,
    map: &'a OneXsMapFrame,
    expected: (u64, Duration),
    stats: kjerag_render::Stats,
}

fn persist_target_evidence(evidence: TargetEvidence<'_>) -> Fallible<()> {
    let TargetEvidence {
        out,
        evidence,
        png,
        map,
        expected: (expected_index, expected_timestamp),
        stats,
    } = evidence;
    if !cfg!(target_endian = "little") {
        return Err("production map evidence requires a little-endian host".into());
    }
    if map.frame().index() != expected_index || map.frame().timestamp() != expected_timestamp {
        return Err("production map differs from the exact requested target frame".into());
    }
    let packed_bytes = map.packed().bytes();
    let alpha_bytes = map.alpha().bytes();
    if packed_bytes.len() != kjerag_render::studio_type2::PACKED_BYTES {
        return Err("production packed map has the wrong byte size".into());
    }
    if alpha_bytes.len() != kjerag_render::studio_type2::ALPHA_BYTES {
        return Err("production alpha map has the wrong byte size".into());
    }
    ensure_evidence_destination(out)?;

    // Re-read both retained descriptors after playback. No source pathname is
    // reopened for receipt identity: the bytes the decoder saw are the bytes
    // authenticated before playback and verified here through the same fds.
    evidence.sources.verify()?;

    // Compute every identity before creating the staging directory. A source
    // or build failure therefore leaves no partial evidence tree behind.
    let source = evidence.sources.receipt();
    let picture_identity = file_identity(png)?;
    let packed_identity = bytes_identity(PACKED_EVIDENCE, packed_bytes);
    let alpha_identity = bytes_identity(ALPHA_EVIDENCE, alpha_bytes);
    let executable_identity = evidence.executable.identity()?;
    let cargo_lock = evidence.workspace.join("Cargo.lock");
    let cargo_lock_identity = file_identity(&cargo_lock)?;

    let receipt = json!({
        "schema": "kjerag.playback-production-map.v1",
        "claim": "one exact installed resident GPU map bound in-process to the selected current Scene delivery",
        "limitations": {
            "opaque_frame_stamp_serialized": false,
            "video_parity_claimed": false,
            "owner_verdict_claimed": false
        },
        "binding": {
            "exact_map_scene_frame_stamp_equality": true,
            "exact_map_shown_view_frame_stamp_and_capture_equality": true,
            "uninterrupted_target_mode_from_frame_zero": true,
            "index": map.frame().index(),
            "timestamp_seconds": map.frame().timestamp().as_secs(),
            "timestamp_nanoseconds": map.frame().timestamp().subsec_nanos(),
            "presented": stats.presented,
            "dropped": stats.dropped,
            "starved": stats.starved
        },
        "source": source,
        "build": {
            "package_version": env!("CARGO_PKG_VERSION"),
            "embedded_git_commit": BUILD_GIT_HEAD,
            "embedded_git_tree": BUILD_GIT_TREE,
            "embedded_git_dirty": BUILD_GIT_DIRTY,
            "dirty_at_build": false,
            "runtime_git_commit": evidence.runtime_head,
            "runtime_git_tree": evidence.runtime_tree,
            "runtime_tracked_tree_clean": true,
            "cargo_lock": cargo_lock_identity,
            "executable": executable_identity
        },
        "artifacts": {
            "picture": picture_identity,
            "packed": packed_identity,
            "alpha": alpha_identity
        }
    });
    let mut encoded = serde_json::to_vec_pretty(&receipt)?;
    encoded.push(b'\n');
    evidence.verify_checkout()?;
    publish_evidence(out, packed_bytes, alpha_bytes, &encoded)?;
    println!(
        "evidence: exact production map receipt {}",
        out.join(RECEIPT_EVIDENCE).display()
    );
    Ok(())
}

fn publish_evidence(out: &Path, packed: &[u8], alpha: &[u8], receipt: &[u8]) -> Fallible<()> {
    ensure_evidence_destination(out)?;
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let stage = evidence_stage(out)?;
    fs::create_dir(&stage).map_err(|error| {
        format!(
            "evidence staging directory {} must be new: {error}",
            stage.display()
        )
    })?;
    let write_result = (|| -> Fallible<()> {
        write_new(&stage.join(PACKED_EVIDENCE), packed)?;
        write_new(&stage.join(ALPHA_EVIDENCE), alpha)?;
        write_new(&stage.join(RECEIPT_EVIDENCE), receipt)?;
        sync_directory(&stage)?;
        rename_noreplace(&stage, out)?;
        sync_directory(out.parent().unwrap_or(Path::new(".")))?;
        Ok(())
    })();
    if write_result.is_err() && stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    write_result?;
    Ok(())
}

fn sync_directory(path: &Path) -> Fallible<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn rename_noreplace(from: &Path, to: &Path) -> Fallible<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_noreplace(_from: &Path, _to: &Path) -> Fallible<()> {
    Err("production evidence no-replace publication requires Linux renameat2".into())
}

/// Build and executable identity retained before an authenticated run starts.
/// This is separate from `EvidenceRun` because range and measurement modes do
/// not publish target `evidence-out=` artifacts.
struct RangeProvenance {
    workspace: PathBuf,
    runtime_head: String,
    runtime_tree: String,
    executable: RetainedFile,
    executable_identity: Value,
}

impl RangeProvenance {
    fn authenticate() -> Fallible<Self> {
        let workspace = workspace_path()?;
        let (runtime_head, runtime_tree) = checkout_state(&workspace)?;
        validate_build_provenance(
            BUILD_GIT_HEAD,
            BUILD_GIT_TREE,
            BUILD_GIT_DIRTY,
            &runtime_head,
            &runtime_tree,
        )?;
        let executable = RetainedFile::open_running_executable()?;
        let executable_identity = executable.identity()?;
        Ok(Self {
            workspace,
            runtime_head,
            runtime_tree,
            executable,
            executable_identity,
        })
    }

    /// Recheck the exact retained executable and checkout immediately before
    /// publication, then return only the identities already bound before
    /// playback. A changed value refuses rather than silently updating the
    /// receipt to describe a different run environment.
    fn verify(&self) -> Fallible<Value> {
        let current_executable = self.executable.identity()?;
        require_bound_identity(
            &self.executable_identity,
            &current_executable,
            "running executable",
        )?;
        let (head, tree) = checkout_state(&self.workspace)?;
        validate_build_provenance(
            BUILD_GIT_HEAD,
            BUILD_GIT_TREE,
            BUILD_GIT_DIRTY,
            &head,
            &tree,
        )?;
        if head != self.runtime_head || tree != self.runtime_tree {
            return Err("runtime checkout changed during authenticated playback".into());
        }
        Ok(range_build_receipt(
            &self.runtime_head,
            &self.runtime_tree,
            self.executable_identity.clone(),
        ))
    }
}

fn require_bound_identity(bound: &Value, current: &Value, label: &str) -> Fallible<()> {
    if bound != current {
        return Err(format!("{label} identity changed during authenticated playback").into());
    }
    Ok(())
}

fn range_build_receipt(runtime_head: &str, runtime_tree: &str, executable: Value) -> Value {
    json!({
        "package_version": env!("CARGO_PKG_VERSION"),
        "embedded_git_commit": BUILD_GIT_HEAD,
        "embedded_git_tree": BUILD_GIT_TREE,
        "embedded_git_dirty": BUILD_GIT_DIRTY,
        "dirty_at_build": false,
        "runtime_git_commit": runtime_head,
        "runtime_git_tree": runtime_tree,
        "runtime_tracked_tree_clean": true,
        "executable": executable
    })
}

struct EvidenceRun {
    workspace: PathBuf,
    runtime_head: String,
    runtime_tree: String,
    sources: AuthenticatedPair,
    executable: RetainedFile,
}

impl EvidenceRun {
    fn authenticate(input: &Path, out: &Path) -> Fallible<Self> {
        ensure_evidence_destination(out)?;
        let workspace = workspace_path()?;
        let (runtime_head, runtime_tree) = checkout_state(&workspace)?;
        validate_build_provenance(
            BUILD_GIT_HEAD,
            BUILD_GIT_TREE,
            BUILD_GIT_DIRTY,
            &runtime_head,
            &runtime_tree,
        )?;
        let executable = RetainedFile::open_running_executable()?;
        let sources = AuthenticatedPair::open(input)?;
        Ok(Self {
            workspace,
            runtime_head,
            runtime_tree,
            sources,
            executable,
        })
    }

    fn verify_checkout(&self) -> Fallible<()> {
        let (head, tree) = checkout_state(&self.workspace)?;
        validate_build_provenance(
            BUILD_GIT_HEAD,
            BUILD_GIT_TREE,
            BUILD_GIT_DIRTY,
            &head,
            &tree,
        )?;
        if head != self.runtime_head || tree != self.runtime_tree {
            return Err("runtime checkout changed during evidence playback".into());
        }
        Ok(())
    }
}

fn workspace_path() -> Fallible<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("spike manifest is not inside the workspace")?
        .to_owned())
}

fn checkout_state(workspace: &Path) -> Fallible<(String, String)> {
    let head = command_line(workspace, &["rev-parse", "--verify", "HEAD"])?;
    let tree = command_line(workspace, &["rev-parse", "--verify", "HEAD^{tree}"])?;
    let status = command_line(
        workspace,
        &["status", "--porcelain=v1", "--untracked-files=no"],
    )?;
    if !status.is_empty() {
        return Err("evidence-out requires a clean tracked runtime worktree".into());
    }
    Ok((head, tree))
}

fn validate_build_provenance(
    build_head: &str,
    build_tree: &str,
    build_dirty: &str,
    runtime_head: &str,
    runtime_tree: &str,
) -> Fallible<()> {
    if build_dirty != "false" {
        return Err(format!(
            "evidence-out requires a clean build, but build-time tracked-tree state is {build_dirty}"
        )
        .into());
    }
    if build_head != runtime_head || build_tree != runtime_tree {
        return Err(format!(
            "build provenance {build_head}/{build_tree} differs from runtime checkout {runtime_head}/{runtime_tree}"
        )
        .into());
    }
    Ok(())
}

struct AuthenticatedPair {
    sources: [AuthenticatedSource; 2],
}

impl AuthenticatedPair {
    fn open(input: &Path) -> Fallible<Self> {
        require_exact_canonical_path(input, "picked source")?;
        let reader = Reader::open(input)?;
        let admitted = reader.paths();
        drop(reader);
        if admitted.len() != 2 {
            return Err(format!(
                "production ONE X2 evidence requires the admitted two-file source pair, got {} files",
                admitted.len()
            )
            .into());
        }
        let picked = fs::canonicalize(input)?;
        let mut picked_count = 0usize;
        let sources = admitted
            .iter()
            .enumerate()
            .map(|(lane, path)| {
                let is_picked = fs::canonicalize(path)? == picked;
                picked_count += usize::from(is_picked);
                AuthenticatedSource::open(path, lane, is_picked)
            })
            .collect::<Fallible<Vec<_>>>()?;
        if picked_count != 1 {
            return Err(format!(
                "picked source must occur exactly once in admitted decoder order, got {picked_count}"
            )
            .into());
        }
        Ok(Self {
            sources: sources
                .try_into()
                .map_err(|_| "admitted source pair did not retain two lanes")?,
        })
    }

    fn descriptor_paths(&self) -> Fallible<[PathBuf; 2]> {
        Ok([
            self.sources[0].descriptor_path()?,
            self.sources[1].descriptor_path()?,
        ])
    }

    fn require_descriptor_order(&self, actual: &[PathBuf]) -> Fallible<()> {
        let expected = self.descriptor_paths()?;
        if actual != expected {
            return Err(format!(
                "live decoder source order {actual:?} differs from authenticated lens order {expected:?}"
            )
            .into());
        }
        Ok(())
    }

    fn verify(&self) -> Fallible<()> {
        for source in &self.sources {
            source.verify()?;
        }
        Ok(())
    }

    fn receipt(&self) -> Vec<Value> {
        self.sources
            .iter()
            .map(AuthenticatedSource::receipt)
            .collect()
    }
}

struct AuthenticatedSource {
    path: PathBuf,
    decoder_lane: usize,
    picked: bool,
    retained: RetainedFile,
    sha256: String,
}

impl AuthenticatedSource {
    fn open(path: &Path, decoder_lane: usize, picked: bool) -> Fallible<Self> {
        require_exact_canonical_path(path, "admitted source")?;
        let before = fs::symlink_metadata(path)?;
        if !before.is_file() {
            return Err(format!("{} is not a regular source file", path.display()).into());
        }
        let mut file = open_nofollow(path)?;
        let opened = file.metadata()?;
        if stable_identity(&before) != stable_identity(&opened) {
            return Err(format!("{} changed while opening", path.display()).into());
        }
        let sha256 = sha256_file(&mut file)?;
        let after = file.metadata()?;
        let named_after = fs::symlink_metadata(path)?;
        if stable_identity(&before) != stable_identity(&after)
            || stable_identity(&after) != stable_identity(&named_after)
        {
            return Err(format!("{} changed while authenticating", path.display()).into());
        }
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            path: path.to_owned(),
            decoder_lane,
            picked,
            retained: RetainedFile {
                file,
                identity: stable_identity(&after),
                label: path.display().to_string(),
            },
            sha256,
        })
    }

    #[cfg(target_os = "linux")]
    fn descriptor_path(&self) -> Fallible<PathBuf> {
        use std::os::fd::AsRawFd as _;
        self.retained.require_identity()?;
        let path = PathBuf::from(format!("/proc/self/fd/{}", self.retained.file.as_raw_fd()));
        if stable_identity(&fs::metadata(&path)?) != self.retained.identity {
            return Err(format!("{} descriptor alias changed", self.path.display()).into());
        }
        Ok(path)
    }

    #[cfg(not(target_os = "linux"))]
    fn descriptor_path(&self) -> Fallible<PathBuf> {
        Err("descriptor-bound production playback requires Linux /proc/self/fd".into())
    }

    fn verify(&self) -> Fallible<()> {
        let actual = self.retained.sha256()?;
        if actual != self.sha256 {
            return Err(format!("{} retained source bytes changed", self.path.display()).into());
        }
        Ok(())
    }

    fn receipt(&self) -> Value {
        json!({
            "path": self.path,
            "bytes": self.retained.identity.len,
            "sha256": self.sha256,
            "stable_identity": self.retained.identity.receipt(),
            "decoder_lane": self.decoder_lane,
            "picked": self.picked
        })
    }
}

struct RetainedFile {
    file: File,
    identity: StableIdentity,
    label: String,
}

impl RetainedFile {
    #[cfg(target_os = "linux")]
    fn open_running_executable() -> Fallible<Self> {
        let file = File::open("/proc/self/exe")?;
        let metadata = file.metadata()?;
        Ok(Self {
            file,
            identity: stable_identity(&metadata),
            label: "/proc/self/exe".to_owned(),
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn open_running_executable() -> Fallible<Self> {
        Err("production evidence requires Linux /proc/self/exe".into())
    }

    fn require_identity(&self) -> Fallible<()> {
        if stable_identity(&self.file.metadata()?) != self.identity {
            return Err(format!("{} retained descriptor changed", self.label).into());
        }
        Ok(())
    }

    fn sha256(&self) -> Fallible<String> {
        self.require_identity()?;
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let sha256 = sha256_file(&mut file)?;
        if stable_identity(&file.metadata()?) != self.identity {
            return Err(format!("{} changed while hashing", self.label).into());
        }
        Ok(sha256)
    }

    fn identity(&self) -> Fallible<Value> {
        Ok(json!({
            "file": self.label,
            "bytes": self.identity.len,
            "sha256": self.sha256()?,
            "stable_identity": self.identity.receipt()
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StableIdentity {
    len: u64,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    nlink: u64,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    mtime: i64,
    #[cfg(unix)]
    mtime_nsec: i64,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
}

impl StableIdentity {
    fn receipt(&self) -> Value {
        #[cfg(unix)]
        return json!({
            "device": self.dev,
            "inode": self.ino,
            "mode": self.mode,
            "links": self.nlink,
            "uid": self.uid,
            "mtime_seconds": self.mtime,
            "mtime_nanoseconds": self.mtime_nsec,
            "ctime_seconds": self.ctime,
            "ctime_nanoseconds": self.ctime_nsec
        });
        #[cfg(not(unix))]
        json!({})
    }
}

fn stable_identity(metadata: &fs::Metadata) -> StableIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        StableIdentity {
            len: metadata.len(),
            dev: metadata.dev(),
            ino: metadata.ino(),
            mode: metadata.mode(),
            nlink: metadata.nlink(),
            uid: metadata.uid(),
            mtime: metadata.mtime(),
            mtime_nsec: metadata.mtime_nsec(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        }
    }
    #[cfg(not(unix))]
    StableIdentity {
        len: metadata.len(),
    }
}

fn require_exact_canonical_path(path: &Path, label: &str) -> Fallible<()> {
    if !path.is_absolute() || fs::canonicalize(path)?.as_os_str() != path.as_os_str() {
        return Err(format!("{label} must be an exact absolute non-symlink path").into());
    }
    Ok(())
}

fn open_nofollow(path: &Path) -> Fallible<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn sha256_file(file: &mut File) -> Fallible<String> {
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        sha.update(&buffer[..read]);
    }
    Ok(digest_hex(sha.finalize()))
}

fn ensure_evidence_destination(out: &Path) -> Fallible<()> {
    if out.exists() {
        return Err(format!("evidence-out directory {} already exists", out.display()).into());
    }
    let stage = evidence_stage(out)?;
    if stage.exists() {
        return Err(format!("evidence staging directory {} must be new", stage.display()).into());
    }
    Ok(())
}

fn evidence_stage(out: &Path) -> Fallible<PathBuf> {
    let parent = out.parent().unwrap_or(Path::new("."));
    let name = out
        .file_name()
        .ok_or("evidence-out must name a directory")?
        .to_string_lossy();
    Ok(parent.join(format!(".{name}.tmp-{}", std::process::id())))
}

fn bytes_identity(name: &str, bytes: &[u8]) -> Value {
    json!({
        "file": name,
        "bytes": bytes.len(),
        "sha256": digest_hex(Sha256::digest(bytes))
    })
}

fn write_new(path: &Path, bytes: &[u8]) -> Fallible<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn file_identity(path: &Path) -> Fallible<Value> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() {
        return Err(format!("{} is not a regular evidence input", path.display()).into());
    }
    let mut file = open_nofollow(path)?;
    let opened = file.metadata()?;
    if stable_identity(&before) != stable_identity(&opened) {
        return Err(format!("{} changed while opening", path.display()).into());
    }
    let sha256 = sha256_file(&mut file)?;
    let after = file.metadata()?;
    let named_after = fs::symlink_metadata(path)?;
    if stable_identity(&before) != stable_identity(&after)
        || stable_identity(&after) != stable_identity(&named_after)
    {
        return Err(format!("{} changed while hashing", path.display()).into());
    }
    Ok(json!({
        "file": path.file_name().ok_or("evidence file has no basename")?.to_string_lossy(),
        "bytes": after.len(),
        "sha256": sha256,
        "stable_identity": stable_identity(&after).receipt()
    }))
}

fn command_line(workspace: &Path, args: &[&str]) -> Fallible<String> {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a string cannot fail");
            output
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn options(arguments: &[&str]) -> Fallible<Options> {
        let mut words = vec!["playback".to_owned()];
        words.extend(arguments.iter().map(|word| (*word).to_owned()));
        Options::parse(&words)
    }

    #[test]
    fn positional_playback_keeps_its_existing_defaults_and_order() {
        let parsed = options(&[
            "flight.insv",
            "12",
            "50",
            "3",
            "71.5",
            "left",
            "58.0",
            "luma",
            "notone",
        ])
        .unwrap();
        assert_eq!(parsed.input, Path::new("flight.insv"));
        assert_eq!(parsed.seconds, 12);
        assert_eq!(parsed.hz, 50);
        assert_eq!(parsed.shots, 3);
        assert_eq!(parsed.yaw, 71.5);
        assert_eq!(parsed.pitch, 0.0);
        assert_eq!(parsed.readout, "left");
        assert_eq!(parsed.fov, 58.0);
        assert_eq!(parsed.sample, "luma");
        assert_eq!(parsed.band, "notone");
        assert_eq!(parsed.target, None);
        assert!(parsed.bench);
        assert!(parsed.lock);
        assert_eq!(parsed.out, None);
        assert_eq!(parsed.evidence_out, None);
        assert_eq!(parsed.range, None);
        assert_eq!(parsed.out_dir, None);
        assert_eq!(parsed.measure, None);
        assert_eq!(parsed.receipt, None);
    }

    #[test]
    fn consecutive_range_has_an_inclusive_checked_end_and_no_timed_end() {
        let parsed = options(&[
            "flight.insv",
            "range=6367:3",
            "out-dir=scratch/frames6367-6369",
            "bench=0",
            "yaw=71.13",
            "pitch=-13.99",
            "fov=57.95",
            "lock=1",
        ])
        .unwrap();
        assert_eq!(
            parsed.range,
            Some(RangeSpec {
                start: 6367,
                count: 3,
                end: 6369
            })
        );
        assert_eq!(
            parsed.out_dir.as_deref(),
            Some(Path::new("scratch/frames6367-6369"))
        );
        let Run::Range(range) = Run::new(&parsed) else {
            panic!("range mode must not have a wall-clock end");
        };
        assert_eq!(range.spec.end, 6369);
        assert_eq!(range.out_dir, Path::new("scratch/frames6367-6369"));
    }

    #[test]
    fn consecutive_range_rejects_bad_counts_overflow_and_competing_modes() {
        for range in ["0:0", "0", ":1", "1:", "1:2:3", "x:1", "1:x"] {
            assert!(
                options(&[
                    "flight.insv",
                    &format!("range={range}"),
                    "out-dir=scratch/r"
                ])
                .is_err(),
                "accepted {range}"
            );
        }
        assert!(
            options(&[
                "flight.insv",
                "range=18446744073709551615:1",
                "out-dir=scratch/r"
            ])
            .is_err()
        );
        assert!(options(&["flight.insv", "range=1:2"]).is_err());
        assert!(options(&["flight.insv", "out-dir=scratch/r"]).is_err());
        assert!(options(&["flight.insv", "range=1:2", "out-dir=scratch/r", "target=2"]).is_err());
        assert!(
            options(&[
                "flight.insv",
                "1",
                "60",
                "1",
                "range=1:2",
                "out-dir=scratch/r"
            ])
            .is_err()
        );
        assert!(
            options(&[
                "flight.insv",
                "range=1:2",
                "out-dir=scratch/r",
                "evidence-out=scratch/evidence"
            ])
            .is_err()
        );
        assert!(
            options(&[
                "flight.insv",
                "range=1:2",
                "out-dir=scratch/r",
                "out=scratch/frame.png"
            ])
            .is_err()
        );
    }

    #[test]
    fn target_view_is_named_and_has_no_timed_end() {
        let parsed = options(&[
            "flight.insv",
            "target=6369",
            "bench=0",
            "yaw=71.13",
            "pitch=-13.99",
            "fov=57.95",
            "lock=1",
            "out=scratch/exact.png",
            "evidence-out=scratch/frame6369-evidence",
        ])
        .unwrap();
        assert_eq!(parsed.target, Some(6369));
        assert!(!parsed.bench);
        assert_eq!(parsed.yaw, 71.13);
        assert_eq!(parsed.pitch, -13.99);
        assert_eq!(parsed.fov, 57.95);
        assert!(parsed.lock);
        assert_eq!(parsed.out.as_deref(), Some(Path::new("scratch/exact.png")));
        assert_eq!(
            parsed.evidence_out.as_deref(),
            Some(Path::new("scratch/frame6369-evidence"))
        );

        let Run::Target(target) = Run::new(&parsed) else {
            panic!("target mode must not have a wall-clock end");
        };
        assert_eq!(target.index, 6369);
        assert_eq!(target.out, Path::new("scratch/exact.png"));
        assert_eq!(
            target.evidence_out.as_deref(),
            Some(Path::new("scratch/frame6369-evidence"))
        );
    }

    #[test]
    fn target_capture_cannot_compete_with_a_timed_burst() {
        let error = options(&["flight.insv", "1", "60", "1", "target=7"])
            .err()
            .expect("target plus positional shots must be refused")
            .to_string();
        assert!(error.contains("shots must be 0"), "{error}");
    }

    #[test]
    fn named_bits_and_output_are_strict() {
        assert!(options(&["flight.insv", "target=0", "lock=2"]).is_err());
        assert!(options(&["flight.insv", "bench=false"]).is_err());
        assert!(options(&["flight.insv", "out=scratch/lost.png"]).is_err());
        assert!(options(&["flight.insv", "evidence-out=scratch/lost"]).is_err());
        assert!(
            options(&["flight.insv", "target=1", "evidence-out=scratch/lost"])
                .err()
                .expect("evidence must disable the path-based benchmark")
                .to_string()
                .contains("bench=0")
        );
        assert!(options(&["flight.insv", "target=1", "target=2"]).is_err());
        assert!(options(&["flight.insv", "surprise=1"]).is_err());
    }

    #[test]
    fn picture_tokens_are_closed_sets_instead_of_typo_fallbacks() {
        for readout in ["file", "off", "right", "left", "down", "up"] {
            assert!(
                options(&["flight.insv", "60", "60", "0", "0", readout]).is_ok(),
                "refused readout {readout}"
            );
        }
        for sampling in ["bilinear", "luma", "sharp"] {
            assert!(
                options(&["flight.insv", "60", "60", "0", "0", "file", "60", sampling]).is_ok(),
                "refused sampling {sampling}"
            );
        }
        for band in ["band", "notone", "noband"] {
            assert!(
                options(&[
                    "flight.insv",
                    "60",
                    "60",
                    "0",
                    "0",
                    "file",
                    "60",
                    "sharp",
                    band
                ])
                .is_ok(),
                "refused band mode {band}"
            );
        }
        assert!(options(&["flight.insv", "60", "60", "0", "0", "flies"]).is_err());
        assert!(options(&["flight.insv", "60", "60", "0", "0", "file", "60", "shrap"]).is_err());
        assert!(
            options(&[
                "flight.insv",
                "60",
                "60",
                "0",
                "0",
                "file",
                "60",
                "sharp",
                "no-tone"
            ])
            .is_err()
        );
    }

    #[test]
    fn measure_mode_has_exact_warmup_window_and_required_unpaced_receipt() {
        let parsed = options(&[
            "flight.insv",
            "measure=200:300",
            "pace=off",
            "receipt=scratch/gpu-baseline.json",
            "bench=0",
            "yaw=71.13",
            "pitch=-13.99",
            "fov=57.95",
            "lock=1",
        ])
        .unwrap();
        assert_eq!(
            parsed.measure,
            Some(MeasureSpec {
                start: 200,
                count: 300,
                end: 499,
            })
        );
        assert_eq!(
            parsed.receipt.as_deref(),
            Some(Path::new("scratch/gpu-baseline.json"))
        );
        assert!(!parsed.bench);
        let Run::Measure(run) = Run::new(&parsed) else {
            panic!("measure mode must not have a wall-clock end");
        };
        assert_eq!(run.spec.start - 1, 199);
        assert_eq!(run.spec.end, 499);
    }

    #[test]
    fn measure_mode_rejects_ambiguous_or_incomplete_requests() {
        let valid = [
            "flight.insv",
            "measure=200:300",
            "pace=off",
            "receipt=scratch/measure.json",
            "bench=0",
        ];
        for request in ["0:1", "1:0", "1", ":1", "1:", "1:2:3", "x:1", "1:x"] {
            let mut words = valid;
            let argument = format!("measure={request}");
            words[1] = &argument;
            assert!(options(&words).is_err(), "accepted measure={request}");
        }
        assert!(options(&["flight.insv", "measure=200:300", "bench=0"]).is_err());
        assert!(options(&["flight.insv", "measure=200:300", "pace=off", "bench=0"]).is_err());
        assert!(
            options(&[
                "flight.insv",
                "measure=200:300",
                "pace=on",
                "receipt=scratch/m.json",
                "bench=0"
            ])
            .is_err()
        );
        assert!(
            options(&[
                "flight.insv",
                "measure=200:300",
                "pace=off",
                "receipt=scratch/m.json"
            ])
            .is_err()
        );
        for competing in [
            "target=499",
            "out=scratch/frame.png",
            "evidence-out=scratch/e",
        ] {
            let mut words = valid.to_vec();
            words.push(competing);
            assert!(options(&words).is_err(), "accepted {competing}");
        }
    }

    #[test]
    fn nearest_rank_summary_is_exact_at_requested_percentiles() {
        let values = (1..=100).rev().collect::<Vec<_>>();
        let summary = distribution(&values).unwrap();
        assert_eq!(summary["median"], 50);
        assert_eq!(summary["p95"], 95);
        assert_eq!(summary["p99"], 99);
        assert_eq!(summary["max"], 100);
        assert!(distribution(&[]).is_err());
    }

    #[test]
    fn measure_receipt_binds_boundaries_counts_phases_and_identity() {
        let run = MeasureRun {
            spec: MeasureSpec {
                start: 200,
                count: 3,
                end: 202,
            },
            receipt: PathBuf::from("scratch/unused.json"),
        };
        let mut samples = MeasureSamples::with_capacity(3);
        for index in 200..=202 {
            samples.push(MeasuredFrame {
                index,
                timestamp: Duration::from_millis(index),
                route: MeasuredRoute::OneX2DirectType2,
                source_ns: index,
                primitive_ns: index + 1,
                prepare_ns: index + 2,
                draw_ns: index + 3,
                transaction_ns: index + 4,
            });
        }
        assert_eq!(samples.frames.capacity(), 3);
        assert_eq!(samples.transaction_ns.capacity(), 3);
        assert_eq!(samples.source_ns.capacity(), 3);
        let adapter = AdapterIdentity {
            name: "gpu".to_owned(),
            vendor: 0x1002,
            device: 0x164e,
            device_type: "IntegratedGpu".to_owned(),
            pci_bus_id: "0000:01:00.0".to_owned(),
            driver: "driver".to_owned(),
            driver_info: "info".to_owned(),
            backend: "Vulkan".to_owned(),
        };
        let encoded = measure_receipt(MeasureReceipt {
            run: &run,
            camera: Camera {
                yaw: 1.0,
                pitch: -0.25,
                fov: 0.75,
            },
            horizon: Horizon::Locked,
            readout: "file",
            sampling: Sampling::Sharp,
            band: true,
            tone: true,
            interval: Duration::from_secs(1),
            stats: kjerag_media::Stats {
                redraws: 4,
                presented: 3,
                dropped: 0,
                starved: 1,
                worst_late: Duration::ZERO,
                audio: None,
            },
            samples: &samples,
            sources: vec![json!({"lane": 0}), json!({"lane": 1})],
            build: json!({"runtime_git_commit": "commit"}),
            adapter: &adapter,
        })
        .unwrap();
        let receipt: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            receipt["schema"],
            "kjerag.playback-transaction-benchmark.v2"
        );
        assert_eq!(receipt["request"]["warmup_end_inclusive"], 199);
        assert_eq!(receipt["request"]["start"], 200);
        assert_eq!(receipt["request"]["count"], 3);
        assert_eq!(receipt["request"]["end_inclusive"], 202);
        assert_eq!(receipt["run"]["presented"], 3);
        assert_eq!(receipt["run"]["starved"], 1);
        assert_eq!(receipt["run"]["transaction_ns"]["median"], 205);
        assert_eq!(receipt["run"]["transaction_ns"]["p95"], 206);
        assert_eq!(receipt["run"]["elapsed_ns"], 1_000_000_000u64);
        assert!(
            receipt["run"]["elapsed_scope"]
                .as_str()
                .unwrap()
                .contains("immediately before the first")
        );
        assert_eq!(receipt["frames"].as_array().unwrap().len(), 3);
        assert_eq!(receipt["frames"][0]["index"], 200);
        assert_eq!(receipt["frames"][0]["route"], "one-x2-direct-type-2");
        assert_eq!(receipt["frames"][2]["index"], 202);
        assert_eq!(receipt["camera_identity"]["product"], "Insta360 ONE X2");
        assert_eq!(receipt["camera_identity"]["lens_type_decimal"], 41);
        assert_eq!(receipt["route"]["name"], "one-x2-direct-type-2");
        assert_eq!(receipt["route"]["draw"], "DirectOneXs");
        assert!(
            receipt["route"]["authentication_scope"]
                .as_str()
                .unwrap()
                .contains("every warm-up and measured transaction")
        );
        assert_eq!(receipt["source"][1]["lane"], 1);
        assert_eq!(receipt["build"]["runtime_git_commit"], "commit");
        assert_eq!(receipt["gpu"]["vendor_id"], 0x1002);
        assert_eq!(receipt["gpu"]["device_id"], 0x164e);
        assert!(
            receipt["gpu"]["identity_scope"]
                .as_str()
                .unwrap()
                .contains("reverified")
        );
    }

    #[test]
    fn measure_route_requires_the_exact_direct_map_for_each_display() {
        let stamp = 17u64;
        let timestamp = Duration::from_millis(250);
        assert_eq!(
            require_measured_route(
                8,
                timestamp,
                Some((8, timestamp)),
                Some(&stamp),
                Some(&stamp),
            )
            .unwrap(),
            MeasuredRoute::OneX2DirectType2
        );

        let error = require_measured_route(8, timestamp, Some((8, timestamp)), Some(&stamp), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("did not select"), "{error}");

        let other = 18u64;
        let error = require_measured_route(
            8,
            timestamp,
            Some((8, timestamp)),
            Some(&stamp),
            Some(&other),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("different delivery"), "{error}");

        let error = require_measured_route(
            8,
            timestamp,
            Some((7, timestamp)),
            Some(&stamp),
            Some(&stamp),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("exact selected"), "{error}");
    }

    #[test]
    fn adapter_identity_recheck_refuses_any_changed_field() {
        let bound = AdapterIdentity {
            name: "gpu".to_owned(),
            vendor: 0x1002,
            device: 0x164e,
            device_type: "IntegratedGpu".to_owned(),
            pci_bus_id: "0000:01:00.0".to_owned(),
            driver: "driver".to_owned(),
            driver_info: "info".to_owned(),
            backend: "Vulkan".to_owned(),
        };
        assert!(require_adapter_identity(&bound, &bound).is_ok());
        let mut changed = bound.clone();
        changed.device ^= 1;
        assert!(require_adapter_identity(&bound, &changed).is_err());
    }

    #[test]
    fn measure_receipt_publication_is_durable_and_never_replaces() {
        let root = std::env::temp_dir().join(format!(
            "kjerag-playback-measure-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let out = root.join("receipt.json");
        publish_measure_receipt(&out, b"first\n").unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"first\n");
        let error = publish_measure_receipt(&out, b"replacement\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(fs::read(&out).unwrap(), b"first\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evidence_publish_refuses_existing_and_partial_directories() {
        let root = std::env::temp_dir().join(format!(
            "kjerag-playback-evidence-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let out = root.join("evidence");

        publish_evidence(&out, b"packed", b"alpha", b"receipt").unwrap();
        assert_eq!(fs::read(out.join(PACKED_EVIDENCE)).unwrap(), b"packed");
        assert_eq!(fs::read(out.join(ALPHA_EVIDENCE)).unwrap(), b"alpha");
        assert_eq!(fs::read(out.join(RECEIPT_EVIDENCE)).unwrap(), b"receipt");
        fs::remove_dir_all(&out).unwrap();

        fs::create_dir(&out).unwrap();
        let error = publish_evidence(&out, b"replacement", b"replacement", b"replacement")
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
        fs::remove_dir_all(&out).unwrap();

        #[cfg(target_os = "linux")]
        {
            let race_stage = root.join("race-stage");
            fs::create_dir(&race_stage).unwrap();
            fs::write(race_stage.join("payload"), b"must not replace").unwrap();
            fs::create_dir(&out).unwrap();
            assert!(rename_noreplace(&race_stage, &out).is_err());
            assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
            assert_eq!(
                fs::read(race_stage.join("payload")).unwrap(),
                b"must not replace"
            );
            fs::remove_dir_all(&out).unwrap();
            fs::remove_dir_all(&race_stage).unwrap();
        }

        fs::create_dir(&out).unwrap();
        fs::write(out.join("partial"), b"belongs to the refused directory").unwrap();
        let error = publish_evidence(&out, b"packed", b"alpha", b"receipt")
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(
            fs::read(out.join("partial")).unwrap(),
            b"belongs to the refused directory"
        );
        assert!(!out.join(RECEIPT_EVIDENCE).exists());

        fs::remove_dir_all(&out).unwrap();
        let stage = root.join(format!(".evidence.tmp-{}", std::process::id()));
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("partial"), b"unfinished earlier attempt").unwrap();
        let error = publish_evidence(&out, b"packed", b"alpha", b"receipt")
            .unwrap_err()
            .to_string();
        assert!(error.contains("staging directory"), "{error}");
        assert!(!out.exists());
        assert_eq!(
            fs::read(stage.join("partial")).unwrap(),
            b"unfinished earlier attempt"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn range_output_refuses_existing_destinations_and_removes_incomplete_staging() {
        let root = std::env::temp_dir().join(format!(
            "kjerag-playback-range-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let out = root.join("range");
        fs::create_dir(&out).unwrap();
        let spec = RangeSpec::parse("4:2").unwrap();
        let error = RangeOutput::begin(&out, spec).err().unwrap().to_string();
        assert!(error.contains("already exists"), "{error}");
        fs::remove_dir(&out).unwrap();

        let output = RangeOutput::begin(&out, spec).unwrap();
        let stage = output.stage.clone().unwrap();
        assert!(stage.is_dir());
        drop(output);
        assert!(!stage.exists());
        assert!(!out.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn range_backend_provenance_counts_only_exact_causal_transactions() {
        let mut output = RangeOutput {
            out: PathBuf::new(),
            stage: None,
            spec: RangeSpec {
                start: 0,
                count: 1,
                end: 0,
            },
            next: 0,
            frames: Vec::new(),
            capture_height: None,
            next_transaction: 0,
            last_transaction: None,
            gpu_pis_transactions: 0,
            cpu_pis_transactions: 0,
        };

        let frame_zero = FrameStamp::for_test(0, Duration::ZERO, None);
        let frame_zero_decoy = FrameStamp::for_test(0, Duration::ZERO, None);
        let frame_one = FrameStamp::for_test(1, Duration::from_secs(1), Some(&frame_zero));
        let frame_two = FrameStamp::for_test(2, Duration::from_secs(2), Some(&frame_one));
        let frame_three = FrameStamp::for_test(3, Duration::from_secs(3), Some(&frame_two));
        let frame_four = FrameStamp::for_test(4, Duration::from_secs(4), Some(&frame_three));

        output
            .observe_transaction_identity(&frame_zero, PisBackend::Gpu)
            .unwrap();
        output
            .observe_transaction_identity(&frame_zero, PisBackend::Gpu)
            .unwrap();
        assert!(
            output
                .observe_transaction_identity(&frame_zero, PisBackend::Cpu)
                .is_err()
        );
        assert!(
            output
                .observe_transaction_identity(&frame_zero_decoy, PisBackend::Gpu)
                .is_err()
        );
        output
            .observe_transaction_identity(&frame_one, PisBackend::Gpu)
            .unwrap();
        assert!(
            output
                .observe_transaction_identity(&frame_zero, PisBackend::Gpu)
                .is_err()
        );
        assert!(
            output
                .observe_transaction_identity(&frame_three, PisBackend::Gpu)
                .is_err()
        );
        assert_eq!(output.next_transaction, 2);
        assert_eq!(output.gpu_pis_transactions, 2);
        assert_eq!(output.cpu_pis_transactions, 0);

        output
            .observe_transaction_identity(&frame_two, PisBackend::Cpu)
            .unwrap();
        assert!(
            output
                .observe_transaction_identity(&frame_four, PisBackend::Gpu)
                .is_err()
        );
        assert_eq!(output.next_transaction, 3);
        assert_eq!(output.gpu_pis_transactions, 2);
        assert_eq!(output.cpu_pis_transactions, 1);
    }

    #[test]
    fn evidence_build_requires_clean_matching_generated_provenance() {
        validate_build_provenance("commit", "tree", "false", "commit", "tree").unwrap();
        assert!(validate_build_provenance("commit", "tree", "true", "commit", "tree").is_err());
        assert!(validate_build_provenance("commit", "tree", "unknown", "commit", "tree").is_err());
        assert!(validate_build_provenance("stale", "tree", "false", "commit", "tree").is_err());
        assert!(validate_build_provenance("commit", "stale", "false", "commit", "tree").is_err());
    }

    #[test]
    fn range_build_refuses_dirty_stale_or_changed_identity() {
        validate_build_provenance("commit", "tree", "false", "commit", "tree").unwrap();
        assert!(validate_build_provenance("commit", "tree", "true", "commit", "tree").is_err());
        assert!(validate_build_provenance("stale", "tree", "false", "commit", "tree").is_err());
        assert!(validate_build_provenance("commit", "stale", "false", "commit", "tree").is_err());

        let bound = json!({"bytes": 10, "sha256": "bound"});
        require_bound_identity(&bound, &bound, "running executable").unwrap();
        let changed = json!({"bytes": 10, "sha256": "changed"});
        let error = require_bound_identity(&bound, &changed, "running executable")
            .unwrap_err()
            .to_string();
        assert!(error.contains("identity changed"), "{error}");
    }

    #[test]
    fn range_build_receipt_names_embedded_runtime_and_executable_identity() {
        let executable = json!({
            "file": "/proc/self/exe",
            "bytes": 123,
            "sha256": "exact-executable",
            "stable_identity": {"device": 4, "inode": 7}
        });
        let receipt = range_build_receipt("runtime-commit", "runtime-tree", executable.clone());
        assert_eq!(receipt["embedded_git_commit"], BUILD_GIT_HEAD);
        assert_eq!(receipt["embedded_git_tree"], BUILD_GIT_TREE);
        assert_eq!(receipt["embedded_git_dirty"], BUILD_GIT_DIRTY);
        assert_eq!(receipt["runtime_git_commit"], "runtime-commit");
        assert_eq!(receipt["runtime_git_tree"], "runtime-tree");
        assert_eq!(receipt["runtime_tracked_tree_clean"], true);
        assert_eq!(receipt["executable"], executable);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn authenticated_source_decode_alias_keeps_the_exact_opened_bytes() {
        let root = std::env::temp_dir().join(format!(
            "kjerag-playback-source-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("lens.insv");
        let saved = root.join("authenticated.insv");
        fs::write(&path, b"authenticated source bytes").unwrap();
        let source = AuthenticatedSource::open(&path, 0, true).unwrap();
        let alias = source.descriptor_path().unwrap();

        fs::rename(&path, &saved).unwrap();
        fs::write(&path, b"replacement path bytes").unwrap();
        assert_eq!(fs::read(alias).unwrap(), b"authenticated source bytes");
        assert!(source.verify().unwrap_err().to_string().contains("changed"));

        fs::remove_dir_all(&root).unwrap();
    }
}
