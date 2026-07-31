//! Reading the horizon off a rendered frame: where it runs, and which side
//! of it is sky.
//!
//! The instrument issue #8's verification rests on, and therefore the thing
//! that has to be shown to work before anything it says is believed. Its own
//! tests do that both ways: a synthetic picture tilted by a known angle,
//! including angles past vertical, has to read back that angle, and a picture
//! with no horizon in it has to read back nothing rather than a number.
//!
//! Three things make it survive real footage.
//!
//! - **The topmost strong edge, not the sharpest.** Looking down from a wing,
//!   the ground is full of harder edges than the horizon: field boundaries,
//!   roads, a river. The sky above it has none.
//! - **Both scan directions.** A camera clamped to a paramotor is rolled most
//!   of a quarter turn, so an unstabilized view has its horizon running down
//!   the picture rather than across it, and a line fitted as `y` against `x`
//!   cannot represent that at all. The picture is scanned both ways and the
//!   better fit wins.
//! - **Two rounds of dropping what does not fit.** A wing across the frame is
//!   a second long bright edge with an angle of its own.

use kyerag_render::Size;

/// How many rows either side of one are averaged into it before the scan line
/// is differenced. Enough to put sensor noise and one wing line under the
/// real edge, and far less than the width of the sky.
const SMOOTH: usize = 3;
/// How far either side of the edge its own position is averaged over. The
/// peak row alone is an integer, and an integer cannot resolve the tenth of a
/// degree this instrument exists to argue about: 0.1 degrees across a 960 px
/// picture is 1.7 px end to end.
const REACH: usize = 8;
/// The share of each scan line, at both ends, that is not searched. The
/// horizon has to be in the picture to be measured, and an edge found on the
/// last row is as likely to be the frame's own boundary.
const MARGIN: f64 = 0.12;
/// Scan lines are taken every this many pixels. At 960 wide that is 240
/// points on a line, which is far more than a line needs.
const STEP: usize = 4;
/// How many rounds of dropping the points that do not fit.
const REFITS: usize = 2;
/// A point further than this many times the fit's own spread is not on the
/// horizon.
const OUTLIER: f64 = 2.0;
/// How many of the scan lines have to survive all of that for the answer to
/// mean anything.
const KEEP: f64 = 0.5;
/// How fast the brightness has to change, in 8-bit codes across two rows of
/// the smoothed scan line, before it is an edge rather than noise.
const CONTRAST: f64 = 6.0;
/// How much of a scan line's own sharpest change an edge has to be worth
/// before the search stops at it.
const SHARE: f64 = 0.5;
/// How much brighter one side of an edge has to be than the other, in 8-bit
/// codes, averaged over a band either side of it.
///
/// A horizon is a **step between two regions**, and this is what says so. The
/// local gradient alone is not enough to identify one: noise clears any
/// gradient threshold, and taking the topmost place it does that in each scan
/// line draws a straight line along the top of the search band, which is a
/// horizon by every other test in this file. The band is wide because sky and
/// ground are wide.
const ACROSS: f64 = 12.0;
/// How far the kept points may sit from their own line, in pixels, before the
/// line is not a horizon. A horizon really is straight in a rectilinear view,
/// because it is a great circle and a great circle projects to one, so this
/// is tight.
const STRAIGHT: f64 = 12.0;
/// How far off the line the two sides are sampled to decide which is sky, as
/// a share of the picture's shorter edge.
const ASIDE: f64 = 0.1;

/// The horizon in one rendered frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skyline {
    /// How far off level, in degrees, in (-90, 90]. Positive is the
    /// right-hand side of the picture lower than the left.
    pub degrees: f64,
    /// Two points it passes through, in the same 0 to 1 coordinates the
    /// camera reads a view ray from. Handed out rather than reconstructed
    /// from the angle, because a horizon running down the picture has an
    /// infinite slope and no reconstruction.
    pub through: [[f64; 2]; 2],
    /// A unit vector in those coordinates, across the line, pointing at
    /// whichever side is brighter. On anything but a night flight that is the
    /// sky, and it is what says which way up an answer read off this line is.
    pub sky: [f64; 2],
    /// The share of searched scan lines that ended up on the line.
    pub agreement: f64,
}

/// Find the horizon in an RGBA picture, or `None` where there is not one to
/// find.
pub fn skyline(pixels: &[u8], size: Size) -> Option<Skyline> {
    let (width, height) = (size.width as usize, size.height as usize);
    if pixels.len() < width * height * 4 || width.min(height) < 8 * REACH {
        return None;
    }
    let luma = luma(pixels, width, height);
    let across = scan(&Picture::new(&luma, width, height, false));
    let down = scan(&Picture::new(&luma, width, height, true));
    match (across, down) {
        (Some(a), Some(b)) => Some(match a.agreement >= b.agreement {
            true => a,
            false => b,
        }),
        (found, None) | (None, found) => found,
    }
}

/// A picture to scan down the columns of, which is the transpose of itself
/// half the time.
struct Picture<'a> {
    luma: &'a [f64],
    /// The picture's own width, which the transposed reading has as its
    /// height.
    stride: usize,
    width: usize,
    height: usize,
    /// Set when this is the transposed reading, so a `(u, v)` found here is a
    /// `(v, u)` in the picture.
    turned: bool,
}

impl<'a> Picture<'a> {
    fn new(luma: &'a [f64], width: usize, height: usize, turned: bool) -> Self {
        let (across, down) = match turned {
            true => (height, width),
            false => (width, height),
        };
        Self {
            luma,
            stride: width,
            width: across,
            height: down,
            turned,
        }
    }

    fn at(&self, x: usize, y: usize) -> f64 {
        match self.turned {
            true => self.luma[x * self.stride + y],
            false => self.luma[y * self.stride + x],
        }
    }

    /// A point of this reading, back in the picture's own 0 to 1 coordinates.
    fn uv(&self, x: f64, y: f64) -> [f64; 2] {
        let (u, v) = (x / self.width as f64, y / self.height as f64);
        match self.turned {
            true => [v, u],
            false => [u, v],
        }
    }
}

/// One reading of the picture: an edge per column, a line through them, and
/// what the line means.
fn scan(picture: &Picture<'_>) -> Option<Skyline> {
    let mut kept: Vec<(f64, f64, f64)> = (0..picture.width)
        .step_by(STEP)
        .filter_map(|x| edge(picture, x))
        .collect();
    let mut line = fit(&kept)?;
    for _ in 0..REFITS {
        let spread = spread(&kept, line);
        kept.retain(|(x, y, _)| (y - (line.0 + line.1 * x)).abs() <= OUTLIER * spread.max(0.5));
        line = fit(&kept)?;
    }
    let agreement = kept.len() as f64 / (picture.width / STEP) as f64;
    if agreement < KEEP || spread(&kept, line) > STRAIGHT {
        return None;
    }

    let (a, b) = line;
    let through = [0.15, 0.85].map(|share| {
        let x = share * picture.width as f64;
        picture.uv(x, a + b * x)
    });
    Some(Skyline {
        degrees: tilt(picture, b),
        through,
        sky: sky(picture, line),
        agreement,
    })
}

/// The line's angle from the picture's own horizontal, in (-90, 90].
fn tilt(picture: &Picture<'_>, slope: f64) -> f64 {
    let degrees = match picture.turned {
        // The fit is x against y in the picture's own axes, so the line's
        // direction there is `(slope, 1)` and the quarter turn falls out of
        // the arctangent of the other ratio.
        true => 90.0 - slope.atan().to_degrees(),
        false => slope.atan().to_degrees(),
    };
    match degrees > 90.0 {
        true => degrees - 180.0,
        false => degrees,
    }
}

/// Which way across the line the picture is brighter, as a unit vector in the
/// picture's own 0 to 1 coordinates.
///
/// A step across the line is a step along its normal, `(-b, 1)` normalized in
/// this reading's own pixels. Both ends of that step are sampled at three
/// places along the line, and the brighter end is the sky.
fn sky(picture: &Picture<'_>, (a, b): (f64, f64)) -> [f64; 2] {
    let aside = ASIDE * picture.width.min(picture.height) as f64;
    let length = (1.0 + b * b).sqrt();
    let normal = [-b / length, 1.0 / length];
    let brightness = |side: f64| {
        [0.3, 0.5, 0.7]
            .iter()
            .map(|share| {
                let x = share * picture.width as f64 + side * aside * normal[0];
                let y = a + b * (share * picture.width as f64) + side * aside * normal[1];
                picture.at(
                    (x.max(0.0) as usize).min(picture.width - 1),
                    (y.max(0.0) as usize).min(picture.height - 1),
                )
            })
            .sum::<f64>()
    };
    let toward = match brightness(1.0) > brightness(-1.0) {
        true => 1.0,
        false => -1.0,
    };
    // Back into the picture's coordinates, where the two axes are scaled
    // differently, so it is renormalized there.
    let step = picture.uv(
        normal[0] * toward * picture.width as f64,
        normal[1] * toward * picture.height as f64,
    );
    let length = step[0].hypot(step[1]).max(f64::MIN_POSITIVE);
    [step[0] / length, step[1] / length]
}

/// Rec.709 luma, which is what the shader's own YUV conversion writes back
/// out.
fn luma(pixels: &[u8], width: usize, height: usize) -> Vec<f64> {
    (0..width * height)
        .map(|at| {
            let rgba = &pixels[at * 4..at * 4 + 3];
            0.2126 * f64::from(rgba[0]) + 0.7152 * f64::from(rgba[1]) + 0.0722 * f64::from(rgba[2])
        })
        .collect()
}

/// The topmost strong brightness change in one column, as `(x, y, weight)`,
/// with `y` refined to a fraction of a row.
fn edge(picture: &Picture<'_>, x: usize) -> Option<(f64, f64, f64)> {
    let smoothed = smoothed(picture, x);
    // Signed, and the sign is kept: an edge is an edge whichever way round
    // the sky is, but the two sides of one edge must not cancel when the
    // position is averaged below.
    let slope = |y: usize| smoothed[y + 1] - smoothed[y - 1];

    let edges = (picture.height as f64 * MARGIN) as usize + SMOOTH + 3 * REACH + 1;
    let rows = edges..picture.height - edges;
    let sharpest = rows.clone().map(|y| slope(y).abs()).fold(0.0f64, f64::max);
    if sharpest < CONTRAST {
        return None;
    }
    let band = |from: usize, to: usize| {
        (from..to).map(|y| smoothed[y]).sum::<f64>() / (to - from).max(1) as f64
    };
    let (peak, steepest) = rows
        .map(|y| (y, slope(y)))
        .filter(|(_, step)| step.abs() >= sharpest * SHARE)
        .find(|(y, step)| {
            let across = band(y + REACH, y + 3 * REACH) - band(y - 3 * REACH, y - REACH);
            across.abs() >= ACROSS && across * step > 0.0
        })?;

    // Where the edge sits, as the centre of mass of the rows that make it up.
    // Only the rows leaning the same way as the peak count, so a bright
    // band's far side does not pull its near side.
    let rows = peak - REACH..=peak + REACH;
    let weights: Vec<f64> = rows
        .clone()
        .map(|y| (slope(y) / steepest).max(0.0))
        .collect();
    let total: f64 = weights.iter().sum();
    let centre: f64 = rows
        .zip(&weights)
        .map(|(y, weight)| y as f64 * weight)
        .sum::<f64>()
        / total;
    Some((x as f64, centre, steepest.abs()))
}

/// One column of this reading with a box blur down it.
fn smoothed(picture: &Picture<'_>, x: usize) -> Vec<f64> {
    (0..picture.height)
        .map(|y| {
            let from = y.saturating_sub(SMOOTH);
            let to = (y + SMOOTH).min(picture.height - 1);
            (from..=to).map(|y| picture.at(x, y)).sum::<f64>() / (to - from + 1) as f64
        })
        .collect()
}

/// A weighted least-squares line, `y = a + b x`.
fn fit(points: &[(f64, f64, f64)]) -> Option<(f64, f64)> {
    let total: f64 = points.iter().map(|(_, _, w)| w).sum();
    if points.len() < 4 || total <= 0.0 {
        return None;
    }
    let mean_x: f64 = points.iter().map(|(x, _, w)| x * w).sum::<f64>() / total;
    let mean_y: f64 = points.iter().map(|(_, y, w)| y * w).sum::<f64>() / total;
    let covariance: f64 = points
        .iter()
        .map(|(x, y, w)| w * (x - mean_x) * (y - mean_y))
        .sum();
    let variance: f64 = points
        .iter()
        .map(|(x, _, w)| w * (x - mean_x) * (x - mean_x))
        .sum();
    if variance <= 0.0 {
        return None;
    }
    let slope = covariance / variance;
    Some((mean_y - slope * mean_x, slope))
}

/// The root mean square distance of the points from the line.
fn spread(points: &[(f64, f64, f64)], (a, b): (f64, f64)) -> f64 {
    let count = points.len().max(1) as f64;
    (points
        .iter()
        .map(|(x, y, _)| (y - (a + b * x)).powi(2))
        .sum::<f64>()
        / count)
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size {
        width: 960,
        height: 540,
    };

    /// A bright sky over dark ground, with the boundary tilted by `degrees`
    /// and a little shading on both.
    fn picture(degrees: f64, extras: impl Fn(usize, usize) -> Option<u8>) -> Vec<u8> {
        let (width, height) = (SIZE.width as usize, SIZE.height as usize);
        let (sin, cos) = degrees.to_radians().sin_cos();
        let mut pixels = vec![255u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                // Distance across the line through the middle of the picture,
                // which is defined at every angle where a slope is not.
                let across =
                    (y as f64 - height as f64 / 2.0) * cos - (x as f64 - width as f64 / 2.0) * sin;
                let shade = match across < 0.0 {
                    true => 200 + (y % 16) as u8,
                    false => 40 + (y % 16) as u8,
                };
                let value = extras(x, y).unwrap_or(shade);
                pixels[(y * width + x) * 4..(y * width + x) * 4 + 3].fill(value);
            }
        }
        pixels
    }

    fn nothing(_x: usize, _y: usize) -> Option<u8> {
        None
    }

    /// The positive control: a picture tilted by a known angle reads back
    /// that angle. Without this the instrument's silence proves nothing.
    ///
    /// The range runs to either side of vertical, because a camera clamped to
    /// a paramotor is rolled most of a quarter turn and the view with the
    /// lock off is where it has to be measured.
    #[test]
    fn a_known_tilt_reads_back_as_that_tilt() {
        for degrees in [-88.0, -70.0, -20.0, -1.0, 0.0, 0.5, 18.0, 60.0, 88.0] {
            let found = skyline(&picture(degrees, nothing), SIZE)
                .unwrap_or_else(|| panic!("no horizon at {degrees} degrees"));
            assert!(
                (found.degrees - degrees).abs() < 0.2,
                "read {} for {degrees}",
                found.degrees
            );
        }
    }

    /// It has to resolve a tenth of a degree, because that is the size of the
    /// residual sway it is being used to argue about.
    #[test]
    fn it_resolves_a_tenth_of_a_degree() {
        let flat = skyline(&picture(0.0, nothing), SIZE).unwrap();
        let leaning = skyline(&picture(0.1, nothing), SIZE).unwrap();

        assert!(
            (leaning.degrees - flat.degrees - 0.1).abs() < 0.03,
            "{} against {}",
            leaning.degrees,
            flat.degrees
        );
    }

    /// Which side is sky, which is what says which way up an answer read off
    /// this line is. The bright half is above the line in this picture, and v
    /// runs down, so the vector across the line points at a smaller v.
    #[test]
    fn the_bright_side_is_named() {
        let level = skyline(&picture(0.0, nothing), SIZE).unwrap();
        assert!(level.sky[1] < -0.9, "{:?}", level.sky);

        // Turned a quarter turn, the sky is to one side instead.
        let turned = skyline(&picture(88.0, nothing), SIZE).unwrap();
        assert!(turned.sky[0].abs() > 0.9, "{:?}", turned.sky);
    }

    /// The two points are on the line, far enough apart to be a line, and at
    /// the angle the line was reported at.
    #[test]
    fn the_points_handed_out_are_on_the_line() {
        for degrees in [0.0, 30.0, 85.0] {
            let found = skyline(&picture(degrees, nothing), SIZE).unwrap();
            let [from, to] = found.through;
            let along = (to[1] - from[1]) * f64::from(SIZE.height);
            let across = (to[0] - from[0]) * f64::from(SIZE.width);
            assert!(across.hypot(along) > 200.0, "{found:?}");
            let angle = along.atan2(across).to_degrees();
            let angle = match angle > 90.0 {
                true => angle - 180.0,
                false => angle,
            };
            assert!((angle - degrees).abs() < 0.3, "{found:?} for {degrees}");
        }
    }

    /// A wing across the frame is what a paramotor capture actually looks
    /// like, and it is a second long bright edge at an angle of its own. The
    /// outlier passes are what stop it being read as the horizon.
    #[test]
    fn a_wing_across_the_frame_is_not_the_horizon() {
        let wing = |x: usize, y: usize| {
            let band = 300 + x / 12;
            (y >= band && y < band + 40).then_some(250)
        };
        let found = skyline(&picture(-4.0, wing), SIZE).expect("no horizon");

        assert!((found.degrees + 4.0).abs() < 0.3, "read {}", found.degrees);
    }

    /// And the other half of an instrument that can be trusted: a picture
    /// with nothing in it answers nothing, rather than answering zero.
    #[test]
    fn a_picture_with_no_horizon_in_it_answers_nothing() {
        let (width, height) = (SIZE.width as usize, SIZE.height as usize);
        let flat = vec![128u8; width * height * 4];
        assert_eq!(skyline(&flat, SIZE), None);

        // Noise with no structure: every scan line finds a different row and
        // no line goes through them. The edge threshold is a share of each
        // line's own sharpest change, so noise always clears it; what rejects
        // this is straightness, which is the property a horizon has.
        let mut noisy = flat.clone();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for byte in noisy.iter_mut() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = (state >> 32) as u8;
        }
        assert_eq!(skyline(&noisy, SIZE), None);
    }

    #[test]
    fn a_picture_smaller_than_the_search_answers_nothing() {
        assert_eq!(skyline(&[0; 16], Size::new(2, 2)), None);
    }
}
