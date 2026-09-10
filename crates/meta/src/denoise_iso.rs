//! The camera's ISO observations from trailer record 9.
//!
//! This module preserves the input and summary produced by Studio 6.0.2's
//! `Iso` constructor. It deliberately does not decide which observation
//! belongs to a decoded frame, interpolate between observations, reject an
//! ISO value, or provide a fallback when the record is absent.
//!
//! The pinned native provenance is `GetIso` at `0xd52a68` and `Iso::Iso` at
//! `0xd57b1c`; `docs/research/studio-denoise-iso-602.md` records the bounded
//! disassembly and file observations behind this parser.

/// One decoded record-9 observation, before any denoise policy is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DenoiseIsoObservation {
    /// Milliseconds relative to the constructor's corrected first-frame
    /// origin. This is signed because the native arithmetic is signed after
    /// its wrapping-width clock correction.
    pub offset_ms: i64,
    /// Integer ISO decoded from the record's packed word.
    pub iso: u32,
}

/// The raw ISO observations and the aggregate stored beside them by Studio.
///
/// A record with at most 40 complete items has a summary but no observations.
/// A longer record discards its 40-item prefix and exposes the remaining
/// timestamped observations. A missing or empty record is an empty track.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DenoiseIsoTrack {
    summary_iso: Option<u32>,
    observations: Vec<DenoiseIsoObservation>,
}

const ITEM_LEN: usize = 48;
const PREFIX_ITEMS: usize = 40;

/// Summarised rather than dumped: measured captures carry tens of thousands
/// of observations.
impl std::fmt::Debug for DenoiseIsoTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DenoiseIsoTrack")
            .field("summary_iso", &self.summary_iso)
            .field("observations", &self.observations.len())
            .field("first", &self.observations.first())
            .field("last", &self.observations.last())
            .finish()
    }
}

impl DenoiseIsoTrack {
    /// Studio's integer aggregate: the first value for 1 through 40 items,
    /// otherwise the unsigned integer mean of items 40 onward.
    pub fn summary_iso(&self) -> Option<u32> {
        self.summary_iso
    }

    /// The constructor's unfiltered timestamp/ISO pairs. No interpolation or
    /// validity filtering has been applied.
    pub fn observations(&self) -> &[DenoiseIsoObservation] {
        &self.observations
    }

    pub fn is_empty(&self) -> bool {
        self.summary_iso.is_none()
    }

    /// Parse complete 48-byte items. A partial tail cannot form an
    /// observation and is ignored, like the caller's byte-count division.
    pub(crate) fn parse(payload: &[u8], first_frame: i64, raw_gyro: bool) -> Self {
        let count = payload.len() / ITEM_LEN;
        if count == 0 {
            return Self::default();
        }
        let first = &payload[..ITEM_LEN];
        if count <= PREFIX_ITEMS {
            return Self {
                summary_iso: Some(decode_iso(first)),
                observations: Vec::new(),
            };
        }

        let suffix_bytes = &payload[PREFIX_ITEMS * ITEM_LEN..count * ITEM_LEN];
        let suffix = suffix_bytes.chunks_exact(ITEM_LEN);
        let suffix_count = count - PREFIX_ITEMS;
        let sum = suffix.clone().fold(0u64, |sum, item| {
            sum.wrapping_add(u64::from(decode_iso(item)))
        });
        let summary_iso = (sum / suffix_count as u64) as u32;

        // `GetFirstFrameTimeOffsetMs` converts the signed metadata field to
        // binary64 and conditionally divides by 1000. The caller then
        // truncates it back to signed i64 before entering the constructor.
        let scale = if raw_gyro { 1_000.0 } else { 1.0 };
        let first_ms = ((first_frame as f64) / scale).trunc() as i64;
        let first_timestamp = timestamp(
            suffix
                .clone()
                .next()
                .expect("a record longer than the prefix has a suffix"),
        );
        let correction = first_timestamp.wrapping_sub(first_ms as u32) as i32;
        let origin = first_ms.wrapping_add(i64::from(correction));

        let observations = suffix
            .map(|item| DenoiseIsoObservation {
                offset_ms: i64::from(timestamp(item)).wrapping_sub(origin),
                iso: decode_iso(item),
            })
            .collect();
        Self {
            summary_iso: Some(summary_iso),
            observations,
        }
    }
}

fn timestamp(item: &[u8]) -> u32 {
    u32::from_le_bytes(item[..4].try_into().expect("complete record-9 item"))
}

fn decode_iso(item: &[u8]) -> u32 {
    let packed = u32::from_le_bytes(item[16..20].try_into().expect("complete record-9 item"));
    ((packed >> 19) * 100) >> 6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(timestamp: u32, shifted_iso: u32) -> [u8; ITEM_LEN] {
        let mut item = [0xA5; ITEM_LEN];
        item[..4].copy_from_slice(&timestamp.to_le_bytes());
        // Low 19 bits are unrelated to the decoded value and deliberately
        // nonzero, proving they are discarded.
        let packed = (shifted_iso << 19) | 0x5_4321;
        item[16..20].copy_from_slice(&packed.to_le_bytes());
        item
    }

    fn payload(items: impl IntoIterator<Item = [u8; ITEM_LEN]>) -> Vec<u8> {
        items.into_iter().flatten().collect()
    }

    #[test]
    fn empty_and_incomplete_records_are_empty() {
        assert!(DenoiseIsoTrack::parse(&[], 0, false).is_empty());
        assert!(DenoiseIsoTrack::parse(&[0; ITEM_LEN - 1], 0, false).is_empty());
    }

    #[test]
    fn one_through_forty_items_keep_only_the_first_summary() {
        for count in [1, 40] {
            let bytes = payload((0..count).map(|index| item(index, 64 + index)));
            let track = DenoiseIsoTrack::parse(&bytes, 0, false);
            assert_eq!(track.summary_iso(), Some(100));
            assert!(track.observations().is_empty());
        }
    }

    #[test]
    fn packed_iso_uses_the_high_thirteen_bits_and_integer_truncation() {
        for (shifted, expected) in [(0, 0), (1, 1), (63, 98), (64, 100), (8_191, 12_798)] {
            assert_eq!(decode_iso(&item(0, shifted)), expected);
        }
    }

    #[test]
    fn forty_one_items_skip_the_prefix_and_timestamp_the_suffix() {
        let bytes = payload((0..41).map(|index| item(10_000 + index, 64 + index)));
        let track = DenoiseIsoTrack::parse(&bytes, 2_000, false);

        assert_eq!(track.summary_iso(), Some((104 * 100) >> 6));
        assert_eq!(
            track.observations(),
            [DenoiseIsoObservation {
                offset_ms: 0,
                iso: (104 * 100) >> 6,
            }]
        );
    }

    #[test]
    fn suffix_summary_is_an_unsigned_integer_mean() {
        let bytes = payload((0..40).map(|index| item(index, 64)).chain([
            item(1_000, 64),
            item(1_001, 65),
            item(1_002, 66),
        ]));
        let values = [100, 101, 103];
        let track = DenoiseIsoTrack::parse(&bytes, 1_000, false);
        assert_eq!(track.summary_iso(), Some(values.iter().sum::<u32>() / 3));
        assert_eq!(
            track
                .observations()
                .iter()
                .map(|sample| sample.iso)
                .collect::<Vec<_>>(),
            values
        );
    }

    #[test]
    fn first_frame_uses_milliseconds_or_truncated_microseconds() {
        let bytes = payload(
            (0..40)
                .map(|index| item(index, 64))
                .chain([item(4_000, 64), item(4_017, 64)]),
        );
        let milliseconds = DenoiseIsoTrack::parse(&bytes, 3_812, false);
        let microseconds = DenoiseIsoTrack::parse(&bytes, 3_812_999, true);
        assert_eq!(milliseconds.observations()[1].offset_ms, 17);
        assert_eq!(microseconds.observations()[1].offset_ms, 17);
        assert_eq!(milliseconds.observations()[0].offset_ms, 0);
        assert_eq!(microseconds.observations()[0].offset_ms, 0);

        // Here the correction crosses i32's sign boundary. Both timebases
        // reach the same truncated 3812 ms and therefore the same native
        // 2^32 offset. Omitting the raw-gyro division would instead yield 0.
        let boundary = payload(
            (0..40)
                .map(|index| item(index, 64))
                .chain([item((1 << 31) + 3_812, 64)]),
        );
        let milliseconds = DenoiseIsoTrack::parse(&boundary, 3_812, false);
        let microseconds = DenoiseIsoTrack::parse(&boundary, 3_812_999, true);
        assert_eq!(milliseconds.observations()[0].offset_ms, 1i64 << 32);
        assert_eq!(microseconds.observations()[0].offset_ms, 1i64 << 32);
    }

    #[test]
    fn clock_correction_preserves_signed_u32_wrap() {
        let bytes = payload(
            (0..40)
                .map(|index| item(index, 64))
                .chain([item(u32::MAX - 2, 64), item(3, 64)]),
        );
        let track = DenoiseIsoTrack::parse(&bytes, 5, false);
        assert_eq!(track.observations()[0].offset_ms, 1i64 << 32);
        assert_eq!(track.observations()[1].offset_ms, 6);
    }

    #[test]
    fn incomplete_tail_is_not_an_item() {
        let mut bytes = payload((0..41).map(|index| item(100 + index, 64)));
        bytes.extend_from_slice(&item(999, 128)[..ITEM_LEN - 1]);
        let track = DenoiseIsoTrack::parse(&bytes, 0, false);
        assert_eq!(track.summary_iso(), Some(100));
        assert_eq!(track.observations().len(), 1);
    }

    #[test]
    #[ignore = "reads the owner's private capture files"]
    fn owner_captures_match_the_record_nine_audit() {
        type CaptureAudit<'a> = (&'a str, usize, u32, &'a [(i64, u32)]);
        let cases: [CaptureAudit<'_>; 2] = [
            (
                "KJERAG_DENOISE_ISO_X4",
                53_985,
                100,
                &[(607_577, 100), (612_082, 100)],
            ),
            ("KJERAG_DENOISE_ISO_X2", 2_048, 178, &[(212_478, 365)]),
        ];
        for (variable, count, summary, expected) in cases {
            let path = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} is required"));
            let calibration = crate::CalibrationSet::from_capture(path).unwrap();
            let track = &calibration.denoise_iso;
            assert_eq!(track.observations().len(), count, "{variable}");
            assert_eq!(track.summary_iso(), Some(summary), "{variable}");
            for &(target, iso) in expected {
                let nearest = track
                    .observations()
                    .iter()
                    .min_by_key(|sample| sample.offset_ms.abs_diff(target))
                    .unwrap();
                assert_eq!(
                    (nearest.offset_ms, nearest.iso),
                    (target, iso),
                    "{variable}"
                );
            }
        }
    }
}
