//! What a hemisphere-aware decode gate would be worth, and what it would
//! cost (issue #10).
//!
//! Four questions, and the first two decide whether the last two are worth
//! asking. None of them needs the app.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin gating -- <file.insv> reach
//! cargo run --release -p kyerag-spike --bin gating -- <file.insv> duty [seconds]
//! cargo run --release -p kyerag-spike --bin gating -- <file.insv> both [seconds]
//! cargo run --release -p kyerag-spike --bin gating -- <file.insv> mapped [seconds]
//! cargo run --release -p kyerag-spike --bin gating -- <file.insv> one  [seconds]
//! cargo run --release -p kyerag-spike --bin gating -- <file.insv> warm [seconds]
//! ```
//!
//! - **reach**: how much of the sphere a gate could be on for at all. A view
//!   is gateable when no ray of it can be in the other lens's picture, which
//!   is the same coverage cap the pass skips projections against
//!   (`Reframe::reaches`). It depends on the field of view, so this sweeps it.
//! - **duty**: how long a gate would actually hold. With the horizon locked,
//!   which is the default, a parked view is not a parked geometry: the body
//!   swings and turns under it and the lens axes sweep past the view with
//!   nobody touching the mouse. This walks the file's own orientation track.
//! - **both** / **mapped** / **one**: what the second stream costs, at the
//!   pace playback runs at. `mapped` decodes both and maps one, which is the
//!   cheapest gate that can be released for nothing; `one` decodes one, which
//!   is the whole prize and the one that goes cold. Run each under a power
//!   sampler.
//! - **warm**: what releasing a gate costs. The far lens's decoder has been
//!   fed nothing, so it has to be walked from a keyframe back up to the
//!   picture, and this measures how long that takes and how many frames of
//!   staleness it is.
//!
//! `both`, `mapped`, `one` and `warm` decode only; there is no GPU work in
//! them, so what they report is the decode side on its own.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ffmpeg_next as ff;
use kyerag_media::{DrmFrame, Fallible, HwDevice, Reader, open_decoder};
use kyerag_meta::{CalibrationSet, Filter, Lens, Quat};
use kyerag_render::{Camera, Held, MAX_LENSES, Reframe, Sampling, Size};

/// The window the app's own numbers are taken at.
const ASPECT: f32 = 2560.0 / 1440.0;

/// Fields of view to sweep, in degrees: `kyerag_render::Camera`'s zoom limits
/// and the default between them.
const FOVS: [f32; 5] = [20.0, 45.0, 90.0, 100.0, 110.0];

/// Parked views the duty cycle is measured over.
const YAWS: [f32; 8] = [0.0, 20.0, 45.0, 90.0, 135.0, 180.0, 225.0, 315.0];
const PITCHES: [f32; 5] = [-60.0, -30.0, 0.0, 30.0, 60.0];

/// How far past the geometric edge a gate would have to be before it engaged,
/// in degrees. The margin is what buys time to warm the far lens back up, so
/// what matters is how much duty cycle is left once it is paid for.
const MARGINS_DEG: [f32; 4] = [0.0, 5.0, 15.0, 30.0];

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(
        args.get(1)
            .ok_or("usage: gating <file.insv> <reach|duty|both|mapped|one|warm> [seconds]")?,
    );
    let what = args.get(2).map_or("reach", String::as_str);
    let seconds: f64 = match args.get(3) {
        Some(raw) => raw.parse()?,
        None => 60.0,
    };

    let calibration = CalibrationSet::from_insv(&input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    println!(
        "lens:   {} {}, {} calibrated, frames {}x{}",
        calibration.camera_model,
        calibration.firmware,
        lenses.len(),
        frame.width,
        frame.height,
    );
    let block = build(&lenses, frame, Camera::default());
    for lens in 0..MAX_LENSES.min(lenses.len()) {
        println!(
            "cap:    lens {lens} sees {:.2} degrees off its own axis",
            block.coverage(lens).unwrap_or_default().to_degrees(),
        );
    }
    println!();

    match what {
        "reach" => {
            reach(&lenses, frame);
            Ok(())
        }
        "duty" => duty(&input, &lenses, frame, seconds),
        "both" => pace(&input, Lanes::Both, seconds),
        "mapped" => pace(&input, Lanes::Mapped, seconds),
        "one" => pace(&input, Lanes::One, seconds),
        "warm" => warm(&input, seconds),
        other => Err(format!("unknown mode {other}").into()),
    }
}

fn build(lenses: &[Lens], frame: Size, camera: Camera) -> Reframe {
    Reframe::new(
        lenses,
        frame,
        camera,
        Held::default(),
        ASPECT,
        false,
        Sampling::default(),
    )
}

fn held(lenses: &[Lens], frame: Size, camera: Camera, body: Quat) -> Reframe {
    let held = Held {
        body_from_world: body.conjugate(),
        ..Held::default()
    };
    Reframe::new(
        lenses,
        frame,
        camera,
        held,
        ASPECT,
        false,
        Sampling::default(),
    )
}

/// Which lens, if any, this view gates off. `None` when both are needed.
fn gated(reframe: &Reframe, margin_deg: f32) -> Option<usize> {
    let margin = margin_deg.to_radians();
    (0..MAX_LENSES).find(|lens| !reframe.reaches(*lens, margin))
}

/// How much of the sphere a gate could be on for at each field of view, with
/// the body held still. The ceiling on the whole idea.
fn reach(lenses: &[Lens], frame: Size) {
    println!("reach:  with the body still, how much of the sphere gates one lens off");
    println!("        fov  cone   view axis within   of the sphere   of yaw/pitch");
    for fov in FOVS {
        let camera = |yaw: f32, pitch: f32| Camera {
            yaw,
            pitch,
            fov: fov.to_radians(),
        };
        let straight = build(lenses, frame, camera(0.0, 0.0));
        let cone = straight.cone();
        let cap = straight.coverage(0).unwrap_or_default();

        // Uniform in solid angle: the polar angle by its cosine, then azimuth.
        let (mut solid, mut solid_gated) = (0u32, 0u32);
        for step in 0..1_000 {
            let pitch = (1.0 - 2.0 * (step as f32 + 0.5) / 1_000.0).asin();
            for turn in 0..72 {
                solid += 1;
                let view = build(
                    lenses,
                    frame,
                    camera((turn as f32 * 5.0).to_radians(), pitch),
                );
                solid_gated += u32::from(gated(&view, 0.0).is_some());
            }
        }
        // And uniform in what the mouse moves, which is a different measure.
        let (mut grid, mut grid_gated) = (0u32, 0u32);
        for up in 0..89 {
            for turn in 0..72 {
                grid += 1;
                let pitch = (up as f32 * 2.0 - 88.0).to_radians();
                let view = build(
                    lenses,
                    frame,
                    camera((turn as f32 * 5.0).to_radians(), pitch),
                );
                grid_gated += u32::from(gated(&view, 0.0).is_some());
            }
        }

        println!(
            "        {fov:3.0} {:5.1}   {:12.1} deg   {:12.1}%   {:11.1}%",
            cone.to_degrees(),
            (std::f32::consts::PI - cap - cone).to_degrees(),
            100.0 * f64::from(solid_gated) / f64::from(solid),
            100.0 * f64::from(grid_gated) / f64::from(grid),
        );
    }
}

/// The number the idea turns on: with the horizon locked, how long does a
/// gate hold on footage of a camera that is flying?
fn duty(input: &Path, lenses: &[Lens], frame: Size, seconds: f64) -> Fallible<()> {
    let calibration = CalibrationSet::from_insv(input)?;
    let orientation = calibration.orientation(Filter::default());
    let exposure = &calibration.exposure[0];
    let timing = Reader::open(input)?.timing();
    let frames = (seconds * timing.fps()) as u64;
    let per_frame = 1.0 / timing.fps();

    println!(
        "duty:   {frames} frames ({seconds:.0} s), over {} parked views",
        YAWS.len() * PITCHES.len()
    );
    // With the lock off the geometry is as still as the view is, so a gate
    // that engages never releases: the duty cycle is just how many of the
    // parked views are gateable. It is the best case the idea has.
    for fov in [45.0f32, 90.0, 110.0] {
        for margin in MARGINS_DEG {
            let gateable = YAWS
                .iter()
                .flat_map(|yaw| PITCHES.iter().map(move |pitch| (*yaw, *pitch)))
                .filter(|(yaw, pitch)| {
                    let camera = Camera {
                        yaw: yaw.to_radians(),
                        pitch: pitch.to_radians(),
                        fov: fov.to_radians(),
                    };
                    gated(&build(lenses, frame, camera), margin).is_some()
                })
                .count();
            println!(
                "        horizon free, fov {fov:3.0}, margin {margin:2.0}: {:4.1}% gated, \
                 and it never releases",
                100.0 * gateable as f64 / (YAWS.len() * PITCHES.len()) as f64,
            );
        }
    }
    println!();
    println!("        horizon locked, which is the default:");
    println!("        fov  margin   gated   releases/min   median run   longest run");

    for fov in [45.0f32, 90.0, 110.0] {
        for margin in MARGINS_DEG {
            let (mut on, mut total, mut releases) = (0u64, 0u64, 0u64);
            let mut runs: Vec<u64> = Vec::new();

            for yaw in YAWS {
                for pitch in PITCHES {
                    let camera = Camera {
                        yaw: yaw.to_radians(),
                        pitch: pitch.to_radians(),
                        fov: fov.to_radians(),
                    };
                    let mut run = 0u64;
                    for index in 0..frames {
                        let at = exposure
                            .frame_time_us(index)
                            .unwrap_or_else(|| timing.time_of(index).as_micros() as i64);
                        let view = held(lenses, frame, camera, orientation.at(at));
                        total += 1;
                        match gated(&view, margin).is_some() {
                            true => {
                                on += 1;
                                run += 1;
                            }
                            false if run > 0 => {
                                releases += 1;
                                runs.push(run);
                                run = 0;
                            }
                            false => {}
                        }
                    }
                    if run > 0 {
                        runs.push(run);
                    }
                }
            }

            runs.sort_unstable();
            println!(
                "        {fov:3.0}  {margin:5.0}   {:4.1}%   {:12.1}   {:8.2} s   {:9.2} s",
                100.0 * on as f64 / total as f64,
                releases as f64 * 60.0 / (total as f64 * per_frame),
                runs.get(runs.len() / 2).copied().unwrap_or(0) as f64 * per_frame,
                runs.last().copied().unwrap_or(0) as f64 * per_frame,
            );
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lanes {
    /// Both decoded and both mapped, which is what the player does today.
    Both,
    /// Both decoded, one mapped. The cheapest gate anyone could build: the
    /// far lane's decoder never goes cold, so letting go costs nothing at
    /// all, and what is saved is the map and the import rather than the
    /// decode.
    Mapped,
    /// One decoded and one mapped. The whole prize.
    One,
}

impl Lanes {
    fn decoded(self, of: usize) -> usize {
        match self {
            Self::One => 1,
            _ => of,
        }
    }

    fn mapped(self, of: usize) -> usize {
        match self {
            Self::Both => of,
            _ => 1,
        }
    }
}

/// One demuxer and one or two decoders, driven at the pace the player runs
/// at. The difference between the two modes is the whole prize a decode gate
/// could win.
fn pace(input: &Path, lanes: Lanes, seconds: f64) -> Fallible<()> {
    let mut rig = Rig::open(input)?;
    let of = rig.streams.len();
    let (decoded, mapped) = (lanes.decoded(of), lanes.mapped(of));
    let interval = Duration::from_secs_f64(1.0 / rig.fps);
    let wanted = (seconds * rig.fps) as u64;

    let start = Instant::now();
    let cpu = Cpu::now();
    let mut shown = 0u64;
    while shown < wanted {
        let due = start + interval * shown as u32;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        if !rig.step(decoded, mapped)? {
            break;
        }
        shown += 1;
    }
    let elapsed = start.elapsed();

    println!(
        "pace:   {decoded} of {of} lane(s) decoded and {mapped} mapped, \
         {shown} frames in {:.1} s ({:.2} fps)",
        elapsed.as_secs_f64(),
        shown as f64 / elapsed.as_secs_f64(),
    );
    println!("cost:   {:.2}% of one core", cpu.percent(elapsed));
    Ok(())
}

/// What releasing a gate costs, measured rather than reasoned about: gate one
/// lane, leave it gated, then let go and time how long until it is showing
/// the same frame as the lane that never stopped.
///
/// Two ways of letting go, because they are the two candidates:
///
/// - **replay**: the packets since the last keyframe were kept while the gate
///   was on, so the release feeds them and decodes forward. Nothing is
///   demuxed twice and the live lane is not disturbed.
/// - **reseek**: nothing was kept, so the release seeks the container back to
///   the keyframe and reads forward, which moves the demuxer the live lane is
///   reading from too.
fn warm(input: &Path, seconds: f64) -> Fallible<()> {
    println!("warm:   one lane gated, then released, at the pace playback runs at");
    println!("        gated for   held packets   catch-up   frames stale");
    for gate_for in [0.5f64, 2.0, 10.0, seconds] {
        let mut rig = Rig::open(input)?;
        let interval = Duration::from_secs_f64(1.0 / rig.fps);
        let start = Instant::now();
        let mut shown = 0u64;
        // Play normally for a beat, then gate lane 1 for `gate_for` seconds.
        let until = (gate_for * rig.fps) as u64 + 30;
        while shown < until {
            let due = start + interval * shown as u32;
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let live = if shown < 30 { rig.streams.len() } else { 1 };
            if !rig.step(live, live)? {
                break;
            }
            shown += 1;
        }

        // Release: feed the far lane everything it missed since its last
        // keyframe, and decode until it catches the lane that never stopped.
        let released = Instant::now();
        let held_packets = rig.held.len();
        let caught = rig.release()?;
        let took = released.elapsed();
        println!(
            "        {gate_for:7.1} s   {held_packets:12}   {:5.0} ms   {:9} frames",
            took.as_secs_f64() * 1000.0,
            (took.as_secs_f64() * rig.fps).ceil() as u64,
        );
        if !caught {
            println!("        (the far lane never caught up)");
        }
    }
    Ok(())
}

/// One container, one decoder per video stream, and the packets a gated lane
/// has been holding on to.
struct Rig {
    input: ff::format::context::Input,
    streams: Vec<usize>,
    decoders: Vec<ff::decoder::Video>,
    /// Lane 1's packets since its last keyframe, kept while it is gated so
    /// that letting go does not need the demuxer moved.
    held: VecDeque<ff::Packet>,
    fps: f64,
    _hw: HwDevice,
}

impl Rig {
    fn open(path: &Path) -> Fallible<Self> {
        ff::init()?;
        let input = ff::format::input(&path)?;
        let hw = HwDevice::vaapi()?;
        let video: Vec<(usize, ff::Rational)> = input
            .streams()
            .filter(|s| s.parameters().medium() == ff::media::Type::Video)
            .map(|s| (s.index(), s.avg_frame_rate()))
            .collect();
        let (_, rate) = *video.first().ok_or("file has no video stream")?;
        let mut decoders = Vec::new();
        for &(stream, _) in &video {
            decoders.push(open_decoder(&input, stream, &hw)?);
        }
        Ok(Self {
            input,
            streams: video.iter().map(|s| s.0).collect(),
            decoders,
            held: VecDeque::new(),
            fps: f64::from(rate.numerator()) / f64::from(rate.denominator()),
            _hw: hw,
        })
    }

    /// Read and decode until every decoded lane has produced a frame. Lanes
    /// past `decoded` are gated: their packets are kept, not decoded. Lanes
    /// past `mapped` are decoded and their frames dropped.
    fn step(&mut self, decoded: usize, mapped: usize) -> Fallible<bool> {
        let mut done = vec![false; decoded];
        while !done.iter().all(|lane| *lane) {
            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ff::Error::Eof) => return Ok(false),
                Err(e) => return Err(e.into()),
            }
            let Some(lane) = self.streams.iter().position(|s| *s == packet.stream()) else {
                continue;
            };
            if lane >= decoded {
                // A keyframe makes everything before it unnecessary, which is
                // what bounds the hold at one GOP.
                if packet.is_key() {
                    self.held.clear();
                }
                self.held.push_back(packet);
                continue;
            }
            self.decoders[lane].send_packet(&packet)?;
            let mut frame = ff::frame::Video::empty();
            while self.decoders[lane].receive_frame(&mut frame).is_ok() {
                // The map is what the renderer would pay, so the paced run
                // pays it too.
                if lane < mapped {
                    let _mapped = DrmFrame::map(&frame)?;
                }
                done[lane] = true;
            }
        }
        Ok(true)
    }

    /// Feed the gated lane everything it kept, and decode until it has a
    /// picture again.
    ///
    /// Only the frame that ends up on screen is mapped. Everything on the way
    /// is decoded and dropped, which is the same trick `Reader::take` uses to
    /// keep a seek from paying the `vaSyncSurface` wait once per frame.
    fn release(&mut self) -> Fallible<bool> {
        let lane = self.decoders.len() - 1;
        let mut caught = false;
        let mut last = None;
        while let Some(packet) = self.held.pop_front() {
            self.decoders[lane].send_packet(&packet)?;
            let mut frame = ff::frame::Video::empty();
            while self.decoders[lane].receive_frame(&mut frame).is_ok() {
                last = Some(frame);
                frame = ff::frame::Video::empty();
                caught = true;
            }
        }
        if let Some(frame) = last {
            let _mapped = DrmFrame::map(&frame)?;
        }
        Ok(caught)
    }
}

/// Process CPU time from `/proc/self/stat`, as `playback` reads it.
struct Cpu(Duration);

impl Cpu {
    fn now() -> Self {
        Self(Self::used())
    }

    fn used() -> Duration {
        let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
            return Duration::ZERO;
        };
        let Some(rest) = stat.rsplit_once(')') else {
            return Duration::ZERO;
        };
        let fields: Vec<&str> = rest.1.split_whitespace().collect();
        let ticks: u64 = [11, 12]
            .iter()
            .filter_map(|i| fields.get(*i)?.parse::<u64>().ok())
            .sum();
        Duration::from_secs_f64(ticks as f64 / 100.0)
    }

    fn percent(&self, over: Duration) -> f64 {
        (Self::used().saturating_sub(self.0)).as_secs_f64() / over.as_secs_f64() * 100.0
    }
}
