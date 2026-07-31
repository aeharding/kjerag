//! Decoded frames in system memory, one instant of every stream at a time.
//!
//! The player's own [`kyerag_media::Reader`] delivers frames into GPU memory,
//! which is where the app wants them and not where an instrument that samples
//! the delivered picture at angles does. `rolling` (issue #9) opened this walk
//! first; `seam` (issue #48) needs the same frames and needs them from an
//! ordinary `.mp4` as well, so it lives here rather than in either binary.
//!
//! Nothing here interprets a stream as a lens: a capture's video streams come
//! out in lens order and the caller decides what they are. Insta360 writes
//! two, one per lens; a stitched export has one.
//!
//! A capture is not always one file. The ONE X2 writes one lens per file, so
//! the walk opens the sibling alongside (issue #79, `kyerag_meta::sibling`)
//! and pairs across the two the same way it pairs across two streams of one:
//! same instant, or neither. Without it the seam instruments answer "this
//! file carries one lens stream" on every X2 capture, which is the file's
//! shape rather than the camera's.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use ffmpeg_next as ff;
use kyerag_media::{Fallible, HwDevice, SwFrame, open_decoder};
use kyerag_meta::Size;

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

/// One stream's luma plane, as the decoder handed it over.
pub struct Plane {
    pub luma: Vec<u8>,
    pub stride: usize,
    pub size: Size,
}

impl Plane {
    fn of(frame: &SwFrame, size: Size) -> Self {
        let (bytes, stride) = frame.plane(0, size.height);
        Self {
            luma: bytes.to_vec(),
            stride: stride as usize,
            size,
        }
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
            && let Some(beside) = kyerag_meta::sibling(path)
        {
            let second = ff::format::input(&beside)?;
            let streams = video_streams(&second);
            if streams.len() == 1 {
                inputs.push(second);
                lanes.push((1, streams[0]));
                if kyerag_meta::lens_index(path) == Some(1) {
                    lanes.swap(0, 1);
                }
            }
        }

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
