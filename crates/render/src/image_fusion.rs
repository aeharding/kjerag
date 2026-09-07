//! Photometric lens matching, not chromatic-aberration geometry.
//!
//! The consumer is read from Studio 6.0.2's embedded DynamicStitch shaders:
//! each original-chart RGB ratio corrects its own lens before alpha blending.
//! Captured X4 uploads and fragment slots 0/1 are 200x100 RGBA32F; the fourth
//! component is not consumed. See the private capture receipt in
//! `scratch/x4-fusion-maps-20260907-01/` and the public on/off oracle manifest.
//!
//! The shared resident capture also owns an automatic GPU producer. Its
//! immutable per-frame outputs use this same consumer; neither captured replay
//! nor numerical reference agreement establishes Studio-output video parity.

mod content;
mod coordinates;
pub(crate) mod gpu;
pub(crate) mod sample;
pub mod solve;
pub mod spatial;

pub use sample::{FusionInputs, PendingOneXsFusionInputs};

use crate::studio_type2::{MAP_HEIGHT, MAP_NODES, MAP_WIDTH, PACKED_BYTES};

/// One renderer-ordinal RGB ratio map. Construction checks shape, not native
/// provenance; a replay caller must authenticate the payload and association.
#[derive(Clone, Debug, PartialEq)]
pub struct RatioMap(Box<[[f32; 4]; MAP_NODES]>);

impl RatioMap {
    pub fn new(values: Vec<[f32; 4]>) -> Result<Self, &'static str> {
        Ok(Self(values.into_boxed_slice().try_into().map_err(
            |_| "image fusion ratio map must contain 200 by 100 float4 nodes",
        )?))
    }

    pub fn values(&self) -> &[[f32; 4]; MAP_NODES] {
        &self.0
    }

    pub fn bytes(&self) -> &[u8] {
        // The boxed array owns exactly MAP_NODES initialized float4 values.
        unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast(), PACKED_BYTES) }
    }

    /// Bilinear repeat-X/clamp-Y sampling in the ORIGINAL chart coordinate.
    /// Unlike packed UV/alpha lookup, no half-texel/height centering is added.
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 3] {
        let p = [uv[0] * MAP_WIDTH as f32, uv[1] * MAP_HEIGHT as f32];
        let f = p.map(|v| v - v.floor());
        let base = std::array::from_fn::<_, 2, _>(|c| {
            (p[c] - if f[c] < 0.5 { 1.0 } else { 0.0 }).floor() as i32
        });
        let weight = f.map(|v| if v < 0.5 { v + 0.5 } else { v - 0.5 });
        let at = |x: i32, y: i32| {
            self.0[y.clamp(0, MAP_HEIGHT as i32 - 1) as usize * MAP_WIDTH
                + x.rem_euclid(MAP_WIDTH as i32) as usize]
        };
        let a = at(base[0], base[1]);
        let b = at(base[0] + 1, base[1]);
        let c = at(base[0], base[1] + 1);
        let d = at(base[0] + 1, base[1] + 1);
        let mix = |a: f32, b: f32, t: f32| a * (1.0 - t) + b * t;
        std::array::from_fn(|i| {
            mix(
                mix(a[i], b[i], weight[0]),
                mix(c[i], d[i], weight[0]),
                weight[1],
            )
        })
    }
}

/// Left applies to packed.xy, right to packed.zw. These are renderer ordinals,
/// not an assumption about the camera's physical front/back lens order.
#[derive(Clone, Debug, PartialEq)]
pub struct RatioPair {
    pub left: RatioMap,
    pub right: RatioMap,
}

/// Studio's `recoverData3f(color, ratio, 1.0)`, before lens alpha blending.
/// A missing correction must bypass this operation, not apply constant ones:
/// even `(color + 1) - 1` can round, and clamping changes out-of-range RGB.
pub fn correct(color: [f32; 3], ratio: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| ((color[c] + 1.0) * ratio[c] - 1.0).clamp(0.0, 1.0))
}

pub(crate) const WGSL: &str = r#"
@group(2) @binding(0) var fusion_left: texture_2d<f32>;
@group(2) @binding(1) var fusion_right: texture_2d<f32>;

fn fusion_at(lens: u32, x: i32, y: i32) -> vec3<f32> {
  let at = vec2<i32>(((x % TYPE2_MAP_W) + TYPE2_MAP_W) % TYPE2_MAP_W,
    clamp(y, 0, TYPE2_MAP_H - 1));
  if lens == 0u { return textureLoad(fusion_left, at, 0).xyz; }
  return textureLoad(fusion_right, at, 0).xyz;
}

fn type2_correct(color: vec3<f32>, uv: vec2<f32>, lens: u32) -> vec3<f32> {
  let q = type2_bilinear_weights(uv);
  let at = vec2<i32>(q.xy);
  let a = fusion_at(lens, at.x, at.y);
  let b = fusion_at(lens, at.x + 1, at.y);
  let c = fusion_at(lens, at.x, at.y + 1);
  let d = fusion_at(lens, at.x + 1, at.y + 1);
  let ratio = mix(mix(a, b, q.z), mix(c, d, q.z), q.w);
  return clamp((color + vec3<f32>(1.0)) * ratio - vec3<f32>(1.0),
    vec3<f32>(0.0), vec3<f32>(1.0));
}
"#;

pub(crate) const FILTERED_WGSL: &str = r#"
@group(2) @binding(0) var fusion_left: texture_2d<f32>;
@group(2) @binding(1) var fusion_right: texture_2d<f32>;
@group(2) @binding(2) var fusion_linear: sampler;

fn type2_correct(color: vec3<f32>, uv: vec2<f32>, lens: u32) -> vec3<f32> {
  var ratio: vec3<f32>;
  if lens == 0u {
    ratio = textureSampleLevel(fusion_left, fusion_linear, uv, 0.0).xyz;
  } else {
    ratio = textureSampleLevel(fusion_right, fusion_linear, uv, 0.0).xyz;
  }
  return clamp((color + vec3<f32>(1.0)) * ratio - vec3<f32>(1.0),
    vec3<f32>(0.0), vec3<f32>(1.0));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_is_exact_and_channel_four_is_not_a_weight() {
        assert!(RatioMap::new(vec![[1.0; 4]; MAP_NODES - 1]).is_err());
        let map = RatioMap::new(vec![[1.0, 2.0, 3.0, 0.0]; MAP_NODES]).unwrap();
        assert_eq!(map.sample([0.0, 0.0]), [1.0, 2.0, 3.0]);
        assert_eq!(map.bytes().len(), PACKED_BYTES);
    }

    #[test]
    fn original_chart_samples_wrap_join_and_clamp_poles() {
        let values = (0..MAP_NODES)
            .map(|i| [(i % MAP_WIDTH) as f32, (i / MAP_WIDTH) as f32, 1.0, 0.0])
            .collect();
        let map = RatioMap::new(values).unwrap();
        assert_eq!(map.sample([0.0, 0.0]), [99.5, 0.0, 1.0]);
        assert_eq!(map.sample([1.0, 1.0]), [99.5, 99.0, 1.0]);
        assert_eq!(map.sample([0.5 / 200.0, 0.5 / 100.0]), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn correction_uses_rgb_ratios_and_clamps_each_lens_before_blending() {
        assert_eq!(correct([0.5; 3], [1.0, 0.5, 2.0]), [0.5, 0.0, 1.0]);
        // It is neither color*ratio nor a reciprocal other-lens correction.
        assert_eq!(correct([0.0; 3], [1.25; 3]), [0.25; 3]);
        assert_eq!(correct([-0.5, 0.5, 1.5], [1.0; 3]), [0.0, 0.5, 1.0]);
    }
}
