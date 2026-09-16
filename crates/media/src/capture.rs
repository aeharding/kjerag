//! Capture discovery and pre-decoder video metadata.

use std::path::{Path, PathBuf};

use ffmpeg_next as ff;

use super::{Fallible, Samples, Size, read_only};

/// A container opened and looked at, before any decoder exists.
pub(super) struct Opened {
    pub(super) path: PathBuf,
    pub(super) input: ff::format::context::Input,
    /// One per video stream, in container order.
    pub(super) videos: Vec<Video>,
    pub(super) time_base: ff::Rational,
    pub(super) start: i64,
}

#[derive(Clone, Copy)]
pub(super) struct Video {
    pub(super) stream: usize,
    pub(super) rate: ff::Rational,
    pub(super) frames: u64,
    pub(super) size: Size,
    pub(super) samples: Samples,
}

/// How one video stream's samples are written, off the container's own three
/// fields: pixel format, range and Y'CbCr matrix.
///
/// Read from the container before decoder construction, where the capture's
/// lens streams can be checked as one set. Range and matrix cannot be inferred
/// from the plane layout. A depth this does not recognize is the 8-bit picture
/// Kjerag drew before there was a second answer, which is what every `.insv`
/// in the corpus is.
///
/// **Big endian is refused rather than drawn.** The shader puts a 16-bit word
/// back together itself, from two 8-bit components, in one order
/// (`kjerag_render`'s `plane_word`), and a stream whose words are the other
/// way round would come out as noise with nothing to say so. Nothing in this
/// path can produce one - ffmpeg names the host's own endianness on a decode
/// and this host is little endian - so the refusal is a claim this reader
/// declines to make rather than a case anyone has met. Errors are the error:
/// what a pilot would read is this sentence.
///
/// **The range's fallback is studio swing, not full**, which is the opposite
/// of what this read on the way in. Full range is claimed only where the
/// container claims it, because that is what the codecs say: `H.264` and
/// `HEVC` both default `video_full_range_flag` to 0, and a file that says
/// nothing is saying studio swing. Measured over the whole sample corpus,
/// 2026-08-09: every Insta360 capture, proxy and GoPro file is `yuvj420p` and
/// tagged `pc`, and every `.OSV` of both units is `yuv420p10le` tagged `tv`.
/// **Not one file in the corpus is untagged**, so this fallback picks nothing
/// that ships today and is written down because the next camera may be the
/// one that needs it.
///
/// # Safety
/// `parameters` must be a live `AVCodecParameters` of a video stream.
unsafe fn written(parameters: &ff::ffi::AVCodecParameters) -> Fallible<Samples> {
    use ff::ffi::{AVColorRange, AVColorSpace, AVPixelFormat};
    let is = |want: AVPixelFormat| parameters.format == want as i32;
    if is(AVPixelFormat::AV_PIX_FMT_P010BE) || is(AVPixelFormat::AV_PIX_FMT_YUV420P10BE) {
        return Err("this video's 10-bit samples are big endian, which Kjerag cannot read".into());
    }
    Ok(Samples {
        wide: is(AVPixelFormat::AV_PIX_FMT_P010LE) || is(AVPixelFormat::AV_PIX_FMT_YUV420P10LE),
        limited: parameters.color_range != AVColorRange::AVCOL_RANGE_JPEG,
        matrix: match parameters.color_space {
            AVColorSpace::AVCOL_SPC_BT709 => crate::ColorMatrix::Bt709,
            AVColorSpace::AVCOL_SPC_SMPTE170M | AVColorSpace::AVCOL_SPC_BT470BG => {
                crate::ColorMatrix::Bt601
            }
            // BT.709 is the compatibility fallback for an unspecified or
            // not-yet-supported matrix. This keeps drawing what the general
            // renderer drew before the matrix was metadata rather than
            // claiming a conversion we do not implement.
            _ => crate::ColorMatrix::Bt709,
        },
    })
}

/// What a file has to agree with its sibling about to be the other lens of
/// one capture, as plain numbers. Split out from [`Opened`] because the rule
/// below is the whole of trust-but-verify and a container is not needed to
/// state it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shape {
    lenses: usize,
    size: Size,
    rate: (i32, i32),
    time_base: (i32, i32),
    frames: u64,
    samples: Samples,
}

impl Shape {
    /// Whether two files are two lenses of one capture rather than two files
    /// that happen to be named alike.
    ///
    /// The name has already said they belong together; this is the verifying
    /// half, and it is deliberately about the pictures rather than about the
    /// trailer, because the second file of an X2 pair **has no trailer** to
    /// check (`kjerag_meta::pair`). Two lenses of one capture are one video
    /// stream each, the same size, the same rate, the same time base and the
    /// same length. Measured on all three X2 pairs on this box: they agree on
    /// every one of those, and their frame counts are exactly one apart.
    ///
    /// A file that fails any of it is not refused, it is left out: the
    /// capture opens with the one lens it named, which is what the player did
    /// before issue #79 and is never worse than it.
    fn pairs_with(self, other: Self) -> bool {
        self.lenses == 1
            && other.lenses == 1
            && self.size == other.size
            && self.rate == other.rate
            && self.time_base == other.time_base
            && self.frames.abs_diff(other.frames) <= 1
            && self.samples == other.samples
    }
}

impl Opened {
    /// Discover one capture from a named file and any files selected beside it.
    pub(super) fn discover(path: &Path, alongside: &[PathBuf]) -> Fallible<Vec<Self>> {
        let named = Self::new(path)?;
        let sources = match partner(path, &named, alongside) {
            // In lens order, which is not the order they were asked for:
            // opening the `_10_` file has to deliver lens 1 second all the
            // same, or every lens the shader reprojects is the other one's
            // and the sphere comes out inside out.
            Some(beside) => match kjerag_meta::lens_index(path) {
                Some(1) => vec![beside, named],
                _ => vec![named, beside],
            },
            None => vec![named],
        };
        Ok(sources)
    }

    /// Open an explicitly selected two-file capture in lens order.
    pub(super) fn pair(first: &Path, second: &Path) -> Fallible<Vec<Self>> {
        let first = Self::new(first)?;
        let second = Self::new(second)?;
        let shape = first.shape().ok_or("first file has no video stream")?;
        if !second
            .shape()
            .is_some_and(|second| shape.pairs_with(second))
        {
            return Err("the explicitly selected files are not two lenses of one capture".into());
        }
        Ok(vec![first, second])
    }

    pub(super) fn new(path: &Path) -> Fallible<Self> {
        let mut input = ff::format::input(&path)?;
        let videos: Vec<Video> = input
            .streams()
            // The cover an Osmo attaches to its container is not a lens, and
            // taken as a third one it fails the open on a time base nobody
            // set: a still has no frame rate to agree with the pictures about
            // ([`super::is_lens`]).
            .filter(super::is_lens)
            .map(|s| {
                // `Parameters` hands out no accessors, and opening a decoder
                // to read two integers before deciding whether this file is
                // even wanted is worse than reading the integers. The same
                // reach `sound_rate` makes, for the same reason.
                let (width, height, samples) = unsafe {
                    let p = *s.parameters().as_ptr();
                    (p.width.max(0) as u32, p.height.max(0) as u32, written(&p)?)
                };
                Ok(Video {
                    stream: s.index(),
                    rate: s.avg_frame_rate(),
                    frames: s.frames().max(0) as u64,
                    size: Size::new(width, height),
                    samples,
                })
            })
            .collect::<Fallible<Vec<Video>>>()?;
        let first = videos.first().ok_or("file has no video stream")?;
        let time_base = input
            .stream(first.stream)
            .ok_or("the video stream went away")?
            .time_base();
        let starts: Vec<i64> = videos
            .iter()
            .filter_map(|video| input.stream(video.stream))
            .map(|s| s.start_time())
            .collect();
        if videos
            .iter()
            .filter_map(|video| input.stream(video.stream))
            .any(|s| s.time_base() != time_base)
        {
            return Err("video streams disagree about their time base".into());
        }
        let start = starts.first().copied().unwrap_or(0);
        // The pictures, and nothing else. The sound of this file is read on a
        // demuxer of its own (issue #97), and leaving it wanted here would
        // have this one seeking across the file for packets nobody takes.
        let wanted: Vec<usize> = videos.iter().map(|video| video.stream).collect();
        read_only(&mut input, &wanted);
        Ok(Self {
            path: path.to_owned(),
            input,
            videos,
            time_base,
            start: match start == ff::ffi::AV_NOPTS_VALUE {
                true => 0,
                false => start,
            },
        })
    }

    /// This file as the numbers [`Shape::pairs_with`] compares. `None` for a
    /// file with no video stream in it, which cannot be a lens of anything.
    fn shape(&self) -> Option<Shape> {
        let first = self.videos.first()?;
        let pair = |r: ff::Rational| (r.numerator(), r.denominator());
        Some(Shape {
            lenses: self.videos.len(),
            size: first.size,
            rate: pair(first.rate),
            time_base: pair(self.time_base),
            frames: first.frames,
            samples: first.samples,
        })
    }
}

pub(super) fn agreed_samples<'a>(
    mut videos: impl Iterator<Item = (usize, &'a Video)>,
) -> Fallible<Samples> {
    let (first_source, first) = videos.next().ok_or("file has no video stream")?;
    if let Some((source, video)) = videos.find(|(_, video)| video.samples != first.samples) {
        return Err(format!(
            "video stream {} of file {} is {}, but video stream {} of file {} is {}",
            video.stream,
            source,
            sample_description(video.samples),
            first.stream,
            first_source,
            sample_description(first.samples)
        )
        .into());
    }
    Ok(first.samples)
}

fn sample_description(samples: Samples) -> String {
    let depth = if samples.wide { "10-bit" } else { "8-bit" };
    let range = if samples.limited {
        "studio swing"
    } else {
        "full range"
    };
    let matrix = match samples.matrix {
        crate::ColorMatrix::Bt709 => "BT.709",
        crate::ColorMatrix::Bt601 => "BT.601",
    };
    format!("{depth}, {range}, {matrix}")
}

/// The file holding this capture's other lens, opened and checked, or `None`
/// for a capture that is one file.
///
/// The lookup only happens for a container that decodes a **single** lens, so
/// an X4-class `.insv` never touches the filesystem for it and its open path
/// is what it always was.
fn partner(path: &Path, first: &Opened, alongside: &[PathBuf]) -> Option<Opened> {
    let shape = first.shape()?;
    if shape.lenses != 1 {
        return None;
    }
    let beside: PathBuf = kjerag_meta::capture::mate_among(path, alongside)
        .map(Path::to_path_buf)
        .or_else(|| kjerag_meta::sibling(path))?;
    let second = match Opened::new(&beside) {
        Ok(second) => second,
        Err(e) => {
            eprintln!(
                "kjerag: {} is not readable, one lens only: {e}",
                beside.display()
            );
            return None;
        }
    };
    if !second
        .shape()
        .is_some_and(|beside| shape.pairs_with(beside))
    {
        eprintln!(
            "kjerag: {} is not this capture's other lens, one lens only",
            beside.display()
        );
        return None;
    }
    Some(second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters(
        format: ff::ffi::AVPixelFormat,
        range: ff::ffi::AVColorRange,
        space: ff::ffi::AVColorSpace,
    ) -> ff::ffi::AVCodecParameters {
        // All fields `written` does not read are inert here. The C struct has
        // no Rust constructor because libavcodec normally allocates it.
        let mut parameters: ff::ffi::AVCodecParameters = unsafe { std::mem::zeroed() };
        parameters.format = format as i32;
        parameters.color_range = range;
        parameters.color_space = space;
        parameters
    }

    #[test]
    fn sample_matrix_is_container_metadata_not_depth_or_range() {
        use ff::ffi::{AVColorRange as Range, AVColorSpace as Matrix, AVPixelFormat as Format};

        let bt709 = unsafe {
            written(&parameters(
                Format::AV_PIX_FMT_YUV420P,
                Range::AVCOL_RANGE_JPEG,
                Matrix::AVCOL_SPC_BT709,
            ))
            .unwrap()
        };
        assert_eq!(
            bt709,
            Samples {
                wide: false,
                limited: false,
                matrix: crate::ColorMatrix::Bt709,
            }
        );

        let bt601 = unsafe {
            written(&parameters(
                Format::AV_PIX_FMT_P010LE,
                Range::AVCOL_RANGE_MPEG,
                Matrix::AVCOL_SPC_SMPTE170M,
            ))
            .unwrap()
        };
        assert_eq!(
            bt601,
            Samples {
                wide: true,
                limited: true,
                matrix: crate::ColorMatrix::Bt601,
            }
        );

        let bt470 = unsafe {
            written(&parameters(
                Format::AV_PIX_FMT_YUV420P,
                Range::AVCOL_RANGE_JPEG,
                Matrix::AVCOL_SPC_BT470BG,
            ))
            .unwrap()
        };
        assert_eq!(bt470.matrix, crate::ColorMatrix::Bt601);
    }

    #[test]
    fn unspecified_or_unsupported_matrix_keeps_the_bt709_compatibility_default() {
        use ff::ffi::{AVColorRange as Range, AVColorSpace as Matrix, AVPixelFormat as Format};

        for matrix in [Matrix::AVCOL_SPC_UNSPECIFIED, Matrix::AVCOL_SPC_BT2020_NCL] {
            let samples = unsafe {
                written(&parameters(
                    Format::AV_PIX_FMT_YUV420P,
                    Range::AVCOL_RANGE_JPEG,
                    matrix,
                ))
                .unwrap()
            };
            assert_eq!(samples.matrix, crate::ColorMatrix::Bt709);
        }
    }

    /// One lens of a ONE X2 pair, as the container describes it: 2880 square,
    /// 30000/1001, time base 1/30000. The real numbers off
    /// `VID_20000101_100000_00_001.insv`.
    fn x2_lens(frames: u64) -> Shape {
        Shape {
            lenses: 1,
            size: Size::new(2880, 2880),
            rate: (30000, 1001),
            time_base: (1, 30000),
            frames,
            samples: Samples {
                matrix: crate::ColorMatrix::Bt601,
                ..Samples::default()
            },
        }
    }

    /// The pair the naming found is accepted when the pictures agree, and the
    /// one frame the two files differ by is inside the rule rather than
    /// outside it: all three X2 pairs on this box have lens 0 running exactly
    /// one frame longer.
    #[test]
    fn the_two_files_of_a_capture_agree_about_everything_but_their_length() {
        assert!(x2_lens(2516).pairs_with(x2_lens(2515)));
        assert!(x2_lens(2515).pairs_with(x2_lens(2516)));
        assert!(x2_lens(8204).pairs_with(x2_lens(8203)));
        assert!(x2_lens(2516).pairs_with(x2_lens(2516)));
    }

    /// And a file that disagrees is left out rather than refused. Each of
    /// these is a way the naming could find the wrong file: another camera's
    /// clip, a different mode, a stitched export, or a clip of another
    /// length entirely.
    #[test]
    fn a_file_that_does_not_match_is_not_this_capture_s_other_lens() {
        let lens = x2_lens(2516);

        assert!(!lens.pairs_with(Shape {
            size: Size::new(3840, 3840),
            ..x2_lens(2516)
        }));
        assert!(!lens.pairs_with(Shape {
            rate: (60000, 1001),
            ..x2_lens(2516)
        }));
        assert!(!lens.pairs_with(Shape {
            time_base: (1, 90000),
            ..x2_lens(2516)
        }));
        assert!(!lens.pairs_with(x2_lens(2600)));
        assert!(!lens.pairs_with(Shape {
            samples: Samples {
                matrix: crate::ColorMatrix::Bt709,
                ..Samples::default()
            },
            ..x2_lens(2516)
        }));
        // An X4-class file, which carries both lenses itself: neither side of
        // this is ever half a capture.
        let both = Shape {
            lenses: 2,
            size: Size::new(3840, 3840),
            ..x2_lens(4546)
        };
        assert!(!both.pairs_with(x2_lens(4546)));
        assert!(!x2_lens(4546).pairs_with(both));
    }

    #[test]
    fn every_selected_lens_stream_agrees_on_complete_sample_metadata() {
        let video = |stream, samples| Video {
            stream,
            rate: ff::Rational::new(30000, 1001),
            frames: 100,
            size: Size::new(3840, 3840),
            samples,
        };
        let first = video(0, Samples::default());
        let same = video(1, Samples::default());
        assert_eq!(
            agreed_samples([(0, &first), (0, &same)].into_iter()).unwrap(),
            Samples::default()
        );

        for different in [
            Samples {
                wide: true,
                ..Samples::default()
            },
            Samples {
                limited: true,
                ..Samples::default()
            },
            Samples {
                matrix: crate::ColorMatrix::Bt601,
                ..Samples::default()
            },
        ] {
            let different = video(1, different);
            assert!(agreed_samples([(0, &first), (0, &different)].into_iter()).is_err());
        }

        let different = video(
            1,
            Samples {
                matrix: crate::ColorMatrix::Bt601,
                ..Samples::default()
            },
        );
        let error = agreed_samples([(0, &first), (0, &different)].into_iter())
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "video stream 1 of file 0 is 8-bit, full range, BT.601, but video stream 0 of file 0 is 8-bit, full range, BT.709"
        );
    }
}
