//! The calibration strings the camera writes, all of them, and the lens sets
//! they turn into.
//!
//! `kjerag_meta` reads `offset_v3` (tag 54) and nothing else, because that is
//! the string every camera in the corpus writes and the one whose grammar
//! `docs/research/insv-format.md` 4 is written against. The X4 Air writes two
//! more: `offset_v2` (tag 53) and **`offset_v6` (tag 111)**, and it declares
//! `capture_offset_version = OFFSET_V6` (tag 136, value 4). So the calibration
//! the camera itself says it was made with is one kjerag has never read.
//!
//! This reads all three, off the same trailer `kjerag_meta` reads, so that the
//! question "what would v6 change in the picture" can be asked. It lives in
//! the instruments and not in `kjerag_meta` on purpose: nothing here is a
//! proposal to ship, and the answer this branch is measuring may well be that
//! v6 is not worth reading.
//!
//! **Grammar, measured** (studio round 2, and re-derived here from the token
//! counts of the owner's own files):
//!
//! | string | tokens | per lens | distortion |
//! | --- | --- | --- | --- |
//! | `offset_v2` | `1 + 16n + 1` | 16 | 4 |
//! | `offset_v3` | `1 + 19n + 1` | 19 | 5 |
//! | `offset_v6` | `1 + 27n + 1` | 27 | 13 |
//!
//! The eleven pose and intrinsic tokens are in the same order and the same
//! slots in v3 and v6 (`xi, fx, fy, cx, cy, yaw, pitch, roll, tx, ty, tz`),
//! and so are the three that close a block (`calib_w, calib_h, lensType`).
//! Only the distortion run in the middle changes length. The trailing token
//! carries the version in bits 16 to 19: `0x30400` on a v3 string and
//! `0x60400` on a v6 one, on this camera.
//!
//! **What kjerag can hold of a v6 block is the eleven and the three.**
//! [`kjerag_meta::Distortion`] is five coefficients, and the shader's
//! `LensBlock` is the same five; there is no slot for a thirteen-term
//! polynomial and no way to load one without changing the projection on both
//! sides. What the slots past the fifth *mean* is also not known: the first
//! four behave like radial terms (see `--bin ceiling`, which computes the two
//! polynomials against each other over the seam's own radius), and the other
//! nine are named nowhere this branch could check. So [`Blocks::lenses`] takes
//! a [`Carry`] saying what to do about it, and no arm this module builds is
//! allowed to call itself v6 without saying which one it used.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use kjerag_media::Fallible;
use kjerag_meta::{Distortion, Intrinsics, Lens, Pose, Size};

/// The trailer footer: 32 reserved bytes, the trailer length, 4 more, and the
/// magic. `kjerag_meta::trailer` is the authority on all three and it is
/// `pub(crate)`, so these are copies; what checks them is that the metadata
/// record this finds decodes, and that the `offset_v3` in it is the string
/// `CalibrationSet` already read (`--bin ceiling`'s first control).
const FOOTER_LEN: i64 = 32 + 4 + 4 + 32;
const RECORD_HEADER_LEN: i64 = 1 + 1 + 4;
const MAGIC: &[u8] = b"8db42d694ccc418790edff439fe026bf";
const METADATA_RECORD: u8 = 1;
const PROTOBUF: u8 = 1;

/// The metadata fields this reads, by protobuf tag.
const TAG_DIMENSION: u64 = 19;
const TAG_CROP: u64 = 27;
const TAG_OFFSET_V2: u64 = 53;
const TAG_OFFSET_V3: u64 = 54;
const TAG_OFFSET_V6: u64 = 111;
const TAG_ORIGINAL_V6: u64 = 112;
const TAG_DECLARED_VERSION: u64 = 136;

/// What one capture's trailer says about its own calibration.
pub struct Written {
    pub v2: Option<String>,
    pub v3: Option<String>,
    pub v6: Option<String>,
    pub original_v6: Option<String>,
    /// `capture_offset_version`, the camera's own word for which of the above
    /// describes the glass. 4 is `OFFSET_V6` on the X4 Air.
    pub declared: Option<u64>,
    /// The delivered frame of one lens, and the sensor window the camera crops
    /// out of the canvas before delivering it. Both are needed to put a
    /// canvas-space calibration into delivered pixels, and both are read here
    /// rather than off [`kjerag_meta::CalibrationSet`] so that this module's
    /// answer is one file read and not two.
    pub dimension: Size,
    pub crop: Size,
}

/// Read the calibration strings out of an `.insv` trailer.
pub fn written(path: &Path) -> Fallible<Written> {
    let mut file = std::fs::File::open(path)?;
    let metadata = metadata_record(&mut file)?;
    let mut out = Written {
        v2: None,
        v3: None,
        v6: None,
        original_v6: None,
        declared: None,
        dimension: Size {
            width: 0,
            height: 0,
        },
        crop: Size {
            width: 0,
            height: 0,
        },
    };
    for (tag, field) in fields(&metadata) {
        match (tag, field) {
            (TAG_OFFSET_V2, Field::Bytes(b)) => out.v2 = text(b),
            (TAG_OFFSET_V3, Field::Bytes(b)) => out.v3 = text(b),
            (TAG_OFFSET_V6, Field::Bytes(b)) => out.v6 = text(b),
            (TAG_ORIGINAL_V6, Field::Bytes(b)) => out.original_v6 = text(b),
            (TAG_DECLARED_VERSION, Field::Varint(v)) => out.declared = Some(v),
            (TAG_DIMENSION, Field::Bytes(b)) => out.dimension = pair(b, 1, 2),
            (TAG_CROP, Field::Bytes(b)) => out.crop = pair(b, 3, 4),
            _ => {}
        }
    }
    if out.v3.is_none() {
        return Err("this trailer's metadata record carries no offset_v3".into());
    }
    Ok(out)
}

fn text(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

/// Two `uint32` fields of a nested message as a size, which is how both
/// `Vector2` and `WindowCropInfo` are read.
fn pair(bytes: &[u8], x: u64, y: u64) -> Size {
    let mut size = Size {
        width: 0,
        height: 0,
    };
    for (tag, field) in fields(bytes) {
        if let Field::Varint(value) = field {
            match tag {
                _ if tag == x => size.width = value as u32,
                _ if tag == y => size.height = value as u32,
                _ => {}
            }
        }
    }
    size
}

// ------------------------------------------------------------ the trailer

/// The metadata record's payload, found by the same backwards walk
/// `kjerag_meta` does: footer, then record headers from the end.
fn metadata_record(file: &mut std::fs::File) -> Fallible<Vec<u8>> {
    let file_len = file.seek(SeekFrom::End(0))? as i64;
    if file_len < FOOTER_LEN {
        return Err("this file is shorter than an Insta360 trailer footer".into());
    }
    let mut footer = [0u8; FOOTER_LEN as usize];
    file.seek(SeekFrom::End(-FOOTER_LEN))?;
    file.read_exact(&mut footer)?;
    if &footer[FOOTER_LEN as usize - MAGIC.len()..] != MAGIC {
        return Err("this file has no Insta360 trailer".into());
    }
    let trailer_len = i64::from(u32::from_le_bytes([
        footer[32], footer[33], footer[34], footer[35],
    ]));
    let trailer_start = file_len - trailer_len;
    let mut back = FOOTER_LEN + RECORD_HEADER_LEN;
    while file_len - back > trailer_start {
        file.seek(SeekFrom::End(-back))?;
        let mut header = [0u8; RECORD_HEADER_LEN as usize];
        file.read_exact(&mut header)?;
        let size = i64::from(u32::from_le_bytes([
            header[2], header[3], header[4], header[5],
        ]));
        let at = file_len - back - size;
        if at < trailer_start {
            break;
        }
        if header[1] == METADATA_RECORD && header[0] == PROTOBUF {
            let mut payload = vec![0u8; size.max(0) as usize];
            file.seek(SeekFrom::Start(at as u64))?;
            file.read_exact(&mut payload)?;
            return Ok(payload);
        }
        back += size + RECORD_HEADER_LEN;
    }
    Err("this trailer carries no metadata record".into())
}

// ------------------------------------------------------------ protobuf

/// One field's payload, in the two wire types this reads.
enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

/// Every top level field of a protobuf message, in wire order. Unknown wire
/// types stop the walk rather than guessing at a length.
fn fields(bytes: &[u8]) -> Vec<(u64, Field<'_>)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let Some((key, next)) = varint(bytes, at) else {
            break;
        };
        at = next;
        let tag = key >> 3;
        match key & 7 {
            0 => match varint(bytes, at) {
                Some((value, next)) => {
                    out.push((tag, Field::Varint(value)));
                    at = next;
                }
                None => break,
            },
            1 => at += 8,
            2 => {
                let Some((len, next)) = varint(bytes, at) else {
                    break;
                };
                let end = next.saturating_add(len as usize);
                if end > bytes.len() {
                    break;
                }
                out.push((tag, Field::Bytes(&bytes[next..end])));
                at = end;
            }
            5 => at += 4,
            _ => break,
        }
    }
    out
}

fn varint(bytes: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

// ------------------------------------------------------------ the grammar

/// The eleven pose and intrinsic tokens every version opens a lens block
/// with, then however many distortion coefficients that version carries, then
/// `calib_w, calib_h, lensType`.
const POSE_TOKENS: usize = 11;
const TAIL_TOKENS: usize = 3;

/// One lens's block, whatever version wrote it.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub xi: f64,
    pub fx: f64,
    pub fy: f64,
    /// On the shared calibration canvas, so lens 1's carries its slot offset.
    pub cx: f64,
    pub cy: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub translation: [f64; 3],
    /// 4 tokens in a v2 string, 5 in a v3 one, 13 in a v6 one. Only the first
    /// five have names kjerag knows, and only on a v3 string.
    pub distortion: Vec<f64>,
    pub canvas: Size,
    pub lens_type: u32,
}

/// One whole calibration string, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Blocks {
    pub blocks: Vec<Block>,
    /// How many tokens one lens block took, which is what names the version.
    pub per_lens: usize,
    /// The token that closes the string. Bits 16 to 19 are the version.
    pub trailing: u64,
}

impl Blocks {
    /// The version the trailing token declares: 2, 3 or 6 on the strings in
    /// the corpus.
    pub fn declared_version(&self) -> u64 {
        (self.trailing >> 16) & 0xf
    }

    /// The version the block length implies, which is the one that decides
    /// how the string was read.
    pub fn read_version(&self) -> Option<u64> {
        match self.per_lens {
            16 => Some(2),
            19 => Some(3),
            27 => Some(6),
            _ => None,
        }
    }

    /// The lens set this calibration makes, in the delivered frame's own
    /// pixels: the same two scalings [`kjerag_meta`] applies on the way in,
    /// reproduced here because that conversion is private and a second copy
    /// of it that disagreed would be the whole measurement.
    ///
    /// What checks the copy is `--bin ceiling`'s first control: this function
    /// over the file's own `offset_v3` against `CalibrationSet::from_insv`,
    /// field by field, to the bit.
    pub fn lenses(&self, dimension: Size, crop: Size, carry: Carry<'_>) -> Fallible<Vec<Lens>> {
        let count = self.blocks.len() as f64;
        let canvas = self.blocks.first().ok_or("no lens blocks")?.canvas;
        if self.blocks.iter().any(|block| block.canvas != canvas) {
            return Err("the lens blocks disagree about the calibration canvas".into());
        }
        let slot = f64::from(canvas.width) / count;
        if slot == 0.0 || canvas.height == 0 || crop.width == 0 || crop.height == 0 {
            return Err("a canvas or crop dimension is zero, so no pixel scale exists".into());
        }
        self.blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                Ok(Lens {
                    intrinsics: Intrinsics {
                        xi: block.xi,
                        fx: block.fx * f64::from(dimension.width) / f64::from(crop.width),
                        fy: block.fy * f64::from(dimension.height) / f64::from(crop.height),
                        cx: (block.cx - index as f64 * slot) * (f64::from(dimension.width) / slot),
                        cy: block.cy * (f64::from(dimension.height) / f64::from(canvas.height)),
                    },
                    distortion: carry.of(block, index)?,
                    pose: Pose {
                        yaw_deg: block.yaw,
                        pitch_deg: block.pitch,
                        roll_deg: block.roll,
                        translation_m: block.translation,
                    },
                    lens_type: block.lens_type,
                })
            })
            .collect()
    }
}

/// What a lens set does about a distortion run kjerag has no slots for.
///
/// Never a default: a v6 block carries thirteen coefficients and
/// [`Distortion`] holds five, so every arm built off one has made a choice
/// here, and the point of naming the choice is that a table row cannot then
/// be labelled "v6" without saying which.
#[derive(Clone, Copy, Debug)]
pub enum Carry<'a> {
    /// The block's own first five, read as `k1, k2, k3, p1, p2`. Correct for
    /// a v3 block by construction and **a guess about slot meaning** on any
    /// other, which is why nothing calls it on one.
    Written,
    /// Another calibration's distortion, lens for lens: the honest subset,
    /// where a v6 block contributes its pose and its intrinsics and its
    /// distortion run is left out of the arm entirely.
    From(&'a [Lens]),
}

impl Carry<'_> {
    fn of(self, block: &Block, index: usize) -> Fallible<Distortion> {
        match self {
            Self::Written => {
                let [k1, k2, k3, p1, p2] = block
                    .distortion
                    .get(..5)
                    .and_then(|run| <[f64; 5]>::try_from(run).ok())
                    .ok_or("this block carries fewer than five distortion coefficients")?;
                Ok(Distortion { k1, k2, k3, p1, p2 })
            }
            Self::From(lenses) => Ok(lenses
                .get(index)
                .ok_or("the carried calibration has fewer lenses than this one")?
                .distortion),
        }
    }
}

/// Split one calibration string into lens blocks.
///
/// The version is **derived from the token count** rather than from the
/// trailing word: the count is what decides how the string is cut, so reading
/// it out of the cut and then checking it against the word is a check, while
/// trusting the word and cutting to it is not.
pub fn parse(text: &str) -> Fallible<Blocks> {
    let tokens: Vec<f64> = text
        .split('_')
        .map(|token| {
            token
                .parse::<f64>()
                .map_err(|_| format!("offset token {token:?} is not a number"))
        })
        .collect::<Result<_, _>>()?;
    let count = *tokens.first().ok_or("an offset string with no tokens")? as usize;
    if count == 0 || tokens.len() < 2 {
        return Err(format!("an offset string that declares {count} lenses").into());
    }
    if !(tokens.len() - 2).is_multiple_of(count) {
        return Err(format!(
            "{} tokens is not 1 + n * {count} + 1 for any whole n",
            tokens.len(),
        )
        .into());
    }
    let per_lens = (tokens.len() - 2) / count;
    if per_lens < POSE_TOKENS + TAIL_TOKENS + 1 {
        return Err(format!("{per_lens} tokens per lens leaves no distortion run").into());
    }
    let trailing = *tokens.last().expect("checked non-empty above") as u64;
    let blocks = (0..count)
        .map(|index| {
            let start = 1 + index * per_lens;
            let block = &tokens[start..start + per_lens];
            let distortion = block[POSE_TOKENS..per_lens - TAIL_TOKENS].to_vec();
            Block {
                xi: block[0],
                fx: block[1],
                fy: block[2],
                cx: block[3],
                cy: block[4],
                yaw: block[5],
                pitch: block[6],
                roll: block[7],
                translation: [block[8], block[9], block[10]],
                distortion,
                canvas: Size {
                    width: block[per_lens - 3] as u32,
                    height: block[per_lens - 2] as u32,
                },
                lens_type: block[per_lens - 1] as u32,
            }
        })
        .collect();
    Ok(Blocks {
        blocks,
        per_lens,
        trailing,
    })
}

/// The Mei normalized radius a ray on the seam great circle lands at, which
/// is where a distortion polynomial has to be compared if the comparison is
/// about this seam.
///
/// A direction 90 degrees off a lens's axis has `z = 0`, so the unified model
/// puts it at `|m| = 1 / xi`: 0.432 on this camera. Nothing about the seam is
/// anywhere else, which is why a coefficient's size in the string says so
/// little on its own and its size *here* says what it is worth.
pub fn seam_radius(xi: f64) -> f64 {
    1.0 / xi
}

/// A radial polynomial's factor at radius `r`, reading `run` as
/// `k1 r^2 + k2 r^4 + ...`.
pub fn radial(run: &[f64], r: f64) -> f64 {
    let mut factor = 1.0;
    let mut power = r * r;
    for coefficient in run {
        factor += coefficient * power;
        power *= r * r;
    }
    factor
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's X4 Air, lens 0, as the file writes it.
    const V3: &str = "2_2.314940_7087.490_7090.350_3837.880_3854.420_-0.103_-0.070_90.534_0.000000_0.000000_0.000000_0.95820886_-1.80141151_3.57555127_-0.00073380_-0.00115458_15360_7680_131_2.314940_7099.030_7097.430_11550.700_3870.180_0.039_-0.193_89.076_-0.002063_0.000334_-0.033284_0.97158086_-2.08655882_4.30578518_-0.00192490_0.00054564_15360_7680_131_197632";

    #[test]
    fn a_v3_string_cuts_into_nineteen_token_blocks() {
        let read = parse(V3).expect("the owner's own v3 string");
        assert_eq!(read.per_lens, 19);
        assert_eq!(read.read_version(), Some(3));
        assert_eq!(read.declared_version(), 3);
        assert_eq!(read.blocks.len(), 2);
        assert_eq!(read.blocks[0].distortion.len(), 5);
        assert_eq!(read.blocks[1].cx, 11550.700);
        assert_eq!(read.blocks[1].lens_type, 131);
    }

    /// The version the tokens imply and the version the trailing word
    /// declares are two readings, and a string is only understood where they
    /// agree. This is the one that would catch a grammar drifting.
    #[test]
    fn the_trailing_word_agrees_with_the_block_length() {
        let read = parse(V3).expect("the owner's own v3 string");
        assert_eq!(read.read_version(), Some(read.declared_version()));
    }

    /// A string whose tokens do not divide is refused rather than cut short.
    #[test]
    fn a_string_that_does_not_divide_is_refused() {
        assert!(parse("2_1.0_2.0_3.0").is_err());
    }

    /// The seam is one radius on the normalized plane and the whole
    /// comparison between two distortion runs happens there.
    #[test]
    fn the_seam_radius_is_the_inverse_mirror_parameter() {
        assert!((seam_radius(2.314940) - 0.431976).abs() < 1e-6);
        assert!((radial(&[], 0.43) - 1.0).abs() < 1e-12);
    }
}
