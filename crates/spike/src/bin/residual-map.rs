//! Write one offline calibrated residual sidecar for a named frame and view.

use std::fs::File;
use std::path::PathBuf;

use kjerag_media::{Cue, Fallible};
use kjerag_meta::CalibrationSet;
use kjerag_render::{Camera, Scene};
use kjerag_spike::{Walk, residual_sidecar};

/// The global body-sphere lattice must resolve the narrow equatorial seam.
///
/// Lower grids did produce syntactically valid maps, but on the infinity
/// sample their NCC sites all fell away from the horizon and made the A/B an
/// apparent no-op.  Keep that failure explicit rather than letting a caller
/// create another plausible-looking, inert sidecar.
const DEFAULT_GRID: (usize, usize) = (128, 64);
const MIN_GRID: (usize, usize) = (96, 48);

fn main() -> Fallible<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or("usage: residual-map <file.insv> <seconds> <out.kjrmap> [yaw,pitch,fov degrees] [grid=128x64; minimum 96x48]")?);
    let seconds: f64 = args.next().ok_or("missing seconds")?.parse()?;
    let output = PathBuf::from(args.next().ok_or("missing output path")?);
    let mut camera = Camera::default();
    let mut grid = DEFAULT_GRID;
    for value in args {
        if let Some((yaw, pitch, fov)) = value
            .split_once(',')
            .and_then(|(yaw, rest)| rest.split_once(',').map(|(pitch, fov)| (yaw, pitch, fov)))
        {
            camera.yaw = yaw.parse::<f32>()?.to_radians();
            camera.pitch = pitch.parse::<f32>()?.to_radians();
            camera.fov = fov.parse::<f32>()?.to_radians();
        } else if let Some(value) = value.strip_prefix("grid=") {
            grid = parse_grid(value)?;
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

fn parse_grid(value: &str) -> Fallible<(usize, usize)> {
    let (width, height) = value.split_once('x').ok_or("grid is WIDTHxHEIGHT")?;
    let grid = (width.parse()?, height.parse()?);
    if grid.0 < MIN_GRID.0 || grid.1 < MIN_GRID.1 {
        return Err(format!(
            "grid {value} is too sparse for a horizon seam; minimum is {}x{}",
            MIN_GRID.0, MIN_GRID.1
        )
        .into());
    }
    Ok(grid)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_GRID, MIN_GRID, parse_grid};

    #[test]
    fn default_grid_resolves_the_supported_horizon_lattice() {
        assert!(DEFAULT_GRID.0 >= MIN_GRID.0);
        assert!(DEFAULT_GRID.1 >= MIN_GRID.1);
        assert_eq!(parse_grid("128x64").unwrap(), DEFAULT_GRID);
    }

    #[test]
    fn refuses_a_grid_known_to_miss_the_horizon_seam() {
        let error = parse_grid("32x16").unwrap_err().to_string();
        assert!(error.contains("too sparse for a horizon seam"));
        assert!(error.contains("96x48"));
    }

    #[test]
    fn requires_both_lattice_axes_to_be_dense_enough() {
        assert!(parse_grid("96x47").is_err());
        assert!(parse_grid("95x48").is_err());
        assert_eq!(parse_grid("96x48").unwrap(), MIN_GRID);
    }
}
