//! Selected Windows X4 photometric spatial reference.
//!
//! This is an explicit diagnostic, not an automatic playback producer or a
//! Mac 6.0.2 parity claim. The inner solve consumes horizontally extended
//! BGR8 images; the ratio map is made after cropping that extension away.
//! Spatial filtering uses the recovered kernel and boundaries, but readable
//! reductions rather than OpenCV's implementation-specific summation order.

use super::{RatioMap, RatioPair, content, coordinates, solve};
use crate::stitch_camera::StitchCamera;
use crate::studio_type2::{MAP_HEIGHT as HEIGHT, MAP_NODES, MAP_WIDTH as WIDTH};

const EXTENSION: usize = 6;
const WORK_WIDTH: usize = WIDTH + 2 * EXTENSION;
type BgrMap = Vec<[f32; 3]>;

/// A completed reference observation. No frame or camera identity is inferred
/// from the bytes: the diagnostic caller owns their geometric association.
pub struct Output {
    pub ratios: RatioPair,
    pub diagnostics: solve::Diagnostics,
}

/// Outer content admission, inner MGP and selected fixed-size ratio-map stages.
/// Input sampling and production scheduling are deliberately separate.
///
/// The ratio matrices are retained, not fresh per observation. In particular,
/// left row 40 is outside both the current ratio writes and the neutral fill;
/// its previous blurred value participates in the next blur. Init seeds both
/// full matrices to one (`0x183c0317e`, `0x183c03210`).
pub struct Reference {
    output_streams: [usize; 2],
    inner: solve::Reference,
    content: content::Gate,
    ratios: [BgrMap; 2],
    coordinates: Vec<[f32; 2]>,
}

impl Default for Reference {
    fn default() -> Self {
        Self::new()
    }
}

impl Reference {
    pub fn new() -> Self {
        Self::for_camera(StitchCamera::OneX2)
    }

    /// Native observations, with output rebased for the admitted renderer.
    /// The public native replay constructor deliberately keeps its old chart.
    pub(crate) fn for_camera(camera: StitchCamera) -> Self {
        Self {
            output_streams: camera.fusion_streams(),
            inner: solve::Reference::new(),
            content: content::Gate::default(),
            ratios: std::array::from_fn(|_| vec![[1.0; 3]; MAP_NODES]),
            coordinates: coordinates::for_camera_output(camera),
        }
    }

    /// Admit two aligned 800x16 BGR8 source bands, area-reduce into the selected
    /// working rows, and run the spatial reference. `None` means retain the
    /// previous ratio output: neither the solve nor spatial ratios advanced.
    /// Coordinate invalidity is still accumulated before the content decision,
    /// because native map updates mark that mask independently of admission.
    /// Sampling these source bands and constructing validity remain the caller's
    /// responsibility. This method does not choose a playback update cadence.
    pub fn observe_bands(
        &mut self,
        bands: [&[u8]; 2],
        invalid: &[u8],
    ) -> Result<Option<Output>, String> {
        content::validate(bands)?;
        solve::validate_invalid(invalid)?;
        self.inner.accumulate_invalid(invalid);
        if !self.content.admit(bands) {
            return Ok(None);
        }
        let working = bands.map(content::working_chart);
        self.observe([&working[0], &working[1]], invalid).map(Some)
    }

    /// Consume aligned 200x100 BGR8 images in the selected working chart,
    /// before current-image row replication (`0x183c00bb0`). The validity
    /// bytes use the inner helper's 212-column, four-row convention,
    /// including its horizontal extension.
    /// This invokes an observation directly; it does not reproduce the outer
    /// admission gate or claim that arbitrary projected images are aligned.
    pub fn observe(&mut self, lenses: [&[u8]; 2], invalid: &[u8]) -> Result<Output, String> {
        for (name, image) in ["left", "right"].into_iter().zip(lenses) {
            if image.len() != MAP_NODES * 3 {
                return Err(format!(
                    "selected X4 fusion {name} aligned image has {} bytes, expected {}",
                    image.len(),
                    MAP_NODES * 3
                ));
            }
        }
        let current = lenses.map(|image| {
            let mut image = image.to_vec();
            prepare_current(&mut image);
            image
        });
        let working = current.each_ref().map(|image| extend(image));
        let input = solve::Inputs::new(&working[0], &working[1], invalid)?;
        let output = self.inner.observe(input);
        let prepared = output.prepared.each_ref().map(|image| crop(image));
        let ratios = self.finish([&current[0], &current[1]], [&prepared[0], &prepared[1]]);
        Ok(Output {
            ratios,
            diagnostics: output.diagnostics,
        })
    }

    fn finish(&mut self, current: [&[u8]; 2], prepared: [&[u8]; 2]) -> RatioPair {
        build_ratios(&mut self.ratios, current, prepared);
        self.ratios[0][35 * WIDTH..40 * WIDTH].fill([1.0; 3]);
        self.ratios[1][60 * WIDTH..66 * WIDTH].fill([1.0; 3]);
        blur(&mut self.ratios[0], 35);
        blur(&mut self.ratios[1], 45);

        // In this fixed-size selected call, give-back touches left [0,25)
        // and right [75,100). Subsequent endpoint normalization touches the
        // ORIGINAL ratios' rows [0,20), not the remap destinations. Neither
        // can feed these remaps or the retained row 40, on any observation.
        // Do not transplant those operations onto the 20-row blur ROI or
        // final map: that would change the observed output.
        let mut left = remap(&self.ratios[0], &self.coordinates, [35.0, 55.0]);
        let mut right = remap(&self.ratios[1], &self.coordinates, [45.0, 65.0]);
        if self.output_streams == [1, 0] {
            std::mem::swap(&mut left, &mut right);
        }
        RatioPair { left, right }
    }
}

/// `0x183c00bb0` with H=100 and p=0.2 extends the four central input rows,
/// without touching those rows themselves. Literal 2.0 at `0x184792380`
/// gives source rows trunc(49.5-2)+1=48 and trunc(49.5+2)=51. The destination
/// interval starts at round((1-p)*H/2)-1=39 and ends at H-39-1=60.
fn prepare_current(image: &mut [u8]) {
    for row in 39..48 {
        image.copy_within(48 * WIDTH * 3..49 * WIDTH * 3, row * WIDTH * 3);
    }
    for row in 52..61 {
        image.copy_within(51 * WIDTH * 3..52 * WIDTH * 3, row * WIDTH * 3);
    }
}

/// Actual selected `0x183c01947` supplies margin 40, NOT camera model id 23.
/// The builder dispatches d in [0, 50-40). Its second pass overwrites the
/// left center row using row 49 and extends the right center toward row 40.
fn build_ratios(ratios: &mut [BgrMap; 2], current: [&[u8]; 2], prepared: [&[u8]; 2]) {
    for d in 0..10 {
        for (lens, row) in [(0, 50 - d), (1, 50 + d)] {
            for x in 0..WIDTH {
                let index = row * WIDTH + x;
                ratios[lens][index] = std::array::from_fn(|channel| {
                    (f32::from(prepared[lens][index * 3 + channel]) + 255.0)
                        / (f32::from(current[lens][index * 3 + channel]) + 255.0)
                });
            }
        }
    }
    for d in 0..10 {
        for x in 0..WIDTH {
            ratios[0][(50 + d) * WIDTH + x] = ratios[0][49 * WIDTH + x];
            ratios[1][(49 - d) * WIDTH + x] = ratios[1][50 * WIDTH + x];
        }
    }
}

/// The selected adapter wraps six whole BGR pixels at each horizontal edge.
fn extend(image: &[u8]) -> Vec<u8> {
    debug_assert_eq!(image.len(), MAP_NODES * 3);
    let mut output = Vec::with_capacity(WORK_WIDTH * HEIGHT * 3);
    for row in image.chunks_exact(WIDTH * 3) {
        output.extend_from_slice(&row[(WIDTH - EXTENSION) * 3..]);
        output.extend_from_slice(row);
        output.extend_from_slice(&row[..EXTENSION * 3]);
    }
    output
}

/// `0x183c0f510`: center crop, not a resize or a 212-to-200 coordinate scale.
fn crop(image: &[u8]) -> Vec<u8> {
    debug_assert_eq!(image.len(), WORK_WIDTH * HEIGHT * 3);
    image
        .chunks_exact(WORK_WIDTH * 3)
        .flat_map(|row| row[EXTENSION * 3..(EXTENSION + WIDTH) * 3].iter().copied())
        .collect()
}

/// `0x183c11260` copies the ROI into a separate, periodically padded Mat.
/// Its 3x11 box therefore wraps X and reflects Y about the ROI's own bounds
/// with BORDER_REFLECT_101. It cannot sample the surrounding original rows.
fn blur(map: &mut BgrMap, first_row: usize) {
    const ROWS: usize = 20;
    let input = &map[first_row * WIDTH..(first_row + ROWS) * WIDTH];
    let mut output = vec![[0.0; 3]; WIDTH * ROWS];
    for y in 0..ROWS {
        for x in 0..WIDTH {
            let mut sum = [0.0; 3];
            for dy in -5..=5 {
                let raw_y = y as isize + dy;
                let sy = if raw_y < 0 {
                    -raw_y
                } else if raw_y >= ROWS as isize {
                    2 * (ROWS as isize - 1) - raw_y
                } else {
                    raw_y
                } as usize;
                for dx in -1..=1 {
                    let sx = (x as isize + dx).rem_euclid(WIDTH as isize) as usize;
                    for (sum, value) in sum.iter_mut().zip(input[sy * WIDTH + sx]) {
                        *sum += value;
                    }
                }
            }
            output[y * WIDTH + x] = sum.map(|value| value / 33.0);
        }
    }
    map[first_row * WIDTH..(first_row + ROWS) * WIDTH].copy_from_slice(&output);
}

/// `0x183c03470` and float worker `0x183c00040`: the coordinates address a
/// 202x100 temporary with one periodic column on either side. Y is absolute
/// in the full map, not relative to the blur ROI. Both validity ends include
/// equality. BGR is reordered only at the final RGB consumer boundary.
fn remap(map: &BgrMap, coords: &[[f32; 2]], rows: [f32; 2]) -> RatioMap {
    let at =
        |x: usize, y: usize| map[y * WIDTH + (x as isize - 1).rem_euclid(WIDTH as isize) as usize];
    let values = coords
        .iter()
        .map(|&[x, y]| {
            if !(y >= rows[0] && y <= rows[1]) {
                return [1.0, 1.0, 1.0, 0.0];
            }
            let ix = x as usize;
            let iy = y as usize;
            let fx = x - ix as f32;
            let fy = y - iy as f32;
            let a = at(ix, iy);
            let b = at((ix + 1).min(WIDTH + 1), iy);
            let c = at(ix, (iy + 1).min(HEIGHT - 1));
            let d = at((ix + 1).min(WIDTH + 1), (iy + 1).min(HEIGHT - 1));
            let bgr: [f32; 3] = std::array::from_fn(|ch| {
                (a[ch] * (1.0 - fx) + b[ch] * fx) * (1.0 - fy)
                    + (c[ch] * (1.0 - fx) + d[ch] * fx) * fy
            });
            [bgr[2], bgr[1], bgr[0], 0.0]
        })
        .collect();
    RatioMap::new(values).expect("selected fusion coordinates have 200 by 100 nodes")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate a native/reference difference on either side of the inner solve.
    /// The recorder's prepared BGR images are actual native MGP outputs. This
    /// diagnostic bypasses only our inner solve, retaining the normal ratio,
    /// blur and remap history. Current denominator rows are reconstructed from
    /// native bands, so a native area-reduction rounding difference is not
    /// excluded. Processing successfully is not a numeric or visual parity gate.
    #[test]
    #[ignore = "requires the captured native fusion bands and prepared outputs"]
    fn replay_captured_native_prepared_outputs() {
        use std::{fs, io::Write, path::PathBuf};
        let root = PathBuf::from(
            std::env::var_os("KJERAG_FUSION_NATIVE_PREPARED_REPLAY")
                .expect("native prepared replay needs its captured input directory"),
        );
        let output = PathBuf::from(
            std::env::var_os("KJERAG_FUSION_NATIVE_PREPARED_OUTPUT")
                .expect("native prepared replay needs a new output directory"),
        );
        fs::create_dir(&output).expect("replay output must be a new directory");
        let manifest = fs::read_to_string(root.join("manifest.tsv")).unwrap();
        let mut full = Reference::new();
        let mut native_prepared = Reference::new();
        let mut inner = solve::Reference::new();
        let mut report = fs::File::create_new(output.join("report.tsv")).unwrap();
        writeln!(
            report,
            "frame\tlens\tarm\tmax_abs\trow\tcolumn\trgb_channel\tactual\tnative"
        )
        .unwrap();
        let mut admitted = 0;
        for line in manifest.lines().filter(|line| {
            !line.is_empty() && !line.starts_with('#') && !line.starts_with("frame\t")
        }) {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 6);
            let frame: u64 = fields[0].parse().unwrap();
            let bands = [fields[1], fields[2]].map(|name| fs::read(root.join(name)).unwrap());
            let invalid = fs::read(root.join(fields[3])).unwrap();
            solve::validate_invalid(&invalid).unwrap();
            inner.accumulate_invalid(&invalid);
            let folder = root.join(format!("frame-{frame:03}"));
            let value = full
                .observe_bands([&bands[0], &bands[1]], &invalid)
                .unwrap();
            assert_eq!(value.is_some(), folder.join("prepared-0.bgr8").exists());
            let Some(value) = value else { continue };
            admitted += 1;
            let current = bands.each_ref().map(|band| {
                let mut image = content::working_chart(band);
                prepare_current(&mut image);
                image
            });
            let prepared = [0, 1].map(|lens| {
                let image = fs::read(folder.join(format!("prepared-{lens}.bgr8"))).unwrap();
                assert_eq!(image.len(), WORK_WIDTH * HEIGHT * 3);
                crop(&image)
            });
            let working = current.each_ref().map(|image| extend(image));
            let inner_output =
                inner.observe(solve::Inputs::new(&working[0], &working[1], &invalid).unwrap());
            for (lens, image) in inner_output.prepared.iter().enumerate() {
                fs::write(
                    output.join(format!("frame-{frame:03}.reference-prepared-{lens}.bgr8")),
                    image,
                )
                .unwrap();
                fs::write(
                    output.join(format!("frame-{frame:03}.reference-current-{lens}.bgr8")),
                    &working[lens],
                )
                .unwrap();
            }
            let bypass =
                native_prepared.finish([&current[0], &current[1]], [&prepared[0], &prepared[1]]);
            for (arm, pair) in [
                ("full-reference", value.ratios),
                ("native-prepared", bypass),
            ] {
                for (lens, ratio) in [&pair.left, &pair.right].into_iter().enumerate() {
                    fs::write(
                        output.join(format!("frame-{frame:03}.{arm}.{lens}.float4")),
                        ratio.bytes(),
                    )
                    .unwrap();
                    let bytes = fs::read(root.join(fields[4 + lens])).unwrap();
                    assert_eq!(bytes.len(), MAP_NODES * 12);
                    let mut maximum = (0.0_f32, 0, 0, 1.0, 1.0);
                    for (node, pixel) in bytes.chunks_exact(12).enumerate() {
                        for channel in 0..3 {
                            let offset = (2 - channel) * 4;
                            let native =
                                f32::from_le_bytes(pixel[offset..offset + 4].try_into().unwrap());
                            let actual = ratio.values()[node][channel];
                            assert!(native.is_finite() && actual.is_finite());
                            if (actual - native).abs() > maximum.0 {
                                maximum = ((actual - native).abs(), node, channel, actual, native);
                            }
                        }
                    }
                    writeln!(
                        report,
                        "{frame}\t{lens}\t{arm}\t{}\t{}\t{}\t{}\t{}\t{}",
                        maximum.0,
                        maximum.1 / WIDTH,
                        maximum.1 % WIDTH,
                        maximum.2,
                        maximum.3,
                        maximum.4
                    )
                    .unwrap();
                }
            }
        }
        assert!(admitted > 0, "capture contained no prepared output");
    }

    #[test]
    fn current_preparation_replicates_only_the_selected_outer_rows() {
        let original: Vec<u8> = (0..MAP_NODES * 3)
            .map(|i| (i / (WIDTH * 3)) as u8)
            .collect();
        let mut image = original.clone();
        prepare_current(&mut image);
        for row in 0..HEIGHT {
            let source_row = match row {
                39..48 => 48,
                52..61 => 51,
                _ => row,
            };
            assert_eq!(
                &image[row * WIDTH * 3..(row + 1) * WIDTH * 3],
                &original[source_row * WIDTH * 3..(source_row + 1) * WIDTH * 3]
            );
        }
    }

    #[test]
    fn ratio_builder_uses_own_ordinal_and_selected_ten_row_extent() {
        let current = [vec![0; MAP_NODES * 3], vec![255; MAP_NODES * 3]];
        let mut prepared = [vec![255; MAP_NODES * 3], vec![0; MAP_NODES * 3]];
        prepared[0][50 * WIDTH * 3..51 * WIDTH * 3].fill(0);
        let mut ratios = std::array::from_fn(|_| vec![[7.0; 3]; MAP_NODES]);
        build_ratios(
            &mut ratios,
            [&current[0], &current[1]],
            [&prepared[0], &prepared[1]],
        );
        assert_eq!(ratios[0][40 * WIDTH], [7.0; 3]); // retained, not current
        assert_eq!(ratios[0][41 * WIDTH], [2.0; 3]);
        assert_eq!(ratios[0][50 * WIDTH], [2.0; 3]); // overwritten from row49
        assert_eq!(ratios[0][59 * WIDTH], [2.0; 3]);
        assert_eq!(ratios[0][60 * WIDTH], [7.0; 3]);
        assert_eq!(ratios[1][39 * WIDTH], [7.0; 3]);
        assert_eq!(ratios[1][40 * WIDTH], [0.5; 3]);
        assert_eq!(ratios[1][59 * WIDTH], [0.5; 3]);
        assert_eq!(ratios[1][60 * WIDTH], [7.0; 3]);
    }

    #[test]
    fn left_boundary_reuses_previous_blur_instead_of_fresh_neutral() {
        let current = vec![0; MAP_NODES * 3];
        let changed = vec![255; MAP_NODES * 3];
        let mut reference = Reference::new();
        reference.finish([&current, &current], [&changed, &changed]);
        let first = reference.ratios[0][40 * WIDTH][0];
        assert!((first - 16.0 / 11.0).abs() < 1.0e-6);
        let output = reference.finish([&current, &current], [&current, &current]);
        let second = reference.ratios[0][40 * WIDTH][0];
        assert!((second - (1.0 + (first - 1.0) / 11.0)).abs() < 1.0e-6);
        assert!(output.left.values().iter().any(|v| v[0] > 1.0));
        assert!(output.right.values().iter().all(|v| v[..3] == [1.0; 3]));
    }

    #[test]
    fn right_inclusive_last_row_is_neutral_and_not_part_of_the_blur() {
        let current = vec![0; MAP_NODES * 3];
        let prepared = vec![255; MAP_NODES * 3];
        let mut reference = Reference::new();
        reference.coordinates.fill([1.0, 65.0]);
        reference.ratios[1][65 * WIDTH..66 * WIDTH].fill([9.0; 3]);
        reference.ratios[1][66 * WIDTH..67 * WIDTH].fill([7.0; 3]);
        let output = reference.finish([&current, &current], [&prepared, &prepared]);
        assert_eq!(reference.ratios[1][64 * WIDTH], [13.0 / 11.0; 3]);
        assert_eq!(reference.ratios[1][65 * WIDTH], [1.0; 3]);
        assert_eq!(reference.ratios[1][66 * WIDTH], [7.0; 3]);
        assert!(
            output
                .right
                .values()
                .iter()
                .all(|&v| v == [1.0, 1.0, 1.0, 0.0])
        );
    }

    #[test]
    fn omitted_outer_operations_cannot_feed_this_frames_or_next_frames_maps() {
        let current = vec![0; MAP_NODES * 3];
        let prepared = vec![255; MAP_NODES * 3];
        let mut ordinary = Reference::new();
        let mut poisoned = Reference::new();
        for _ in 0..2 {
            poisoned.ratios[0][..25 * WIDTH].fill([100.0; 3]);
            poisoned.ratios[1][..20 * WIDTH].fill([200.0; 3]);
            poisoned.ratios[1][75 * WIDTH..].fill([300.0; 3]);
            let a = ordinary.finish([&current, &current], [&prepared, &prepared]);
            let b = poisoned.finish([&current, &current], [&prepared, &prepared]);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn same_input_without_correction_is_neutral_on_repeated_observations() {
        let image = vec![96; MAP_NODES * 3];
        let mut reference = Reference::new();
        for _ in 0..2 {
            let output = reference
                .observe([&image, &image], &vec![0; WORK_WIDTH * 4])
                .unwrap();
            for map in [output.ratios.left, output.ratios.right] {
                assert!(map.values().iter().all(|&v| v == [1.0, 1.0, 1.0, 0.0]));
            }
        }
    }

    #[test]
    fn full_reference_maps_reduce_an_admitted_color_difference() {
        let left: Vec<u8> = [80, 100, 120].repeat(MAP_NODES);
        let right: Vec<u8> = [110, 100, 90].repeat(MAP_NODES);
        let output = Reference::new()
            .observe([&left, &right], &vec![0; WORK_WIDTH * 4])
            .unwrap();
        assert!(output.diagnostics.admitted > 0);
        let source = [[120.0, 100.0, 80.0], [90.0, 100.0, 110.0]].map(|rgb| rgb.map(|v| v / 255.0));
        // This chart landmark maps to working y=49.5, inside both ratio
        // intervals. Test the actual RGB consumer, not the BGR solver alone.
        let node = 50 * WIDTH + 50;
        let ratios = [output.ratios.left, output.ratios.right];
        let corrected: [[f32; 3]; 2] = std::array::from_fn(|lens| {
            super::super::correct(
                source[lens],
                ratios[lens].values()[node][..3].try_into().unwrap(),
            )
        });
        let difference = |pair: [[f32; 3]; 2]| -> f32 {
            (0..3).map(|c| (pair[0][c] - pair[1][c]).powi(2)).sum()
        };
        assert!(difference(corrected) < difference(source));
        assert!(corrected[0][0] < source[0][0]);
        assert!(corrected[1][0] > source[1][0]);
        assert!(corrected[0][2] > source[0][2]);
        assert!(corrected[1][2] < source[1][2]);
    }

    #[test]
    fn band_gate_holds_spatial_state_and_admitted_observations_match_direct_reference() {
        let pixels = content::BAND_WIDTH * content::BAND_HEIGHT;
        let mut bands = [[80, 100, 120].repeat(pixels), [110, 100, 90].repeat(pixels)];
        let invalid = vec![0; WORK_WIDTH * 4];
        let mut gated = Reference::new();
        let mut direct = Reference::new();
        for _ in 0..2 {
            let actual = gated
                .observe_bands([&bands[0], &bands[1]], &invalid)
                .unwrap()
                .unwrap();
            let working = bands.each_ref().map(|band| content::working_chart(band));
            let expected = direct
                .observe([&working[0], &working[1]], &invalid)
                .unwrap();
            assert_eq!(actual.ratios, expected.ratios);
            assert_eq!(actual.diagnostics, expected.diagnostics);
            let retained = gated.ratios.clone();
            assert!(
                gated
                    .observe_bands([&bands[0], &bands[1]], &invalid)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(gated.ratios, retained);
            bands[0].iter_mut().for_each(|byte| *byte += 4);
        }
    }

    #[test]
    fn malformed_band_inputs_do_not_consume_first_admission() {
        let band = vec![96; content::BAND_WIDTH * content::BAND_HEIGHT * 3];
        let invalid = vec![0; WORK_WIDTH * 4];
        let mut reference = Reference::new();
        assert!(
            reference
                .observe_bands([&band[..3], &band], &invalid)
                .is_err()
        );
        assert!(reference.observe_bands([&band, &band], &[0]).is_err());
        assert!(
            reference
                .observe_bands([&band, &band], &invalid)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn unchanged_content_still_retains_invalid_coordinates_for_the_next_solve() {
        let pixels = content::BAND_WIDTH * content::BAND_HEIGHT;
        let mut bands = [[80, 100, 120].repeat(pixels), [110, 100, 90].repeat(pixels)];
        let valid = vec![0; WORK_WIDTH * 4];
        let mut reference = Reference::new();
        let first = reference
            .observe_bands([&bands[0], &bands[1]], &valid)
            .unwrap()
            .unwrap();
        assert!(first.diagnostics.admitted > 0);
        assert!(
            reference
                .observe_bands([&bands[0], &bands[1]], &vec![255; WORK_WIDTH * 4])
                .unwrap()
                .is_none()
        );
        bands[0].iter_mut().for_each(|byte| *byte += 4);
        let next = reference
            .observe_bands([&bands[0], &bands[1]], &valid)
            .unwrap()
            .unwrap();
        assert_eq!(next.diagnostics.admitted, first.diagnostics.admitted);
        assert_eq!(next.diagnostics.current_metric, None);
        assert!(next.diagnostics.used_stale_control);
    }

    #[test]
    fn only_four_source_rows_can_feed_spatial_outputs_across_observations() {
        let pixels = content::BAND_WIDTH * content::BAND_HEIGHT;
        let bands = [[80, 100, 120].repeat(pixels), [110, 100, 90].repeat(pixels)];
        let working = bands.each_ref().map(|band| content::working_chart(band));
        let mut poisoned = working.clone();
        for (lens, image) in poisoned.iter_mut().enumerate() {
            for (index, byte) in image.iter_mut().enumerate() {
                if !(48..52).contains(&(index / (WIDTH * 3))) {
                    *byte = ((index * 19 + lens * 61) % 256) as u8;
                }
            }
        }
        let mut ordinary = Reference::new();
        let mut altered = Reference::new();
        for _ in 0..2 {
            let a = ordinary
                .observe([&working[0], &working[1]], &vec![0; WORK_WIDTH * 4])
                .unwrap();
            let b = altered
                .observe([&poisoned[0], &poisoned[1]], &vec![0; WORK_WIDTH * 4])
                .unwrap();
            assert_eq!(a.ratios, b.ratios);
            assert_eq!(a.diagnostics, b.diagnostics);
        }
    }

    #[test]
    fn bad_shapes_fail_before_advancing_either_retained_state() {
        let image = vec![96; MAP_NODES * 3];
        let mut reference = Reference::new();
        assert!(
            reference
                .observe([&image[..3], &image], &vec![0; WORK_WIDTH * 4])
                .is_err()
        );
        assert!(reference.observe([&image, &image], &[0]).is_err());
        assert!(reference.ratios.iter().flatten().all(|v| *v == [1.0; 3]));
    }

    #[test]
    fn extension_wraps_whole_pixels_and_crop_restores_every_byte() {
        let image: Vec<u8> = (0..MAP_NODES * 3).map(|i| (i % 251) as u8).collect();
        let working = extend(&image);
        assert_eq!(working.len(), WORK_WIDTH * HEIGHT * 3);
        assert_eq!(crop(&working), image);
        for (source, extended) in image
            .chunks_exact(WIDTH * 3)
            .zip(working.chunks_exact(WORK_WIDTH * 3))
        {
            assert_eq!(
                &extended[..EXTENSION * 3],
                &source[(WIDTH - EXTENSION) * 3..]
            );
            assert_eq!(
                &extended[(WIDTH + EXTENSION) * 3..],
                &source[..EXTENSION * 3]
            );
        }
    }

    #[test]
    fn box_wraps_x_reflects_roi_y_and_does_not_read_outside_it() {
        let mut map = vec![[900.0; 3]; MAP_NODES];
        map[35 * WIDTH..55 * WIDTH].fill([0.0; 3]);
        // Row 36 is mirrored into the top kernel. Column 199 is the left
        // neighbor of column zero; replication would miss this impulse.
        map[36 * WIDTH + WIDTH - 1] = [33.0, 66.0, 99.0];
        blur(&mut map, 35);
        assert_eq!(map[35 * WIDTH], [2.0, 4.0, 6.0]);
        assert_eq!(map[35 * WIDTH + 1], [0.0; 3]);
        assert_eq!(map[34 * WIDTH], [900.0; 3]);
        assert_eq!(map[55 * WIDTH], [900.0; 3]);
    }

    #[test]
    fn remap_uses_absolute_rows_inclusive_gates_and_rgb_order() {
        let map = (0..MAP_NODES)
            .map(|i| [(i / WIDTH) as f32, (i % WIDTH) as f32, 7.0])
            .collect();
        let mut coords = vec![[1.0, 34.99]; MAP_NODES];
        coords[0] = [1.0, 35.0];
        coords[1] = [2.5, 55.0];
        coords[2] = [200.5, 45.5];
        coords[3] = [1.0, 55.01];
        let result = remap(&map, &coords, [35.0, 55.0]);
        assert_eq!(result.values()[0], [7.0, 0.0, 35.0, 0.0]);
        assert_eq!(result.values()[1], [7.0, 1.5, 55.0, 0.0]);
        assert_eq!(result.values()[2], [7.0, 99.5, 45.5, 0.0]);
        assert!(
            result.values()[3..]
                .iter()
                .all(|&v| v == [1.0, 1.0, 1.0, 0.0])
        );
    }

    #[test]
    fn neutral_map_stays_neutral_through_blur_and_selected_chart() {
        let mut map = vec![[1.0; 3]; MAP_NODES];
        for first_row in [35, 45] {
            blur(&mut map, first_row);
            let output = remap(
                &map,
                &coordinates::selected_x4(),
                [first_row as f32, (first_row + 20) as f32],
            );
            assert!(output.values().iter().all(|&v| v == [1.0, 1.0, 1.0, 0.0]));
        }
    }
}
