//! Decoded frames in system memory, one instant of every stream at a time.
//!
//! [`Reader`](super::Reader) delivers frames into GPU memory, which is where
//! the picture wants them and not where anything that reads the delivered
//! **pixels** at angles can go. `rolling` (issue #9) opened this walk first;
//! `seam` (issue #48) measured the seam with it, and since the seam fit now
//! runs at open in the app as well as in the instruments it lives here rather
//! than in `kjerag-spike`.
//!
//! Nothing here interprets a stream as a lens: a capture's video streams come
//! out in lens order and the caller decides what they are. Insta360 writes
//! two, one per lens; a stitched export has one.
//!
//! A capture is not always one file. The ONE X2 writes one lens per file, so
//! the walk opens the sibling alongside (issue #79, `kjerag_meta::sibling`)
//! and pairs across the two the same way it pairs across two streams of one:
//! same instant, or neither. Without it the seam instruments answer "this
//! file carries one lens stream" on every X2 capture, which is the file's
//! shape rather than the camera's.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ffmpeg_next as ff;

use super::{Fallible, HwDevice, Size, SwFrame, open_decoder};

/// Every video stream of one container, in container order.
fn video_streams(input: &ff::format::context::Input) -> Vec<usize> {
    input
        .streams()
        .filter(|s| s.parameters().medium() == ff::media::Type::Video)
        .map(|s| s.index())
        .collect()
}

/// One frame of every stream, at one instant.
pub struct Pair {
    pub index: u64,
    pub at: Duration,
    pub lenses: Vec<Plane>,
}

/// One stream's planes, as the decoder handed them over.
pub struct Plane {
    pub luma: Vec<u8>,
    pub stride: usize,
    pub size: Size,
    /// The interleaved Cb/Cr plane at half the resolution each way, or `None`
    /// where the decoder handed over something that is not NV12.
    ///
    /// Only an instrument that asks whether the two lenses disagree about
    /// **colour** reads this (issue #103, stage 3); everything else on this
    /// walk is geometry, and geometry is in the luma.
    pub chroma: Option<Chroma>,
}

/// One frame's Cb/Cr, interleaved as NV12 writes them.
pub struct Chroma {
    pub bytes: Vec<u8>,
    pub stride: usize,
}

impl Plane {
    fn of(frame: &SwFrame, size: Size) -> Self {
        let (bytes, stride) = frame.plane(0, size.height);
        Self {
            luma: bytes.to_vec(),
            stride: stride as usize,
            size,
            chroma: frame.nv12().then(|| {
                let (bytes, stride) = frame.plane(1, size.height / 2);
                Chroma {
                    bytes: bytes.to_vec(),
                    stride: stride as usize,
                }
            }),
        }
    }

    /// Cb and Cr at a **luma** pixel, each as a signed offset from neutral in
    /// 8-bit codes, or `None` outside the picture or on a frame with no
    /// chroma plane.
    ///
    /// Nearest neighbour, and deliberately: the chroma grid is half the luma
    /// one each way, so a bilinear read of it would mix two colours a whole
    /// luma pixel apart, and what is being asked here is what colour the
    /// content at this direction is rather than where its edges are.
    pub fn chroma_at(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let chroma = self.chroma.as_ref()?;
        let (column, row) = ((x * 0.5) as usize, (y * 0.5) as usize);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let at = row * chroma.stride + 2 * column;
        let pair = chroma.bytes.get(at..at + 2)?;
        Some((f64::from(pair[0]) - 128.0, f64::from(pair[1]) - 128.0))
    }

    /// Bilinear, in delivered-frame pixels. `None` outside the picture, which
    /// a patch takes as a patch it cannot use.
    pub fn at(&self, x: f64, y: f64) -> Option<f64> {
        let (left, top) = (x.floor(), y.floor());
        if left < 0.0 || top < 0.0 {
            return None;
        }
        let (left, top) = (left as usize, top as usize);
        if left + 1 >= self.size.width as usize || top + 1 >= self.size.height as usize {
            return None;
        }
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let code = |x: usize, y: usize| f64::from(self.luma[y * self.stride + x]);
        let upper = code(left, top) * (1.0 - fx) + code(left + 1, top) * fx;
        let lower = code(left, top + 1) * (1.0 - fx) + code(left + 1, top + 1) * fx;
        Some(upper * (1.0 - fy) + lower * fy)
    }
}

/// A walk forward through every video stream of one capture from one instant,
/// delivering frames in step.
pub struct Walk {
    /// One demuxer per file: two only for a capture written one lens per
    /// file, and they share a frame grid exactly (docs/research 1).
    inputs: Vec<ff::format::context::Input>,
    decoders: Vec<ff::decoder::Video>,
    /// `(file, stream)` per lane, in lens order.
    lanes: Vec<(usize, usize)>,
    queues: Vec<VecDeque<(i64, Plane)>>,
    time_base: ff::Rational,
    start: i64,
    from_pts: i64,
    fps: f64,
    size: Size,
    drained: Vec<bool>,
    /// Held so the VA-API device outlives the decoders that reference it.
    _hw: HwDevice,
}

impl Walk {
    /// A walk over the capture one path names, looking beside it for the
    /// other lens if this file carries only one.
    ///
    /// Beside is where a bare path's mate is, and a bare path is what a
    /// command line and every instrument hands over. It is not where a picked
    /// file's mate is: for a capture somebody has already composed, use
    /// [`Walk::over`].
    pub fn open(path: &Path, from: f64, size: Size) -> Fallible<Self> {
        ff::init()?;
        let hw = HwDevice::vaapi()?;
        let mut inputs = vec![ff::format::input(&path)?];
        let mut lanes = Vec::new();
        for stream in video_streams(&inputs[0]) {
            lanes.push((0, stream));
        }
        // One lens in this file: the other one may be in the file beside it.
        // Lens order is the marker's, not the order they were named in.
        if lanes.len() == 1
            && let Some(beside) = kjerag_meta::sibling(path)
        {
            let second = ff::format::input(&beside)?;
            let streams = video_streams(&second);
            if streams.len() == 1 {
                inputs.push(second);
                lanes.push((1, streams[0]));
                if kjerag_meta::lens_index(path) == Some(1) {
                    lanes.swap(0, 1);
                }
            }
        }
        Self::walking(inputs, lanes, from, size, hw)
    }

    /// A walk over a capture that is already composed: every file of it, in
    /// lens order, as [`Reader::paths`](crate::Reader::paths) hands them over.
    ///
    /// Taken as given, with nothing looked up, because looking again can only
    /// find less. A capture whose two halves were picked in a sandbox's file
    /// chooser arrives as two documents in two directories that hold one file
    /// each, so its second lens exists in the picked set and nowhere on the
    /// filesystem beside the first (issue #123). One path is a capture nobody
    /// has composed, and takes [`Walk::open`]'s lookup.
    pub fn over(files: &[PathBuf], from: f64, size: Size) -> Fallible<Self> {
        if let [only] = files {
            return Self::open(only, from, size);
        }
        ff::init()?;
        let hw = HwDevice::vaapi()?;
        let mut inputs = Vec::new();
        let mut lanes = Vec::new();
        for path in files {
            let input = ff::format::input(path)?;
            for stream in video_streams(&input) {
                lanes.push((inputs.len(), stream));
            }
            inputs.push(input);
        }
        Self::walking(inputs, lanes, from, size, hw)
    }

    /// One decoder per lane, every demuxer seeked to the same instant, and
    /// the timing the whole capture is read on: what the two constructors
    /// above share once they have settled which files and streams the capture
    /// is.
    fn walking(
        mut inputs: Vec<ff::format::context::Input>,
        lanes: Vec<(usize, usize)>,
        from: f64,
        size: Size,
        hw: HwDevice,
    ) -> Fallible<Self> {
        let (file, index) = *lanes.first().ok_or("this file carries no video stream")?;
        let stream = inputs[file].stream(index).ok_or("no video stream")?;
        let time_base = stream.time_base();
        let start = stream.start_time().max(0);
        let rate = stream.avg_frame_rate();
        let fps = f64::from(rate.numerator()) / f64::from(rate.denominator());
        let decoders = lanes
            .iter()
            .map(|(file, index)| open_decoder(&inputs[*file], *index, &hw))
            .collect::<Fallible<Vec<_>>>()?;

        let target = (from * 1e6) as i64;
        for input in &mut inputs {
            input.seek(target, ..target)?;
        }
        let from_pts = start
            + (from * f64::from(time_base.denominator()) / f64::from(time_base.numerator())) as i64;
        Ok(Self {
            drained: vec![false; inputs.len()],
            inputs,
            decoders,
            queues: lanes.iter().map(|_| VecDeque::new()).collect(),
            lanes,
            time_base,
            start,
            from_pts,
            fps,
            size,
            _hw: hw,
        })
    }

    /// How many lenses this capture carries. Two is a dual-fisheye capture,
    /// in one file or in two; one is a stitched export or half a pair, and a
    /// caller that needs a seam has to say so itself.
    pub fn streams(&self) -> usize {
        self.lanes.len()
    }

    /// How long the capture runs, from the containers' own duration. The
    /// shorter of the two for a capture written one lens per file: past that
    /// there are no pairs left to walk.
    pub fn duration(&self) -> Duration {
        let ticks = self
            .inputs
            .iter()
            .map(|input| input.duration())
            .min()
            .unwrap_or(0);
        Duration::from_secs_f64((ticks as f64 / f64::from(ff::ffi::AV_TIME_BASE)).max(0.0))
    }

    /// Carry the walk to another instant of the same capture, so a caller that
    /// wants frames from several places pays the container open once.
    ///
    /// Every decoder is flushed and every queue emptied: a decoder handed
    /// packets from a new place without one keeps answering with the frames it
    /// was mid-way through, which would pair a frame from here with a frame
    /// from there.
    pub fn jump(&mut self, to: f64) -> Fallible<()> {
        let target = (to * 1e6) as i64;
        for input in &mut self.inputs {
            input.seek(target, ..target)?;
        }
        for decoder in &mut self.decoders {
            decoder.flush();
        }
        for queue in &mut self.queues {
            queue.clear();
        }
        self.from_pts = self.start + self.ticks(to);
        self.drained = vec![false; self.inputs.len()];
        Ok(())
    }

    /// A time in seconds as this file's own stream ticks.
    fn ticks(&self, seconds: f64) -> i64 {
        (seconds * f64::from(self.time_base.denominator()) / f64::from(self.time_base.numerator()))
            as i64
    }

    /// The next instant every stream has a frame for, at or after the one the
    /// walk was opened on.
    pub fn next_pair(&mut self) -> Fallible<Option<Pair>> {
        loop {
            let heads: Vec<i64> = self
                .queues
                .iter()
                .filter_map(|q| Some(q.front()?.0))
                .collect();
            if heads.len() == self.queues.len() {
                let newest = heads.iter().copied().fold(i64::MIN, i64::max);
                if heads.iter().all(|pts| *pts == newest) {
                    let lenses: Vec<Plane> = self
                        .queues
                        .iter_mut()
                        .filter_map(|queue| Some(queue.pop_front()?.1))
                        .collect();
                    let at = Duration::from_nanos(
                        ((newest - self.start).max(0) as u128
                            * self.time_base.numerator() as u128
                            * 1_000_000_000
                            / self.time_base.denominator() as u128) as u64,
                    );
                    return Ok(Some(Pair {
                        index: (at.as_secs_f64() * self.fps).round() as u64,
                        at,
                        lenses,
                    }));
                }
                // A head with no partner is dropped rather than paired with a
                // different instant of the other lens.
                for queue in &mut self.queues {
                    if queue.front().is_some_and(|(pts, _)| *pts < newest) {
                        queue.pop_front();
                    }
                }
                continue;
            }
            if self.drained.iter().all(|done| *done) {
                return Ok(None);
            }
            self.pump()?;
        }
    }

    /// Reads one packet from the file with the fewest frames waiting, so a
    /// pair stays in step instead of one file running away with the memory.
    fn pump(&mut self) -> Fallible<()> {
        let queued = |file: usize| {
            self.lanes
                .iter()
                .enumerate()
                .filter(|(_, (owner, _))| *owner == file)
                .map(|(lane, _)| self.queues[lane].len())
                .min()
                .unwrap_or(0)
        };
        let file = (0..self.inputs.len())
            .filter(|file| !self.drained[*file])
            .min_by_key(|file| queued(*file))
            .unwrap_or(0);

        let mut packet = ff::Packet::empty();
        match packet.read(&mut self.inputs[file]) {
            Ok(()) => {
                let at = (file, packet.stream());
                let Some(lane) = self.lanes.iter().position(|owner| *owner == at) else {
                    return Ok(());
                };
                self.decoders[lane].send_packet(&packet)?;
                self.drain(lane)
            }
            Err(ff::Error::Eof) => {
                self.drained[file] = true;
                let lanes: Vec<usize> = (0..self.lanes.len())
                    .filter(|lane| self.lanes[*lane].0 == file)
                    .collect();
                for lane in lanes {
                    self.decoders[lane].send_eof()?;
                    self.drain(lane)?;
                }
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Everything this decoder has ready. Frames before the cue are dropped
    /// without being copied out of the GPU, which is what keeps the walk from
    /// a keyframe cheap: the copy is 15 MB a lens.
    fn drain(&mut self, lane: usize) -> Fallible<()> {
        loop {
            let mut frame = ff::frame::Video::empty();
            if self.decoders[lane].receive_frame(&mut frame).is_err() {
                return Ok(());
            }
            let Some(pts) = frame.timestamp() else {
                continue;
            };
            if pts < self.from_pts {
                continue;
            }
            let taken = SwFrame::transfer(&frame)?;
            self.queues[lane].push_back((pts, Plane::of(&taken, self.size)));
        }
    }
}
