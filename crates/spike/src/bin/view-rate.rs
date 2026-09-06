//! High-refresh view diagnostic through the player's resident Scene path.
//!
//! Run through scripts/quiet.sh: view-rate <file.insv> [width] [height] [time].
//! Both phases measure uncapped changing-view capacity, first paused then
//! with ordinary decoding and stitching running. These are NOT
//! native-window fps: compositor/input/presentation are absent, and each draw
//! waits for GPU completion rather than pipelining several display frames.
//! No pixel/map readback occurs inside either timed phase.

use std::path::Path;
use std::time::{Duration, Instant};

use kjerag_media::Fallible;
use kjerag_render::{Accuracy, Camera, Horizon, Next, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Offscreen, aspect};
use serde_json::json;

const HZ: f64 = 240.0;

fn main() -> Fallible<()> {
    let args: Vec<_> = std::env::args().collect();
    let input = args
        .get(1)
        .ok_or("usage: view-rate <file.insv> [width] [height] [time]")?;
    let width = args.get(2).map_or(Ok(2560), |s| s.parse::<u32>())?;
    let height = args.get(3).map_or(Ok(1440), |s| s.parse::<u32>())?;
    let time = args.get(4).map_or(Ok(0.0), |s| s.parse::<f64>())?;
    if width == 0 || height == 0 || !time.is_finite() || time < 0.0 {
        return Err("view-rate needs nonzero dimensions and a finite nonnegative time".into());
    }
    let gpu = Gpu::open()?;
    let mut scene = Scene::open(Path::new(input))?;
    scene.pause(Instant::now());
    scene.set_horizon(Horizon::Locked);
    if time + 12.0 >= scene.duration().as_secs_f64() {
        return Err("view-rate needs at least 12 seconds of footage after its start time".into());
    }
    scene.seek(Duration::from_secs_f64(time), Accuracy::Exact);
    let mut pipeline = ScenePipeline::new(&gpu.device, &gpu.queue, FORMAT);
    let target = Offscreen::new(&gpu.device, Size::new(width, height), FORMAT);
    let camera = Camera {
        yaw: 71.13f32.to_radians(),
        pitch: -13.99f32.to_radians(),
        fov: 57.95f32.to_radians(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        redraw(&scene, &mut pipeline, &gpu, &target, camera)?;
        if scene.displayed_frame_stamp().is_some() && !scene.is_seeking() {
            break;
        }
        if Instant::now() >= deadline {
            return Err("view-rate initial source frame did not finish within 30 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // Authenticate the selected map before measurement; never time a blank or
    // generic fallback and call it fast selected stitching.
    let map = scene
        .diagnostic_one_xs_displayed_map()?
        .ok_or("view-rate requires the selected resident ONE X2 path")?;
    if Some(map.frame().clone()) != scene.displayed_frame_stamp() {
        return Err("view-rate source and displayed map identities differ".into());
    }
    drop(map);
    println!(
        "{}",
        json!({"gpu": gpu.name, "output": [width, height], "target_hz": HZ,
            "measurement": "offscreen CPU+GPU completed redraw, not native presentation",
            "start": scene.displayed_frame(), "sampling": "selected type-2 box filter"})
    );
    measure(&scene, &mut pipeline, &gpu, &target, camera, false)?;
    scene.play();
    measure(&scene, &mut pipeline, &gpu, &target, camera, true)
}

fn redraw(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    gpu: &Gpu,
    target: &Offscreen,
    camera: Camera,
) -> Fallible<()> {
    if let Next::Stopped(stall) = scene.pump(Instant::now()) {
        return Err(stall.to_string().into());
    }
    // Ignore the video-due wakeup: changing camera input asks for a redraw
    // independently, exactly as the shader widget's mouse handler does.
    pipeline.prepare(
        &scene.primitive(camera),
        &gpu.device,
        &gpu.queue,
        aspect(target.size()),
    );
    target.render(&gpu.device, &gpu.queue, pipeline)
}

fn measure(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    gpu: &Gpu,
    target: &Offscreen,
    camera: Camera,
    playing: bool,
) -> Fallible<()> {
    let duration = Duration::from_secs(if playing { 12 } else { 2 });
    let mut timings = Vec::new();
    let first = scene.displayed_frame().ok_or("view-rate has no picture")?;
    let mut last = first;
    let mut changes = 0;
    let initial_stats = scene.stats().unwrap_or_default();
    let began = Instant::now();
    while began.elapsed() < duration {
        let start = Instant::now();
        // Bounded movement across the reported riser, not repeated identical
        // uniforms or a render aimed forever away from the seam.
        let yaw = camera.yaw + (began.elapsed().as_secs_f32() * 2.0).sin() * 0.2;
        redraw(scene, pipeline, gpu, target, Camera { yaw, ..camera })?;
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
        let displayed = scene
            .displayed_frame()
            .ok_or("view-rate lost its picture")?;
        if displayed.0 != last.0 {
            if displayed.0 != last.0 + 1 {
                return Err("view-rate skipped a displayed source frame".into());
            }
            changes += 1;
            last = displayed;
        }
    }
    let elapsed = began.elapsed().as_secs_f64();
    if playing && changes == 0 {
        return Err("view-rate playing phase never advanced the displayed source".into());
    }
    if !playing && changes != 0 {
        return Err("view-rate paused phase advanced the source".into());
    }
    timings.sort_by(f64::total_cmp);
    let quantile = |p: f64| timings[((timings.len() - 1) as f64 * p).round() as usize];
    println!(
        "{}",
        json!({"phase": if playing { "playing-unpaced" } else { "paused-unpaced" },
            "seconds": elapsed, "redraws": timings.len(),
            "completed_redraws_per_second": timings.len() as f64 / elapsed,
            "redraw_ms": {"median": quantile(0.5), "p95": quantile(0.95),
                "p99": quantile(0.99), "max": quantile(1.0)},
            "redraws_over_budget": timings.iter().filter(|ms| **ms > 1000.0 / HZ).count(),
            "source_changes": changes, "source_frames_per_second": changes as f64 / elapsed,
            "source_seconds_advanced": (last.1 - first.1).as_secs_f64(),
            "first_source": first, "last_source": last,
            "player_stats": format!("{:?}", scene.stats().map(|stats| stats.since(initial_stats)))})
    );
    Ok(())
}
