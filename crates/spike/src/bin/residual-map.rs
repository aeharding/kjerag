//! Write one offline calibrated residual sidecar for a named frame and view.

use std::fs::File;
use std::path::PathBuf;

use kjerag_media::{Cue, Fallible};
use kjerag_meta::CalibrationSet;
use kjerag_render::{Camera, Scene};
use kjerag_spike::{Walk, residual_sidecar};

fn main() -> Fallible<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or("usage: residual-map <file.insv> <seconds> <out.kjrmap> [yaw,pitch,fov degrees] [grid=32x16]")?);
    let seconds: f64 = args.next().ok_or("missing seconds")?.parse()?;
    let output = PathBuf::from(args.next().ok_or("missing output path")?);
    let mut camera = Camera::default();
    let mut grid = (32usize, 16usize);
    for value in args {
        if let Some((yaw, pitch, fov)) = value
            .split_once(',')
            .and_then(|(yaw, rest)| rest.split_once(',').map(|(pitch, fov)| (yaw, pitch, fov)))
        {
            camera.yaw = yaw.parse::<f32>()?.to_radians();
            camera.pitch = pitch.parse::<f32>()?.to_radians();
            camera.fov = fov.parse::<f32>()?.to_radians();
        } else if let Some(value) = value.strip_prefix("grid=") {
            let (width, height) = value.split_once('x').ok_or("grid is WIDTHxHEIGHT")?;
            grid = (width.parse()?, height.parse()?);
        } else {
            return Err(format!("unrecognized argument {value}").into());
        }
    }
    let calibration = CalibrationSet::from_insv(&input)?;
    let frame = kjerag_render::Size::new(calibration.dimension.width, calibration.dimension.height);
    // Match the player/instrument traversal rather than constructing a map
    // at a cold seek: held orientation and any frame-owned state are reached
    // by the same forward walk the picture uses.
    let mut scene = Scene::still(&input, Cue::Time(std::time::Duration::ZERO))?;
    loop {
        let (_, at) = scene.frame().ok_or("no frame during warm traversal")?;
        if at.as_secs_f64() >= seconds || !scene.advance()? {
            break;
        }
    }
    let (_, at) = scene.frame().ok_or("no frame at requested time")?;
    let map = scene
        .mapped(camera, 1.0)
        .ok_or("no calibrated map at requested cue")?;
    let mut walk = Walk::open(&input, at.as_secs_f64(), frame)?;
    let pair = walk
        .next_pair()?
        .ok_or("no synchronized raw pair at requested cue")?;
    if pair.at != at {
        return Err("refused: Scene and raw pair PTS differ".into());
    }
    let residual = residual_sidecar::generate(&map, &pair.lenses, grid.0, grid.1);
    let header = residual_sidecar::Header {
        camera_key: calibration.camera_key(),
        calibration: [0.0; 5],
        pts_ns: at.as_nanos().try_into().unwrap_or(u64::MAX),
        camera: [camera.yaw, camera.pitch, camera.fov],
    };
    residual_sidecar::write(File::create(&output)?, header, &residual)?;
    let accepted = residual
        .texels
        .iter()
        .filter(|texel| texel[2] > 0.0)
        .count();
    println!(
        "residual-map: PTS {:.9} s; grid {}x{}; accepted {accepted}/{}; {}",
        at.as_secs_f64(),
        grid.0,
        grid.1,
        residual.texels.len(),
        output.display()
    );
    Ok(())
}
