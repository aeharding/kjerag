//! Every video stream of a capture, frames handed out in pairs.
//!
//! An X4-class `.insv` carries the two lenses as two HEVC streams of one MP4,
//! and a reframed view is a function of both of them at the same instant. So
//! the unit this layer delivers is not a frame, it is [`Frames`]: the same
//! instant from every lens, mapped and ready to import. A lens is never
//! delivered without its partner, which is how the two cannot drift.
//!
//! **A capture is not always one file (issue #79).** The ONE X2 and the
//! models before it write one lens per file, so the same invariant has to
//! hold across two containers: [`Reader::open`] finds the sibling
//! (`kjerag_meta::sibling`), opens a demuxer for each, and pumps whichever
//! one is behind. Two files of one capture share a frame grid exactly -
//! measured on all three X2 pairs on this box, both files carry `time_base`
//! 1/30000, a `start_time` of 0 and the identical PTS series 0, 1001,
//! 2002, ... - so there is no drift between them to correct and nothing to
//! resample. What they do not share is their **length**: the lens 0 file
//! runs exactly one frame longer in all three pairs, and a frame with no
//! partner is dropped rather than shown as half a sphere.
//!
//! Lanes are matched on frame index rather than on raw PTS for the same
//! reason: each file carries its own `start_time`, and media time measured
//! from it is the one number that means the same thing in both timelines.
//!
//! Reading is by [`Cue`] as well as forward, because pinning every consumer
//! to frame 0 is what issue #4's first comment asked us to stop doing: #8's
//! Studio-diff harness and #5's seek both need to name a frame.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ffmpeg_next as ff;

use super::sound::Sound;
use super::track::Track;
use super::{DrmFrame, Fallible, HwDevice, NANOS, Size, decode, media_time, read_only};

/// Which frame a caller wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Counting from the first frame of the file.
    Index(u64),
    /// Media time from the start of the file. Rounded to the nearest frame.
    Time(Duration),
}

impl Cue {
    pub fn index(self, timing: Timing) -> u64 {
        match self {
            Self::Index(index) => index,
            Self::Time(time) => timing.index_at(time),
        }
    }

    pub fn time(self, timing: Timing) -> Duration {
        timing.time_of(self.index(timing))
    }
}

/// How exact a seek has to be, which is the whole difference between a scrub
/// and a landing (issue #5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accuracy {
    /// The keyframe at or before the cue: one decode per lens, wherever in
    /// the file it is, and a picture within a second of what was asked for.
    /// What a slider being dragged asks for.
    Keyframe,
    /// The frame itself: that keyframe, and then every frame between it and
    /// the cue, decoded and dropped without being mapped.
    Exact,
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

/// What a read stopped on.
#[derive(Debug)]
pub enum Read {
    /// A complete set of lenses.
    Frames(Frames),
    /// The end of the file.
    Ended,
    /// The interrupt asked for the read to stop. Nothing was thrown away:
    /// the lanes keep everything they have decoded, so a read that is not
    /// followed by a seek carries on from where this one stopped.
    Interrupted,
}

/// One instant of the recording: every lens at the same PTS, mapped to
/// DRM_PRIME and ready for `kjerag_render::dmabuf::import`.
///
/// Dropping this returns the surfaces to the decoder's pools, so it has to
/// outlive every texture imported from it (`kjerag_render::ScenePipeline`
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

/// One demuxer per file of the capture, and one VA-API decoder per video
/// stream of each.
pub struct Reader {
    /// One file each. Two only for the cameras that write one lens per file;
    /// everything else opens exactly one and reads it as it always did.
    sources: Vec<Source>,
    lanes: Vec<Lane>,
    /// The capture's sound, when it has one and a device took it. It carries
    /// a demuxer of its own over [`SOUND_SOURCE`]'s file: a real capture's
    /// interleave will not let it share one, and the three seconds of silence
    /// that proved it are issue #97 ([`Track`]).
    track: Option<Track>,
    timing: Timing,
    size: Size,
    lookahead: usize,
    skip_before: u64,
    /// Set from a seek until the frame it landed on has been handed over.
    /// The lookahead is a pipeline: it costs the frames in it before the
    /// first picture comes out, which is 24 ms of a 46 ms scrub on this
    /// camera. A seek gives that up once and fills the pipeline behind it.
    landing: bool,
    /// Held so the device outlives the decoders that reference it.
    _hw: HwDevice,
}

/// Which source the sound is taken from.
///
/// Both files of an X2 pair carry an AAC stream of the same length, and they
/// are two recordings of the same moment rather than two halves of one, so
/// one of them is the sound and the other is skipped. Lens 0's is the one
/// kept, for the same reason its trailer is the capture's: it is the file
/// the camera writes everything else in.
const SOUND_SOURCE: usize = 0;

/// One file: its demuxer, its own timeline, and whether it has been read to
/// the end.
struct Source {
    /// Kept because the sound opens the same file again, for its own demuxer
    /// ([`Track::open`]).
    path: PathBuf,
    input: ff::format::context::Input,
    /// Stream time base, shared by every video stream of this file (checked
    /// at open, and checked between files when there are two).
    time_base: ff::Rational,
    /// PTS of the first frame, which a container is free to put anywhere.
    /// Media time is measured from it, per file.
    start: i64,
    drained: bool,
}

/// One video stream: which file and stream it is, its decoder, and the frames
/// it has decoded but not yet been asked for.
struct Lane {
    source: usize,
    stream: usize,
    decoder: ff::decoder::Video,
    queue: VecDeque<ff::frame::Video>,
}

/// A container opened and looked at, before any decoder exists: what
/// [`Reader::open`] needs to decide whether a second file belongs with it.
struct Opened {
    path: PathBuf,
    input: ff::format::context::Input,
    /// One per video stream, in container order.
    videos: Vec<Video>,
    time_base: ff::Rational,
    start: i64,
}

#[derive(Clone, Copy)]
struct Video {
    stream: usize,
    rate: ff::Rational,
    frames: u64,
    size: Size,
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
    }
}

impl Reader {
    /// Opens the capture and every decoder it needs. Cheap: this reads the
    /// MP4 index, not the video.
    ///
    /// One file is the usual case. A file that decodes a single lens is
    /// looked up against its sibling first (issue #79), and a pair that
    /// checks out is read as one source of two lenses.
    pub fn open(path: &Path) -> Fallible<Self> {
        ff::init()?;
        let hw = HwDevice::vaapi()?;
        let named = Opened::new(path)?;
        let sources = match partner(path, &named) {
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
        Self::over(sources, hw)
    }

    /// One decoder per video stream of every source, and the timing the
    /// whole capture is read on.
    fn over(sources: Vec<Opened>, hw: HwDevice) -> Fallible<Self> {
        let mut lanes = Vec::new();
        for (source, opened) in sources.iter().enumerate() {
            for video in &opened.videos {
                lanes.push(Lane {
                    source,
                    stream: video.stream,
                    decoder: decode::open_decoder(&opened.input, video.stream, &hw)?,
                    queue: VecDeque::new(),
                });
            }
        }
        let size = Size::new(lanes[0].decoder.width(), lanes[0].decoder.height());
        if let Some(lane) = lanes
            .iter()
            .find(|l| Size::new(l.decoder.width(), l.decoder.height()) != size)
        {
            return Err(format!(
                "lens {}, stream {} of file {}, is {}x{}, but the first lens is {}x{}",
                lanes.len(),
                lane.stream,
                lane.source,
                lane.decoder.width(),
                lane.decoder.height(),
                size.width,
                size.height
            )
            .into());
        }

        let videos = || sources.iter().flat_map(|opened| opened.videos.iter());
        let rate = videos().next().ok_or("file has no video stream")?.rate;
        // The shortest lens is what the capture is: a frame the other lenses
        // cannot match is a frame that would be shown as half a sphere. The
        // X2 pairs on this box are one frame apart, always in lens 0's
        // favour.
        let frames = videos().map(|video| video.frames).min().unwrap_or(0);

        Ok(Self {
            sources: sources.into_iter().map(Opened::into_source).collect(),
            lanes,
            track: None,
            timing: Timing::new(rate, frames)?,
            size,
            lookahead: 0,
            skip_before: 0,
            landing: false,
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

    /// Decode this capture's sound as well, into `sound`'s ring (issue #13).
    ///
    /// A file with no audio stream takes this and stays silent. What it costs
    /// is a second open of [`SOUND_SOURCE`]'s file, because the sound is read
    /// on a demuxer of its own (issue #97, [`Track`]).
    pub fn listen(mut self, sound: &Sound) -> Fallible<Self> {
        let Some(source) = self.sources.get(SOUND_SOURCE) else {
            return Ok(self);
        };
        self.track = Track::open(&source.path, sound.pipe(), sound.rate(), sound.channels())?;
        Ok(self)
    }

    /// The sample rate of the capture's audio stream, or `None` for one with
    /// no sound in it. Read before a device is opened, so the device can be
    /// asked for the rate that needs no resampling.
    pub fn sound_rate(&self) -> Option<u32> {
        let stream = self
            .sources
            .get(SOUND_SOURCE)?
            .input
            .streams()
            .find(|s| s.parameters().medium() == ff::media::Type::Audio)?;
        // `Parameters` hands out no accessors, and opening a second decoder to
        // read one integer is worse than reading the integer.
        let rate = unsafe { (*stream.parameters().as_ptr()).sample_rate };
        u32::try_from(rate).ok().filter(|rate| *rate > 0)
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// One per video stream of every file: 2 for an `.insv` the camera wrote
    /// in one file, 2 for a paired per-lens capture, and 1 for a per-lens
    /// file whose sibling is not on the card.
    pub fn lenses(&self) -> usize {
        self.lanes.len()
    }

    /// How many files this capture was opened from: 1 for everything but a
    /// paired per-lens capture, which is 2. For the report line only.
    pub fn files(&self) -> usize {
        self.sources.len()
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
        match self.read_until(|| false)? {
            Read::Frames(frames) => Ok(Some(frames)),
            // A read that never asks to stop only stops at the end.
            Read::Ended | Read::Interrupted => Ok(None),
        }
    }

    /// The next complete set of lenses, giving up if `interrupted` says the
    /// work has been overtaken.
    ///
    /// This is what lets a scrub abandon the lookahead refill it started
    /// behind the last landing: three pair decodes, and 38 ms of the 59 ms a
    /// scrub update used to cost (issue #37's table, issue #46's fix), spent
    /// on pictures the pilot has already dragged past.
    ///
    /// Reading is a run of packet reads and the caller only gets a say
    /// between them, so a read is given up one packet after the interrupt
    /// turns true rather than at once. What that grain costs is inside the
    /// 26 ms a scrub now takes through the [`super::Player`] against this
    /// reader's own 21.
    pub fn read_until(&mut self, mut interrupted: impl FnMut() -> bool) -> Fallible<Read> {
        loop {
            // Before the pictures, and on the way past every one of them: the
            // sound reads on its own demuxer now, and this is what turns it
            // (issue #97). Once per pair is the slowest this can happen, and
            // a pair is 33 ms against a ring holding 500.
            if let Some(track) = &mut self.track {
                track.pump()?;
            }
            if let Some(frames) = self.take()? {
                return Ok(Read::Frames(frames));
            }
            if self.drained() {
                return Ok(Read::Ended);
            }
            if interrupted() {
                return Ok(Read::Interrupted);
            }
            self.pump()?;
        }
    }

    /// Whether every file has been read to its end. The lens 0 file of an X2
    /// pair is a frame longer than its partner, so one source runs out first
    /// and the read carries on until the other does too; the frame left over
    /// has no partner and never becomes a [`Frames`].
    fn drained(&self) -> bool {
        self.sources.iter().all(|source| source.drained)
    }

    /// The frame a [`Cue`] names, wherever it is in the file: seek to the
    /// keyframe at or before it, then decode forward. Frames on the way are
    /// dropped without being mapped, so the walk costs decode and no waiting.
    pub fn frame(&mut self, at: Cue) -> Fallible<Frames> {
        self.seek(at, Accuracy::Exact)?;
        self.next_frames()?
            .ok_or_else(|| format!("file ended before frame {}", at.index(self.timing)).into())
    }

    /// Positions the reader so that the next [`Reader::next_frames`] returns
    /// the frame `at` names, or the keyframe at or before it.
    ///
    /// There is no keyframe index to build here, which is what issue #5
    /// expected: libavformat parses the whole of `stss`/`stco` out of `moov`
    /// when the file is opened, so the index already exists in memory and
    /// `av_seek_frame` is a lookup in it. That is why the cost of a seek does
    /// not depend on where in a 36 GB file it lands. Building a second copy
    /// of that table would buy nothing;
    /// `cargo run --release -p kjerag-spike --bin seek` is the measurement.
    pub fn seek(&mut self, at: Cue, accuracy: Accuracy) -> Fallible<()> {
        let index = at.index(self.timing);
        // Stream index -1 means the timestamp is in AV_TIME_BASE units,
        // which is microseconds, and `..ts` asks for the keyframe at or
        // before it. GOP is 1.001 s on this camera, so the walk that
        // follows is at most 30 frames (docs/research/gpu-pipeline.md 7).
        let target = self.timing.time_of(index).as_micros() as i64;
        // Every file of the capture goes to the same media time. They share a
        // frame grid exactly, so this lands both of them on the same frame.
        for source in &mut self.sources {
            source.input.seek(target, ..target)?;
            source.drained = false;
        }
        for lane in &mut self.lanes {
            lane.decoder.flush();
            lane.queue.clear();
        }
        // The sound goes with them, on its own demuxer and to the same media
        // time. Everything already decoded is from before the seek, and a
        // scrub that leaves a tail of it playing is the thing the epoch
        // discipline exists to stop.
        if let Some(track) = &mut self.track {
            track.seek(target)?;
        }
        self.skip_before = match accuracy {
            Accuracy::Exact => index,
            // Nothing to walk to: the picture is whatever the seek landed on.
            Accuracy::Keyframe => 0,
        };
        self.landing = true;
        Ok(())
    }

    /// Reads one packet from the file that is furthest behind, and decodes
    /// whatever it completes.
    ///
    /// Which file is behind is what keeps a pair in step: reading them turn
    /// and turn about would work too, but a file whose packets are laid out
    /// differently would run ahead and the queue in front of it would grow
    /// without bound. The lane with the fewest decoded frames is the one
    /// holding [`Self::ready`] up, so its file is the one to read.
    fn pump(&mut self) -> Fallible<()> {
        let source = self.hungriest();
        let mut packet = ff::Packet::empty();
        match packet.read(&mut self.sources[source].input) {
            Ok(()) => {
                let stream = packet.stream();
                let Some(lane) = self
                    .lanes
                    .iter_mut()
                    .find(|l| (l.source, l.stream) == (source, stream))
                else {
                    return Ok(());
                };
                lane.decoder.send_packet(&packet)?;
                lane.drain()
            }
            Err(ff::Error::Eof) => {
                self.sources[source].drained = true;
                for lane in self.lanes.iter_mut().filter(|l| l.source == source) {
                    lane.decoder.send_eof()?;
                    lane.drain()?;
                }
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The file with the fewest decoded frames waiting, skipping the ones
    /// already read to the end. Source 0 when there is only one, which is
    /// every capture that is one file.
    fn hungriest(&self) -> usize {
        let queued = |source: usize| {
            self.lanes
                .iter()
                .filter(|lane| lane.source == source)
                .map(|lane| lane.queue.len())
                .min()
                .unwrap_or(0)
        };
        (0..self.sources.len())
            .filter(|source| !self.sources[*source].drained)
            .min_by_key(|source| queued(*source))
            .unwrap_or(0)
    }

    /// The head of every lane, if they agree on a frame and there is enough
    /// behind them to have paid for the map.
    fn take(&mut self) -> Fallible<Option<Frames>> {
        loop {
            self.align();
            let Some(index) = self.ready()? else {
                return Ok(None);
            };
            // The first lens's own media time rather than the grid's. The
            // index is what pairs the lanes; the container's PTS is what
            // paces playback, and it is unchanged by the pairing.
            let timestamp = self.head_time().unwrap_or_default();
            if index >= self.skip_before {
                self.landing = false;
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

    /// The frame every lane's head agrees on, or `None` if the queues are
    /// not deep enough yet.
    ///
    /// A frame **index** rather than a PTS, because two files of one capture
    /// carry their own `start_time` and a raw PTS means nothing across them.
    /// Within one file the two are the same question asked twice: the
    /// container writes a PTS per frame on its own grid, so equal indices
    /// there are equal timestamps.
    fn ready(&self) -> Fallible<Option<u64>> {
        // At the end of the file there is nothing left to hide the map
        // behind, and right after a seek there is nobody to hide it from:
        // both hand frames over as they come.
        let depth = if self.drained() || self.landing {
            1
        } else {
            self.lookahead + 1
        };
        if self.lanes.iter().any(|lane| lane.queue.len() < depth) {
            return Ok(None);
        }
        let heads: Vec<u64> = self.lanes.iter().filter_map(|lane| self.at(lane)).collect();
        if heads.len() != self.lanes.len() {
            return Err("a decoded frame has no timestamp".into());
        }
        match heads.iter().all(|index| *index == heads[0]) {
            true => Ok(Some(heads[0])),
            false => Ok(None),
        }
    }

    /// Drops heads that have no partner, so a lens is never handed over
    /// paired with a different instant of the other one.
    ///
    /// Inside one file this should never fire: both lenses are recorded by
    /// one camera at one rate. Across two files it fires exactly once per
    /// capture, at the end, where the X2's lens 0 file runs one frame longer
    /// than its partner.
    fn align(&mut self) {
        loop {
            let heads: Vec<u64> = self.lanes.iter().filter_map(|lane| self.at(lane)).collect();
            if heads.len() != self.lanes.len() {
                return;
            }
            let Some(&newest) = heads.iter().max() else {
                return;
            };
            if heads.iter().all(|index| *index == newest) {
                return;
            }
            let behind: Vec<bool> = self
                .lanes
                .iter()
                .map(|lane| self.at(lane).is_some_and(|index| index < newest))
                .collect();
            for (lane, behind) in self.lanes.iter_mut().zip(behind) {
                if behind {
                    lane.queue.pop_front();
                }
            }
        }
    }

    /// Which frame of the capture this lane's head is, on its own file's
    /// timeline.
    fn at(&self, lane: &Lane) -> Option<u64> {
        Some(
            self.timing
                .index_at(self.media_time(lane.source, lane.head()?)),
        )
    }

    /// The media time of the first lane's head, which is the instant the
    /// pair is stamped with.
    fn head_time(&self) -> Option<Duration> {
        let lane = self.lanes.first()?;
        Some(self.media_time(lane.source, lane.head()?))
    }

    fn media_time(&self, source: usize, pts: i64) -> Duration {
        let source = &self.sources[source];
        media_time(pts, source.start, source.time_base)
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

impl Opened {
    fn new(path: &Path) -> Fallible<Self> {
        let mut input = ff::format::input(&path)?;
        let videos: Vec<Video> = input
            .streams()
            .filter(|s| s.parameters().medium() == ff::media::Type::Video)
            .map(|s| {
                // `Parameters` hands out no accessors, and opening a decoder
                // to read two integers before deciding whether this file is
                // even wanted is worse than reading the integers. The same
                // reach `sound_rate` makes, for the same reason.
                let (width, height) = unsafe {
                    let p = *s.parameters().as_ptr();
                    (p.width.max(0) as u32, p.height.max(0) as u32)
                };
                Video {
                    stream: s.index(),
                    rate: s.avg_frame_rate(),
                    frames: s.frames().max(0) as u64,
                    size: Size::new(width, height),
                }
            })
            .collect();
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

    fn into_source(self) -> Source {
        Source {
            path: self.path,
            input: self.input,
            time_base: self.time_base,
            start: self.start,
            drained: false,
        }
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
        })
    }
}

/// The file holding this capture's other lens, opened and checked, or `None`
/// for a capture that is one file.
///
/// The lookup only happens for a container that decodes a **single** lens, so
/// an X4-class `.insv` never touches the filesystem for it and its open path
/// is what it always was.
fn partner(path: &Path, first: &Opened) -> Option<Opened> {
    let shape = first.shape()?;
    if shape.lenses != 1 {
        return None;
    }
    let beside: PathBuf = kjerag_meta::sibling(path)?;
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

    fn ntsc() -> Timing {
        Timing::new(ff::Rational::new(30000, 1001), 53940).unwrap()
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

    /// The first `.insv` under `~/Videos`, or whatever `KJERAG_TEST_INSV`
    /// points at.
    fn test_capture() -> Option<std::path::PathBuf> {
        if let Ok(path) = std::env::var("KJERAG_TEST_INSV") {
            return Some(path.into());
        }
        let videos = std::path::PathBuf::from(std::env::var("HOME").ok()?).join("Videos");
        let mut captures: Vec<_> = std::fs::read_dir(videos)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("insv"))
            })
            .collect();
        captures.sort();
        captures.into_iter().next()
    }

    /// **Issue #79, on the owner's own path.** A capture written one lens
    /// per file opens as one source of two lenses, from either of its two
    /// files, and the pair invariant holds across the two containers.
    ///
    /// Ignored because it needs a per-lens pair on disk. Run it with
    /// `KJERAG_TEST_INSV=~/Videos/Insta/VID_20000101_110000_00_002.insv \
    ///  cargo test -p kjerag-media -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a per-lens .insv pair, named by KJERAG_TEST_INSV"]
    fn a_per_lens_pair_opens_as_one_capture_from_either_file() {
        let Some(lens0) = test_capture() else {
            eprintln!("no .insv found, skipping");
            return;
        };
        let Some(lens1) = kjerag_meta::sibling(&lens0) else {
            eprintln!("{} has no sibling, skipping", lens0.display());
            return;
        };

        for path in [&lens0, &lens1] {
            let mut reader = Reader::open(path).unwrap();
            assert_eq!(reader.files(), 2, "{}", path.display());
            assert_eq!(reader.lenses(), 2, "{}", path.display());

            // Every delivery is a complete set, and the indices run in
            // order: a lens is never handed over without its partner.
            for expected in 0..90 {
                let frames = reader.next_frames().unwrap().expect("ended early");
                assert_eq!(frames.lenses.len(), 2);
                assert_eq!(frames.index, expected);
            }

            // And a seek lands both files on the same frame.
            reader.seek(Cue::Index(1200), Accuracy::Exact).unwrap();
            let landed = reader.next_frames().unwrap().unwrap();
            assert_eq!(landed.index, 1200);
            assert_eq!(landed.lenses.len(), 2);
        }

        // The capture is the shorter of the two files: lens 0 runs a frame
        // longer, and a frame with no partner is not a frame of the
        // capture.
        let paired = Reader::open(&lens0).unwrap().timing().frames;
        let alone = Reader::open(&lens1).unwrap().timing().frames;
        assert_eq!(paired, alone, "both ends of the pair count the same");
    }

    /// **Issue #97, on the owner's own path.** His April capture has one
    /// place where the camera left 67 MB of picture between two audio
    /// samples, 4.885 s in. Read off the pictures' demuxer, the sound for
    /// the three and a half seconds after it arrived up to a second late,
    /// was dropped by the splice, and he heard silence from 4.9 s to 8.2 s.
    /// Two of the four large captures on this box have such a gap, both of
    /// them within half a second of where he heard his.
    ///
    /// So the sound reads on a demuxer of its own, and this is the claim
    /// that rests on: pumped once per frame, as
    /// [`Reader::read_until`] pumps it, and drained as a device drains it,
    /// the ring never runs dry through that region. The pictures are not
    /// decoded here, so this needs no GPU; it is the sound's half of the
    /// path he played.
    ///
    /// Ignored because the footage is 36 GB and lives on one box. Run it with
    /// `cargo test -p kjerag-media -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs real footage at ~/Videos/*.insv"]
    fn the_sound_plays_through_a_gap_in_the_interleave() {
        const RATE: u32 = 48_000;
        const CHANNELS: usize = 2;

        let Some(path) = test_capture() else {
            eprintln!("no .insv found, skipping");
            return;
        };
        let pipe = crate::audio::Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        let Some(mut track) = Track::open(&path, pipe.clone(), RATE, CHANNELS).unwrap() else {
            eprintln!("{} has no sound, skipping", path.display());
            return;
        };

        // One frame of this camera, and the sound a device takes while it is
        // on screen.
        let interval = Duration::from_micros(33_367);
        let frames = (f64::from(RATE) * interval.as_secs_f64()) as usize;
        let mut out = vec![0.0; frames * CHANNELS];
        let mut due = Duration::ZERO;
        while due < Duration::from_secs(12) {
            track.pump().unwrap();
            pipe.fill(&mut out, Some(due));
            due += interval;
        }

        let health = pipe.health();
        assert_eq!(health.underruns, 0, "the sound stopped: {health:?}");
        assert_eq!(health.dropped, 0, "the ring overflowed: {health:?}");
        // And it is still reading ahead at the end, not scraping along
        // empty: the room the reader leaves is what covers the next gap.
        assert!(health.queued > 100_000, "the ring ran down: {health:?}");
    }

    /// The two claims issue #5 rests on, which no arithmetic can check: an
    /// exact seek lands on the frame it was asked for, and a keyframe seek
    /// lands at or before it and never past it. A picture from past the cue
    /// would make a scrub run ahead of the pilot's hand.
    ///
    /// Ignored because the footage is 36 GB and lives on one box. Run it with
    /// `cargo test -p kjerag-media -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs real footage at ~/Videos/*.insv"]
    fn a_real_file_lands_where_it_is_told() {
        let Some(path) = test_capture() else {
            eprintln!("no .insv found, skipping");
            return;
        };
        let mut reader = Reader::open(&path).unwrap().lookahead(2);
        let timing = reader.timing();

        for place in [0.01, 0.5, 0.97, 0.33] {
            let at = timing.duration().mul_f64(place);
            let wanted = timing.index_at(at);

            reader.seek(Cue::Time(at), Accuracy::Exact).unwrap();
            let exact = reader.next_frames().unwrap().unwrap();
            assert_eq!(exact.index, wanted, "exact seek to {at:?}");

            reader.seek(Cue::Time(at), Accuracy::Keyframe).unwrap();
            let key = reader.next_frames().unwrap().unwrap();
            assert!(
                key.index <= wanted && wanted - key.index < 60,
                "keyframe seek to {at:?} landed on {} for {wanted}",
                key.index
            );

            // Giving a read up must cost nothing but the time already spent:
            // the lanes keep what they decoded, so the frame the abandoned
            // read was reaching for is the one the next read hands over.
            assert!(matches!(
                reader.read_until(|| true).unwrap(),
                Read::Interrupted
            ));

            // And reading on from a landing carries on in order, which is
            // what playing after a scrub depends on.
            let next = reader.next_frames().unwrap().unwrap();
            assert_eq!(next.index, key.index + 1);
        }
    }
}
