//! What Insta360 Studio's **Chromatic Calibration** toggle does to a stitched
//! frame, and whether the shape of what it does is the shape a **source
//! matching** correction has (issue #103, stage 10).
//!
//! ```sh
//! # the whole reading on one on/off pair, controls and pictures included
//! cargo run --release -p kjerag-spike --bin oracle -- \
//!   ~/Videos/studio_onoff/chromatic_calibration out=scratch/oracle tag=pair1
//! # the memo's own far-field blocks, re-measured, as a cross-check
//! cargo run --release -p kjerag-spike --bin oracle -- <dir> mode=control block=0:0:1000:400
//! # one part at a time, when only one table is wanted
//! cargo run --release -p kjerag-spike --bin oracle -- <dir> mode=o2 places=9
//! ```
//!
//! **The question this exists for.** The owner has ruled that Studio's
//! chromatic correction is seam-line aware: in other places it only changes
//! things close to the seam line. A twenty times amplified difference shows a
//! dark line down the middle of the changed band. The hypothesis that predicts
//! a dark line is **local antisymmetric source matching under a crossfade**:
//! each lens's contribution is pulled towards the local mean of the two before
//! they are mixed, so where the mix is even the two pulls cancel exactly and
//! the visible change peaks in the flanks either side. Written out, with `w`
//! the weight the stitcher gives lens 0 and `d` the pull,
//!
//! ```text
//! ON - OFF  =  w (l0 + d) + (1 - w) (l1 - d)  -  (w l0 + (1 - w) l1)
//!           =  d (2w - 1)
//! ```
//!
//! which is zero at `w = 1/2`, odd about that line, and bounded by the pull,
//! which is itself bounded by half the disagreement it is closing. **A free
//! additive field has none of those three properties**, and a free additive
//! field over wide support is the thing this campaign has refused twice. So
//! the three properties are three tests, and they are the three modes below.
//!
//! **What a stitched output can and cannot settle.** Both frames are already
//! blended. The two lenses' own pictures are not in the file, so the
//! disagreement `l1 - l0` is never read directly, only inferred from the trend
//! each side of the seam leaves. Every number here is honest about which of
//! the two it is.
//!
//! **The export is not equirectangular and that is measured, not assumed.**
//! 3840x2160 is 16:9 and a whole equirectangular frame is 2:1, and the horizon
//! in the owner's own render is a large arc rather than a row. These are
//! Studio's reframed wide views, so no constant turns a column into a degree
//! of longitude. What turns pixels into degrees here is a projection fitted to
//! the seam itself, under the one thing the seam is known to be: **a great
//! circle**, because it is where two hemispheres meet. The fit's residual is
//! printed beside every number it scales, and a run that cannot fit says so
//! and reports pixels alone.
//!
//! **Controls, because a control that has never been shown able to fire has
//! cleared nothing.** Three of them, and all three are the same code over
//! different pixels rather than a second code path:
//!
//! - the **null**, OFF against OFF, which has to read exactly zero;
//! - the **decoy**, the real pair read about a great circle ninety degrees
//!   away from the seam, which is what a pair of independent JPEG encodes and
//!   the scene's own texture are worth in the units every other number is in;
//! - the **plant**, a known odd lobe of known amplitude and known width added
//!   to a copy of OFF, which the tracer has to find and the profile machinery
//!   has to measure back.
//!
//! PNGs land in gitignored `scratch/`: these are frames of somebody's real
//! flights and this repo is public.

use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kjerag_media::Fallible;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let pair = Pair::read(&options.off(), &options.on())?;
    println!(
        "oracle: {} against {}\n\t{}x{} rgb24, read through ffmpeg the way `--bin colour`'s \
         studio mode reads a competitor's export.",
        options.off().display(),
        options.on().display(),
        pair.off.width,
        pair.off.height,
    );
    let seam = Seam::find(&pair, &options)?;
    if matches!(options.mode, Mode::All | Mode::Trace) {
        seam.report(&pair);
    }
    if matches!(options.mode, Mode::All | Mode::Control) {
        control(&pair, &seam, &options);
    }
    let wanted = matches!(
        options.mode,
        Mode::All | Mode::O1 | Mode::O2 | Mode::O3 | Mode::Pictures
    );
    let profiles = if wanted {
        seam.profiles(&pair, &options)
    } else {
        Vec::new()
    };
    // The floor first, because it is the gate every verdict below is given
    // through: an azimuth whose change does not clear the decoy gets a row and
    // not a verdict.
    let floor = if wanted {
        decoy_floor(&pair, &seam, &options)
    } else {
        0.0
    };
    let readings: Vec<Reading> = profiles
        .iter()
        .filter_map(|p| Reading::read(p, &options))
        .collect();
    if wanted {
        println!(
            "\n  the floor, measured before anything is judged against it: the same profiles \n\
             \tread a quarter turn from the seam, where there is no handover, come to {floor:.4} \n\
             \tcodes rms. An azimuth is given a verdict below only if its own change is {CLEARS:.0} \n\
             \ttimes that.",
        );
    }
    if matches!(options.mode, Mode::All | Mode::O1) {
        question_one(&readings, &seam, floor, &options);
    }
    if matches!(options.mode, Mode::All | Mode::O2) {
        question_two(&readings, floor);
    }
    if matches!(options.mode, Mode::All | Mode::O3) {
        question_three(&readings, floor);
    }
    if matches!(options.mode, Mode::All | Mode::Controls) {
        controls(&pair, &seam, &options)?;
    }
    if matches!(options.mode, Mode::All | Mode::Pictures) {
        pictures(&pair, &seam, &readings, &options)?;
    }
    Ok(())
}

/// Which of the tables one run prints. Every mode traces the seam first,
/// because every other number is measured against the line it finds.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    All,
    /// The far field, which is what says the pair differs by one toggle, and
    /// the pair's own JPEG noise floor.
    Control,
    /// The seam, found from the difference and fitted as a great circle.
    Trace,
    /// Is the difference odd about the line, with its zero crossing on it.
    O1,
    /// Does the falloff collapse onto one shape once each azimuth is divided
    /// by its own peak.
    O2,
    /// Is the correction bounded by the disagreement it is closing.
    O3,
    /// The null, the decoy and the plant.
    Controls,
    /// The amplified difference, the traced line and the profile overlays.
    Pictures,
}

// ------------------------------------------------------------ the frames

/// One decoded frame as gamma-coded R, G and B in codes.
///
/// The frame arrives through ffmpeg as raw rgb24 on a pipe, which is the
/// pattern `--bin colour`'s studio mode already uses: it is one dependency the
/// harness has anyway, and no decoder in this repo opens a finished JPEG.
struct Frame {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Frame {
    /// One frame of whatever ffmpeg can decode, as rgb24.
    fn read(path: &Path) -> Fallible<Self> {
        let (width, height) = size_of(path)?;
        let mut child = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdout(Stdio::piped())
            .spawn()?;
        let mut rgb = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            std::io::Read::read_to_end(&mut out, &mut rgb)?;
        }
        child.wait()?;
        if rgb.len() < width * height * 3 {
            return Err(format!(
                "ffmpeg gave {} bytes for a {width}x{height} rgb24 frame of {}",
                rgb.len(),
                path.display(),
            )
            .into());
        }
        rgb.truncate(width * height * 3);
        Ok(Self { width, height, rgb })
    }

    fn code(&self, x: usize, y: usize, channel: usize) -> f64 {
        f64::from(self.rgb[(y * self.width + x) * 3 + channel])
    }

    /// The three channels at a fractional place, bilinear, or `None` where the
    /// place is off the picture.
    fn at(&self, x: f64, y: f64) -> Option<[f64; 3]> {
        if !(x >= 0.0 && y >= 0.0) {
            return None;
        }
        let (fx, fy) = (x.floor(), y.floor());
        let (ix, iy) = (fx as usize, fy as usize);
        if ix + 1 >= self.width || iy + 1 >= self.height {
            return None;
        }
        let (tx, ty) = (x - fx, y - fy);
        let mut out = [0.0; 3];
        for (channel, held) in out.iter_mut().enumerate() {
            let a = self.code(ix, iy, channel);
            let b = self.code(ix + 1, iy, channel);
            let c = self.code(ix, iy + 1, channel);
            let d = self.code(ix + 1, iy + 1, channel);
            *held = a + (b - a) * tx + (c - a) * ty + (a - b - c + d) * tx * ty;
        }
        Some(out)
    }
}

/// The frame size ffprobe reports for a file. `--bin colour`'s own, because a
/// second reader is a second answer to the one question both ask.
fn size_of(path: &Path) -> Fallible<(usize, usize)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(path)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (width, height) = text
        .trim()
        .split_once('x')
        .ok_or_else(|| format!("ffprobe said {text:?} for {}", path.display()))?;
    Ok((width.trim().parse()?, height.trim().parse()?))
}

/// The toggle's two states of one frame, which is the whole experiment.
struct Pair {
    off: Frame,
    on: Frame,
}

impl Pair {
    fn read(off: &Path, on: &Path) -> Fallible<Self> {
        let off = Frame::read(off)?;
        let on = Frame::read(on)?;
        if off.width != on.width || off.height != on.height {
            return Err(format!(
                "the two frames are {}x{} and {}x{}, which is not one experiment",
                off.width, off.height, on.width, on.height,
            )
            .into());
        }
        Ok(Self { off, on })
    }

    fn size(&self) -> (usize, usize) {
        (self.off.width, self.off.height)
    }
}

// ------------------------------------------------------------ the geometry

/// How a reframed export maps an angle off its own axis to a radius in
/// pixels.
///
/// Studio does not write down which of these it used and the file does not
/// carry it, so the fit tries them all and the residual picks one. The last is
/// in the list to be **refused**: a rectilinear reframe draws every great
/// circle as a straight line, so if the seam is an arc then rectilinear is
/// wrong, and a family that cannot be wrong is a family that is not being
/// tested.
#[derive(Clone, Copy, PartialEq)]
enum Model {
    Equidistant,
    Stereographic,
    Equisolid,
    Orthographic,
    Rectilinear,
    /// `r = theta + bend theta^3`, one number that walks through all five of
    /// the named families and past them.
    ///
    /// It is here because none of the five fits the owner's own render: the
    /// best of them leaves a hundred pixels rms on a seam that the trace shows
    /// as a clean smooth curve, which means Studio's reframe is not any of the
    /// textbook ones. Expanded to third order the five are `bend` of 0,
    /// 1/12, -1/24, -1/6 and 1/3, so a free `bend` is not a new idea, it is
    /// the same idea with the coefficient measured instead of assumed.
    Bent,
    /// Longitude across, latitude down, which is what an equirectangular frame
    /// is over a whole sphere and is also what a reframe of one looks like if
    /// the exporter simply cropped a window out of it.
    ///
    /// Not radially symmetric, so it is not in the family above and cannot be
    /// reached from it. `bend` here is the vertical scale over the horizontal
    /// one, because a 16:9 frame holding 360 by 180 degrees is stretched by
    /// exactly 1.125 and there is no reason to assume the exporter did not.
    Equirect,
    /// The same across, but `tan` of the latitude down, which keeps a vertical
    /// straight and is the shape a wide "natural" view usually is.
    Cylinder,
}

impl Model {
    const ALL: [Self; 8] = [
        Self::Equidistant,
        Self::Stereographic,
        Self::Equisolid,
        Self::Orthographic,
        Self::Rectilinear,
        Self::Bent,
        Self::Equirect,
        Self::Cylinder,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Equidistant => "equidistant",
            Self::Stereographic => "stereographic",
            Self::Equisolid => "equisolid",
            Self::Orthographic => "orthographic",
            Self::Rectilinear => "rectilinear",
            Self::Bent => "bent cubic",
            Self::Equirect => "equirect crop",
            Self::Cylinder => "cylinder",
        }
    }

    /// Whether the family maps a radius to an angle at all. The two below do
    /// not: they are one rule across and another down.
    fn radial(self) -> bool {
        !matches!(self, Self::Equirect | Self::Cylinder)
    }

    /// The radius, in focal lengths, an angle of `theta` radians off the axis
    /// lands at.
    fn radius(self, theta: f64, bend: f64) -> f64 {
        match self {
            Self::Equidistant => theta,
            Self::Stereographic => 2.0 * (theta / 2.0).tan(),
            Self::Equisolid => 2.0 * (theta / 2.0).sin(),
            Self::Orthographic => theta.sin(),
            Self::Rectilinear => theta.tan(),
            Self::Bent => theta + bend * theta * theta * theta,
            Self::Equirect | Self::Cylinder => theta,
        }
    }

    /// The angle a radius came from, or `None` where the family has no picture
    /// that far out.
    fn angle(self, radius: f64, bend: f64) -> Option<f64> {
        let theta = match self {
            Self::Equidistant if radius <= std::f64::consts::PI => radius,
            Self::Stereographic => 2.0 * (radius / 2.0).atan(),
            Self::Equisolid if radius <= 2.0 => 2.0 * (radius / 2.0).asin(),
            Self::Orthographic if radius <= 1.0 => radius.asin(),
            Self::Rectilinear => radius.atan(),
            // Bisection, and bracketed to the stretch where the cubic is still
            // going up: a projection that folds back on itself is not one an
            // exporter wrote, and inverting it would answer two angles for one
            // pixel.
            Self::Bent => {
                let top = if bend < 0.0 {
                    (-1.0 / (3.0 * bend)).sqrt().min(std::f64::consts::PI)
                } else {
                    std::f64::consts::PI
                };
                if radius > Self::Bent.radius(top, bend) {
                    return None;
                }
                let (mut lo, mut hi) = (0.0, top);
                for _ in 0..48 {
                    let mid = (lo + hi) / 2.0;
                    if Self::Bent.radius(mid, bend) < radius {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                (lo + hi) / 2.0
            }
            _ => return None,
        };
        Some(theta)
    }

    /// How many values of `bend` are worth trying for this family. Only the
    /// bent cubic has one.
    fn bends(self) -> Vec<f64> {
        match self {
            Self::Bent => (0..=24).map(|i| -0.5 + i as f64 / 24.0).collect(),
            Self::Equirect | Self::Cylinder => (0..=16).map(|i| 0.6 + i as f64 * 0.075).collect(),
            _ => vec![1.0],
        }
    }
}

/// A projection with its scale and its principal point, and nothing else: the
/// reframe's own rotation lives in whatever great circle is being fitted
/// through it, so it is never a parameter here.
#[derive(Clone, Copy)]
struct Map {
    model: Model,
    /// The focal length in pixels, which is the one free number every family
    /// has.
    focal: f64,
    /// The cubic term, which only [`Model::Bent`] uses.
    bend: f64,
    centre: (f64, f64),
}

impl Map {
    /// The direction a pixel looks in, or `None` where the family has no ray
    /// for it.
    fn ray(&self, x: f64, y: f64) -> Option<[f64; 3]> {
        let u = (x - self.centre.0) / self.focal;
        let v = (y - self.centre.1) / self.focal;
        if !self.model.radial() {
            let longitude = u;
            let latitude = match self.model {
                Model::Cylinder => (-v / self.bend).atan(),
                _ => -v / self.bend,
            };
            if longitude.abs() > std::f64::consts::PI
                || latitude.abs() > std::f64::consts::FRAC_PI_2
            {
                return None;
            }
            let (sl, cl) = longitude.sin_cos();
            let (sp, cp) = latitude.sin_cos();
            return Some([cp * sl, sp, cp * cl]);
        }
        let r = u.hypot(v);
        if r < 1e-12 {
            return Some([0.0, 0.0, 1.0]);
        }
        let theta = self.model.angle(r, self.bend)?;
        let (s, c) = theta.sin_cos();
        Some([s * u / r, s * v / r, c])
    }

    /// Where a direction lands, or `None` where it lands behind the family's
    /// own horizon.
    fn pixel(&self, d: [f64; 3]) -> Option<(f64, f64)> {
        if !self.model.radial() {
            let latitude = d[1].clamp(-1.0, 1.0).asin();
            let longitude = d[0].atan2(d[2]);
            let down = match self.model {
                Model::Cylinder => latitude.tan(),
                _ => latitude,
            };
            if !down.is_finite() {
                return None;
            }
            return Some((
                self.centre.0 + self.focal * longitude,
                self.centre.1 - self.focal * self.bend * down,
            ));
        }
        let theta = d[2].clamp(-1.0, 1.0).acos();
        if !(theta.is_finite() && theta < std::f64::consts::PI * 0.999) {
            return None;
        }
        if matches!(self.model, Model::Rectilinear) && theta > 1.5 {
            return None;
        }
        if matches!(self.model, Model::Orthographic) && theta > std::f64::consts::FRAC_PI_2 {
            return None;
        }
        let r = self.focal * self.model.radius(theta, self.bend);
        if !r.is_finite() {
            return None;
        }
        let h = d[0].hypot(d[1]);
        if h < 1e-12 {
            return Some(self.centre);
        }
        Some((self.centre.0 + r * d[0] / h, self.centre.1 + r * d[1] / h))
    }
}

/// A great circle on the sphere, as the frame the profiles are read in: its
/// normal, and two unit vectors that span it.
///
/// `along` and `across` exist so that one point of the circle and one offset
/// off it are one expression rather than a rotation each time:
/// `p(phi) = along cos phi + across sin phi`, and the perpendicular walk from
/// there is `p cos s + normal sin s`, which is a great circle at right angles
/// to this one. **That is the only across-seam direction with a constant
/// meaning**, and it is not the picture's own perpendicular, which a reframe
/// bends.
#[derive(Clone, Copy)]
struct Circle {
    normal: [f64; 3],
    along: [f64; 3],
    across: [f64; 3],
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    if n < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        [a[0] / n, a[1] / n, a[2] / n]
    }
}

impl Circle {
    fn from_normal(normal: [f64; 3]) -> Self {
        let normal = unit(normal);
        let seed = if normal[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let along = unit(cross(normal, seed));
        let across = cross(normal, along);
        Self {
            normal,
            along,
            across,
        }
    }

    /// The point of the circle at `phi` radians.
    fn at(&self, phi: f64) -> [f64; 3] {
        let (s, c) = phi.sin_cos();
        [
            self.along[0] * c + self.across[0] * s,
            self.along[1] * c + self.across[1] * s,
            self.along[2] * c + self.across[2] * s,
        ]
    }

    /// The point `s` radians off the circle, perpendicular to it, at `phi`.
    fn off(&self, phi: f64, s: f64) -> [f64; 3] {
        let p = self.at(phi);
        let (ss, cs) = s.sin_cos();
        [
            p[0] * cs + self.normal[0] * ss,
            p[1] * cs + self.normal[1] * ss,
            p[2] * cs + self.normal[2] * ss,
        ]
    }

    /// How far a direction is off the circle, in radians, signed towards the
    /// normal.
    fn distance(&self, d: [f64; 3]) -> f64 {
        dot(self.normal, d).clamp(-1.0, 1.0).asin()
    }

    /// The same circle turned a quarter turn, which is the decoy: a great
    /// circle through the same picture that no handover happens on.
    fn turned(&self) -> Self {
        let mut turned = Self::from_normal(self.across);
        turned.along = self.along;
        turned.across = self.normal;
        turned
    }

    /// The same circle tilted by `radians`, which is where a planted lobe goes:
    /// somewhere the tracer has to actually find rather than somewhere it is
    /// already looking.
    fn tilted(&self, radians: f64) -> Self {
        let (s, c) = radians.sin_cos();
        Self::from_normal([
            self.normal[0] * c + self.across[0] * s,
            self.normal[1] * c + self.across[1] * s,
            self.normal[2] * c + self.across[2] * s,
        ])
    }
}

/// Eigenvalues and eigenvectors of a symmetric 3x3, by Jacobi rotations.
///
/// The instrument's own, for `--bin colour`'s reason: a solver borrowed from a
/// shipped crate is one this cannot be run against a second build of that
/// crate with. Vectors come back as columns, in no particular order.
fn eigen3(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[0.0; 3]; 3];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..64 {
        let mut worst = 0.0;
        let (mut p, mut q) = (0, 1);
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            if a[i][j].abs() > worst {
                worst = a[i][j].abs();
                p = i;
                q = j;
            }
        }
        if worst < 1e-18 {
            break;
        }
        let theta = 0.5 * (a[q][q] - a[p][p]) / a[p][q];
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        for row in a.iter_mut() {
            let (akp, akq) = (row[p], row[q]);
            row[p] = c * akp - s * akq;
            row[q] = s * akp + c * akq;
        }
        let (row_p, row_q) = (a[p], a[q]);
        for k in 0..3 {
            a[p][k] = c * row_p[k] - s * row_q[k];
            a[q][k] = s * row_p[k] + c * row_q[k];
        }
        for row in v.iter_mut() {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// The direction of least scatter of a set of unit vectors, which for points
/// that lie on a great circle is that circle's normal.
fn plane_of(rays: &[[f64; 3]]) -> Option<([f64; 3], f64)> {
    if rays.len() < 3 {
        return None;
    }
    let mut m = [[0.0; 3]; 3];
    for d in rays {
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += d[i] * d[j];
            }
        }
    }
    let (values, vectors) = eigen3(m);
    let mut best = 0;
    for i in 1..3 {
        if values[i] < values[best] {
            best = i;
        }
    }
    let normal = unit([vectors[0][best], vectors[1][best], vectors[2][best]]);
    let residual = (values[best].max(0.0) / rays.len() as f64).sqrt();
    Some((normal, residual))
}

// ------------------------------------------------------------ the tracer

/// How many picture columns (or rows) one trace point is pooled over, and how
/// far along the scan line the pooled profile is then smoothed.
///
/// Both are sized against the noise rather than against the picture. A pair of
/// independent JPEG encodes differs by a couple of codes per pixel, and the
/// line has to be found in a difference of about the same size, so a trace
/// point is a mean of `TRACE_BLOCK` times `2 * TRACE_SMOOTH + 1` pixels and its
/// noise is a fortieth of one pixel's.
const TRACE_BLOCK: usize = 32;
const TRACE_SMOOTH: usize = 20;

/// How much a trace point has to swing, top to bottom, before it is a reading
/// of anything, in codes summed over the three channels.
///
/// The floor under it is the block's own, which is around a tenth of a code,
/// so this is ten times the noise and not a hopeful threshold.
const TRACE_SWING: f64 = 1.0;

/// How far apart the two lobes of one scan line are allowed to be, and how far
/// from the end of the scan line each of them has to sit, in pixels.
///
/// Both are guards against the same failure, and the failure is what the first
/// version of this did: a scan line that crosses the band near the edge of the
/// picture has one lobe cut off by the frame, so the surviving extreme is
/// somewhere in the scene's own texture and the midpoint of the two is
/// hundreds of pixels from the seam. That version fitted a great circle to
/// those midpoints and left 117 pixels rms, which is not a great circle and is
/// not a seam. A pair further apart than `TRACE_SPAN` is not one band's two
/// lobes, and a lobe inside `TRACE_EDGE` of the end is one that may have been
/// cut short.
const TRACE_SPAN: f64 = 1400.0;
const TRACE_EDGE: usize = 80;

/// What share of the strongest scan line's swing a scan line has to reach
/// before its mark is fitted through.
const TRACE_SHARE: f64 = 0.25;

/// How far either side of an already-fitted line the second sweep looks, in
/// degrees.
///
/// The first sweep has to search the whole scan line because it knows nothing;
/// the second knows roughly where the seam is and can refuse everything else,
/// which is what turns a rough fit into a sharp one.
const TRACE_WINDOW: f64 = 12.0;

/// Which feature of the difference along a scan line is taken as the line.
///
/// Three, because which one to use is a question and not a decision, and the
/// answer is a measurement: the seam is a great circle, so the definition whose
/// points lie on one is the definition that is tracking the seam. The other two
/// are then reported against it, which is what O1 is.
#[derive(Clone, Copy, PartialEq)]
enum Centre {
    /// Halfway between the two lobes. Says nothing about where the difference
    /// changes sign, and is biased whenever the two lobes are different widths.
    Lobes,
    /// Where the difference changes sign.
    Crossing,
    /// Where the SIZE of the difference is smallest between the two lobes,
    /// which is the dark line an amplified diff shows. Not the same as the
    /// crossing whenever the profile has an even part.
    Dark,
}

impl Centre {
    fn name(self) -> &'static str {
        match self {
            Self::Lobes => "lobe midpoint",
            Self::Crossing => "zero crossing",
            Self::Dark => "dark line",
        }
    }
}

/// One point the seam was seen at, and how strongly.
#[derive(Clone, Copy)]
struct Mark {
    x: f64,
    y: f64,
    swing: f64,
    /// Which way the scan ran, so the two sweeps can be reported apart.
    rows: bool,
}

/// A running mean over `2 * half + 1` samples, clamped at the ends.
fn boxcar(v: &[f64], half: usize) -> Vec<f64> {
    let mut out = vec![0.0; v.len()];
    for (i, held) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(v.len());
        *held = v[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
    }
    out
}

/// Every place the difference swings from one sign to the other, swept one way
/// across the picture.
///
/// **The centre is taken as the midpoint of the two lobes and not as the zero
/// crossing**, and that is the whole reason O1 is a test rather than a
/// tautology: a line defined by the crossing cannot then be asked whether the
/// crossing sits on it. The lobes are the two extremes of the smoothed
/// difference, which is where the correction is largest, and their midpoint is
/// a statement about the band and not about where it changes sign.
fn scan(pair: &Pair, rows: bool, guide: Option<(&Map, &Circle)>, centre: Centre) -> Vec<Mark> {
    let (width, height) = pair.size();
    let (across, along) = if rows {
        (height, width)
    } else {
        (width, height)
    };
    let place = |i: usize, block: usize| {
        let fixed = block as f64 + TRACE_BLOCK as f64 / 2.0;
        if rows {
            (i as f64, fixed)
        } else {
            (fixed, i as f64)
        }
    };
    let mut marks = Vec::new();
    let mut block = 0;
    while block + TRACE_BLOCK <= across {
        let mut bands: [Vec<f64>; 3] = [vec![0.0; along], vec![0.0; along], vec![0.0; along]];
        for i in 0..along {
            for k in 0..TRACE_BLOCK {
                let (x, y) = if rows { (i, block + k) } else { (block + k, i) };
                for (c, band) in bands.iter_mut().enumerate() {
                    band[i] +=
                        (pair.on.code(x, y, c) - pair.off.code(x, y, c)) / TRACE_BLOCK as f64;
                }
            }
        }
        let bands = bands.map(|b| boxcar(&b, TRACE_SMOOTH));
        let smooth: Vec<f64> = (0..along)
            .map(|i| bands[0][i] + bands[1][i] + bands[2][i])
            .collect();
        let sizes: Vec<f64> = (0..along)
            .map(|i| {
                (bands[0][i] * bands[0][i] + bands[1][i] * bands[1][i] + bands[2][i] * bands[2][i])
                    .sqrt()
            })
            .collect();
        let allowed: Vec<usize> = (TRACE_EDGE..along.saturating_sub(TRACE_EDGE))
            .filter(|i| match guide {
                None => true,
                Some((map, circle)) => {
                    let (x, y) = place(*i, block);
                    map.ray(x, y)
                        .is_some_and(|d| circle.distance(d).abs().to_degrees() <= TRACE_WINDOW)
                }
            })
            .collect();
        block += TRACE_BLOCK;
        if allowed.len() < 16 {
            continue;
        }
        // The best pair of lobes no further apart than the band can be. Both
        // ways round, because whichever extreme is global the other one has to
        // be looked for beside it and not across the whole picture.
        let mut best: Option<(usize, usize, f64)> = None;
        for anchor in [
            *allowed
                .iter()
                .max_by(|a, b| smooth[**a].total_cmp(&smooth[**b]))
                .unwrap_or(&0),
            *allowed
                .iter()
                .min_by(|a, b| smooth[**a].total_cmp(&smooth[**b]))
                .unwrap_or(&0),
        ] {
            let near: Vec<usize> = allowed
                .iter()
                .copied()
                .filter(|i| (*i as f64 - anchor as f64).abs() <= TRACE_SPAN)
                .collect();
            let (Some(top), Some(bottom)) = (
                near.iter()
                    .max_by(|a, b| smooth[**a].total_cmp(&smooth[**b])),
                near.iter()
                    .min_by(|a, b| smooth[**a].total_cmp(&smooth[**b])),
            ) else {
                continue;
            };
            let swing = smooth[*top] - smooth[*bottom];
            if best.is_none_or(|(_, _, s)| swing > s) {
                best = Some((*top, *bottom, swing));
            }
        }
        let Some((top, bottom, swing)) = best else {
            continue;
        };
        if swing < TRACE_SWING {
            continue;
        }
        let (from, to) = (top.min(bottom), top.max(bottom));
        let middle = match centre {
            Centre::Lobes => (top + bottom) as f64 / 2.0,
            // Per channel and weighted by each channel's own swing, and not
            // off their sum, because the hue the correction moves along TURNS
            // along the seam: on the owner's dirt render R and B move together
            // at one end and opposite each other at the other, so their sum
            // passes through nothing in the middle and its crossing is the
            // noise's. Weighted per channel the crossing survives the turn.
            Centre::Crossing => {
                let mut weight = 0.0;
                let mut total = 0.0;
                for band in &bands {
                    let (mut hi, mut lo) = (from, from);
                    for i in from..=to {
                        if band[i] > band[hi] {
                            hi = i;
                        }
                        if band[i] < band[lo] {
                            lo = i;
                        }
                    }
                    let swing = band[hi] - band[lo];
                    let (a, b) = (hi.min(lo), hi.max(lo));
                    for i in a..b {
                        if (band[i] <= 0.0) != (band[i + 1] <= 0.0) {
                            let rise = band[i + 1] - band[i];
                            let f = if rise.abs() < 1e-12 {
                                0.5
                            } else {
                                -band[i] / rise
                            };
                            total += swing * (i as f64 + f);
                            weight += swing;
                            break;
                        }
                    }
                }
                if weight <= 0.0 {
                    continue;
                }
                total / weight
            }
            Centre::Dark => {
                let size = &sizes;
                let mut best = from;
                for i in from..=to {
                    if size[i] < size[best] {
                        best = i;
                    }
                }
                // A parabola through the three samples round the smallest, so
                // the line is not quantized to whole pixels of a scan that is
                // already smoothed over forty of them.
                if best > 0 && best + 1 < size.len() {
                    let (a, b, c) = (size[best - 1], size[best], size[best + 1]);
                    let curve = a - 2.0 * b + c;
                    if curve > 1e-12 {
                        best as f64 + (a - c) / (2.0 * curve)
                    } else {
                        best as f64
                    }
                } else {
                    best as f64
                }
            }
        };
        let fixed = block as f64 - TRACE_BLOCK as f64 / 2.0;
        let (x, y) = if rows {
            (middle, fixed)
        } else {
            (fixed, middle)
        };
        marks.push(Mark { x, y, swing, rows });
    }
    marks
}

/// The seam: where it was seen, what projection makes those places a great
/// circle, and the circle itself.
struct Seam {
    map: Map,
    circle: Circle,
    marks: Vec<Mark>,
    kept: Vec<bool>,
    /// What the fit leaves, in degrees of arc and in picture pixels.
    residual_deg: f64,
    residual_px: f64,
    /// The same fit's residual at half and at twice the focal length it chose,
    /// which is what says whether the focal length is a measurement or a
    /// shrug.
    slack: (f64, f64),
    /// Every family's residual, so a reader can see the one that was picked
    /// beside the ones that were not.
    league: Vec<Shape>,
    /// Which definition of the line this one is.
    centre: Centre,
    /// The stretch of the circle that is inside the picture, in radians.
    visible: (f64, f64),
    /// What each of the three definitions of the line leaves once a great
    /// circle is fitted to it, which is how the used one was chosen.
    rivals: Vec<(Centre, f64, Circle)>,
}

impl Seam {
    /// The seam, in two passes: one that knows nothing and searches the whole
    /// of every scan line, and one that knows roughly where the line is and
    /// refuses everything more than [`TRACE_WINDOW`] degrees from it.
    fn find(pair: &Pair, options: &Options) -> Fallible<Self> {
        let mut rivals = Vec::new();
        let mut answer = None;
        for centre in [Centre::Lobes, Centre::Crossing, Centre::Dark] {
            let Ok(found) = Self::one(pair, options, centre) else {
                continue;
            };
            rivals.push((centre, found.residual_px, found.circle));
            let take = match options.centre {
                None => answer
                    .as_ref()
                    .is_none_or(|best: &Self| found.residual_px < best.residual_px),
                Some(wanted) => centre == wanted,
            };
            if take {
                answer = Some(found);
            }
        }
        let mut answer = answer.ok_or("nothing could be traced on this picture")?;
        answer.rivals = rivals;
        Ok(answer)
    }

    /// One definition of the line, in two passes: one that knows nothing and
    /// searches the whole of every scan line, and one that knows roughly where
    /// the line is and refuses everything more than [`TRACE_WINDOW`] degrees
    /// from it.
    fn one(pair: &Pair, options: &Options, centre: Centre) -> Fallible<Self> {
        let rough = Self::from_marks(pair, options, sweeps(pair, None, centre), centre)?;
        let guide = Some((&rough.map, &rough.circle));
        match Self::from_marks(pair, options, sweeps(pair, guide, centre), centre) {
            Ok(sharp) if sharp.residual_px <= rough.residual_px => Ok(sharp),
            _ => Ok(rough),
        }
    }

    fn from_marks(
        pair: &Pair,
        options: &Options,
        marks: Vec<Mark>,
        centre: Centre,
    ) -> Fallible<Self> {
        if marks.len() < 8 {
            return Err(format!(
                "only {} places on this picture swing by {TRACE_SWING} codes, which is not a seam",
                marks.len(),
            )
            .into());
        }
        let (width, height) = pair.size();
        let middle = (width as f64 / 2.0, height as f64 / 2.0);
        let mut league = Vec::new();
        let mut best: Option<Shape> = None;
        for model in Model::ALL {
            if let Some(shape) = sweep_focal(&marks, model, middle, width) {
                league.push(shape);
                if best.is_none_or(|b| shape.residual < b.residual) {
                    best = Some(shape);
                }
            }
        }
        let shape = match options.model {
            Some(wanted) => league
                .iter()
                .copied()
                .find(|s| s.model == wanted)
                .ok_or_else(|| format!("no {} fit converged", wanted.name()))?,
            None => best.ok_or("no projection family fits this seam at all")?,
        };
        let Shape {
            model, focal, bend, ..
        } = shape;
        let map = Map {
            model,
            focal,
            bend,
            centre: middle,
        };
        let fit = fit_circle(&marks, &map)
            .ok_or("the seam marks do not lie on a great circle under any focal length")?;
        let half = sweep_at(&marks, model, middle, focal * 0.5, bend).unwrap_or(f64::NAN);
        let twice = sweep_at(&marks, model, middle, focal * 2.0, bend).unwrap_or(f64::NAN);
        let visible = visible_arc(&map, &fit.circle, (width, height));
        Ok(Self {
            map,
            circle: fit.circle,
            marks,
            kept: fit.kept,
            residual_deg: fit.residual_rad.to_degrees(),
            residual_px: fit.residual_px,
            slack: (half, twice),
            league,
            centre,
            visible,
            rivals: Vec::new(),
        })
    }

    /// How many picture pixels one degree across the seam covers at `phi`,
    /// measured off the map rather than derived from a field of view that
    /// nothing in the file states.
    fn scale(&self, phi: f64) -> f64 {
        perpendicular_scale(&self.map, &self.circle, phi)
    }

    fn report(&self, pair: &Pair) {
        let (width, height) = pair.size();
        let (cols, rows) = self
            .marks
            .iter()
            .zip(&self.kept)
            .filter(|(_, keep)| **keep)
            .fold(
                (0, 0),
                |(c, r), (m, _)| {
                    if m.rows { (c, r + 1) } else { (c + 1, r) }
                },
            );
        println!(
            "\n== the seam, found from the difference and fitted as a great circle\n\n  \
             {} of {} trace points kept, {cols} from the column sweep and {rows} from the row \n\
             \tsweep. A point is the midpoint of the two lobes of the smoothed difference over a \n\
             \t{TRACE_BLOCK} pixel block, which is a statement about the band and NOT about where \n\
             \tit changes sign, so O1 below is a test and not a tautology.",
            self.kept.iter().filter(|k| **k).count(),
            self.marks.len(),
        );
        println!(
            "\n  the picture is {width}x{height}, which is 16:9. A whole equirectangular frame is \n\
             \t2:1, so THIS IS NOT AN EQUIRECTANGULAR EXPORT and no constant turns a column into \n\
             \ta degree of longitude. What follows fits the projection to the seam instead, under \n\
             \tthe one thing a seam is known to be, which is a great circle."
        );
        let swings: Vec<f64> = self
            .marks
            .iter()
            .zip(&self.kept)
            .filter(|(_, keep)| **keep)
            .map(|(m, _)| m.swing)
            .collect();
        println!(
            "  a kept point swings {:.1} codes at the weakest and {:.1} at the strongest, summed \n\
             \tover the three channels, against a block noise floor near a tenth of a code.",
            swings.iter().fold(f64::MAX, |a, b| a.min(*b)),
            swings.iter().fold(0.0f64, |a, b| a.max(*b)),
        );
        println!(
            "\n  three definitions of the line, each fitted with its own projection, scored by \n\
             \twhat a GREAT CIRCLE leaves on it. The seam between two lenses is a plane through \n\
             \tthe camera, so the definition that lies on a great circle is the one tracking the \n\
             \tseam and the others are tracking something about the correction's own shape.\n"
        );
        for (centre, residual, circle) in &self.rivals {
            // Where the two circles are apart ON THE PICTURE and not in the
            // angle between their normals: the visible arc can be a quarter of
            // a circle, and two circles that cross inside it are only degrees
            // apart in their normals while lying on top of each other over
            // every pixel anybody can see.
            let mut square = 0.0;
            let mut counted = 0.0;
            for i in 0..=200 {
                let phi = self.visible.0 + (self.visible.1 - self.visible.0) * i as f64 / 200.0;
                let away = circle.distance(self.circle.at(phi)).to_degrees();
                square += away * away;
                counted += 1.0;
            }
            let apart = (square / counted).sqrt();
            println!(
                "    {:>15}  great circle leaves {residual:>8.1} px rms, and runs {apart:>6.2} \
                 deg from the used one over the visible arc{}",
                centre.name(),
                if *centre == self.centre { "  <-" } else { "" },
            );
        }
        println!(
            "\n    {:>15} {:>12} {:>9} {:>15}",
            "family", "focal, px", "bend", "residual, px"
        );
        for shape in &self.league {
            println!(
                "    {:>15} {:>12.1} {:>9.4} {:>15.2}{}",
                shape.model.name(),
                shape.focal,
                shape.bend,
                shape.residual,
                if shape.model == self.map.model {
                    "  <- used"
                } else {
                    ""
                },
            );
        }
        println!(
            "\n  the fit leaves {:.2} picture pixels rms, which is {:.4} degrees of arc. At half \n\
             \tthe focal length it leaves {:.2} pixels and at twice it leaves {:.2}, so the focal \n\
             \tlength is measured and not a shrug.",
            self.residual_px, self.residual_deg, self.slack.0, self.slack.1,
        );
        println!(
            "\n  where the line was seen, every fourth point, and how far the fitted circle \n\
             \tpasses from it in pixels. `col` is the sweep it came from and `-` means the fit \n\
             \tdropped it."
        );
        for (n, (mark, keep)) in self.marks.iter().zip(&self.kept).enumerate() {
            if n % 4 != 0 {
                continue;
            }
            let away = self
                .map
                .ray(mark.x, mark.y)
                .and_then(|d| {
                    radian_in_pixels(&self.map, &self.circle, d)
                        .map(|s| self.circle.distance(d) * s)
                })
                .unwrap_or(f64::NAN);
            println!(
                "    {:>4},{:<4} swing {:>6.1} {:>4} off by {away:>8.1} px {}",
                mark.x.round(),
                mark.y.round(),
                mark.swing,
                if mark.rows { "row" } else { "col" },
                if *keep { "" } else { "-" },
            );
        }
        println!(
            "  the arc inside the picture runs {:.1} to {:.1} degrees of its own parameter, \n\
             \t{:.1} degrees of seam in all. One degree across the seam is {:.1} pixels at the \n\
             \tstart, {:.1} in the middle and {:.1} at the end, which is the reframe's own \n\
             \tstretch and is why a width in pixels is not a width.",
            self.visible.0.to_degrees(),
            self.visible.1.to_degrees(),
            (self.visible.1 - self.visible.0).to_degrees(),
            self.scale(self.visible.0),
            self.scale((self.visible.0 + self.visible.1) / 2.0),
            self.scale(self.visible.1),
        );
    }
}

/// Both sweeps' marks in one list.
///
/// A column sweep reads a nearly horizontal band badly and a row sweep reads a
/// nearly vertical one badly, and which is which is a property of the picture
/// rather than something to be decided in advance. Both are run, both are
/// thrown in, and the robust fit drops whichever ones do not belong.
fn sweeps(pair: &Pair, guide: Option<(&Map, &Circle)>, centre: Centre) -> Vec<Mark> {
    let mut marks = scan(pair, false, guide, centre);
    marks.extend(scan(pair, true, guide, centre));
    // A scan line whose swing is a small share of the strongest one on the
    // picture is a scan line the band barely crosses, and where it barely
    // crosses, the two lobes are wide apart and shallow and the feature between
    // them is somebody's soil. The absolute floor above says a mark is not
    // noise; this says it is not the seam.
    let strongest = marks.iter().fold(0.0f64, |a, m| a.max(m.swing));
    marks.retain(|m| m.swing >= TRACE_SHARE * strongest);
    marks
}

/// The best focal length for one family, by a coarse sweep and then a
/// bisection, scored by what the great circle fit leaves **in pixels**.
///
/// In pixels and not in degrees, and that is not a presentation choice: as the
/// focal length grows the whole picture subtends a smaller angle, so an angular
/// residual falls towards zero for every picture and every seam whether the
/// family is right or wrong. The first version of this scored degrees, chose an
/// 845000 pixel focal length for all five families, and reported a seam a third
/// of a degree long. A pixel is the same size at every focal length, which is
/// what makes it the only honest score here.
fn sweep_focal(marks: &[Mark], model: Model, centre: (f64, f64), width: usize) -> Option<Shape> {
    let (lo, hi) = (0.15 * width as f64, 20.0 * width as f64);
    let steps = 240;
    let mut best: Option<Shape> = None;
    for bend in model.bends() {
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let focal = lo * (hi / lo).powf(t);
            if let Some(residual) = sweep_at(marks, model, centre, focal, bend)
                && best.is_none_or(|s| residual < s.residual)
            {
                {
                    best = Some(Shape {
                        model,
                        focal,
                        bend,
                        residual,
                    });
                }
            }
        }
    }
    let mut best = best?;
    let mut span = best.focal * 0.25;
    let mut bend_span = if model.bends().len() > 1 { 0.01 } else { 0.0 };
    for _ in 0..48 {
        let mut moved = false;
        for (df, db) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let focal = best.focal + df * span;
            let bend = best.bend + db * bend_span;
            if focal <= 0.0 {
                continue;
            }
            if let Some(residual) = sweep_at(marks, model, centre, focal, bend)
                && residual < best.residual
            {
                best = Shape {
                    model,
                    focal,
                    bend,
                    residual,
                };
                moved = true;
            }
        }
        if !moved {
            span *= 0.5;
            bend_span *= 0.5;
        }
    }
    Some(best)
}

/// One family's best answer: its scale, its bend and what it leaves in pixels.
#[derive(Clone, Copy)]
struct Shape {
    model: Model,
    focal: f64,
    bend: f64,
    residual: f64,
}

/// What the great circle fit leaves at one focal length, in pixels rms.
fn sweep_at(
    marks: &[Mark],
    model: Model,
    centre: (f64, f64),
    focal: f64,
    bend: f64,
) -> Option<f64> {
    let map = Map {
        model,
        focal,
        bend,
        centre,
    };
    // Every pixel of an export is a ray somebody exported, so a projection that
    // has no picture at the corners of this frame is not the projection this
    // frame was written with. Without this a family wins by covering only the
    // middle: orthographic at a short focal length, and the bent cubic at the
    // bend where it folds over, both fit the handful of marks near the centre
    // beautifully and have nothing at all to say about the rest of the picture.
    for corner in [
        (0.0, 0.0),
        (2.0 * centre.0 - 1.0, 0.0),
        (0.0, 2.0 * centre.1 - 1.0),
        (2.0 * centre.0 - 1.0, 2.0 * centre.1 - 1.0),
    ] {
        map.ray(corner.0, corner.1)?;
    }
    fit_circle(marks, &map).map(|fit| fit.residual_px)
}

/// A fitted seam: the circle, which trace points it is fitted to, and what it
/// leaves in both units.
struct Fit {
    circle: Circle,
    kept: Vec<bool>,
    residual_rad: f64,
    residual_px: f64,
}

/// The great circle through the trace points, with the points it had to drop.
///
/// Three rounds of drop-and-refit, because the two sweeps disagree where the
/// seam runs nearly along the scan line: a column sweep reads a nearly
/// horizontal band badly and a row sweep reads a nearly vertical one badly, and
/// which is which is a property of the picture rather than something to be
/// decided in advance. Both are run, both are thrown in, and the fit drops
/// whichever ones do not belong.
fn fit_circle(marks: &[Mark], map: &Map) -> Option<Fit> {
    let rays: Vec<Option<[f64; 3]>> = marks.iter().map(|m| map.ray(m.x, m.y)).collect();
    let mut kept: Vec<bool> = rays.iter().map(Option::is_some).collect();
    // A family with no picture past some angle can otherwise win by having no
    // opinion about most of the frame: orthographic at a short focal length
    // covers the middle of the picture, drops every mark outside it, and fits
    // the handful that are left beautifully. A fit has to answer for nearly
    // every mark or it is not a fit of this seam.
    if kept.iter().filter(|k| **k).count() * 10 < marks.len() * 9 {
        return None;
    }
    let mut answer = None;
    for _ in 0..3 {
        let live: Vec<[f64; 3]> = rays
            .iter()
            .zip(&kept)
            .filter_map(|(r, keep)| if *keep { *r } else { None })
            .collect();
        let (normal, residual) = plane_of(&live)?;
        answer = Some((normal, residual));
        if residual < 1e-12 {
            break;
        }
        let mut next = kept.clone();
        let mut alive = 0;
        for ((slot, ray), keep) in next.iter_mut().zip(&rays).zip(&kept) {
            let Some(d) = ray else { continue };
            *slot = *keep && dot(normal, *d).abs() < 3.0 * residual;
            if *slot {
                alive += 1;
            }
        }
        if alive * 10 < live.len() * 6 {
            break;
        }
        kept = next;
    }
    let (normal, residual) = answer?;
    let circle = Circle::from_normal(normal);
    let mut square = 0.0;
    let mut counted = 0usize;
    for (ray, keep) in rays.iter().zip(&kept) {
        let (Some(d), true) = (ray, *keep) else {
            continue;
        };
        let Some(scale) = radian_in_pixels(map, &circle, *d) else {
            continue;
        };
        let away = dot(normal, *d).clamp(-1.0, 1.0).asin();
        square += (away * scale).powi(2);
        counted += 1;
    }
    if counted == 0 {
        return None;
    }
    Some(Fit {
        circle,
        kept,
        residual_rad: residual.asin(),
        residual_px: (square / counted as f64).sqrt(),
    })
}

/// How many picture pixels one radian across the circle is worth at a
/// direction, measured off the map by moving the direction and looking.
fn radian_in_pixels(map: &Map, circle: &Circle, d: [f64; 3]) -> Option<f64> {
    let step = 1e-3;
    let s = dot(circle.normal, d).clamp(-1.0, 1.0).asin();
    let plane = unit([
        d[0] - circle.normal[0] * s.sin(),
        d[1] - circle.normal[1] * s.sin(),
        d[2] - circle.normal[2] * s.sin(),
    ]);
    let walk = |at: f64| {
        let (sn, cs) = at.sin_cos();
        map.pixel([
            plane[0] * cs + circle.normal[0] * sn,
            plane[1] * cs + circle.normal[1] * sn,
            plane[2] * cs + circle.normal[2] * sn,
        ])
    };
    let (a, b) = (walk(s + step)?, walk(s - step)?);
    Some((a.0 - b.0).hypot(a.1 - b.1) / (2.0 * step))
}

/// How many picture pixels one degree across the seam covers at `phi`.
fn perpendicular_scale(map: &Map, circle: &Circle, phi: f64) -> f64 {
    let step = 0.002;
    let (Some(a), Some(b)) = (
        map.pixel(circle.off(phi, -step)),
        map.pixel(circle.off(phi, step)),
    ) else {
        return f64::NAN;
    };
    (a.0 - b.0).hypot(a.1 - b.1) / (2.0 * step).to_degrees()
}

/// The longest stretch of the circle that is inside the picture, in radians.
fn visible_arc(map: &Map, circle: &Circle, size: (usize, usize)) -> (f64, f64) {
    let steps = 3600;
    let inside: Vec<bool> = (0..steps)
        .map(|i| {
            let phi = std::f64::consts::TAU * i as f64 / steps as f64;
            map.pixel(circle.at(phi))
                .is_some_and(|(x, y)| in_frame(x, y, size, 4.0))
        })
        .collect();
    let (mut best, mut best_len) = ((0, 0), 0);
    let mut run_from = None;
    for i in 0..(steps * 2) {
        if inside[i % steps] {
            run_from.get_or_insert(i);
        } else if let Some(from) = run_from.take()
            && i - from > best_len
        {
            best_len = i - from;
            best = (from, i);
        }
    }
    if run_from.is_some() && best_len == 0 {
        return (0.0, std::f64::consts::TAU);
    }
    let per = std::f64::consts::TAU / steps as f64;
    (best.0 as f64 * per, best.1 as f64 * per)
}

fn in_frame(x: f64, y: f64, size: (usize, usize), margin: f64) -> bool {
    x >= margin && y >= margin && x < size.0 as f64 - margin && y < size.1 as f64 - margin
}

// ------------------------------------------------------------ the profiles

/// How few along-seam samples make a row of a profile not worth printing.
///
/// The whole noise argument here is the count: one pixel of a difference of two
/// JPEG encodes carries a couple of codes of noise and the smallest thing being
/// looked for is under two, so nothing single-pixel is evidence and every row
/// below is a mean of some hundreds.
const ALONG_MIN: usize = 40;

/// One azimuth's reading: both frames' own profiles across the seam, pooled
/// along it.
struct Profile {
    /// Where along the seam, in radians of the circle's own parameter.
    phi: f64,
    /// The pixel the line runs through there.
    at: (f64, f64),
    /// Picture pixels per degree across the seam at that place.
    scale: f64,
    /// The offsets the rows are read at, in degrees across the seam.
    s: Vec<f64>,
    off: Vec<[f64; 3]>,
    on: Vec<[f64; 3]>,
    count: Vec<usize>,
}

impl Profile {
    fn delta(&self, channel: usize) -> Vec<f64> {
        self.on
            .iter()
            .zip(&self.off)
            .map(|(a, b)| a[channel] - b[channel])
            .collect()
    }

    /// The OFF level either side of the line, and pooled: what every amplitude
    /// below is a percentage of. **Reported per side because the two sides of a
    /// seam are two lenses' pictures of different content**, and because dark
    /// content is judged relative.
    fn level(&self, channel: usize, span: (f64, f64)) -> Option<f64> {
        let mut sum = 0.0;
        let mut count = 0;
        for (i, s) in self.s.iter().enumerate() {
            if self.count[i] >= ALONG_MIN && *s >= span.0 && *s <= span.1 {
                sum += self.off[i][channel];
                count += 1;
            }
        }
        (count > 0).then(|| sum / count as f64)
    }

    /// Where the row nearest an offset is.
    fn index(&self, s: f64) -> Option<usize> {
        let step = self.s.get(1)? - self.s.first()?;
        let at = ((s - self.s[0]) / step).round();
        let at = if at < 0.0 { return None } else { at as usize };
        (at < self.s.len()).then_some(at)
    }

    /// A series read at an arbitrary offset, linearly between rows, or `None`
    /// where either row is missing.
    fn read(&self, v: &[f64], s: f64) -> Option<f64> {
        let step = self.s[1] - self.s[0];
        let t = (s - self.s[0]) / step;
        if t < 0.0 {
            return None;
        }
        let i = t as usize;
        if i + 1 >= v.len() || self.count[i] < ALONG_MIN || self.count[i + 1] < ALONG_MIN {
            return None;
        }
        let f = t - i as f64;
        Some(v[i] * (1.0 - f) + v[i + 1] * f)
    }
}

impl Seam {
    /// Every azimuth's profile, evenly spaced over the arc that is inside the
    /// picture.
    ///
    /// **Spaced in the circle's own parameter and not in pixels**, because the
    /// reframe's stretch is exactly the thing being divided out, and a spacing
    /// that used pixels would smuggle it back in.
    fn profiles(&self, pair: &Pair, options: &Options) -> Vec<Profile> {
        let margin = options.along.to_radians() / 2.0;
        let (lo, hi) = (self.visible.0 + margin, self.visible.1 - margin);
        if hi <= lo {
            return Vec::new();
        }
        (0..options.places)
            .map(|i| {
                let t = if options.places == 1 {
                    0.5
                } else {
                    i as f64 / (options.places - 1) as f64
                };
                self.profile(pair, options, lo + (hi - lo) * t)
            })
            .collect()
    }

    fn profile(&self, pair: &Pair, options: &Options, phi: f64) -> Profile {
        let step = options.step;
        let rows = (2.0 * options.reach / step).round() as usize + 1;
        let along = (options.along / options.step).round().max(1.0) as usize;
        let mut s = Vec::with_capacity(rows);
        let mut off = vec![[0.0; 3]; rows];
        let mut on = vec![[0.0; 3]; rows];
        let mut count = vec![0usize; rows];
        for i in 0..rows {
            s.push(-options.reach + step * i as f64);
        }
        for k in 0..=along {
            let t = (k as f64 / along as f64 - 0.5) * options.along;
            let at = phi + t.to_radians();
            for (i, s) in s.iter().enumerate() {
                let d = self.circle.off(at, s.to_radians());
                let Some((x, y)) = self.map.pixel(d) else {
                    continue;
                };
                let (Some(a), Some(b)) = (pair.off.at(x, y), pair.on.at(x, y)) else {
                    continue;
                };
                for c in 0..3 {
                    off[i][c] += a[c];
                    on[i][c] += b[c];
                }
                count[i] += 1;
            }
        }
        for i in 0..rows {
            if count[i] > 0 {
                for c in 0..3 {
                    off[i][c] /= count[i] as f64;
                    on[i][c] /= count[i] as f64;
                }
            }
        }
        let at = self
            .map
            .pixel(self.circle.at(phi))
            .unwrap_or((f64::NAN, f64::NAN));
        let scale = self.scale(phi);
        Profile {
            phi,
            at,
            scale,
            s,
            off,
            on,
            count,
        }
    }
}

// ------------------------------------------------------------ the shapes

/// The two lobes of one across-seam profile and the crossing between them.
#[derive(Clone, Copy)]
struct Lobes {
    high_at: f64,
    high: f64,
    low_at: f64,
    low: f64,
    /// Where the profile changes sign between the two, in degrees.
    crossing: f64,
}

impl Lobes {
    /// The peak to peak of the change, which is what O3 weighs against the
    /// disagreement it would have to be closing.
    fn swing(self) -> f64 {
        self.high - self.low
    }
}

/// The two extremes of a profile and the sign change between them, or `None`
/// where there is no sign change at all, which is what a one-sided field
/// would look like.
fn lobes(profile: &Profile, v: &[f64], window: f64) -> Option<Lobes> {
    let live: Vec<usize> = (0..v.len())
        .filter(|i| profile.count[*i] >= ALONG_MIN && profile.s[*i].abs() <= window)
        .collect();
    if live.len() < 8 {
        return None;
    }
    let high = *live.iter().max_by(|a, b| v[**a].total_cmp(&v[**b]))?;
    let low = *live.iter().min_by(|a, b| v[**a].total_cmp(&v[**b]))?;
    let (from, to) = (high.min(low), high.max(low));
    let middle = (from + to) as f64 / 2.0;
    let mut crossing: Option<(f64, f64)> = None;
    for i in from..to {
        if profile.count[i] < ALONG_MIN || profile.count[i + 1] < ALONG_MIN {
            continue;
        }
        if (v[i] <= 0.0 && v[i + 1] > 0.0) || (v[i] >= 0.0 && v[i + 1] < 0.0) {
            let f = if (v[i + 1] - v[i]).abs() < 1e-12 {
                0.5
            } else {
                -v[i] / (v[i + 1] - v[i])
            };
            let here = i as f64 + f;
            if crossing.is_none_or(|(best, _)| (here - middle).abs() < (best - middle).abs()) {
                crossing = Some((here, profile.s[i] + f * (profile.s[i + 1] - profile.s[i])));
            }
        }
    }
    let (_, crossing) = crossing?;
    Some(Lobes {
        high_at: profile.s[high],
        high: v[high],
        low_at: profile.s[low],
        low: v[low],
        crossing,
    })
}

/// What the profile's own mirror image about a point says about it.
#[derive(Clone, Copy)]
struct Symmetry {
    /// The energy in the part that survives mirroring, and in the part that
    /// changes sign under it.
    even: f64,
    odd: f64,
    /// The correlation of the profile against its own negated mirror. One is
    /// perfectly odd, minus one perfectly even, and a mean of noise is zero.
    correlation: f64,
    /// How far either side the two were compared over, in degrees.
    span: f64,
}

impl Symmetry {
    fn ratio(self) -> f64 {
        if self.odd > 0.0 {
            self.even / self.odd
        } else {
            f64::NAN
        }
    }
}

/// The odd and even halves of a profile about a point, and how well the
/// profile is its own negated mirror there.
///
/// **This is the test of the source-matching form**, because
/// `ON - OFF = d (2w - 1)` is odd about the even-mix line for any pull `d` that
/// is itself even, and a free additive field has no reason to be odd about
/// anything.
fn symmetry(profile: &Profile, v: &[f64], about: f64, span: f64) -> Option<Symmetry> {
    let step = profile.s[1] - profile.s[0];
    let mut even = 0.0;
    let mut odd = 0.0;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut u = step;
    while u <= span {
        if let (Some(a), Some(b)) = (profile.read(v, about + u), profile.read(v, about - u)) {
            even += ((a + b) / 2.0).powi(2);
            odd += ((a - b) / 2.0).powi(2);
            right.push(a);
            left.push(-b);
        }
        u += step;
    }
    if right.len() < 8 {
        return None;
    }
    let mean = |x: &[f64]| x.iter().sum::<f64>() / x.len() as f64;
    let (ma, mb) = (mean(&right), mean(&left));
    let mut num = 0.0;
    let (mut da, mut db) = (0.0, 0.0);
    for (a, b) in right.iter().zip(&left) {
        num += (a - ma) * (b - mb);
        da += (a - ma).powi(2);
        db += (b - mb).powi(2);
    }
    Some(Symmetry {
        even,
        odd,
        correlation: if da > 0.0 && db > 0.0 {
            num / (da * db).sqrt()
        } else {
            f64::NAN
        },
        span,
    })
}

/// One azimuth reduced to the one number per offset that carries its change:
/// the profile projected onto the direction the change actually moves in.
///
/// The chroma direction turns along the seam, from a green-magenta axis at one
/// end to a blue-amber one at the other, so no fixed channel is the right one
/// everywhere. The direction is taken from the profile's own odd part, which is
/// the part under test, and the sign is fixed so the positive lobe is positive.
fn projected(profile: &Profile, window: f64) -> Option<(Vec<f64>, [f64; 3])> {
    let deltas: Vec<Vec<f64>> = (0..3).map(|c| profile.delta(c)).collect();
    let mut m = [[0.0; 3]; 3];
    for (i, s) in profile.s.iter().enumerate() {
        if profile.count[i] < ALONG_MIN {
            continue;
        }
        let Some(mirror) = profile.index(-s) else {
            continue;
        };
        if profile.count[mirror] < ALONG_MIN {
            continue;
        }
        let odd: Vec<f64> = deltas.iter().map(|d| (d[i] - d[mirror]) / 2.0).collect();
        for a in 0..3 {
            for b in 0..3 {
                m[a][b] += odd[a] * odd[b];
            }
        }
    }
    let (values, vectors) = eigen3(m);
    let mut best = 0;
    for i in 1..3 {
        if values[i] > values[best] {
            best = i;
        }
    }
    let mut axis = unit([vectors[0][best], vectors[1][best], vectors[2][best]]);
    let mut w: Vec<f64> = (0..profile.s.len())
        .map(|i| (0..3).map(|c| axis[c] * deltas[c][i]).sum())
        .collect();
    let shape = lobes(profile, &w, window)?;
    if shape.high_at < shape.low_at {
        for v in &mut w {
            *v = -*v;
        }
        for a in &mut axis {
            *a = -*a;
        }
    }
    Some((w, axis))
}

/// How far a normalized profile reaches, by a rule stated rather than eyeballed.
///
/// `reach` is how far the picture let the profile be read, which is not a
/// detail: a 1/e distance printed as if it had been measured, when the profile
/// ran off the edge of the frame first, is a number invented by the edge of the
/// frame. Where the crossing is not reached inside `reach` the two distances
/// come back as NaN and `tail` says what was still left there.
#[derive(Clone, Copy)]
struct Falloff {
    /// Where the lobe peaks, in degrees from the line.
    peak_at: f64,
    peak: f64,
    /// Where the normalized profile first falls back under 1/e of its own peak,
    /// and under a half, walking outwards from the peak. Measured FROM THE LINE
    /// so that it is a half-width of the support and not a width of the flank.
    over_e: f64,
    over_half: f64,
    reach: f64,
    tail: f64,
}

/// One side's falloff, `side` being +1 outwards from the line or -1 inwards.
fn falloff(
    profile: &Profile,
    v: &[f64],
    about: f64,
    side: f64,
    span: f64,
    window: f64,
) -> Option<Falloff> {
    let step = profile.s[1] - profile.s[0];
    let mut peak = 0.0;
    let mut peak_at = 0.0;
    let mut reach = 0.0;
    let mut tail = 0.0;
    let mut u = step;
    while u <= span {
        let Some(a) = profile.read(v, about + side * u) else {
            break;
        };
        reach = u;
        tail = a * side;
        if a * side > peak && u <= window {
            peak = a * side;
            peak_at = u;
        }
        u += step;
    }
    if peak <= 0.0 {
        return None;
    }
    let cross = |fraction: f64| {
        let mut u = peak_at;
        while u <= reach {
            match profile.read(v, about + side * u) {
                Some(a) if a * side < fraction * peak => return u,
                None => return f64::NAN,
                _ => {}
            }
            u += step;
        }
        f64::NAN
    };
    Some(Falloff {
        peak_at,
        peak,
        over_e: cross(std::f64::consts::E.recip()),
        over_half: cross(0.5),
        reach,
        tail: tail / peak,
    })
}

// ------------------------------------------------------------ the control

/// How big a far-field block is, in pixels a side.
const BLOCK: usize = 120;

/// The far field, which is what says the two frames differ by one toggle, and
/// the pair's own JPEG noise, which is what says which of the differences near
/// the seam are real.
///
/// **Both halves are the control and neither is the finding.** Away from the
/// seam Studio's toggle has to change nothing, and if it changes something then
/// the two exports differ by more than the toggle and every other number here
/// is measuring two things at once. The blocks are chosen by their angle from
/// the fitted seam rather than by eye, and they are bucketed by how bright they
/// are, because a gain of a thousandth is a code on 190-code sky and nothing at
/// all on 20-code soil, and pooling the two ends is the mistake this project
/// has made in both directions.
fn control(pair: &Pair, seam: &Seam, options: &Options) {
    let (width, height) = pair.size();
    println!(
        "\n== the control: one toggle, and what a pair of JPEG encodes is worth\n\n  \
         far-field blocks are {BLOCK}x{BLOCK} px whose centre is more than {:.0} degrees from the \n\
         \tfitted seam, bucketed by their own OFF brightness. Studio has to change NOTHING here.",
        options.far,
    );
    let mut blocks: Vec<([f64; 3], [f64; 3])> = Vec::new();
    let mut y = 0;
    while y + BLOCK <= height {
        let mut x = 0;
        while x + BLOCK <= width {
            let cx = (x + BLOCK / 2) as f64;
            let cy = (y + BLOCK / 2) as f64;
            let far = seam
                .map
                .ray(cx, cy)
                .map(|d| seam.circle.distance(d).abs().to_degrees() >= options.far);
            if far == Some(true) {
                blocks.push(block_means(pair, x, y));
            }
            x += BLOCK;
        }
        y += BLOCK;
    }
    if blocks.is_empty() {
        println!("  no block on this picture is that far from the seam.");
    } else {
        blocks.sort_by(|a, b| luma(a.0).total_cmp(&luma(b.0)));
        println!(
            "\n    {:>10} {:>7} {:>22} {:>26} {:>22}",
            "bucket", "blocks", "OFF level R/G/B", "ON over OFF", "difference, codes"
        );
        let buckets = 4;
        for b in 0..buckets {
            let from = blocks.len() * b / buckets;
            let to = blocks.len() * (b + 1) / buckets;
            if to <= from {
                continue;
            }
            let slice = &blocks[from..to];
            let mut off = [0.0; 3];
            let mut on = [0.0; 3];
            for (a, b) in slice {
                for c in 0..3 {
                    off[c] += a[c];
                    on[c] += b[c];
                }
            }
            for c in 0..3 {
                off[c] /= slice.len() as f64;
                on[c] /= slice.len() as f64;
            }
            println!(
                "    {:>10} {:>7} {:>22} {:>26} {:>22}",
                match b {
                    0 => "darkest",
                    1 => "dark",
                    2 => "bright",
                    _ => "brightest",
                },
                slice.len(),
                format!("{:.1} / {:.1} / {:.1}", off[0], off[1], off[2]),
                format!(
                    "{:.4} / {:.4} / {:.4}",
                    on[0] / off[0],
                    on[1] / off[1],
                    on[2] / off[2]
                ),
                format!(
                    "{:+.3} / {:+.3} / {:+.3}",
                    on[0] - off[0],
                    on[1] - off[1],
                    on[2] - off[2]
                ),
            );
        }
        let worst = blocks
            .iter()
            .map(|(a, b)| (0..3).map(|c| (b[c] - a[c]).abs()).fold(0.0f64, f64::max))
            .fold(0.0f64, f64::max);
        println!(
            "  the WORST single far-field block moves by {worst:.2} codes in its worst channel, \n\
             \tover {} blocks. That is the whole of what the toggle does away from the seam.",
            blocks.len(),
        );
    }
    for (x, y, w, h) in &options.blocks {
        let (off, on) = block_at(pair, *x, *y, *w, *h);
        println!(
            "\n  the named block {x},{y} {w}x{h}: OFF {:.1} / {:.1} / {:.1}, \
             ON over OFF {:.4} / {:.4} / {:.4}, difference {:+.3} / {:+.3} / {:+.3}",
            off[0],
            off[1],
            off[2],
            on[0] / off[0],
            on[1] / off[1],
            on[2] / off[2],
            on[0] - off[0],
            on[1] - off[1],
            on[2] - off[2],
        );
    }
    noise(pair, seam, options);
}

fn luma(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn block_means(pair: &Pair, x: usize, y: usize) -> ([f64; 3], [f64; 3]) {
    block_at(pair, x, y, BLOCK, BLOCK)
}

fn block_at(pair: &Pair, x: usize, y: usize, w: usize, h: usize) -> ([f64; 3], [f64; 3]) {
    let mut off = [0.0; 3];
    let mut on = [0.0; 3];
    let w = w.min(pair.off.width.saturating_sub(x));
    let h = h.min(pair.off.height.saturating_sub(y));
    for j in y..(y + h) {
        for i in x..(x + w) {
            for c in 0..3 {
                off[c] += pair.off.code(i, j, c);
                on[c] += pair.on.code(i, j, c);
            }
        }
    }
    let n = (w * h).max(1) as f64;
    for c in 0..3 {
        off[c] /= n;
        on[c] /= n;
    }
    (off, on)
}

/// What a pair of independent JPEG encodes of the same picture is worth, and
/// therefore what a feature has to clear before it is a feature.
///
/// Reported over the whole frame, which is the figure the memo already carries
/// for the first pair and is therefore the cross-check, and over the far field
/// alone, which is the honest one because the whole frame includes the
/// correction. The last line is the number every profile in this instrument is
/// judged against: the same per-pixel noise after the along-seam pooling a
/// profile row is a mean over.
fn noise(pair: &Pair, seam: &Seam, options: &Options) {
    let (width, height) = pair.size();
    let mut same = 0usize;
    let mut all = 0usize;
    let mut sum = [0.0; 3];
    let mut square = [0.0; 3];
    let mut far_sum = [0.0; 3];
    let mut far_square = [0.0; 3];
    let mut far = 0usize;
    for y in 0..height {
        for x in 0..width {
            let mut identical = true;
            let mut d = [0.0; 3];
            for (c, held) in d.iter_mut().enumerate() {
                *held = pair.on.code(x, y, c) - pair.off.code(x, y, c);
                identical &= *held == 0.0;
            }
            all += 1;
            same += usize::from(identical);
            for (c, held) in d.iter().enumerate() {
                sum[c] += held;
                square[c] += held * held;
            }
            let outside = seam
                .map
                .ray(x as f64, y as f64)
                .is_some_and(|r| seam.circle.distance(r).abs().to_degrees() >= options.far);
            if outside {
                far += 1;
                for (c, held) in d.iter().enumerate() {
                    far_sum[c] += held;
                    far_square[c] += held * held;
                }
            }
        }
    }
    let stats = |sum: [f64; 3], square: [f64; 3], n: usize| {
        let n = n.max(1) as f64;
        let mut mean = [0.0; 3];
        let mut sd = [0.0; 3];
        for c in 0..3 {
            mean[c] = sum[c] / n;
            sd[c] = (square[c] / n - mean[c] * mean[c]).max(0.0).sqrt();
        }
        (mean, sd)
    };
    let (mean, sd) = stats(sum, square, all);
    let (far_mean, far_sd) = stats(far_sum, far_square, far);
    println!(
        "\n  the pair's own noise. {:.1} percent of pixels are bit-identical between the two \n\
         \tfiles; the per-pixel difference has mean {:+.3} / {:+.3} / {:+.3} codes and standard \n\
         \tdeviation {:.2} / {:.2} / {:.2} over the whole frame, and mean {:+.3} / {:+.3} / {:+.3} \n\
         \twith standard deviation {:.2} / {:.2} / {:.2} over the far field alone, which is the \n\
         \tone with no correction in it.",
        100.0 * same as f64 / all as f64,
        mean[0],
        mean[1],
        mean[2],
        sd[0],
        sd[1],
        sd[2],
        far_mean[0],
        far_mean[1],
        far_mean[2],
        far_sd[0],
        far_sd[1],
        far_sd[2],
    );
}

// ------------------------------------------------------------ the questions

/// One azimuth reduced to everything the three questions ask of it, so that no
/// two of them can be reading a different lobe of the same profile.
struct Reading<'a> {
    profile: &'a Profile,
    /// The profile projected onto the direction its own change moves in.
    w: Vec<f64>,
    axis: [f64; 3],
    shape: Lobes,
    /// The falloff outwards from the line, and back the other way.
    out: Falloff,
    back: Falloff,
}

impl Reading<'_> {
    fn read<'a>(profile: &'a Profile, options: &Options) -> Option<Reading<'a>> {
        let (w, axis) = projected(profile, options.window)?;
        let shape = lobes(profile, &w, options.window)?;
        let span = profile.s[profile.s.len() - 1] - shape.crossing.abs();
        let out = falloff(profile, &w, shape.crossing, 1.0, span, options.window)?;
        let back = falloff(profile, &w, shape.crossing, -1.0, span, options.window)?;
        Some(Reading {
            profile,
            w,
            axis,
            shape,
            out,
            back,
        })
    }

    /// The peak of the change, as the mean of the two lobes.
    fn peak(&self) -> f64 {
        (self.out.peak + self.back.peak) / 2.0
    }

    fn peak_at(&self) -> f64 {
        (self.out.peak_at + self.back.peak_at) / 2.0
    }

    fn over_e(&self) -> f64 {
        (self.out.over_e + self.back.over_e) / 2.0
    }

    fn over_half(&self) -> f64 {
        (self.out.over_half + self.back.over_half) / 2.0
    }

    /// Whether this azimuth's change is big enough against the instrument's own
    /// floor to be worth a verdict. The floor is the decoy's, measured on the
    /// same pair with the same machinery a quarter turn away.
    fn clears(&self, floor: f64) -> bool {
        self.peak() >= CLEARS * floor
    }

    /// The odd part of the projected profile, divided by its own peak, on a
    /// grid of offsets from the line in degrees.
    fn normalized(&self) -> Vec<f64> {
        let peak = self.peak();
        (0..CURVE)
            .map(|i| {
                let u = i as f64 * CURVE_STEP;
                match (
                    self.profile.read(&self.w, self.shape.crossing + u),
                    self.profile.read(&self.w, self.shape.crossing - u),
                ) {
                    (Some(a), Some(b)) => (a - b) / 2.0 / peak,
                    _ => f64::NAN,
                }
            })
            .collect()
    }
}

/// How many times the floor an azimuth's own change has to be before it is
/// given a verdict rather than a row.
///
/// Four, which is the same shape of rule the band's own trust gate is: what is
/// being guarded against is not a wrong answer but a confident one, and the
/// azimuth this cuts on the owner's dirt render is the one whose profile runs
/// straight through the pilot's own body.
const CLEARS: f64 = 4.0;

/// The grid the normalized profiles are compared on: how far out, and how
/// finely.
const CURVE: usize = 601;
const CURVE_STEP: f64 = 0.05;

// ------------------------------------------------------------ O1

/// Is the difference odd about the seam line, with its zero crossing on it, and
/// is the size of it a MINIMUM there rather than a maximum.
///
/// The three together are the signature of a correction that matches two
/// sources to each other under a crossfade. Any one of them alone is weak: a
/// smooth field that happens to change sign somewhere has a crossing, and a
/// noisy profile mirrored about its own best point has some odd energy. Taken
/// together with the line fixed BEFORE the crossing is looked for, and fixed as
/// a GREAT CIRCLE, which is two numbers for the whole picture rather than one
/// per azimuth, they are a test the free additive field fails.
fn question_one(readings: &[Reading], seam: &Seam, floor: f64, options: &Options) {
    println!(
        "\n== O1: is the difference odd about the line, and is the line dark\n\n  \
         the line is a great circle fitted to the {rule}, over the whole picture at once, and \n\
         \tit is whichever of the three definitions a great circle fits best. Its two free \n\
         \tnumbers are placed \n\
         \tby {marks} trace points that then scatter {residual:.1} pixels about it, so it is not \n\
         \tfollowing any one azimuth and a zero crossing landing on it is a finding rather than \n\
         \ta definition. {read} of {asked} azimuths gave a profile at all.\n\
         \t`even/odd` is the ratio of the two energies about the crossing and `mirror` is the \n\
         \tcorrelation of the profile against its own negated mirror, 1 for perfectly odd and -1 \n\
         \tfor perfectly even. `proj` is the three channels projected onto the direction the \n\
         \tchange actually moves in at that azimuth, which turns along the seam.",
        rule = seam.centre.name(),
        marks = seam.kept.iter().filter(|k| **k).count(),
        residual = seam.residual_px,
        read = readings.len(),
        asked = options.places,
    );
    println!(
        "\n    {:>9} {:>14} {:>6} {:>10} {:>10} {:>9} {:>8} {:>8} {:>7}",
        "azimuth", "pixel", "chan", "cross deg", "cross px", "even/odd", "mirror", "swing", "over"
    );
    for reading in readings {
        let profile = reading.profile;
        let series: Vec<(&str, Vec<f64>)> = vec![
            ("R", profile.delta(0)),
            ("G", profile.delta(1)),
            ("B", profile.delta(2)),
            ("proj", reading.w.clone()),
        ];
        for (name, v) in &series {
            let Some(shape) = lobes(profile, v, options.window) else {
                continue;
            };
            let span = reading
                .out
                .reach
                .min(reading.back.reach)
                .min(2.0 * reading.over_half().max(reading.peak_at()));
            let Some(sym) = symmetry(profile, v, shape.crossing, span) else {
                continue;
            };
            println!(
                "    {:>9.1} {:>14} {:>6} {:>10.3} {:>10.1} {:>9.3} {:>8.3} {:>8.2} {:>7.1}{}",
                profile.phi.to_degrees(),
                format!("{:.0},{:.0}", profile.at.0, profile.at.1),
                name,
                shape.crossing,
                shape.crossing * profile.scale,
                sym.ratio(),
                sym.correlation,
                shape.swing(),
                sym.span,
                if reading.clears(floor) {
                    ""
                } else {
                    "  under floor"
                },
            );
        }
    }
    println!(
        "\n  and the dark line itself: the SIZE of the change, as the length of the \n\
         \tthree-channel difference vector, at the line against at the two lobes. A source \n\
         \tmatching correction cancels on the line and is a MINIMUM there; a field free to paint \n\
         \tanything has no reason to be."
    );
    println!(
        "\n    {:>9} {:>12} {:>12} {:>12} {:>9} {:>16} {:>12}",
        "azimuth",
        "|d| at line",
        "|d| lobe +",
        "|d| lobe -",
        "ratio",
        "rises both ways",
        "clears floor"
    );
    for reading in readings {
        let profile = reading.profile;
        let size: Vec<f64> = (0..profile.s.len())
            .map(|i| {
                (0..3)
                    .map(|c| (profile.on[i][c] - profile.off[i][c]).powi(2))
                    .sum::<f64>()
                    .sqrt()
            })
            .collect();
        let at = |s: f64| profile.read(&size, s).unwrap_or(f64::NAN);
        let here = at(reading.shape.crossing);
        let (high, low) = (at(reading.shape.high_at), at(reading.shape.low_at));
        // Read at a stated distance rather than over every sample in between:
        // the profile is a mean of a few hundred pixels and still has a tenth
        // of a code of wobble in it, and a rule that every single sample be
        // higher is a rule that reports the wobble instead of the shape.
        let mut minimum = true;
        for side in [-1.0, 1.0] {
            for step in [0.25, 0.5, 1.0] {
                if let Some(v) = profile.read(&size, reading.shape.crossing + side * step) {
                    minimum &= v > here;
                }
            }
        }
        println!(
            "    {:>9.1} {here:>12.3} {high:>12.3} {low:>12.3} {:>9.3} {:>16} {:>12}",
            profile.phi.to_degrees(),
            here / high.max(low),
            if minimum { "yes" } else { "no" },
            if reading.clears(floor) { "yes" } else { "no" },
        );
    }
}

// ------------------------------------------------------------ O2

/// Does the falloff collapse onto one curve once every azimuth is divided by
/// its own peak.
///
/// **This is the discriminator and it decides the spreading rule.** If the
/// normalized profiles lie on top of each other, Studio has one spatial kernel
/// and only the amplitude varies, which is a fixed shape times a varying number
/// and is a thing an estimator can copy. If instead the decay length itself
/// moves with the content, the support is adaptive and nothing with a fixed
/// kernel reproduces it.
///
/// **The axis the collapse is judged on is the whole of the question**, and it
/// is why this instrument fits a projection at all. The export is a reframe, so
/// one degree of the camera's own angle is 12 picture pixels in the middle of
/// the owner's render and 40 near the edge. A kernel that is one width in the
/// camera is three widths in pixels, and a run that measured pixels would
/// report an adaptive support and be wrong. So the spread is reported on all
/// three axes and the reader can see which one it collapses on.
fn question_two(readings: &[Reading], floor: f64) {
    println!(
        "\n== O2: the falloff, each azimuth normalized by its own peak\n\n  \
         `1/e` and `half` are how far from the LINE the normalized profile has fallen back under \n\
         \tthose fractions of its own peak, walking outwards, averaged over the two sides. \n\
         \t`reach` is how far the picture let it be read and `tail` is what was still left there, \n\
         \tso a NaN is the frame's edge and not a measurement. The level is OFF's own, per side, \n\
         \tbecause dark content is judged relative and the two ends of this seam are not one \n\
         \tpopulation."
    );
    println!(
        "\n    {:>9} {:>13} {:>19} {:>16} {:>19} {:>19} {:>7} {:>7} {:>7} {:>6} {:>6} {:>6}",
        "azimuth",
        "pixel",
        "peak codes R/G/B",
        "level R/G/B",
        "percent of level",
        "hue axis R/G/B",
        "px/deg",
        "peak d",
        "1/e d",
        "1/e px",
        "reach",
        "tail"
    );
    let mut kept: Vec<&Reading> = Vec::new();
    for reading in readings {
        let profile = reading.profile;
        let mut codes = [0.0; 3];
        let mut level = [0.0; 3];
        let mut percent = [0.0; 3];
        for (c, held) in codes.iter_mut().enumerate() {
            let d = profile.delta(c);
            let hi = profile
                .read(&d, reading.shape.crossing + reading.out.peak_at)
                .unwrap_or(0.0);
            let lo = profile
                .read(&d, reading.shape.crossing - reading.back.peak_at)
                .unwrap_or(0.0);
            *held = (hi.abs() + lo.abs()) / 2.0;
            let a = profile
                .level(
                    c,
                    (
                        reading.shape.crossing,
                        reading.shape.crossing + reading.out.peak_at,
                    ),
                )
                .unwrap_or(f64::NAN);
            let b = profile
                .level(
                    c,
                    (
                        reading.shape.crossing - reading.back.peak_at,
                        reading.shape.crossing,
                    ),
                )
                .unwrap_or(f64::NAN);
            level[c] = (a + b) / 2.0;
            percent[c] = 100.0 * *held / level[c];
        }
        println!(
            "    {:>9.1} {:>13} {:>19} {:>16} {:>19} {:>19} {:>7.1} {:>7.2} {:>7.2} {:>6.0} {:>6.1} {:>6.2}{}",
            profile.phi.to_degrees(),
            format!("{:.0},{:.0}", profile.at.0, profile.at.1),
            format!("{:.2} / {:.2} / {:.2}", codes[0], codes[1], codes[2]),
            format!("{:.0} / {:.0} / {:.0}", level[0], level[1], level[2]),
            format!("{:.1} / {:.1} / {:.1}", percent[0], percent[1], percent[2]),
            format!(
                "{:+.2} /{:+.2} /{:+.2}",
                reading.axis[0], reading.axis[1], reading.axis[2]
            ),
            profile.scale,
            reading.peak_at(),
            reading.over_e(),
            reading.over_e() * profile.scale,
            reading.out.reach.min(reading.back.reach),
            (reading.out.tail + reading.back.tail) / 2.0,
            if reading.clears(floor) {
                ""
            } else {
                "  under floor"
            },
        );
        if reading.clears(floor) {
            kept.push(reading);
        }
    }
    if kept.len() < 3 {
        println!(
            "  {} azimuths clear the floor, which is too few for a collapse test.",
            kept.len()
        );
        return;
    }
    let lengths: Vec<f64> = kept
        .iter()
        .map(|r| r.over_e())
        .filter(|l| l.is_finite())
        .collect();
    let halves: Vec<f64> = kept
        .iter()
        .map(|r| r.over_half())
        .filter(|l| l.is_finite())
        .collect();
    let scales: Vec<f64> = kept.iter().map(|r| r.profile.scale).collect();
    let curves: Vec<(Vec<f64>, f64, f64)> = kept
        .iter()
        .map(|r| (r.normalized(), r.over_half(), r.profile.scale))
        .collect();
    let widest = halves.iter().fold(0.0f64, |a, b| a.max(*b));
    let degrees = curve_spread(&curves, Axis::Degrees, 2.0 * widest);
    let pixels = curve_spread(&curves, Axis::Pixels, 2.0 * widest * mean(&scales));
    let own = curve_spread(&curves, Axis::Own, 2.0);
    println!("\n  over the {} azimuths that clear the floor:", kept.len());
    report_spread("the 1/e half-width, degrees", &lengths);
    report_spread("the half-max half-width, degrees", &halves);
    let in_pixels: Vec<f64> = kept
        .iter()
        .filter(|r| r.over_half().is_finite())
        .map(|r| r.over_half() * r.profile.scale)
        .collect();
    report_spread("the same half-max width, PIXELS", &in_pixels);
    println!(
        "\n  the normalized profiles' rms distance from their own mean, laid on three axes:\n\
         \t{degrees:.4} on the camera's own DEGREES\n\
         \t{pixels:.4} on picture PIXELS\n\
         \t{own:.4} on an axis each has divided by its OWN width\n\
         \tThe first two are the question: degrees is Studio's own working space and pixels is \n\
         \tthis reframe's. The third is the floor of the comparison, because a curve divided by \n\
         \tits own width cannot disagree about width."
    );
    let verdict = if degrees < pixels * 0.7 && degrees < 0.10 {
        "the profiles COLLAPSE on the camera's own angle and do NOT on picture pixels. One \n\
         \tfixed ANGULAR kernel, amplitude set by the local split. The pixel widths vary \n\
         \tbecause the reframe's scale varies, not because Studio's support does."
    } else if degrees < 0.10 {
        "the profiles COLLAPSE. One fixed kernel, amplitude set by the local split."
    } else if own < degrees * 0.6 {
        "the profiles do NOT collapse on either axis but DO once each is divided by its own \n\
         \twidth, so the shape is one shape and the WIDTH is what varies. The support is \n\
         \tadaptive."
    } else {
        "the profiles neither collapse nor collapse after rescaling, so neither the amplitude \n\
         \tnor the width alone describes what Studio does here."
    };
    println!("\n  VERDICT: {verdict}");
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn report_spread(what: &str, v: &[f64]) {
    if v.is_empty() {
        println!("    {what:>34}: nothing readable");
        return;
    }
    let m = mean(v);
    let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let lo = v.iter().fold(f64::MAX, |a, b| a.min(*b));
    let hi = v.iter().fold(f64::MIN, |a, b| a.max(*b));
    println!(
        "    {what:>34}: {lo:.2} to {hi:.2} over {} azimuths, mean {m:.2}, sd {sd:.2}, \
         which is {:.0} percent of the mean and a factor of {:.2} end to end",
        v.len(),
        100.0 * sd / m,
        hi / lo,
    );
}

/// Which axis a set of normalized profiles is compared on.
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    /// Degrees of the camera's own angle across the seam, which is the space
    /// Studio's own correction was applied in.
    Degrees,
    /// Picture pixels of this reframe, which is the space an instrument that
    /// did not fit a projection would have had to use.
    Pixels,
    /// Each profile's own width, which cannot disagree about width and is
    /// therefore the floor of the comparison.
    Own,
}

/// How far a set of normalized profiles are from their own mean on one axis.
fn curve_spread(curves: &[(Vec<f64>, f64, f64)], axis: Axis, span: f64) -> f64 {
    let samples = 160;
    let mut total = 0.0;
    let mut counted = 0usize;
    for k in 0..samples {
        let t = k as f64 / samples as f64;
        let mut values = Vec::new();
        for (curve, width, scale) in curves {
            let degrees = match axis {
                Axis::Degrees => t * span,
                Axis::Pixels => t * span / scale,
                Axis::Own => t * span * width,
            };
            let at = degrees / CURVE_STEP;
            let i = at as usize;
            if i + 1 >= curve.len() {
                continue;
            }
            let f = at - i as f64;
            let v = curve[i] * (1.0 - f) + curve[i + 1] * f;
            if v.is_finite() {
                values.push(v);
            }
        }
        if values.len() < 3 {
            continue;
        }
        let m = mean(&values);
        total += values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / values.len() as f64;
        counted += 1;
    }
    if counted == 0 {
        return f64::NAN;
    }
    (total / counted as f64).sqrt()
}

// ------------------------------------------------------------ O3

/// Is the correction bounded by the disagreement it would have to be closing.
///
/// **What this can settle.** A source-matching pull adds `+d` to one lens and
/// `-d` to the other, so whatever the picture does either side of the line, the
/// two lobes of the change have to be the same size. That is a property of the
/// change alone, it needs nothing from either lens's own picture, and a field
/// free to paint anything has no reason to have it. So the first two columns
/// are the test that a stitched output really does support.
///
/// **What it cannot settle, and this is the larger half.** The disagreement
/// `l1 - l0` is not in the file. Both frames are already blended, so what is
/// left of the disagreement at the line is whatever the crossfade did not
/// absorb, and on this content that is not separable from the scene: the
/// support measured in O2 is a dozen degrees wide, and a trend fitted outside
/// twelve degrees of a wide reframe is fitted across a horizon. The step
/// columns below are printed WITH the scene slope each was fitted through, and
/// where that slope is large the step is the scene and not the seam. A ratio of
/// change to step is therefore reported and is not a measurement of the share
/// of the disagreement being closed. **This instrument cannot bound the
/// correction against the disagreement from a stitched output, and says so
/// rather than printing a number that looks like it did.**
fn question_three(readings: &[Reading], floor: f64) {
    println!(
        "\n== O3: is the change bounded by the disagreement it would be closing\n\n  \
         the first test is the one a stitched output supports: the two lobes of a source \n\
         \tmatching change are +d and -d and have to be the same size. `lobe +` and `lobe -` are \n\
         \ttheir peaks in codes and `split` is the smaller over the larger, which is 1.00 for a \n\
         \tsymmetric split.\n\
         \tthe second is the one it does not: `step OFF` is each side's own trend extrapolated \n\
         \tto the line and subtracted, fitted just outside the change's own peak, and `slope` is \n\
         \thow steep the scene itself was over that band, in codes per degree. Where the slope \n\
         \tis large the step is the scene. The disagreement between the two lenses is NOT in a \n\
         \tblended file and nothing here recovers it."
    );
    println!(
        "\n    {:>9} {:>6} {:>9} {:>9} {:>7} {:>9} {:>9} {:>9} {:>12}",
        "azimuth",
        "chan",
        "lobe +",
        "lobe -",
        "split",
        "step OFF",
        "step ON",
        "slope",
        "ON step down"
    );
    let mut splits = Vec::new();
    let mut whole = Vec::new();
    for reading in readings {
        if !reading.clears(floor) {
            continue;
        }
        whole.push(
            reading.out.peak.min(reading.back.peak) / reading.out.peak.max(reading.back.peak),
        );
        let profile = reading.profile;
        // Just outside the peak and no further: the change is a dozen degrees
        // wide, so there is no band that is both outside its support and still
        // one piece of scene, and the nearer band is the lesser of the two
        // wrongs.
        let far = (
            reading.peak_at() * 1.2,
            (reading.peak_at() * 3.0).min(reading.out.reach.min(reading.back.reach) * 0.95),
        );
        for c in 0..3 {
            let d = profile.delta(c);
            let high = profile
                .read(&d, reading.shape.crossing + reading.out.peak_at)
                .unwrap_or(f64::NAN);
            let low = profile
                .read(&d, reading.shape.crossing - reading.back.peak_at)
                .unwrap_or(f64::NAN);
            let split = high.abs().min(low.abs()) / high.abs().max(low.abs());
            let off: Vec<f64> = profile.off.iter().map(|v| v[c]).collect();
            let on: Vec<f64> = profile.on.iter().map(|v| v[c]).collect();
            let (Some(step_off), Some(step_on), Some(slope)) = (
                step_across(profile, &off, reading.shape.crossing, far),
                step_across(profile, &on, reading.shape.crossing, far),
                scene_slope(profile, &off, reading.shape.crossing, far),
            ) else {
                continue;
            };
            // A channel whose own lobes are inside the floor has no ratio
            // worth taking: at the sky end of the owner's dirt render green
            // moves by a code against a floor of eight tenths of one.
            if split.is_finite() && high.abs().min(low.abs()) >= 2.0 * floor {
                splits.push(split);
            }
            println!(
                "    {:>9.1} {:>6} {high:>9.2} {low:>9.2} {split:>7.2} {step_off:>9.2} \
                 {step_on:>9.2} {slope:>9.1} {:>12}",
                profile.phi.to_degrees(),
                ["R", "G", "B"][c],
                if step_on.abs() < step_off.abs() {
                    "yes"
                } else {
                    "no"
                },
            );
        }
    }
    if splits.is_empty() {
        println!("  no azimuth clears the floor, so there is nothing to weigh.");
        return;
    }
    let worst = splits.iter().fold(f64::MAX, |a, b| a.min(*b));
    println!(
        "\n  on the projected profile, which is the best measured thing at each azimuth, the \n\
         \tsplit is {:.2} at its worst and {:.2} on average over {} azimuths.\n\
         \tper channel, over the readings whose own lobes clear twice the floor, it is {:.2} at \n\
         \tits worst and {:.2} on average over {} readings, against 1.00 for a correction that \n\
         \tadds +d to one lens and -d to the other. That is the source matching form, on the \n\
         \tonly test a blended file supports.\n\
         \tThe step columns are NOT a bound: the change is a dozen degrees wide, so the nearest \n\
         \tband outside its own peak still spans a good part of a wide reframe's scene, and the \n\
         \tslope column says by how much. What a stitched output cannot do is show the two \n\
         \tlenses apart, and without that there is no disagreement to bound anything against.",
        whole.iter().fold(f64::MAX, |a, b| a.min(*b)),
        mean(&whole),
        whole.len(),
        worst,
        mean(&splits),
        splits.len(),
    );
}

/// How steep the picture itself is over the band a step was fitted through, in
/// codes per degree, as the mean of the two sides' own slopes.
///
/// Printed beside every step, because a step across a line is only a step if
/// the two sides were flat enough for the extrapolation to mean anything, and
/// on a wide reframe at twelve degrees out they are usually not.
fn scene_slope(profile: &Profile, v: &[f64], about: f64, band: (f64, f64)) -> Option<f64> {
    let mut total = 0.0;
    for side in [1.0, -1.0] {
        let step = profile.s[1] - profile.s[0];
        let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut u = band.0;
        while u <= band.1 {
            if let Some(a) = profile.read(v, about + side * u) {
                sx += u;
                sy += a;
                sxx += u * u;
                sxy += u * a;
                n += 1.0;
            }
            u += step;
        }
        if n < 8.0 {
            return None;
        }
        let denominator = n * sxx - sx * sx;
        if denominator.abs() < 1e-9 {
            return None;
        }
        total += ((n * sxy - sx * sy) / denominator).abs();
    }
    Some(total / 2.0)
}

/// What a picture still steps by across the line, once each side's own trend
/// over `band` is extrapolated in. `--bin colour`'s across-seam statistic, in
/// this instrument's own geometry: a scene's own gradient reports zero and a
/// handover that changes colour does not.
fn step_across(profile: &Profile, v: &[f64], about: f64, band: (f64, f64)) -> Option<f64> {
    let mut ends = [0.0; 2];
    for (side, held) in [1.0, -1.0].iter().zip(ends.iter_mut()) {
        let step = profile.s[1] - profile.s[0];
        let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut u = band.0;
        while u <= band.1 {
            if let Some(a) = profile.read(v, about + side * u) {
                sx += u;
                sy += a;
                sxx += u * u;
                sxy += u * a;
                n += 1.0;
            }
            u += step;
        }
        if n < 8.0 {
            return None;
        }
        let denominator = n * sxx - sx * sx;
        if denominator.abs() < 1e-9 {
            return None;
        }
        *held = (sxx * sy - sx * sxy) / denominator;
    }
    Some(ends[0] - ends[1])
}

// ------------------------------------------------------------ the controls

/// The three controls, each of them the same code over different pixels.
///
/// A control that has never been shown able to fire has cleared nothing, so
/// the plant is here to make the machinery fire on a known signal and the null
/// and the decoy are here to make it read nothing on two different kinds of
/// nothing. The decoy is the one that sets the floor every number above is
/// judged against, because it carries the real pair's real JPEG noise and the
/// real scene's own texture and no correction at all.
fn controls(pair: &Pair, seam: &Seam, options: &Options) -> Fallible<()> {
    println!("\n== the controls\n");
    let null = Pair {
        off: Frame {
            width: pair.off.width,
            height: pair.off.height,
            rgb: pair.off.rgb.clone(),
        },
        on: Frame {
            width: pair.off.width,
            height: pair.off.height,
            rgb: pair.off.rgb.clone(),
        },
    };
    let profiles = seam.profiles(&null, options);
    println!(
        "  NULL, OFF against itself, read about the same line: profile rms {:.5} codes, largest \n\
         \tabsolute value {:.5}. Anything but zero here is a bug in the sampler.",
        rms_of(&profiles),
        peak_of(&profiles),
    );

    let decoyed = decoy(seam, pair).profiles(pair, options);
    println!(
        "  DECOY, the real pair read about a great circle a quarter turn from the seam: profile \n\
         \trms {:.4} codes, largest absolute value {:.4}. THAT IS THIS INSTRUMENT'S FLOOR, in the \n\
         \tsame units as every amplitude above, and it is JPEG noise plus the scene's own \n\
         \tgradient with no correction anywhere in it.",
        rms_of(&decoyed),
        peak_of(&decoyed),
    );

    let (amplitude, width) = options.plant;
    let planted_circle = seam.circle.tilted(options.tilt.to_radians());
    let planted = plant(pair, seam, &planted_circle, amplitude, width);
    let found = Seam::find(&planted, options)?;
    let turned = dot(found.circle.normal, planted_circle.normal)
        .abs()
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    let missed = dot(found.circle.normal, seam.circle.normal)
        .abs()
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    println!(
        "\n  PLANT, a known odd lobe of {amplitude:.1} codes peak and {width:.2} degrees width \n\
         \tadded to the R channel of a copy of OFF, about a circle tilted {:.1} degrees off the \n\
         \treal seam so the tracer has to find it rather than already be looking at it.",
        options.tilt,
    );
    println!(
        "    the tracer found a circle {turned:.2} degrees from the planted one and {missed:.2} \n\
         \tfrom the real seam, so it followed the plant.",
    );
    let expectations = [
        ("peak at, deg", width, 0.0),
        ("1/e from line, deg", 2.125 * width, 0.0),
        ("half from line, deg", 1.925 * width, 0.0),
        ("peak amplitude, codes", amplitude, 0.0),
        ("even/odd energy", 0.0, 0.0),
        ("crossing, deg", 0.0, 0.0),
    ];
    let profiles = found.profiles(&planted, options);
    println!(
        "\n    {:>22} {:>12} {:>12} {:>12}",
        "what", "planted", "read back", "error"
    );
    let mut read = vec![Vec::new(); expectations.len()];
    for profile in &profiles {
        let v = profile.delta(0);
        let Some(shape) = lobes(profile, &v, options.window) else {
            continue;
        };
        let span = profile.s[profile.s.len() - 1] - shape.crossing.abs();
        let (Some(out), Some(back)) = (
            falloff(profile, &v, shape.crossing, 1.0, span, options.window),
            falloff(profile, &v, shape.crossing, -1.0, span, options.window),
        ) else {
            continue;
        };
        let Some(sym) = symmetry(profile, &v, shape.crossing, span.min(6.0 * width)) else {
            continue;
        };
        read[0].push((out.peak_at + back.peak_at) / 2.0);
        read[1].push((out.over_e + back.over_e) / 2.0);
        read[2].push((out.over_half + back.over_half) / 2.0);
        read[3].push((out.peak + back.peak) / 2.0);
        read[4].push(sym.ratio());
        read[5].push(shape.crossing);
    }
    for (i, (what, planted, _)) in expectations.iter().enumerate() {
        if read[i].is_empty() {
            continue;
        }
        let mean = read[i].iter().sum::<f64>() / read[i].len() as f64;
        println!(
            "    {what:>22} {planted:>12.3} {mean:>12.3} {:>12.3}",
            mean - planted
        );
    }
    println!(
        "    the expected 1/e and half distances are 2.125 and 1.925 times the width, which is \n\
         \twhat the planted lobe's own algebra gives, so those two rows check the RULE as well \n\
         \tas the reading."
    );
    Ok(())
}

/// The seam's own great circle turned a quarter turn: a line through the same
/// picture that no handover happens on, which is what the scene and the JPEG
/// pair are worth on their own.
fn decoy(seam: &Seam, pair: &Pair) -> Seam {
    let circle = seam.circle.turned();
    Seam {
        map: seam.map,
        visible: visible_arc(&seam.map, &circle, pair.size()),
        circle,
        marks: Vec::new(),
        kept: Vec::new(),
        residual_deg: seam.residual_deg,
        residual_px: seam.residual_px,
        slack: seam.slack,
        league: Vec::new(),
        centre: seam.centre,
        rivals: Vec::new(),
    }
}

/// What this instrument reads where there is nothing to read.
fn decoy_floor(pair: &Pair, seam: &Seam, options: &Options) -> f64 {
    rms_of(&decoy(seam, pair).profiles(pair, options))
}

fn rms_of(profiles: &[Profile]) -> f64 {
    let mut total = 0.0;
    let mut n = 0usize;
    for profile in profiles {
        for c in 0..3 {
            for (i, v) in profile.delta(c).iter().enumerate() {
                if profile.count[i] >= ALONG_MIN {
                    total += v * v;
                    n += 1;
                }
            }
        }
    }
    if n == 0 {
        f64::NAN
    } else {
        (total / n as f64).sqrt()
    }
}

fn peak_of(profiles: &[Profile]) -> f64 {
    let mut worst = 0.0f64;
    for profile in profiles {
        for c in 0..3 {
            for (i, v) in profile.delta(c).iter().enumerate() {
                if profile.count[i] >= ALONG_MIN {
                    worst = worst.max(v.abs());
                }
            }
        }
    }
    worst
}

/// A copy of OFF with a known odd lobe added to its R channel about a known
/// circle, as the positive control.
///
/// The lobe is `A (s/W) exp(1/2 - s^2 / 2W^2)`, which peaks at exactly `A` at
/// exactly `s = W` either side and is odd by construction. It is rounded into
/// the frame rather than added at the sampler, so the control carries the same
/// eight-bit quantization the real pictures do and runs through one code path
/// with them.
fn plant(pair: &Pair, seam: &Seam, circle: &Circle, amplitude: f64, width: f64) -> Pair {
    let mut rgb = pair.off.rgb.clone();
    let (w, h) = pair.size();
    for y in 0..h {
        for x in 0..w {
            let Some(d) = seam.map.ray(x as f64, y as f64) else {
                continue;
            };
            let s = circle.distance(d).to_degrees();
            let u = s / width;
            let add = amplitude * u * (0.5 - u * u / 2.0).exp();
            let at = (y * w + x) * 3;
            rgb[at] = (f64::from(rgb[at]) + add).round().clamp(0.0, 255.0) as u8;
        }
    }
    Pair {
        off: Frame {
            width: w,
            height: h,
            rgb: pair.off.rgb.clone(),
        },
        on: Frame {
            width: w,
            height: h,
            rgb,
        },
    }
}

// ------------------------------------------------------------ the pictures

/// A picture being written, in plain rgb8.
struct Canvas {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize, fill: u8) -> Self {
        Self {
            width,
            height,
            rgb: vec![fill; width * height * 3],
        }
    }

    fn set(&mut self, x: i64, y: i64, colour: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return;
        }
        let at = (y as usize * self.width + x as usize) * 3;
        self.rgb[at..at + 3].copy_from_slice(&colour);
    }

    fn line(&mut self, from: (f64, f64), to: (f64, f64), colour: [u8; 3]) {
        let steps = ((to.0 - from.0).abs().max((to.1 - from.1).abs()) as usize).max(1);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            self.set(x.round() as i64, y.round() as i64, colour);
        }
    }

    fn write(&self, path: &Path) -> Fallible<()> {
        let mut png = png::Encoder::new(
            BufWriter::new(std::fs::File::create(path)?),
            self.width as u32,
            self.height as u32,
        );
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(&self.rgb)?;
        println!("  wrote {}", path.display());
        Ok(())
    }
}

/// The amplified difference, the line that was traced on it, and the profiles,
/// as pictures, because a diff nobody has looked at is a diff nobody has
/// checked.
///
/// Two of the difference: the SIGNED one, where grey is no change and the two
/// lobes are opposite colours, and the SIZE one, where black is no change and
/// the dark line down the middle of a bright band is the whole of what the
/// hypothesis predicts. The size picture is pooled harder than the signed one
/// because a dark line a fifth of a code deep does not survive a pair of JPEG
/// encodes at one pixel.
fn pictures(pair: &Pair, seam: &Seam, readings: &[Reading], options: &Options) -> Fallible<()> {
    println!("\n== the pictures\n");
    std::fs::create_dir_all(options.out())?;
    let (width, height) = pair.size();
    let reduce = options.reduce;
    let (rw, rh) = (width / reduce, height / reduce);
    let mut mean = vec![[0.0f64; 3]; rw * rh];
    for y in 0..rh {
        for x in 0..rw {
            let mut sum = [0.0; 3];
            for j in 0..reduce {
                for i in 0..reduce {
                    for (c, held) in sum.iter_mut().enumerate() {
                        *held += pair.on.code(x * reduce + i, y * reduce + j, c)
                            - pair.off.code(x * reduce + i, y * reduce + j, c);
                    }
                }
            }
            for (c, held) in mean[y * rw + x].iter_mut().enumerate() {
                *held = sum[c] / (reduce * reduce) as f64;
            }
        }
    }
    let smooth = |source: &[[f64; 3]], half: usize| {
        let mut out = vec![[0.0f64; 3]; rw * rh];
        for y in 0..rh {
            for x in 0..rw {
                let mut sum = [0.0; 3];
                let mut n = 0.0;
                for j in y.saturating_sub(half)..(y + half + 1).min(rh) {
                    for i in x.saturating_sub(half)..(x + half + 1).min(rw) {
                        for (c, held) in sum.iter_mut().enumerate() {
                            *held += source[j * rw + i][c];
                        }
                        n += 1.0;
                    }
                }
                for (c, held) in out[y * rw + x].iter_mut().enumerate() {
                    *held = sum[c] / n;
                }
            }
        }
        out
    };
    let light = smooth(&mean, 1);
    let heavy = smooth(&mean, 4);

    let mut signed = Canvas::new(rw, rh, 128);
    for (i, v) in light.iter().enumerate() {
        for (c, held) in v.iter().enumerate() {
            signed.rgb[i * 3 + c] = (128.0 + options.amplify * held).clamp(0.0, 255.0) as u8;
        }
    }
    let mut size = Canvas::new(rw, rh, 0);
    for (i, v) in heavy.iter().enumerate() {
        let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let code = (options.amplify * m).clamp(0.0, 255.0) as u8;
        for c in 0..3 {
            size.rgb[i * 3 + c] = code;
        }
    }
    for canvas in [&mut signed, &mut size] {
        draw_circle(canvas, seam, reduce, [255, 0, 0]);
        for reading in readings {
            let at = (
                reading.profile.at.0 / reduce as f64,
                reading.profile.at.1 / reduce as f64,
            );
            canvas.line((at.0 - 6.0, at.1), (at.0 + 6.0, at.1), [0, 255, 255]);
            canvas.line((at.0, at.1 - 6.0), (at.0, at.1 + 6.0), [0, 255, 255]);
        }
    }
    signed.write(&options.out().join(format!("{}-signed.png", options.tag)))?;
    size.write(&options.out().join(format!("{}-size.png", options.tag)))?;

    let mut chart = Canvas::new(1200, 700, 255);
    let colours = [
        [200, 0, 0],
        [200, 110, 0],
        [140, 150, 0],
        [0, 150, 60],
        [0, 130, 190],
        [60, 60, 200],
        [150, 0, 170],
        [0, 0, 0],
        [120, 120, 120],
    ];
    let bounds = (-options.reach, options.reach, -1.4, 1.4);
    chart.line((0.0, 350.0), (1200.0, 350.0), [200, 200, 200]);
    chart.line((600.0, 0.0), (600.0, 700.0), [200, 200, 200]);
    for (n, reading) in readings.iter().enumerate() {
        let profile = reading.profile;
        let colour = colours[n % colours.len()];
        let mut last: Option<(f64, f64)> = None;
        for (i, s) in profile.s.iter().enumerate() {
            if profile.count[i] < ALONG_MIN {
                last = None;
                continue;
            }
            let x = (s - reading.shape.crossing - bounds.0) / (bounds.1 - bounds.0) * 1200.0;
            let y =
                700.0 - (reading.w[i] / reading.peak() - bounds.2) / (bounds.3 - bounds.2) * 700.0;
            if let Some(previous) = last {
                chart.line(previous, (x, y), colour);
            }
            last = Some((x, y));
        }
    }
    chart.write(&options.out().join(format!("{}-profiles.png", options.tag)))?;
    Ok(())
}

fn draw_circle(canvas: &mut Canvas, seam: &Seam, reduce: usize, colour: [u8; 3]) {
    let steps = 4000;
    let mut last: Option<(f64, f64)> = None;
    for i in 0..=steps {
        let phi = seam.visible.0 + (seam.visible.1 - seam.visible.0) * i as f64 / steps as f64;
        let here = seam
            .map
            .pixel(seam.circle.at(phi))
            .map(|(x, y)| (x / reduce as f64, y / reduce as f64));
        if let (Some(a), Some(b)) = (last, here) {
            canvas.line(a, b, colour);
        }
        last = here;
    }
}

// ------------------------------------------------------------ options

struct Options {
    input: PathBuf,
    off: Option<PathBuf>,
    on: Option<PathBuf>,
    mode: Mode,
    /// How many azimuths the profiles are read at, evenly over the arc that is
    /// inside the picture.
    places: usize,
    /// How far either side of the line a profile reaches, and how finely it is
    /// sampled, in degrees across the seam.
    reach: f64,
    step: f64,
    /// How much seam each profile row is pooled along, in degrees. This is the
    /// whole noise budget: a row is a mean of a few hundred pixels and its
    /// floor is what the decoy control reports.
    along: f64,
    /// How far from the seam a block has to be to count as far field, in
    /// degrees.
    far: f64,
    /// How far either side of the line the two lobes are looked for, in
    /// degrees.
    ///
    /// Not a cosmetic bound. At one azimuth of the owner's own dirt render the
    /// profile runs straight through the pilot's body, and with the whole reach
    /// to search in, the largest difference on that line was the pilot and not
    /// the seam: the crossing came back at 17 degrees and 207 pixels out. A
    /// lobe further from the line than this is not this band's lobe.
    window: f64,
    /// The planted lobe's peak amplitude in codes and its width in degrees,
    /// and how far off the real seam it is planted.
    plant: (f64, f64),
    tilt: f64,
    model: Option<Model>,
    /// Which feature of the difference is taken as the line, or `None` for
    /// whichever of the three a great circle fits best, which is the default
    /// and is a measurement rather than a preference.
    centre: Option<Centre>,
    amplify: f64,
    reduce: usize,
    blocks: Vec<(usize, usize, usize, usize)>,
    out: Option<PathBuf>,
    tag: String,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            off: None,
            on: None,
            mode: Mode::All,
            places: 7,
            reach: 30.0,
            step: 0.05,
            along: 12.0,
            far: 40.0,
            window: 15.0,
            plant: (6.0, 1.5),
            tilt: 5.0,
            model: None,
            centre: None,
            amplify: 20.0,
            reduce: 3,
            blocks: Vec::new(),
            out: None,
            tag: "oracle".to_owned(),
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "all" => Mode::All,
                        "control" => Mode::Control,
                        "trace" => Mode::Trace,
                        "o1" => Mode::O1,
                        "o2" => Mode::O2,
                        "o3" => Mode::O3,
                        "controls" => Mode::Controls,
                        "pictures" => Mode::Pictures,
                        _ => return Err(format!("no mode called {value}").into()),
                    }
                }
                Some(("off", value)) => options.off = Some(PathBuf::from(value)),
                Some(("on", value)) => options.on = Some(PathBuf::from(value)),
                Some(("places", value)) => options.places = value.parse()?,
                Some(("reach", value)) => options.reach = value.parse()?,
                Some(("step", value)) => options.step = value.parse()?,
                Some(("along", value)) => options.along = value.parse()?,
                Some(("far", value)) => options.far = value.parse()?,
                Some(("window", value)) => options.window = value.parse()?,
                Some(("tilt", value)) => options.tilt = value.parse()?,
                Some(("centre", value)) => {
                    options.centre = match value {
                        "auto" => None,
                        "lobes" => Some(Centre::Lobes),
                        "crossing" => Some(Centre::Crossing),
                        "dark" => Some(Centre::Dark),
                        _ => return Err(format!("no centre rule called {value}").into()),
                    }
                }
                Some(("amplify", value)) => options.amplify = value.parse()?,
                Some(("reduce", value)) => options.reduce = value.parse::<usize>()?.max(1),
                Some(("plant", value)) => {
                    let (a, w) = value
                        .split_once(':')
                        .ok_or_else(|| format!("{value:?} is not codes:degrees"))?;
                    options.plant = (a.parse()?, w.parse()?);
                }
                Some(("model", value)) => {
                    options.model = match value {
                        "auto" => None,
                        _ => Some(
                            Model::ALL
                                .into_iter()
                                .find(|m| m.name() == value)
                                .ok_or_else(|| format!("no projection called {value}"))?,
                        ),
                    }
                }
                Some(("block", value)) => {
                    let mut parts = value.split(':').map(str::parse::<usize>);
                    let mut next = || {
                        parts
                            .next()
                            .ok_or("block wants x:y:w:h")?
                            .map_err(|e| e.to_string())
                    };
                    options.blocks.push((next()?, next()?, next()?, next()?));
                }
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
                Some(("tag", value)) => options.tag = value.to_owned(),
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        if options.input.as_os_str().is_empty() && options.off.is_none() {
            return Err(USAGE.into());
        }
        Ok(options)
    }

    fn off(&self) -> PathBuf {
        self.off
            .clone()
            .unwrap_or_else(|| self.input.join("off.jpg"))
    }

    fn on(&self) -> PathBuf {
        self.on.clone().unwrap_or_else(|| self.input.join("on.jpg"))
    }

    fn out(&self) -> PathBuf {
        self.out
            .clone()
            .unwrap_or_else(|| PathBuf::from("scratch/oracle"))
    }
}

const USAGE: &str = "usage: oracle <dir holding off.jpg and on.jpg> [off=path] [on=path] \
     [mode=all|control|trace|o1|o2|o3|controls|pictures] [places=n] [reach=deg] [step=deg] \
     [along=deg] [far=deg] [window=deg] [plant=codes:deg] [tilt=deg] [centre=auto|lobes|crossing|dark] \
     [model=auto|equidistant|stereographic|equisolid|orthographic|rectilinear] [amplify=n] \
     [reduce=n] [block=x:y:w:h] [out=dir] [tag=name]";
