//! High-refresh view diagnostic through the player's resident Scene path.
//!
//! Run through scripts/quiet.sh:
//! view-rate <file.insv> [width] [height] [time] [yaw] [pitch] [fov] [unorm|srgb].
//! View angles are degrees; omitted angles retain the ONE X2 review view.
//! An explicit format compares BGRA8 targets with or without sRGB encoding.
//! The default remains the screenshot instrument's RGBA8 unorm. Native iced
//! normally selects an sRGB target, which also enables the view shader's
//! inverse transfer function; unorm timings do not include that work.
//! Both phases measure uncapped changing-view capacity, first paused then
//! with ordinary decoding and stitching running. These are NOT
//! native-window fps: compositor/input/presentation are absent, and each draw
//! waits for GPU completion rather than pipelining several display frames.
//! Completion uses a queue callback and nonblocking polls, with up to 100 us
//! sleeps between polls. Unlike PollType::Wait this leaves submission free for
//! the concurrent stitch worker; polling/wakeup overhead stays in the result.
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
    let input = args.get(1).ok_or(
        "usage: view-rate <file.insv> [width] [height] [time] [yaw] [pitch] [fov] [unorm|srgb]",
    )?;
    let width = args.get(2).map_or(Ok(2560), |s| s.parse::<u32>())?;
    let height = args.get(3).map_or(Ok(1440), |s| s.parse::<u32>())?;
    let time = args.get(4).map_or(Ok(0.0), |s| s.parse::<f64>())?;
    let camera = Camera {
        yaw: args
            .get(5)
            .map_or(Ok(71.13), |s| s.parse::<f32>())?
            .to_radians(),
        pitch: args
            .get(6)
            .map_or(Ok(-13.99), |s| s.parse::<f32>())?
            .to_radians(),
        fov: args
            .get(7)
            .map_or(Ok(57.95), |s| s.parse::<f32>())?
            .to_radians(),
    };
    if !camera.yaw.is_finite()
        || !camera.pitch.is_finite()
        || !camera.fov.is_finite()
        || camera.fov <= 0.0
    {
        return Err("view-rate needs finite view angles and a positive field of view".into());
    }
    if width == 0 || height == 0 || !time.is_finite() || time < 0.0 {
        return Err("view-rate needs nonzero dimensions and a finite nonnegative time".into());
    }
    let format = target_format(args.get(8).map(String::as_str))?;
    let gpu = Gpu::open()?;
    let mut scene = Scene::open(Path::new(input))?;
    scene.pause(Instant::now());
    scene.set_horizon(Horizon::Locked);
    if time + 12.0 >= scene.duration().as_secs_f64() {
        return Err("view-rate needs at least 12 seconds of footage after its start time".into());
    }
    scene.seek(Duration::from_secs_f64(time), Accuracy::Exact);
    let mut pipeline = ScenePipeline::new(&gpu.device, &gpu.queue, format);
    let target = Offscreen::new(&gpu.device, Size::new(width, height), format);
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
        .ok_or("view-rate requires resident stitching")?;
    if Some(map.frame().clone()) != scene.displayed_frame_stamp() {
        return Err("view-rate source and displayed map identities differ".into());
    }
    drop(map);
    println!(
        "{}",
        json!({"gpu": gpu.name, "output": [width, height], "target_hz": HZ,
            "target_format": format!("{format:?}"),
            "view_degrees": [camera.yaw.to_degrees(), camera.pitch.to_degrees(), camera.fov.to_degrees()],
            "measurement": "offscreen CPU+GPU completed redraw, not native presentation",
            "completion": "queue-prefix callback, nonblocking poll, 100 us timeout",
            "start": scene.displayed_frame(), "sampling": "selected type-2 box filter"})
    );
    measure(&scene, &mut pipeline, &gpu, &target, camera, false)?;
    scene.play();
    measure(&scene, &mut pipeline, &gpu, &target, camera, true)
}

fn target_format(argument: Option<&str>) -> Fallible<wgpu::TextureFormat> {
    match argument {
        None => Ok(FORMAT),
        Some("unorm") => Ok(wgpu::TextureFormat::Bgra8Unorm),
        Some("srgb") => Ok(wgpu::TextureFormat::Bgra8UnormSrgb),
        Some(_) => Err("view-rate target format must be unorm or srgb".into()),
    }
}

fn redraw(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    gpu: &Gpu,
    target: &Offscreen,
    camera: Camera,
) -> Fallible<(Duration, Duration, Instant)> {
    let began = Instant::now();
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
    let prepared = Instant::now();
    target.render_callback_completed(&gpu.device, &gpu.queue, pipeline)?;
    let completed = Instant::now();
    Ok((prepared - began, completed - prepared, completed))
}

fn clock_minus_pts_ms(clock: Duration, pts: Duration) -> f64 {
    if clock >= pts {
        (clock - pts).as_secs_f64() * 1000.0
    } else {
        -(pts - clock).as_secs_f64() * 1000.0
    }
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
    let mut prepare_timings = Vec::new();
    let mut completion_timings = Vec::new();
    let mut display_ages = Vec::new();
    let mut arrival_lateness = Vec::new();
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
        let (prepare, completion, completed) =
            redraw(scene, pipeline, gpu, target, Camera { yaw, ..camera })?;
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
        prepare_timings.push(prepare.as_secs_f64() * 1000.0);
        completion_timings.push(completion.as_secs_f64() * 1000.0);
        let displayed = scene
            .displayed_frame()
            .ok_or("view-rate lost its picture")?;
        let display_age = clock_minus_pts_ms(scene.position(completed), displayed.1);
        display_ages.push(display_age);
        if displayed.0 != last.0 {
            if displayed.0 != last.0 + 1 {
                return Err("view-rate skipped a displayed source frame".into());
            }
            // The retained identity changes in prepare, but this is sampled at
            // the first completed redraw that could actually contain it.
            arrival_lateness.push(display_age);
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
    prepare_timings.sort_by(f64::total_cmp);
    completion_timings.sort_by(f64::total_cmp);
    display_ages.sort_by(f64::total_cmp);
    arrival_lateness.sort_by(f64::total_cmp);
    let summary = |values: &[f64]| {
        let q = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
        json!({"median": q(0.5), "p95": q(0.95), "p99": q(0.99), "max": q(1.0)})
    };
    let signed_summary = |values: &[f64]| {
        if values.is_empty() {
            return json!({"samples": 0, "min": null, "median": null, "p95": null, "p99": null, "max": null});
        }
        let q = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
        json!({"samples": values.len(), "min": q(0.0), "median": q(0.5),
            "p95": q(0.95), "p99": q(0.99), "max": q(1.0)})
    };
    println!(
        "{}",
        json!({"phase": if playing { "playing-unpaced" } else { "paused-unpaced" },
            "seconds": elapsed, "redraws": timings.len(),
            "completed_redraws_per_second": timings.len() as f64 / elapsed,
            "redraw_ms": summary(&timings),
            // Host wall time, not GPU timestamps. Completion can include
            // queued stitching as well as the view itself. Do not add these
            // independent percentiles to reconstruct the redraw percentile.
            "pump_prepare_ms": summary(&prepare_timings),
            "draw_and_queue_completion_ms": summary(&completion_timings),
            // Signed presentation-clock position minus displayed frame PTS.
            // Audio follows this clock, but these are not compositor, sound
            // device, or physical output-latency measurements.
            "clock_minus_displayed_pts_ms": signed_summary(&display_ages),
            "first_completed_redraw_arrival_lateness_ms": signed_summary(&arrival_lateness),
            "redraws_over_budget": timings.iter().filter(|ms| **ms > 1000.0 / HZ).count(),
            "source_changes": changes, "source_frames_per_second": changes as f64 / elapsed,
            "source_seconds_advanced": (last.1 - first.1).as_secs_f64(),
            "first_source": first, "last_source": last,
            "player_stats": format!("{:?}", scene.stats().map(|stats| stats.since(initial_stats)))})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_formats_change_only_the_bgra_transfer_encoding() {
        assert_eq!(target_format(None).unwrap(), FORMAT);
        let unorm = target_format(Some("unorm")).unwrap();
        let srgb = target_format(Some("srgb")).unwrap();
        assert!(!unorm.is_srgb());
        assert!(srgb.is_srgb());
        assert_eq!(unorm.add_srgb_suffix(), srgb);
        assert!(target_format(Some("unknown")).is_err());
    }

    #[test]
    fn clock_relative_age_keeps_either_sign() {
        assert_eq!(
            clock_minus_pts_ms(Duration::from_millis(20), Duration::from_millis(30)),
            -10.0
        );
        assert_eq!(
            clock_minus_pts_ms(Duration::from_millis(30), Duration::from_millis(20)),
            10.0
        );
    }
}
