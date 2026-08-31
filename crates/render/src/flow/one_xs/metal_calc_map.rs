//! Studio's selected `CalcMap` Metal source, transcribed as ordinary Rust.
//!
//! The pinned worker embeds one complete 9,559-byte Metal program.  Its
//! `FlowstateParam`, `CalcMap`, quaternion helpers and selected model-3
//! projection are all source-read.  This module keeps that program available
//! as a small scalar diagnostic while the general per-frame input owner
//! remains open.
//!
//! This is a transcription of the source semantics, not a promise of
//! bit-identical host and Metal arithmetic.  Metal may contract expressions
//! and its transcendental functions are not Rust's host implementations.  The
//! caller must also supply the already-packed binary32 parameters that Studio
//! uploaded.  The selected path is ordinary and finite; Metal leaves
//! conversion of a non-finite or non-`int`-representable pose index
//! target-specific, so this transcription makes no claim for that exceptional
//! domain.

use super::base_map::{FlowstateRoi, SELECTED_FLOWSTATE_COLS, SELECTED_FLOWSTATE_ROWS};

/// The selected Metal output has 100 rows and 200 columns per lens.
const ROWS: usize = SELECTED_FLOWSTATE_ROWS;
const COLS: usize = SELECTED_FLOWSTATE_COLS;

const PI: f32 = std::f32::consts::PI;
const SENTINEL: [f32; 2] = [-1.0, -1.0];

/// The model-3 subset of Studio's uploaded `FlowstateParam`.
///
/// Every floating value is already binary32, exactly as the host wrapper
/// supplied it to Metal.  The boolean specializes the uploaded integer and
/// model 3 is specialized by this type.  Keeping packing out of the arithmetic
/// makes conversions and quaternion roles visible at the caller.
#[derive(Clone, Debug, PartialEq)]
#[doc(hidden)]
pub struct MetalCalcMapParams {
    pub center: [f32; 2],
    pub focal: [f32; 2],
    pub src_size: [f32; 2],
    pub inv_src_size: [f32; 2],
    pub qci: [f32; 4],
    pub qwm: [f32; 4],
    pub q_c0_f0: [f32; 4],
    pub shift: [f32; 2],
    pub xi: f32,
    pub is_horizon_sweep: bool,
    pub flip: f32,
    pub max_fov: f32,
    pub distort_coeffs: [f32; 5],
    pub pos_scale: [f32; 2],
}

/// Evaluate Studio's selected model-3 `CalcMap` source for one lens.
///
/// The returned layout is the same tightly packed row-major `CV_32FC2` half
/// consumed by the selected native `mapMerge` call.
#[doc(hidden)]
pub fn diagnostic_metal_calc_map(
    param: &MetalCalcMapParams,
    quat_buffer: &[[f32; 4]; 51],
) -> FlowstateRoi {
    let mut values = Vec::with_capacity(ROWS * COLS);
    for row in 0..ROWS {
        for column in 0..COLS {
            values.push(calc_cell(column, row, param, quat_buffer));
        }
    }
    FlowstateRoi::from_row_major_values(ROWS, COLS, values)
        .expect("the fixed CalcMap raster has its declared shape")
}

fn calc_cell(
    column: usize,
    row: usize,
    param: &MetalCalcMapParams,
    quat_buffer: &[[f32; 4]; 51],
) -> [f32; 2] {
    // Preserve the source expression grouping: gid.x*2*pi/width and
    // gid.y*pi/(height-1).
    let dst_pos = [
        ((column as f32 * 2.0) * PI) / COLS as f32,
        (row as f32 * PI) / (ROWS - 1) as f32,
    ];
    let (initial_ok, mut src_pos) = remap(dst_pos, param.q_c0_f0, param);
    if !initial_ok {
        return SENTINEL;
    }

    for _ in 0..4 {
        let pre_pos = src_pos;
        let factor = scan_factor(src_pos, param);
        let pose = get_quat(quat_buffer, factor);
        let corrected = quaternion_multiply(param.qci, pose);
        let corrected = quaternion_multiply(corrected, param.qwm);

        // The Metal source deliberately ignores the intermediate status.
        (_, src_pos) = remap(dst_pos, corrected, param);
        let difference = [
            (src_pos[0] - pre_pos[0]) * param.pos_scale[0],
            (src_pos[1] - pre_pos[1]) * param.pos_scale[1],
        ];
        let movement = (difference[0] * difference[0] + difference[1] * difference[1]).sqrt();
        if movement < 4.0 {
            break;
        }
    }

    let factor = scan_factor(src_pos, param);
    let pose = get_quat(quat_buffer, factor);
    let corrected = quaternion_multiply(param.qci, pose);
    let corrected = quaternion_multiply(corrected, param.qwm);
    let (final_ok, final_pos) = remap(dst_pos, corrected, param);
    if final_ok {
        [final_pos[0] + param.shift[0], final_pos[1] + param.shift[1]]
    } else {
        SENTINEL
    }
}

fn scan_factor(position: [f32; 2], param: &MetalCalcMapParams) -> f32 {
    if param.is_horizon_sweep {
        position[0] * param.inv_src_size[0]
    } else {
        position[1] * param.inv_src_size[1]
    }
}

fn get_quat(quat_buffer: &[[f32; 4]; 51], factor: f32) -> [f32; 4] {
    let factor_index = factor * 50.0;
    let floored = factor_index.floor();
    if (0.0..50.0).contains(&floored) {
        let index = floored as usize;
        let scalar = factor_index - floored;
        quaternion_slerp(quat_buffer[index], quat_buffer[index + 1], scalar)
    } else if floored < 0.0 {
        quat_buffer[0]
    } else {
        quat_buffer[50]
    }
}

fn quaternion_multiply(q1: [f32; 4], q2: [f32; 4]) -> [f32; 4] {
    let x = q2[3] * q1[0] + q2[2] * q1[1] - q2[1] * q1[2] + q2[0] * q1[3];
    let y = -q2[2] * q1[0] + q2[3] * q1[1] + q2[0] * q1[2] + q2[1] * q1[3];
    let z = q2[1] * q1[0] - q2[0] * q1[1] + q2[3] * q1[2] + q2[2] * q1[3];
    let w = -q2[0] * q1[0] - q2[1] * q1[1] - q2[2] * q1[2] + q2[3] * q1[3];
    [x, y, z, w]
}

fn quaternion_rotate(q: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let first_cross = cross(u, vector);
    let t = [
        2.0 * first_cross[0],
        2.0 * first_cross[1],
        2.0 * first_cross[2],
    ];
    let second_cross = cross(u, t);
    [
        (vector[0] + q[3] * t[0]) + second_cross[0],
        (vector[1] + q[3] * t[1]) + second_cross[1],
        (vector[2] + q[3] * t[2]) + second_cross[2],
    ]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn quaternion_slerp(q1: [f32; 4], mut q2: [f32; 4], t: f32) -> [f32; 4] {
    let mut cosa = q1[0] * q2[0] + q1[1] * q2[1] + q1[2] * q2[2] + q1[3] * q2[3];
    if cosa < 0.0 {
        q2 = q2.map(|value| -value);
        cosa = -cosa;
    }

    let (k0, k1) = if cosa > 0.9995 {
        (1.0 - t, t)
    } else {
        let sina = (1.0 - cosa * cosa).sqrt();
        let angle = m_atan2(sina, cosa);
        (((1.0 - t) * angle).sin() / sina, (t * angle).sin() / sina)
    };
    [
        k0 * q1[0] + k1 * q2[0],
        k0 * q1[1] + k1 * q2[1],
        k0 * q1[2] + k1 * q2[2],
        k0 * q1[3] + k1 * q2[3],
    ]
}

fn m_atan2(y: f32, x: f32) -> f32 {
    if x > 0.0 {
        (y / x).atan()
    } else if x < 0.0 {
        if y >= 0.0 {
            (y / x).atan() + PI
        } else {
            (y / x).atan() - PI
        }
    } else if y > 0.0 {
        PI / 2.0
    } else if y < 0.0 {
        -(PI / 2.0)
    } else {
        -1.0
    }
}

fn remap(dst_pos: [f32; 2], quaternion: [f32; 4], param: &MetalCalcMapParams) -> (bool, [f32; 2]) {
    let width_scalar = 1.0 / (param.src_size[0] - 1.0);
    let height_scalar = 1.0 / (param.src_size[1] - 1.0);
    let ray = back_project(dst_pos);
    let ray = quaternion_rotate(quaternion, ray);
    let (valid, position) = omni_projection(ray, param);
    (
        valid,
        [position[0] * width_scalar, position[1] * height_scalar],
    )
}

fn back_project(position: [f32; 2]) -> [f32; 3] {
    let theta = 0.5 * PI - position[1];
    let phi = 2.0 * PI - position[0];
    let z = theta.sin();
    let radius = theta.cos();
    [radius * phi.cos(), radius * phi.sin(), z]
}

fn omni_projection(ray: [f32; 3], param: &MetalCalcMapParams) -> (bool, [f32; 2]) {
    let [x, y, z] = ray;
    let rho2 = x * x + y * y;
    if rho2 > 0.0 {
        let radius = (rho2 + z * z).sqrt();
        let minimum_normalized_z = param.max_fov.cos();
        if z / radius < minimum_normalized_z - 0.01 {
            return (false, SENTINEL);
        }
        let reciprocal = 1.0 / (z + param.xi * radius);
        let undistorted = [reciprocal * x, reciprocal * y];
        let distorted = radtan_distort(undistorted, &param.distort_coeffs);
        (
            true,
            [
                (distorted[0] * param.focal[0]) * param.flip + param.center[0],
                distorted[1] * param.focal[1] + param.center[1],
            ],
        )
    } else {
        (true, param.center)
    }
}

fn radtan_distort(position: [f32; 2], coefficients: &[f32; 5]) -> [f32; 2] {
    let mx2 = position[0] * position[0];
    let my2 = position[1] * position[1];
    let mxy = position[0] * position[1];
    let rho2 = mx2 + my2;
    let rho4 = rho2 * rho2;
    let rho6 = rho4 * rho2;
    let radial = coefficients[0] * rho2 + coefficients[1] * rho2 * rho2 + coefficients[2] * rho6;
    [
        position[0]
            + position[0] * radial
            + 2.0 * coefficients[3] * mxy
            + coefficients[4] * (rho2 + 2.0 * mx2),
        position[1]
            + position[1] * radial
            + 2.0 * coefficients[4] * mxy
            + coefficients[3] * (rho2 + 2.0 * my2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_projection_uses_the_metal_raster_orientation() {
        let north = back_project([0.0, 0.0]);
        assert!(north[0].abs() < 1.0e-6);
        assert!(north[1].abs() < 1.0e-6);
        assert_eq!(north[2], 1.0);

        let equator = back_project([0.0, PI / 2.0]);
        assert_eq!(equator[0], 1.0);
        assert!(equator[1].abs() < 1.0e-6);
        assert_eq!(equator[2], 0.0);

        let quarter_turn = back_project([PI / 2.0, PI / 2.0]);
        assert!(quarter_turn[0].abs() < 1.0e-6);
        assert!((quarter_turn[1] + 1.0).abs() < 1.0e-6);
        assert_eq!(quarter_turn[2], 0.0);

        let south = back_project([0.0, PI]);
        assert!(south[0].abs() < 1.0e-6);
        assert!(south[1].abs() < 1.0e-6);
        assert!((south[2] + 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn quaternion_identity_preserves_a_vector() {
        assert_eq!(
            quaternion_rotate([0.0, 0.0, 0.0, 1.0], [0.25, -0.5, 0.75]),
            [0.25, -0.5, 0.75]
        );
    }

    #[test]
    fn quaternion_endpoints_are_selected_without_interpolation() {
        let mut quaternions = [[0.0; 4]; 51];
        quaternions[0] = [1.0, 2.0, 3.0, 4.0];
        quaternions[50] = [5.0, 6.0, 7.0, 8.0];
        assert_eq!(get_quat(&quaternions, -0.1), quaternions[0]);
        assert_eq!(get_quat(&quaternions, 1.1), quaternions[50]);
    }
}
