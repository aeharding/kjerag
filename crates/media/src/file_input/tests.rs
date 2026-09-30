use super::*;
use crate::capture_fixture::FixtureDir;

fn bytes_file() -> (FixtureDir, PathBuf, Vec<u8>) {
    let fixture = FixtureDir::new();
    let anchor = fixture.write("anchor.insv", crate::Size::new(32, 16), 1, 0);
    let path = anchor.with_file_name("bytes.bin");
    let bytes: Vec<_> = (0..(BLOCK * (BLOCKS + 1) + 123))
        .map(|at| (at % 251) as u8)
        .collect();
    std::fs::write(&path, &bytes).unwrap();
    (fixture, path, bytes)
}

#[test]
fn cached_reads_preserve_bytes_boundaries_and_memory_bound() {
    let (_fixture, path, bytes) = bytes_file();
    let source = SharedFile::open(&path).unwrap();
    let mut output = [0; 256];
    for at in [0, 93, BLOCK - 13, BLOCK, bytes.len() - 23, bytes.len()] {
        let count = source.read_at(&mut output, at as u64).unwrap();
        let expected = output.len().min(bytes.len() - at).min(BLOCK - at % BLOCK);
        assert_eq!(count, expected);
        assert_eq!(&output[..count], &bytes[at..at + count]);
    }
    for block in 0..=BLOCKS {
        source.read_at(&mut output, (block * BLOCK) as u64).unwrap();
        let cache = source.blocks.lock().unwrap();
        assert!(cache.len() <= BLOCKS);
        assert!(cache.iter().map(|(_, data)| data.len()).sum::<usize>() <= BLOCK * BLOCKS);
    }
    let before = source.blocks.lock().unwrap().len();
    let count = source
        .read_at(&mut output, (BLOCK * BLOCKS + 17) as u64)
        .unwrap();
    assert_eq!(source.blocks.lock().unwrap().len(), before);
    assert_eq!(
        &output[..count],
        &bytes[BLOCK * BLOCKS + 17..BLOCK * BLOCKS + 17 + count]
    );
}

#[test]
fn independent_cursors_seek_without_moving_the_other_or_reopening_the_file() {
    let (_fixture, path, bytes) = bytes_file();
    let source = SharedFile::open(&path).unwrap();
    let first = Io::new(source.clone(), 91).unwrap();
    let second = Io::new(source.clone(), 2 * BLOCK as u64).unwrap();
    std::fs::rename(&path, path.with_extension("renamed")).unwrap();
    std::fs::write(&path, b"replacement must not be opened").unwrap();
    let mut output = [0; 256];
    unsafe {
        assert_eq!(read(first.cursor.cast(), output.as_mut_ptr(), 256), 256);
        assert_eq!(&output, &bytes[91..347]);
        assert_eq!((*second.cursor).at, 2 * BLOCK as u64);
        assert_eq!(
            seek(second.cursor.cast(), -123, 2),
            (bytes.len() - 123) as i64
        );
        assert_eq!(read(second.cursor.cast(), output.as_mut_ptr(), 256), 123);
        assert_eq!(&output[..123], &bytes[bytes.len() - 123..]);
        assert_eq!(
            read(second.cursor.cast(), output.as_mut_ptr(), 256),
            ff::ffi::AVERROR_EOF
        );
        assert_eq!(
            seek(first.cursor.cast(), 0, ff::ffi::AVSEEK_SIZE),
            bytes.len() as i64
        );
        assert_eq!(
            seek(first.cursor.cast(), -1, 0),
            i64::from(-ff::error::EINVAL)
        );
        assert_eq!((*first.cursor).at, 347);
        assert_eq!(seek(first.cursor.cast(), 7, ff::ffi::AVSEEK_FORCE), 7);
        assert_eq!(
            seek(first.cursor.cast(), i64::MAX, 1),
            i64::from(-ff::error::EINVAL)
        );
        assert_eq!((*first.cursor).at, 7);
    }
    assert!(Arc::ptr_eq(&first.source, &second.source));
}

fn assert_packet(left: &ff::Packet, right: &ff::Packet) {
    assert_eq!(left.data(), right.data());
    assert_eq!(left.stream(), right.stream());
    assert_eq!(left.pts(), right.pts());
    assert_eq!(left.dts(), right.dts());
    assert_eq!(left.duration(), right.duration());
    assert_eq!(left.position(), right.position());
    assert_eq!(left.flags(), right.flags());
}

#[test]
fn custom_io_and_reopened_demux_preserve_original_packets_and_independent_seeks() {
    let fixture = FixtureDir::new();
    let path = fixture.write("two-cursors.insv", crate::Size::new(32, 16), 129, 0);
    let mut original = ff::format::input(&path).unwrap();
    let mut first = Input::open(&path).unwrap();
    let mut second = Input::open_shared(first.source()).unwrap();
    for target in [None, Some(1_500_000), Some(0)] {
        if let Some(to) = target {
            original.seek(to, ..to).unwrap();
            first.seek(to, ..to).unwrap();
            second.seek(to, ..to).unwrap();
        }
        loop {
            let mut expected = ff::Packet::empty();
            let mut actual = ff::Packet::empty();
            let mut other = ff::Packet::empty();
            let result = expected.read(&mut original);
            assert_eq!(actual.read(&mut first), result);
            assert_eq!(other.read(&mut second), result);
            if result == Err(ff::Error::Eof) {
                break;
            }
            result.unwrap();
            assert_packet(&expected, &actual);
            assert_packet(&expected, &other);
        }
        second.seek(0, ..0).unwrap();
        let mut other = ff::Packet::empty();
        other.read(&mut second).unwrap();
        let mut eof = ff::Packet::empty();
        assert_eq!(eof.read(&mut first), Err(ff::Error::Eof));
    }
}

#[test]
fn reopened_demux_reads_the_retained_file_not_a_replaced_path() {
    let fixture = FixtureDir::new();
    let path = fixture.write("retained.insv", crate::Size::new(32, 16), 129, 0);
    let mut first = Input::open(&path).unwrap();
    std::fs::rename(&path, path.with_extension("original")).unwrap();
    fixture.write("retained.insv", crate::Size::new(64, 32), 3, 0);
    let mut second = Input::open_shared(first.source()).unwrap();
    assert_eq!(second.stream(0).unwrap().frames(), 129);
    first.seek(1_500_000, ..1_500_000).unwrap();
    second.seek(1_500_000, ..1_500_000).unwrap();
    let mut expected = ff::Packet::empty();
    let mut actual = ff::Packet::empty();
    expected.read(&mut first).unwrap();
    actual.read(&mut second).unwrap();
    assert_packet(&expected, &actual);
}

#[test]
fn callback_read_failure_remains_the_underlying_error() {
    let (_fixture, path, _) = bytes_file();
    let source = SharedFile::open(&path).unwrap();
    let io = Io::new(source.clone(), 0).unwrap();
    let _ = std::panic::catch_unwind(|| {
        let _cache = source.blocks.lock().unwrap();
        panic!("fixture poison");
    });
    let mut output = [0; 16];
    assert_eq!(
        unsafe { read(io.cursor.cast(), output.as_mut_ptr(), 16) },
        -ff::error::EIO
    );
    assert_eq!(
        io.failure().as_deref(),
        Some("input byte cache is poisoned")
    );
    assert!(io.failure().is_none());
}

#[test]
fn repeated_open_seek_and_close_keeps_demux_io_owned_through_allocation_churn() {
    let fixture = FixtureDir::new();
    let path = fixture.write("reopen.insv", crate::Size::new(32, 16), 129, 0);
    for round in 0..32 {
        let mut input = Input::open(&path).unwrap();
        // Keep other AVIO allocations alive. Correctness must not depend on
        // the allocator returning the address of a previously closed context.
        let churn: Vec<_> = (0..8)
            .map(|_| Io::new(input.source(), 0).unwrap())
            .collect();
        let to = if round % 2 == 0 { 1_500_000 } else { 0 };
        input.seek(to, ..to).unwrap();
        let mut reference = ff::format::input(&path).unwrap();
        reference.seek(to, ..to).unwrap();
        for _ in 0..10 {
            let mut expected = ff::Packet::empty();
            let mut actual = ff::Packet::empty();
            expected.read(&mut reference).unwrap();
            actual.read(&mut input).unwrap();
            assert_packet(&expected, &actual);
        }
        drop(input);
        drop(churn);
    }
}

#[test]
fn shared_audio_and_video_packet_workers_keep_independent_timelines() {
    let fixture = FixtureDir::new();
    let anchor = fixture.write("anchor.insv", crate::Size::new(32, 16), 1, 0);
    let path = anchor.with_file_name("audio-video.insv");
    let generated = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-cpucount",
            "1",
            "-filter_threads",
            "1",
            "-filter_complex_threads",
            "1",
            "-f",
            "lavfi",
            "-i",
            "color=size=32x16:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "3",
            "-c:v",
            "mpeg4",
            "-threads",
            "1",
            "-g",
            "1",
            "-c:a",
            "aac",
            "-f",
            "mov",
        ])
        .arg(&path)
        .output()
        .expect("generate synthetic audio/video input");
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let mut video = Input::open(&path).unwrap();
    let mut audio = Input::open_shared(video.source()).unwrap();
    let mut reference = ff::format::input(&path).unwrap();
    crate::read_only(&mut video, &[0]);
    crate::read_only(&mut audio, &[1]);
    crate::read_only(&mut reference, &[1]);
    let mut video =
        crate::packet_input::PacketInput::new(video, crate::packet_input::Limits::VIDEO).unwrap();
    let mut audio =
        crate::packet_input::PacketInput::new(audio, crate::packet_input::Limits::AUDIO).unwrap();
    video.read().unwrap().unwrap();
    for cue in [0, 1_500_000, 0] {
        audio.seek(cue).unwrap();
        reference.seek(cue, ..cue).unwrap();
        loop {
            let mut expected = ff::Packet::empty();
            match expected.read(&mut reference) {
                Ok(()) => assert_packet(&expected, &audio.read().unwrap().unwrap()),
                Err(ff::Error::Eof) => {
                    assert!(audio.read().unwrap().is_none());
                    break;
                }
                Err(error) => panic!("reference audio read failed: {error}"),
            }
        }
    }
    // Audio reached EOF and sought twice while the video consumer stayed put.
    // Its prefetched picture sequence must not have moved with those seeks.
    let mut reference = ff::format::input(&path).unwrap();
    crate::read_only(&mut reference, &[0]);
    let mut first = ff::Packet::empty();
    first.read(&mut reference).unwrap();
    loop {
        let mut expected = ff::Packet::empty();
        match expected.read(&mut reference) {
            Ok(()) => assert_packet(&expected, &video.read().unwrap().unwrap()),
            Err(ff::Error::Eof) => {
                assert!(video.read().unwrap().is_none());
                break;
            }
            Err(error) => panic!("reference video read failed: {error}"),
        }
    }
}
