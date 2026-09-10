use super::*;
use crate::sampling::Sampling;
use crate::{Camera, Held, Size};

fn reframe(yaw: f32, pitch: f32, fov: f32, aspect: f32) -> Reframe {
    Reframe::new(
        &[],
        Size::new(3840, 3840),
        Camera { yaw, pitch, fov },
        Held::default(),
        aspect,
        false,
        Sampling::Bilinear,
    )
}

fn contains(rectangles: &[[u32; 4]], x: u32, y: u32) -> bool {
    rectangles
        .iter()
        .any(|r| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
}

fn panorama_uv(reframe: &Reframe, uv: [f32; 2]) -> [f64; 2] {
    let body = unit(reframe.body_ray(reframe.view_ray(uv).unwrap())).unwrap();
    [
        body[0].atan2(-body[2]).rem_euclid(TAU) / TAU,
        body[1].clamp(-1.0, 1.0).acos() / PI,
    ]
}

fn sampled_axis(value: f64, size: u32, repeat: bool) -> [u32; 2] {
    let coordinate = value * f64::from(size) - 0.5;
    let low = coordinate.floor() as i64;
    let map = |index: i64| {
        if repeat {
            index.rem_euclid(i64::from(size)) as u32
        } else {
            index.clamp(0, i64::from(size) - 1) as u32
        }
    };
    [map(low), map(low + 1)]
}

#[test]
fn dense_rectilinear_rays_keep_the_exact_linear_texture_footprint() {
    let full = [7680, 3840];
    for reframe in [
        reframe(-1.4, -0.8, 95.45_f32.to_radians(), 16.0 / 9.0),
        reframe(3.12, 0.1, 70_f32.to_radians(), 16.0 / 9.0),
        reframe(0.4, 0.2, 80_f32.to_radians(), 9.0 / 16.0),
    ] {
        let regions = ViewRegions::for_view(full, &reframe).unwrap();
        validate(full, regions.rgb()).unwrap();
        for row in 0..=72 {
            for column in 0..=128 {
                let uv = [column as f32 / 128.0, row as f32 / 72.0];
                let panorama = panorama_uv(&reframe, uv);
                for y in sampled_axis(panorama[1], full[1], false) {
                    for x in sampled_axis(panorama[0], full[0], true) {
                        assert!(contains(regions.rgb(), x, y), "missing {x},{y} at {uv:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn fusion_expansion_contains_centered_chroma_taps() {
    let full = [7680, 3840];
    let regions =
        ViewRegions::for_view(full, &reframe(3.12, 0.1, 70_f32.to_radians(), 16.0 / 9.0)).unwrap();
    for rgb in regions.rgb() {
        for x in [rgb[0], rgb[0] + rgb[2] - 1] {
            for y in [rgb[1], rgb[1] + rgb[3] - 1] {
                let qx = (f64::from(x) + 0.5) * 0.5 - 0.5;
                let qy = (f64::from(y) + 0.5) * 0.5 - 0.5;
                for uv_x in [qx.floor() as i64, qx.floor() as i64 + 1] {
                    for uv_y in [qy.floor() as i64, qy.floor() as i64 + 1] {
                        let uv_x = uv_x.clamp(0, i64::from(full[0] / 2) - 1) as u32;
                        let uv_y = uv_y.clamp(0, i64::from(full[1] / 2) - 1) as u32;
                        assert!(regions.fusion().iter().any(|r| {
                            uv_x >= r[0] / 2
                                && uv_x < (r[0] + r[2]) / 2
                                && uv_y >= r[1] / 2
                                && uv_y < (r[1] + r[3]) / 2
                        }));
                    }
                }
            }
        }
    }
}

#[test]
fn wrap_pole_portrait_and_wide_fallback_are_bounded() {
    let full = [7680, 3840];
    let wrap = ViewRegions::for_view(
        full,
        &reframe(PI as f32, 0.0, 60_f32.to_radians(), 16.0 / 9.0),
    )
    .unwrap();
    assert_eq!(wrap.rgb().len(), 2);
    assert_eq!(wrap.rgb()[0][0], 0);
    assert_eq!(wrap.rgb()[1][0] + wrap.rgb()[1][2], full[0]);

    let pole = ViewRegions::for_view(
        full,
        &reframe(0.0, FRAC_PI_2 as f32, 60_f32.to_radians(), 16.0 / 9.0),
    )
    .unwrap();
    assert_eq!(pole.rgb()[0][0], 0);
    assert_eq!(pole.rgb()[0][2], full[0]);

    let portrait =
        ViewRegions::for_view(full, &reframe(0.2, 0.3, 80_f32.to_radians(), 9.0 / 16.0)).unwrap();
    validate(full, portrait.rgb()).unwrap();

    let wide =
        ViewRegions::for_view(full, &reframe(0.0, 0.0, 130_f32.to_radians(), 16.0 / 9.0)).unwrap();
    assert_eq!(wide.rgb(), &[[0, 0, 7680, 3840]]);
    assert_eq!(wide.fusion(), wide.rgb());
}

#[test]
fn validation_rejects_bad_geometry_but_allows_touching() {
    assert!(validate([0, 10], &[[0, 0, 2, 2]]).is_err());
    assert!(validate([10, 10], &[]).is_err());
    assert!(validate([10, 10], &[[0, 0, 3, 2]]).is_err());
    assert!(validate([10, 10], &[[0, 0, 2, 0]]).is_err());
    assert!(validate([10, 10], &[[8, 0, 4, 2]]).is_err());
    assert!(validate([10, 10], &[[0, 0, 6, 4], [4, 2, 4, 4]]).is_err());
    validate([10, 10], &[[0, 0, 4, 4], [4, 0, 6, 4]]).unwrap();
}

#[test]
fn one_texel_rgb_halo_is_load_bearing() {
    let full = [100, 50];
    let without = raster_rectangle(full, 0.26, 0.50, 0.26, 0.50, 0);
    let with = raster_rectangle(full, 0.26, 0.50, 0.26, 0.50, RGB_TEXEL_HALO);
    let x = sampled_axis(0.26, full[0], true)[0];
    let y = sampled_axis(0.26, full[1], false)[0];
    assert!(!contains(&[without], x, y));
    assert!(contains(&[with], x, y));
}

#[test]
fn periodic_halo_splits_a_cap_which_stops_just_short_of_the_seam() {
    let full = [100, 50];
    let texel = TAU / f64::from(full[0]);
    let longitude = 2.0 * texel;
    let geometric_half_width = 1.75 * texel;
    assert!(longitude - geometric_half_width > 0.0);
    let edge_u = (longitude - geometric_half_width) / TAU;
    assert_eq!(sampled_axis(edge_u, full[0], true), [full[0] - 1, 0]);

    let spans = padded_longitude_spans(longitude, geometric_half_width, full[0]);
    assert_eq!(spans.len(), 2);
    let rectangles: Vec<_> = spans
        .into_iter()
        .map(|(u0, u1)| raster_rectangle(full, u0, u1, 0.4, 0.6, RGB_TEXEL_HALO))
        .collect();
    assert!(rectangles.iter().any(|rectangle| rectangle[0] == 0));
    assert!(
        rectangles
            .iter()
            .any(|rectangle| rectangle[0] + rectangle[2] == full[0])
    );
    for x in sampled_axis(edge_u, full[0], true) {
        assert!(contains(&rectangles, x, full[1] / 2));
    }
}
