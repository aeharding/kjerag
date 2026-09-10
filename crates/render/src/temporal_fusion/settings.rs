//! Automatic effective denoise settings for the two authenticated camera routes.
//!
//! This consumes the camera's source-time ISO track. It does not choose frame
//! history, invent settings for an unsupported source, or substitute an ISO
//! when metadata is absent. The table and selector provenance is recorded in
//! `docs/research/studio-denoise-config-602.md`.

use kjerag_meta::{CalibrationSet, Size};

use super::iso;

const X4_AIR: &str = "Insta360 X4 Air";
const ONE_X2: &str = "Insta360 ONE X2";
const X4_AIR_RATE_LIMIT: f64 = f64::from_bits(0x4049_0ccc_cccc_cccd); // 50.1
const NORMALIZATION: f32 = 255.0 * 64.0;

const X4_AIR_ROWS: [Row; 7] = [
    Row::new(100, 700, 3, 10),
    Row::new(200, 900, 3, 12),
    Row::new(400, 900, 3, 14),
    Row::new(800, 1_200, 3, 16),
    Row::new(1_600, 1_500, 3, 18),
    Row::new(2_200, 1_700, 3, 22),
    Row::new(5_000, 2_000, 3, 26),
];

const COMMON_ROWS: [Row; 8] = [
    Row::new(100, 100, 0, 15),
    Row::new(200, 120, 0, 15),
    Row::new(400, 150, 0, 15),
    Row::new(401, 200, 1, 15),
    Row::new(800, 400, 2, 15),
    Row::new(1_600, 600, 2, 15),
    Row::new(2_200, 800, 3, 15),
    Row::new(5_000, 1_200, 3, 15),
];

#[derive(Clone, Debug, PartialEq)]
pub struct FusionParams {
    pub noise: f32,
    pub limit: f32,
    pub y_limits: [f32; 256],
    pub uv_limits: [f32; 256],
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffParams {
    pub iso: i32,
    /// Native temporal radius. The history owner decides which centered
    /// references exist at startup and tail boundaries.
    pub radius: u32,
    /// Unnormalised noise supplied to motion-confidence construction.
    pub noise_integer: i32,
    pub confidence_y: [f32; 256],
    pub confidence_uv: [f32; 256],
    pub fusion: FusionParams,
}

pub struct Provider {
    table: Table,
    lookup: iso::Lookup,
}

impl Provider {
    /// Select the native table from source metadata. Only routes whose inputs
    /// and control flow have been authenticated are accepted.
    pub fn new(calibration: &CalibrationSet, source_fps: f32) -> Result<Self, Error> {
        let table = select(
            &calibration.camera_model,
            calibration.source_group_type,
            calibration.dimension,
            source_fps,
        )?;
        Ok(Self {
            table,
            lookup: iso::Lookup::new(&calibration.denoise_iso)?,
        })
    }

    /// Start a new monotonically advancing source-time run after a seek, loop,
    /// or source epoch replacement.
    pub fn reset(&mut self) {
        self.lookup.reset();
    }

    pub fn parameters_at(&mut self, source_time_ms: f64) -> Result<EffParams, Error> {
        let iso = self.lookup.iso_at(source_time_ms)?;
        Ok(self.table.parameters(iso))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Table {
    X4Air,
    Common,
}

impl Table {
    fn rows(self) -> &'static [Row] {
        match self {
            Self::X4Air => &X4_AIR_ROWS,
            Self::Common => &COMMON_ROWS,
        }
    }

    fn parameters(self, iso: i32) -> EffParams {
        let rows = self.rows();
        let upper = rows.partition_point(|row| row.iso < iso);
        let row = match upper {
            0 => rows[0],
            index if index == rows.len() => rows[index - 1],
            index => Row::interpolate(rows[index - 1], rows[index], iso),
        };
        EffParams {
            iso,
            radius: row.radius as u32,
            noise_integer: row.noise,
            confidence_y: [1.0; 256],
            confidence_uv: [2.0; 256],
            fusion: FusionParams {
                noise: row.noise as f32 / NORMALIZATION,
                limit: row.limit as f32 / 255.0,
                y_limits: [1.0; 256],
                uv_limits: [0.5; 256],
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    iso: i32,
    noise: i32,
    radius: i32,
    limit: i32,
}

impl Row {
    const fn new(iso: i32, noise: i32, radius: i32, limit: i32) -> Self {
        Self {
            iso,
            noise,
            radius,
            limit,
        }
    }

    fn interpolate(lower: Self, upper: Self, iso: i32) -> Self {
        let fraction = (iso - lower.iso) as f32 / (upper.iso - lower.iso) as f32;
        Self {
            iso,
            noise: interpolate_integer(lower.noise, upper.noise, fraction),
            radius: interpolate_integer(lower.radius, upper.radius, fraction),
            limit: interpolate_integer(lower.limit, upper.limit, fraction),
        }
    }
}

fn interpolate_integer(lower: i32, upper: i32, fraction: f32) -> i32 {
    let lower_part = (1.0_f32 - fraction) * lower as f32;
    ((upper as f32).mul_add(fraction, lower_part) + 0.5) as i32
}

fn select(
    camera_model: &str,
    source_group_type: Option<i32>,
    dimension: Size,
    source_fps: f32,
) -> Result<Table, Error> {
    let valid_rate = source_fps.is_finite() && source_fps > 0.0;
    if camera_model == X4_AIR
        && source_group_type == Some(0)
        && dimension
            == (Size {
                width: 3_840,
                height: 3_840,
            })
        && valid_rate
        && f64::from(source_fps) < X4_AIR_RATE_LIMIT
    {
        return Ok(Table::X4Air);
    }
    if camera_model == ONE_X2 && valid_rate {
        return Ok(Table::Common);
    }
    Err(Error::UnsupportedSource {
        camera_model: camera_model.to_owned(),
        source_group_type,
        dimension,
        source_fps_bits: source_fps.to_bits(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Iso(iso::Error),
    UnsupportedSource {
        camera_model: String,
        source_group_type: Option<i32>,
        dimension: Size,
        source_fps_bits: u32,
    },
}

impl From<iso::Error> for Error {
    fn from(value: iso::Error) -> Self {
        Self::Iso(value)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Iso(error) => error.fmt(f),
            Self::UnsupportedSource {
                camera_model,
                source_group_type,
                dimension,
                source_fps_bits,
            } => write!(
                f,
                "temporal denoise settings are not authenticated for {camera_model}, group {source_group_type:?}, {}x{}, source fps bits {source_fps_bits:#010x}",
                dimension.width, dimension.height
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x4_air_interpolation_uses_f32_fma_rounding_and_endpoint_clamps() {
        let below = Table::X4Air.parameters(50);
        assert_eq!((below.noise_integer, below.radius), (700, 3));
        assert_eq!(below.fusion.limit, 10.0 / 255.0);

        let middle = Table::X4Air.parameters(150);
        assert_eq!((middle.noise_integer, middle.radius), (800, 3));
        assert_eq!(middle.fusion.limit, 11.0 / 255.0);

        let above = Table::X4Air.parameters(6_000);
        assert_eq!((above.noise_integer, above.radius), (2_000, 3));
        assert_eq!(above.fusion.limit, 26.0 / 255.0);
    }

    #[test]
    fn common_radius_transition_is_not_replaced_by_x4_history() {
        let iso365 = Table::Common.parameters(365);
        assert_eq!((iso365.noise_integer, iso365.radius), (145, 0));
        assert_eq!(Table::Common.parameters(400).radius, 0);
        assert_eq!(Table::Common.parameters(401).radius, 1);
    }

    #[test]
    fn fixed_tables_and_normalisation_feed_the_two_consumers() {
        let value = Table::X4Air.parameters(100);
        assert_eq!(value.noise_integer, 700);
        assert_eq!(value.fusion.noise, 700.0 / 16_320.0);
        assert_eq!(value.fusion.limit, 10.0 / 255.0);
        assert!(value.confidence_y.iter().all(|&entry| entry == 1.0));
        assert!(value.confidence_uv.iter().all(|&entry| entry == 2.0));
        assert!(value.fusion.y_limits.iter().all(|&entry| entry == 1.0));
        assert!(value.fusion.uv_limits.iter().all(|&entry| entry == 0.5));
    }

    #[test]
    fn selector_accepts_only_the_authenticated_routes() {
        let square = Size {
            width: 3_840,
            height: 3_840,
        };
        let rate = 30_000.0_f32 / 1_001.0_f32;
        assert_eq!(select(X4_AIR, Some(0), square, rate), Ok(Table::X4Air));
        assert_eq!(select(ONE_X2, None, square, rate), Ok(Table::Common));

        assert!(select(X4_AIR, None, square, rate).is_err());
        assert!(select(X4_AIR, Some(8), square, rate).is_err());
        // The f32 immediately below the binary64 threshold still takes the
        // branch; its successor does not.
        let rounded_limit = 50.1_f32;
        assert_eq!(
            select(X4_AIR, Some(0), square, rounded_limit),
            Ok(Table::X4Air)
        );
        assert!(
            select(
                X4_AIR,
                Some(0),
                square,
                f32::from_bits(rounded_limit.to_bits() + 1)
            )
            .is_err()
        );
        assert!(select(X4_AIR, Some(0), square, f32::NAN).is_err());
        assert!(
            select(
                X4_AIR,
                Some(0),
                Size {
                    width: 7_680,
                    height: 3_840
                },
                rate
            )
            .is_err()
        );
        assert!(select("another camera", None, square, rate).is_err());
    }

    #[test]
    #[ignore = "reads the owner's private capture files"]
    fn owner_captures_select_the_authenticated_tables_and_times() {
        let cases = [
            (
                "KJERAG_DENOISE_ISO_X4",
                &[(607_574.0, 100, 700, 3), (612_078.0, 100, 700, 3)][..],
            ),
            (
                "KJERAG_DENOISE_ISO_X2",
                &[(212_478.0, 365, 145, 0), (212_512.0, 365, 145, 0)][..],
            ),
        ];
        for (variable, expected) in cases {
            let path = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} is required"));
            let calibration = CalibrationSet::from_capture(path).unwrap();
            let mut provider = Provider::new(&calibration, 30_000.0_f32 / 1_001.0_f32).unwrap();
            for &(time, iso, noise, radius) in expected {
                let parameters = provider.parameters_at(time).unwrap();
                assert_eq!(parameters.iso, iso, "{variable} at {time}");
                assert_eq!(parameters.noise_integer, noise, "{variable} at {time}");
                assert_eq!(parameters.radius, radius, "{variable} at {time}");
            }
        }
    }
}
