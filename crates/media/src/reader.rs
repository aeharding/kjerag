//! One demuxer, every video stream of the file, frames handed out in pairs.
//!
//! An `.insv` carries the two lenses as two HEVC streams of one MP4, and a
//! reframed view is a function of both of them at the same instant. So the
//! unit this layer delivers is not a frame, it is [`Frames`]: the same PTS
//! from every video stream, mapped and ready to import. A lens is never
//! delivered without its partner, which is how the two streams cannot drift.
//!
//! Reading is by [`Cue`] as well as forward, because pinning every consumer
//! to frame 0 is what issue #4's first comment asked us to stop doing: #8's
//! Studio-diff harness and #5's seek both need to name a frame.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use ffmpeg_next as ff;

use super::{DrmFrame, Fallible, HwDevice, Size, decode};

/// Which frame a caller wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Counting from the first frame of the file.
    Index(u64),
    /// Media time from the start of the file. Rounded to the nearest frame.
    Time(Duration),
}

/// The container's own timing, kept as the rational it is written as: the
/// rate is 30000/1001, and 29.97 is a display convenience. Rounding it to
/// 30 drifts a frame every 33 seconds, which is exactly the judder this
/// milestone exists to avoid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    rate_num: u64,
    rate_den: u64,
    /// Frames in one video stream, as the container's index reports it.
    pub frames: u64,
}

const NANOS: u64 = 1_000_000_000;

impl Timing {
    pub fn new(rate: ff::Rational, frames: u64) -> Fallible<Self> {
        let (num, den) = (rate.numerator(), rate.denominator());
        if num <= 0 || den <= 0 {
            return Err(format!("stream has no frame rate ({num}/{den})").into());
        }
        Ok(Self {
            rate_num: num as u64,
            rate_den: den as u64,
            frames,
        })
    }

    /// Time between two frames: 1001/30000 s, not 33 ms.
    pub fn interval(self) -> Duration {
        self.time_of(1)
    }

    pub fn time_of(self, index: u64) -> Duration {
        let nanos = u128::from(index) * u128::from(self.rate_den) * u128::from(NANOS)
            / u128::from(self.rate_num);
        Duration::from_nanos(nanos as u64)
    }

    /// The frame covering `at`, rounded to nearest so that a timestamp
    /// recovered from a frame's own PTS names that frame again.
    pub fn index_at(self, at: Duration) -> u64 {
        let ticks = at.as_nanos() * u128::from(self.rate_num);
        let per_frame = u128::from(self.rate_den) * u128::from(NANOS);
        ((ticks + per_frame / 2) / per_frame) as u64
    }

    pub fn duration(self) -> Duration {
        self.time_of(self.frames)
    }

    /// For reports only. The engine never paces on this.
    pub fn fps(self) -> f64 {
        self.rate_num as f64 / self.rate_den as f64
    }
}

/// One instant of the recording: every lens at the same PTS, mapped to
/// DRM_PRIME and ready for `kyerag_render::dmabuf::import`.
///
/// Dropping this returns the surfaces to the decoder's pools, so it has to
/// outlive every texture imported from it (`kyerag_render::ScenePipeline`
/// keeps a few alive behind the one it is drawing).
pub struct Frames {
    /// Counting from the first frame of the file, derived from the PTS.
    pub index: u64,
    /// Media time of this frame, from the container.
    pub timestamp: Duration,
    /// One per video stream, in stream order: lens 0, then lens 1.
    pub lenses: Vec<DrmFrame>,
    pub size: Size,
}

impl std::fmt::Debug for Frames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frames")
            .field("index", &self.index)
            .field("timestamp", &self.timestamp)
            .field("lenses", &self.lenses.len())
            .finish()
    }
}

/// One demuxer and one VA-API decoder per video stream.
pub struct Reader {
    input: ff::format::context::Input,
    lanes: Vec<Lane>,
    timing: Timing,
    size: Size,
    /// Stream time base, shared by both video streams (checked at open).
    time_base: ff::Rational,
    /// PTS of the first frame, which the container is free to start away
    /// from zero. Media time is measured from it.
    start: i64,
    lookahead: usize,
    skip_before: u64,
    drained: bool,
    /// Held so the device outlives the decoders that reference it.
    _hw: HwDevice,
}

/// One video stream: its decoder, and the frames it has decoded but not yet
/// been asked for.
struct Lane {
    stream: usize,
    decoder: ff::decoder::Video,
    queue: VecDeque<ff::frame::Video>,
}

impl Reader {
    /// Opens the file and both decoders. Cheap: this reads the MP4 index,
    /// not the video.
    pub fn open(path: &Path) -> Fallible<Self> {
        ff::init()?;
        let input = ff::format::input(&path)?;
        let hw = HwDevice::vaapi()?;

        let video: Vec<(usize, ff::Rational, ff::Rational, i64, i64)> = input
            .streams()
            .filter(|s| s.parameters().medium() == ff::media::Type::Video)
            .map(|s| {
                (
                    s.index(),
                    s.time_base(),
                    s.avg_frame_rate(),
                    s.frames(),
                    s.start_time(),
                )
            })
            .collect();
        let (first, time_base, rate, frames, start) =
            *video.first().ok_or("file has no video stream")?;
        if video.iter().any(|s| s.1 != time_base) {
            return Err("video streams disagree about their time base".into());
        }

        let mut lanes = Vec::with_capacity(video.len());
        for &(stream, ..) in &video {
            lanes.push(Lane {
                stream,
                decoder: decode::open_decoder(&input, stream, &hw)?,
                queue: VecDeque::new(),
            });
        }
        let size = Size::new(lanes[0].decoder.width(), lanes[0].decoder.height());
        if let Some(lane) = lanes
            .iter()
            .find(|l| Size::new(l.decoder.width(), l.decoder.height()) != size)
        {
            return Err(format!(
                "stream {} is {}x{}, but stream {first} is {}x{}",
                lane.stream,
                lane.decoder.width(),
                lane.decoder.height(),
                size.width,
                size.height
            )
            .into());
        }

        Ok(Self {
            input,
            lanes,
            timing: Timing::new(rate, frames.max(0) as u64)?,
            size,
            time_base,
            start: if start == ff::ffi::AV_NOPTS_VALUE {
                0
            } else {
                start
            },
            lookahead: 0,
            skip_before: 0,
            drained: false,
            _hw: hw,
        })
    }

    /// How many frames each lane decodes past a surface before that surface
    /// is mapped.
    ///
    /// `av_hwframe_map` waits for the decode of the frame it maps
    /// (`vaSyncSurface`, measured at 7.64 ms/frame in the M0 spike, which
    /// kept one frame in flight). Mapping the oldest queued frame instead of
    /// the newest means the wait has already been paid by the time we ask.
    /// docs/ARCHITECTURE.md: "keep 2-3 frames in flight".
    ///
    /// Zero, the default, is for the still-frame callers: it maps as soon as
    /// a frame exists.
    pub fn lookahead(mut self, frames: usize) -> Self {
        self.lookahead = frames;
        self
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// One per video stream: 2 for an `.insv` the camera wrote in one file,
    /// 1 for the older per-lens files.
    pub fn lenses(&self) -> usize {
        self.lanes.len()
    }

    /// Surfaces in one lane's frame pool, which is the ceiling on how many
    /// decoded frames the engine may hold at once. `None` until the first
    /// frame has been decoded: ffmpeg builds the pool when the decoder first
    /// picks a hardware format, not when it is opened.
    pub fn pool_size(&self) -> Option<i32> {
        decode::pool_size(&self.lanes[0].decoder)
    }

    /// The next complete set of lenses, or `None` at the end of the file.
    pub fn next_frames(&mut self) -> Fallible<Option<Frames>> {
        loop {
            if let Some(frames) = self.take()? {
                return Ok(Some(frames));
            }
            if self.drained {
                return Ok(None);
            }
            self.pump()?;
        }
    }

    /// The frame a [`Cue`] names, wherever it is in the file: seek to the
    /// keyframe at or before it, then decode forward. Frames on the way are
    /// dropped without being mapped, so the walk costs decode and no waiting.
    pub fn frame(&mut self, at: Cue) -> Fallible<Frames> {
        let index = self.index_of(at);
        self.seek(at)?;
        self.next_frames()?
            .ok_or_else(|| format!("file ended before frame {index}").into())
    }

    /// Positions the reader so that the next [`Reader::next`] returns the
    /// frame `at` names. #5 builds the keyframe index and the scrub UX on
    /// this; the walk here is the plain one.
    pub fn seek(&mut self, at: Cue) -> Fallible<()> {
        let index = self.index_of(at);
        // Stream index -1 means the timestamp is in AV_TIME_BASE units,
        // which is microseconds, and `..ts` asks for the keyframe at or
        // before it. GOP is 1.001 s on this camera, so the walk that
        // follows is at most 30 frames (docs/research/gpu-pipeline.md 7).
        let target = self.timing.time_of(index).as_micros() as i64;
        self.input.seek(target, ..target)?;
        for lane in &mut self.lanes {
            lane.decoder.flush();
            lane.queue.clear();
        }
        self.skip_before = index;
        self.drained = false;
        Ok(())
    }

    fn index_of(&self, at: Cue) -> u64 {
        match at {
            Cue::Index(index) => index,
            Cue::Time(time) => self.timing.index_at(time),
        }
    }

    /// Reads one packet and decodes whatever it completes.
    fn pump(&mut self) -> Fallible<()> {
        let mut packet = ff::Packet::empty();
        match packet.read(&mut self.input) {
            Ok(()) => {
                let Some(lane) = self.lanes.iter_mut().find(|l| l.stream == packet.stream()) else {
                    return Ok(());
                };
                lane.decoder.send_packet(&packet)?;
                lane.drain()
            }
            Err(ff::Error::Eof) => {
                self.drained = true;
                for lane in &mut self.lanes {
                    lane.decoder.send_eof()?;
                    lane.drain()?;
                }
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The head of every lane, if they agree on a PTS and there is enough
    /// behind them to have paid for the map.
    fn take(&mut self) -> Fallible<Option<Frames>> {
        loop {
            self.align();
            let Some(pts) = self.ready()? else {
                return Ok(None);
            };
            let timestamp = self.media_time(pts);
            let index = self.timing.index_at(timestamp);
            if index >= self.skip_before {
                let mut lenses = Vec::with_capacity(self.lanes.len());
                for lane in &mut self.lanes {
                    let frame = lane.queue.pop_front().ok_or("lane emptied under us")?;
                    lenses.push(DrmFrame::map(&frame)?);
                }
                return Ok(Some(Frames {
                    index,
                    timestamp,
                    lenses,
                    size: self.size,
                }));
            }
            // Decoded on the way to a cue. Dropping it here, before the map,
            // is what keeps a seek from paying the sync wait per frame.
            for lane in &mut self.lanes {
                lane.queue.pop_front();
            }
        }
    }

    /// The PTS every lane's head agrees on, or `None` if the queues are not
    /// deep enough yet.
    fn ready(&self) -> Fallible<Option<i64>> {
        // At the end of the file there is nothing left to hide the map
        // behind, so the last frames are mapped as they come.
        let depth = if self.drained { 1 } else { self.lookahead + 1 };
        if self.lanes.iter().any(|lane| lane.queue.len() < depth) {
            return Ok(None);
        }
        let heads: Vec<i64> = self.lanes.iter().filter_map(Lane::head).collect();
        if heads.len() != self.lanes.len() {
            return Err("a decoded frame has no timestamp".into());
        }
        match heads.iter().all(|pts| *pts == heads[0]) {
            true => Ok(Some(heads[0])),
            false => Ok(None),
        }
    }

    /// Drops heads that have no partner. Both lenses are recorded by one
    /// camera at one rate, so this should never fire; it exists so that a
    /// file where it does fire loses a frame instead of pairing lens 0 with
    /// a different instant of lens 1.
    fn align(&mut self) {
        loop {
            let heads: Vec<i64> = self.lanes.iter().filter_map(Lane::head).collect();
            if heads.len() != self.lanes.len() {
                return;
            }
            let Some(&newest) = heads.iter().max() else {
                return;
            };
            if heads.iter().all(|pts| *pts == newest) {
                return;
            }
            for lane in &mut self.lanes {
                if lane.head().is_some_and(|pts| pts < newest) {
                    lane.queue.pop_front();
                }
            }
        }
    }

    fn media_time(&self, pts: i64) -> Duration {
        let ticks = pts.saturating_sub(self.start).max(0) as u128;
        let nanos = ticks * self.time_base.numerator() as u128 * u128::from(NANOS)
            / self.time_base.denominator() as u128;
        Duration::from_nanos(nanos as u64)
    }
}

impl Lane {
    fn head(&self) -> Option<i64> {
        self.queue.front()?.timestamp()
    }

    fn drain(&mut self) -> Fallible<()> {
        loop {
            let mut frame = ff::frame::Video::empty();
            if self.decoder.receive_frame(&mut frame).is_err() {
                return Ok(());
            }
            self.queue.push_back(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ntsc() -> Timing {
        Timing::new(ff::Rational::new(30000, 1001), 53940).unwrap()
    }

    #[test]
    fn the_rate_is_the_container_s_rational_not_thirty() {
        let timing = ntsc();
        assert_eq!(timing.interval(), Duration::from_nanos(33_366_666));
        assert_eq!(timing.time_of(30), Duration::from_nanos(1_001_000_000));
        // Where rounding to 30 fps would already be a frame out.
        assert_eq!(timing.index_at(Duration::from_secs(30)), 899);
    }

    #[test]
    fn a_frame_s_own_timestamp_names_it_again() {
        let timing = ntsc();
        for index in [0, 1, 2, 29, 30, 1799, 53939] {
            assert_eq!(timing.index_at(timing.time_of(index)), index);
        }
    }

    #[test]
    fn a_time_between_frames_names_the_nearest() {
        let timing = ntsc();
        let half = timing.interval() / 2;
        assert_eq!(timing.index_at(timing.time_of(10) + half / 2), 10);
        assert_eq!(timing.index_at(timing.time_of(10) + half + half / 2), 11);
    }

    #[test]
    fn duration_is_the_frame_count_at_the_real_rate() {
        // 53940 frames of 1001/30000 s is 1799.798 s, which is what ffprobe
        // reports for the fixture file.
        assert_eq!(ntsc().duration(), Duration::from_nanos(1_799_798_000_000));
    }

    #[test]
    fn a_stream_with_no_rate_is_an_error() {
        assert!(Timing::new(ff::Rational::new(0, 0), 0).is_err());
    }
}
