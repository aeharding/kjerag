//! How a landed sample is taken out of the plane it landed in (issue #11).
//!
//! [`super::projection`] says **where** in a lens's picture an output pixel
//! looks; this says how much of that picture one output pixel is worth and
//! what to do about it. Zoomed in far enough, an output pixel sits inside a
//! single source texel and hardware bilinear is reading a tent between four
//! of them, which is soft and slightly faceted. Zoomed out, an output pixel
//! spans several texels and bilinear is the right answer, so nothing here
//! runs at all.
//!
//! The switch between the two is not a field of view and not a zoom step: it
//! is the **local** texel-to-pixel ratio, [`Reframe::texels_per_pixel`], and
//! it has to be local because nothing about it is uniform. Down the X4 Air's
//! own axis the delivered frame carries 1106 texels per radian and at the rim
//! of its picture 948 radially, and a rectilinear output's density rises
//! towards its corners: at the widest view the player offers, a 2560 px
//! window is past 1:1 in the middle (1.23 texels to the pixel) and two thirds
//! of the way inside it at the corners (0.74).
//!
//! **Two thresholds, not one**, because NV12 is two planes: chroma is half
//! the grid, so one output pixel covers half as many chroma texels and the
//! chroma plane is magnified an octave of zoom before luma is. [`plane_ratio`]
//! is the whole of that and it reads the size off the texture rather than
//! assuming the subsampling. Measuring it separately is also what settled
//! what to do about it, which was nothing: see [`Sampling`].
//!
//! [`Reframe::texels_per_pixel`]: super::Reframe::texels_per_pixel

/// Which planes the pass upgrades where the view magnifies the source.
///
/// [`Self::Luma`] is what ships, and the two either side of it are what that
/// was chosen against. They are here for the same reason
/// [`FrameClock::Container`] is: a quality change is a difference between two
/// pictures, and the losing side has to be renderable through the **same**
/// pass as the winner or what is measured is the harness. Nothing in the
/// shell offers the choice; `kyerag-spike --bin zoom` is what reaches it.
///
/// **The chroma plane is not upgraded**, and that is a measurement rather
/// than an oversight. Chroma is magnified at every field of view this player
/// offers, so upgrading it is not a cost paid at high zoom but a cost paid
/// always, and it is the larger half of the bill: at 2560x1440 the pass goes
/// from 0.69 ms to 0.90 with the luma plane upgraded and to 1.23 with both.
/// What it buys, on 8-bit 4:2:0 chroma that HEVC has already smoothed, is
/// 0.41 codes on 40% of pixels and **no** measurable change in detail (mean
/// absolute Laplacian 4.606 either way, against 4.120 bilinear). Rendered
/// side by side at four times life size on the highest-contrast content in
/// half an hour of footage, the two are indistinguishable. Footage with hard
/// saturated colour edges in it, which paramotor flying does not have much
/// of, would change the answer, and `Sampling::Sharp` is what would measure
/// it again.
///
/// [`FrameClock::Container`]: super::FrameClock::Container
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Sampling {
    /// Bilinear everywhere, whatever the magnification: the pass exactly as
    /// it was before issue #11, and what the quality delta is measured
    /// against.
    Bilinear,
    /// Cubic where the **luma** plane is magnified, and bilinear on chroma
    /// whatever chroma is doing. What ships.
    #[default]
    Luma,
    /// Cubic wherever either plane is magnified, each plane deciding on its
    /// own grid. Measured and cut.
    Sharp,
}

impl Sampling {
    /// How far the upgrade may engage on each plane, luma first, which is
    /// what the uniform block carries. Zero is bilinear, and it is exact:
    /// the shader's cubic branch is not taken at all, so that plane is the
    /// bits it was.
    pub fn limits(self) -> [f32; 2] {
        match self {
            Self::Bilinear => [0.0, 0.0],
            Self::Luma => [1.0, 0.0],
            Self::Sharp => [1.0, 1.0],
        }
    }
}

/// Where the cubic kernel is fully engaged, in texels per output pixel.
///
/// One half is two output pixels across one source texel, which is where an
/// eye can see the tent between four of them. Between here and 1:1 the
/// kernel is mixed rather than switched, so scrolling the zoom through the
/// threshold has no frame in it that the frames either side do not lead to
/// ([`sharpen`], and `the_upgrade_arrives_without_a_step`).
const SHARPEN_FULL: f32 = 0.5;

/// How far the cubic kernel is engaged at this magnification: 1 fully cubic,
/// 0 plain bilinear.
///
/// **Exactly** 0 at 1:1 and wider, which is the property the rest of the
/// picture rests on: a view that is not magnifying takes the same one fetch
/// it always did and writes the same bits. `limit` is [`Sampling`]'s, and
/// zero there is that same exactness at every ratio.
///
/// WGSL twin: `sharpen`.
pub fn sharpen(ratio: f32, limit: f32) -> f32 {
    limit * (1.0 - smoothstep(SHARPEN_FULL, 1.0, ratio))
}

/// WGSL twin: `smoothstep`, written out because the two have to agree on the
/// endpoints exactly. At `x >= high` this is 1 and nothing else.
fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One plane's own texels per output pixel, from the delivered frame's.
///
/// The NV12 wrinkle in one line. The landing is in delivered-frame pixels,
/// which is the luma plane's grid; the chroma plane holds the same picture on
/// half the grid, so an output pixel covers half as many of its texels and it
/// is magnified while luma still is not. On this camera at a 2560 px window
/// that is not a corner case: chroma is under 1:1 at **every** field of view
/// the player offers, which is what [`Sampling`] settles and how.
///
/// The size comes off the texture rather than out of a constant: 4:2:0 is
/// what this camera family delivers, but the plane knows its own shape and
/// nothing here has to.
///
/// WGSL twin: `plane_ratio`.
pub fn plane_ratio(ratio: f32, plane_width: f32, frame_width: f32) -> f32 {
    ratio * plane_width / frame_width
}

/// The four weights one axis of the kernel puts on four consecutive texels,
/// mixed `sharpen` of the way from linear interpolation to Catmull-Rom.
///
/// Catmull-Rom rather than a B-spline because the kernel has to **pass
/// through** the texels it is given: a B-spline is the one the four-tap
/// trick is usually written for, and it blurs a magnified picture rather
/// than resolving it, which is the opposite of the job.
///
/// Mixing the weights is what makes the engagement smooth without sampling
/// the picture twice. At `sharpen` 0 this is `[0, 1 - t, t, 0]`, exactly, so
/// the two outer taps weigh nothing and the kernel is the linear one; at 1 it
/// is Catmull-Rom; in between it is a valid interpolating kernel of its own,
/// because both ends sum to 1 at every `t` and a mix of them does too. The
/// alternative, sampling the picture both ways and crossfading, costs two
/// kernels wherever the zoom sits between the thresholds and answers the same
/// thing.
///
/// WGSL twin: `kernel`.
pub fn weights(t: f32, sharpen: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    let cardinal = [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ];
    let linear = [0.0, 1.0 - t, t, 0.0];
    // `mix` as WGSL defines it, in that order, so that `sharpen` of 0 leaves
    // the linear weights untouched rather than nearly untouched.
    std::array::from_fn(|i| linear[i] * (1.0 - sharpen) + cardinal[i] * sharpen)
}

/// The half of the shader this file mirrors, with the constant it owns
/// written into it. `crates/render/src/scene.rs` concatenates it after
/// `projection::wgsl`, whose uniform block it reads.
pub(crate) fn wgsl() -> String {
    format!("const SHARPEN_FULL = {SHARPEN_FULL:?};\n{WGSL}")
}

const WGSL: &str = r#"
// How far the cubic kernel is engaged at this magnification. Rust twin:
// `sampling::sharpen`.
fn sharpen(ratio: f32, limit: f32) -> f32 {
  return limit * (1.0 - smoothstep(SHARPEN_FULL, 1.0, ratio));
}

// One plane's own texels per output pixel. Rust twin: `sampling::plane_ratio`.
fn plane_ratio(ratio: f32, plane_width: f32) -> f32 {
  return ratio * plane_width / reframe.frame_width;
}

// One plane, at whatever quality its own magnification asks for.
//
// `limit` is this plane's own, because the two planes are not the same size
// and the decision about one is not the decision about the other. The
// sampler is passed in rather than read from the module, because this file
// is concatenated before the one that binds it.
fn plane(tex: texture_2d<f32>, samp: sampler, uv: vec2<f32>, ratio: f32, limit: f32) -> vec4<f32> {
  let size = vec2<f32>(textureDimensions(tex));
  let engaged = sharpen(plane_ratio(ratio, size.x), limit);
  // Not an optimization: this is what keeps a view that is not magnifying
  // byte for byte the picture it was before issue #11. The cubic branch at
  // `engaged` of zero is the same arithmetic in principle and a different
  // rounding of the texel grid in practice, and that difference is the only
  // discontinuity in the whole engagement. It is one code: at fov 110 and
  // 1920x1080, where the threshold runs through the corners of the picture,
  // 0.29% of pixels sit on it and none of them moves further than that.
  if engaged <= 0.0 {
    return textureSampleLevel(tex, samp, uv, 0.0);
  }
  return cubic(tex, samp, uv, size, engaged);
}

// The kernel's four weights along one axis. Rust twin: `sampling::weights`.
fn kernel(t: f32, sharpen: f32) -> vec4<f32> {
  let t2 = t * t;
  let t3 = t2 * t;
  let cardinal = vec4<f32>(
    0.5 * (-t3 + 2.0 * t2 - t),
    0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
    0.5 * (-3.0 * t3 + 4.0 * t2 + t),
    0.5 * (t3 - t2),
  );
  let linear = vec4<f32>(0.0, 1.0 - t, t, 0.0);
  return mix(linear, cardinal, sharpen);
}

// Catmull-Rom over the sixteen texels around the landing, as **nine**
// bilinear fetches rather than sixteen point ones.
//
// Each axis has four weights and the middle two are both positive, so that
// pair is one fetch placed between its two texels and weighed by their sum:
// three fetches an axis, nine in two dimensions, and the picture is the
// sixteen-tap one to the precision of the sampler's own filter weights.
// Measured against this kernel written as sixteen `textureLoad`s, on the
// highest-contrast view in half an hour of footage: 0.14 codes RMS and one
// code at worst. The outer two taps cannot join anything, so they stay their
// own: Catmull-Rom's outer lobes are negative, which is where the resolving
// comes from, and a sampler cannot weigh a fetch by less than nothing.
//
// The middle pair's weights sum to at least 1 at every `t` and every
// engagement (`the_middle_pair_never_vanishes`), so `pair` cannot divide by
// zero.
fn cubic(tex: texture_2d<f32>, samp: sampler, uv: vec2<f32>, size: vec2<f32>, sharpen: f32)
  -> vec4<f32> {
  // Texel centres sit at integer coordinates here, which is half a texel off
  // the corner a texture coordinate counts from.
  let coord = uv * size - vec2<f32>(0.5);
  let base = floor(coord);
  let t = coord - base;
  let wx = kernel(t.x, sharpen);
  let wy = kernel(t.y, sharpen);
  let x = (vec3<f32>(base.x - 1.0, base.x + pair(wx), base.x + 2.0) + vec3<f32>(0.5)) / size.x;
  let y = (vec3<f32>(base.y - 1.0, base.y + pair(wy), base.y + 2.0) + vec3<f32>(0.5)) / size.y;
  let cx = vec3<f32>(wx.x, wx.y + wx.z, wx.w);
  let cy = vec3<f32>(wy.x, wy.y + wy.z, wy.w);
  return cy.x * row(tex, samp, x, cx, y.x)
    + cy.y * row(tex, samp, x, cx, y.y)
    + cy.z * row(tex, samp, x, cx, y.z);
}

// Where between its two texels the middle fetch of an axis goes: the far
// texel's share of the pair.
fn pair(w: vec4<f32>) -> f32 {
  return w.z / (w.y + w.z);
}

// One row of three fetches, weighed.
fn row(tex: texture_2d<f32>, samp: sampler, x: vec3<f32>, cx: vec3<f32>, y: f32) -> vec4<f32> {
  return cx.x * textureSampleLevel(tex, samp, vec2<f32>(x.x, y), 0.0)
    + cx.y * textureSampleLevel(tex, samp, vec2<f32>(x.y, y), 0.0)
    + cx.z * textureSampleLevel(tex, samp, vec2<f32>(x.z, y), 0.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The delivered frame and its chroma plane, which is what the two
    /// thresholds are separated by.
    const LUMA: f32 = 3840.0;
    const CHROMA: f32 = 1920.0;

    fn sweep(steps: usize) -> impl Iterator<Item = f32> {
        (0..=steps).map(move |step| step as f32 / steps as f32)
    }

    /// A kernel whose weights do not sum to 1 does not interpolate: it
    /// brightens or darkens whatever it is over, by however much they miss.
    /// True at both ends and everywhere in between, because the engagement
    /// sits in between whenever the zoom is near the threshold.
    #[test]
    fn the_kernel_sums_to_one() {
        for sharpen in sweep(16) {
            for t in sweep(64) {
                let total: f32 = weights(t, sharpen).iter().sum();
                assert!(
                    (total - 1.0).abs() < 1e-6,
                    "{t} at engagement {sharpen} sums to {total}"
                );
            }
        }
    }

    /// Not engaged is not nearly linear: it is the linear weights, to the
    /// bit, which is what lets the shader skip its outer taps and take the
    /// one fetch it always took.
    #[test]
    fn no_engagement_is_exactly_bilinear() {
        for t in sweep(64) {
            assert_eq!(weights(t, 0.0), [0.0, 1.0 - t, t, 0.0], "at {t}");
        }
    }

    /// And it interpolates: on a texel centre the kernel is that texel and
    /// nothing else, at every engagement. A kernel that is not interpolating
    /// blurs a magnified picture instead of resolving it, which is the whole
    /// difference between Catmull-Rom and the B-spline the four-tap trick is
    /// usually written for.
    #[test]
    fn the_kernel_is_interpolating_at_a_texel_centre() {
        for sharpen in sweep(16) {
            assert_eq!(weights(0.0, sharpen), [0.0, 1.0, 0.0, 0.0]);
        }
    }

    /// The middle pair is one bilinear fetch placed by its two weights, so
    /// the shader divides by their sum. It never approaches zero: the pair
    /// carries at least the whole sample and the outer lobes take away from
    /// it, never from each other.
    #[test]
    fn the_middle_pair_never_vanishes() {
        for sharpen in sweep(16) {
            for t in sweep(256) {
                let w = weights(t, sharpen);
                assert!(w[1] + w[2] >= 1.0 - 1e-6, "{t} at {sharpen}: {w:?}");
            }
        }
    }

    /// What the eye reads while the zoom is scrolled through the threshold:
    /// the kernel arrives rather than appearing. Swept 4000 steps of the
    /// texel ratio across the whole engagement band, no weight ever moves
    /// more than a thousandth in one step, and the engagement is exactly
    /// zero from 1:1 outwards.
    ///
    /// The 4000 steps are far finer than a zoom notch: one scroll step is 12%
    /// of the field of view, which crosses the whole band in about six.
    #[test]
    fn the_upgrade_arrives_without_a_step() {
        for t in [0.0, 0.15, 0.37, 0.5, 0.62, 0.9] {
            let mut held = weights(t, sharpen(0.4, 1.0));
            let mut worst: f32 = 0.0;

            for step in 1..=4000 {
                let ratio = 0.4 + step as f32 * 0.0002;
                let engaged = sharpen(ratio, 1.0);
                let now = weights(t, engaged);
                for lobe in 0..4 {
                    worst = worst.max((now[lobe] - held[lobe]).abs());
                }
                held = now;
            }

            assert!(worst < 1e-3, "a weight jumped by {worst} at t {t}");
            assert_eq!(held, weights(t, 0.0), "the sweep did not end bilinear");
        }
    }

    /// And the far end is exact rather than small: at 1:1 and wider the
    /// engagement is zero, which is what the shader tests to take its one
    /// fetch.
    #[test]
    fn nothing_is_engaged_at_one_to_one_or_wider() {
        for ratio in [1.0, 1.0001, 1.5, 8.0] {
            assert_eq!(sharpen(ratio, 1.0), 0.0, "at {ratio}");
        }
        assert!(sharpen(0.999, 1.0) > 0.0);
        assert_eq!(sharpen(SHARPEN_FULL, 1.0), 1.0);
        assert_eq!(sharpen(0.1, 1.0), 1.0);
    }

    /// The measurable loser: bilinear everywhere, at every magnification.
    #[test]
    fn the_settings_are_the_planes_they_upgrade() {
        for ratio in [0.05, 0.5, 0.9, 1.0, 4.0] {
            let [luma, chroma] = Sampling::Bilinear.limits();
            assert_eq!(sharpen(ratio, luma), 0.0, "luma at {ratio}");
            assert_eq!(sharpen(ratio, chroma), 0.0, "chroma at {ratio}");
        }
        // The chroma plane is magnified twice as hard as luma, so "luma
        // only" is the setting that costs the cubic kernel only where the
        // delivered frame itself has run out of texels.
        let [luma, chroma] = Sampling::Luma.limits();
        assert_eq!(sharpen(plane_ratio(0.4, LUMA, LUMA), luma), 1.0);
        assert_eq!(sharpen(plane_ratio(0.4, CHROMA, LUMA), chroma), 0.0);
        assert_eq!(Sampling::Sharp.limits(), [1.0, 1.0]);
    }

    /// The NV12 wrinkle, as the two thresholds it really is: chroma is half
    /// the grid, so it is magnified twice as hard as luma at the same view
    /// and it engages first. There is a whole octave of zoom where the
    /// chroma plane is upgraded and the luma plane is not, and it covers the
    /// player's own default view.
    #[test]
    fn chroma_reaches_magnification_before_luma() {
        let engaged = |ratio: f32, plane| sharpen(plane_ratio(ratio, plane, LUMA), 1.0);

        // Luma still at or past 1:1 through the whole octave below it, and
        // chroma already inside its own threshold.
        for ratio in [1.0, 1.2, 1.6, 1.99] {
            assert_eq!(engaged(ratio, LUMA), 0.0, "luma at {ratio}");
            assert!(engaged(ratio, CHROMA) > 0.0, "chroma at {ratio}");
        }
        // Chroma is never behind luma, at any zoom.
        for step in 0..=400 {
            let ratio = step as f32 * 0.01;
            assert!(engaged(ratio, CHROMA) >= engaged(ratio, LUMA), "at {ratio}");
        }
        // And past two octaves both are fully engaged, which is where a
        // zoomed view of this footage sits.
        assert_eq!(engaged(0.4, LUMA), 1.0);
        assert_eq!(engaged(0.4, CHROMA), 1.0);
    }
}
