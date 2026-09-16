//! Tiny real containers for tests which must exercise libavformat itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use ffmpeg_next as ff;

use super::Size;

const TIME_BASE: ff::Rational = ff::Rational(1, 30_000);
const FRAME_RATE: ff::Rational = ff::Rational(30_000, 1_001);
const FRAME_TICKS: i64 = 1_001;
const FIXTURE_TAG: &str = "kjerag synthetic capture fixture";

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// A unique, inspectable scratch directory containing disposable test media.
pub(crate) struct FixtureDir {
    path: PathBuf,
}

impl FixtureDir {
    pub(crate) fn new() -> Self {
        ff::init().expect("initialize ffmpeg for capture fixtures");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/capture-fixtures");
        fs::create_dir_all(&root).expect("create capture fixture scratch root");

        loop {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("{}-{serial}", process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create capture fixture directory: {error}"),
            }
        }
    }

    /// Write one all-intra MPEG-4 video stream in a MOV container.
    pub(crate) fn write(&self, name: &str, size: Size, frames: usize, start: i64) -> PathBuf {
        assert_eq!(
            Path::new(name).file_name(),
            Some(std::ffi::OsStr::new(name))
        );
        assert!(size.width > 0 && size.height > 0);
        assert!(size.width.is_multiple_of(2) && size.height.is_multiple_of(2));
        assert!(frames > 0);

        let path = self.path.join(name);
        let codec =
            ff::encoder::find(ff::codec::Id::MPEG4).expect("the native MPEG-4 encoder is present");
        let mut output = ff::format::output_as(&path, "mov").expect("create fixture MOV");
        let global_header = output
            .format()
            .flags()
            .contains(ff::format::Flags::GLOBAL_HEADER);

        let mut encoder = ff::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .expect("construct fixture video encoder");
        encoder.set_width(size.width);
        encoder.set_height(size.height);
        encoder.set_format(ff::format::Pixel::YUV420P);
        encoder.set_time_base(TIME_BASE);
        encoder.set_frame_rate(Some(FRAME_RATE));
        encoder.set_gop(1);
        encoder.set_max_b_frames(0);
        encoder.set_bit_rate(200_000);
        if global_header {
            encoder.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }
        let mut encoder = encoder.open_as(codec).expect("open fixture MPEG-4 encoder");

        {
            let mut stream = output.add_stream(codec).expect("add fixture video stream");
            stream.set_parameters(&encoder);
            stream.set_time_base(TIME_BASE);
            stream.set_rate(FRAME_RATE);
            stream.set_avg_frame_rate(FRAME_RATE);
        }

        let description = format!(
            "{FIXTURE_TAG}; {}x{}; frames={frames}; start={start}",
            size.width, size.height
        );
        let mut metadata = ff::Dictionary::new();
        metadata.set("comment", &description);
        output.set_metadata(metadata);

        let mut options = ff::Dictionary::new();
        options.set("avoid_negative_ts", "disabled");
        // MOV needs the leading empty edit to retain a positive track origin.
        options.set("use_editlist", "1");
        options.set("video_track_timescale", "30000");
        let unused = output
            .write_header_with(options)
            .expect("write fixture MOV header");
        assert!(
            unused.iter().next().is_none(),
            "fixture MOV options were not accepted: {unused:?}"
        );
        drop(unused);
        let mux_time_base = output.stream(0).expect("fixture video stream").time_base();

        for index in 0..frames {
            let mut frame =
                ff::frame::Video::new(ff::format::Pixel::YUV420P, size.width, size.height);
            frame.data_mut(0).fill(32 + (index % 160) as u8);
            frame.data_mut(1).fill(96 + (index % 32) as u8);
            frame.data_mut(2).fill(160 - (index % 32) as u8);
            frame.set_kind(ff::picture::Type::I);
            frame.set_pts(Some(start + index as i64 * FRAME_TICKS));
            encoder
                .send_frame(&frame)
                .expect("send synthetic fixture frame");
            write_packets(&mut encoder, &mut output, mux_time_base);
        }

        encoder.send_eof().expect("flush fixture MPEG-4 encoder");
        write_packets(&mut encoder, &mut output, mux_time_base);
        output.write_trailer().expect("finish fixture MOV");
        path
    }
}

fn write_packets(
    encoder: &mut ff::encoder::Video,
    output: &mut ff::format::context::Output,
    mux_time_base: ff::Rational,
) {
    loop {
        let mut packet = ff::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(0);
                packet.set_duration(FRAME_TICKS);
                packet.rescale_ts(TIME_BASE, mux_time_base);
                packet.set_position(-1);
                packet
                    .write_interleaved(output)
                    .expect("write synthetic fixture packet");
            }
            Err(ff::Error::Other {
                errno: ff::error::EAGAIN,
            })
            | Err(ff::Error::Eof) => break,
            Err(error) => panic!("receive synthetic fixture packet: {error}"),
        }
    }
}

#[test]
fn mov_fixture_preserves_authenticated_shape_and_timestamps() {
    let directory = FixtureDir::new();
    let path = directory.write("shifted.mov", Size::new(32, 32), 4, 60_000);
    let mut input = ff::format::input(&path).expect("reopen fixture MOV");

    assert_eq!(
        input.metadata().get("comment"),
        Some("kjerag synthetic capture fixture; 32x32; frames=4; start=60000")
    );
    let stream = input.stream(0).expect("fixture video stream");
    assert_eq!(stream.parameters().id(), ff::codec::Id::MPEG4);
    assert_eq!(stream.time_base(), TIME_BASE);
    assert_eq!(stream.rate(), FRAME_RATE);
    assert_eq!(stream.avg_frame_rate(), FRAME_RATE);
    assert_eq!(stream.start_time(), 60_000);
    assert_eq!(stream.frames(), 4);
    let video = ff::codec::context::Context::from_parameters(stream.parameters())
        .expect("read fixture video parameters")
        .decoder()
        .video()
        .expect("fixture stream is video");
    assert_eq!((video.width(), video.height()), (32, 32));

    let packets = input
        .packets()
        .map(|(_, packet)| {
            assert!(packet.is_key());
            assert_eq!(packet.duration(), FRAME_TICKS);
            packet.pts().expect("fixture packet PTS")
        })
        .collect::<Vec<_>>();
    assert_eq!(packets, [60_000, 61_001, 62_002, 63_003]);
}
