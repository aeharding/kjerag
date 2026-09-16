//! CPU-only container regressions for the shared capture admission and Walk
//! queue path. Software decode supplies actual decoded timestamps; hardware
//! mapping and the player still require the separate real-footage gates.

use super::*;
use crate::capture_fixture::FixtureDir;

const FIRST: &str = "VID_20000101_100000_00_001.insv";
const SECOND: &str = "VID_20000101_100000_10_001.insv";

#[test]
fn discovered_lenses_have_the_same_order_from_either_file() {
    let fixtures = FixtureDir::new();
    let first = fixtures.write(FIRST, Size::new(32, 32), 30, 0);
    let second = fixtures.write(SECOND, Size::new(32, 32), 29, 60_000);
    for named in [&first, &second] {
        let sources = Opened::discover(named, &[]).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].path, first);
        assert_eq!(sources[1].path, second);
        assert_eq!((sources[0].start, sources[1].start), (0, 60_000));
    }
}

#[test]
fn mismatched_sibling_is_left_out_but_an_explicit_bad_pair_is_an_error() {
    for (size, frames) in [(Size::new(64, 64), 30), (Size::new(32, 32), 27)] {
        let fixtures = FixtureDir::new();
        let first = fixtures.write(FIRST, Size::new(32, 32), 30, 0);
        let second = fixtures.write(SECOND, size, frames, 0);
        let sources = Opened::discover(&first, &[]).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].path, first);
        let error = Opened::pair(&first, &second)
            .err()
            .expect("bad pair accepted");
        assert_eq!(
            error.to_string(),
            "the explicitly selected files are not two lenses of one capture"
        );
    }
}

#[test]
fn an_unreadable_sibling_does_not_refuse_the_named_lens() {
    let fixtures = FixtureDir::new();
    let first = fixtures.write(FIRST, Size::new(32, 32), 30, 0);
    std::fs::write(first.with_file_name(SECOND), b"not a video").unwrap();
    let sources = Opened::discover(&first, &[]).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].path, first);
}

#[test]
fn explicitly_picked_partner_is_preferred_to_an_unsuitable_neighbor() {
    let local = FixtureDir::new();
    let picked = FixtureDir::new();
    let first = local.write(FIRST, Size::new(32, 32), 30, 0);
    local.write(SECOND, Size::new(64, 64), 30, 0);
    let second = picked.write(SECOND, Size::new(32, 32), 30, 60_000);
    let sources = Opened::discover(&first, std::slice::from_ref(&second)).unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].path, first);
    assert_eq!(sources[1].path, second);
    // A previously composed pair needs neither a common directory nor another
    // name lookup, just the same shape check as Reader's explicit-pair path.
    let explicit = Opened::pair(&first, &second).unwrap();
    assert_eq!(explicit.len(), 2);
    assert_eq!(explicit[1].path, second);
}

fn drain_cpu(decoder: &mut ff::decoder::Video, state: &mut WalkState, lane: usize) {
    loop {
        let mut frame = ff::frame::Video::empty();
        match decoder.receive_frame(&mut frame) {
            Ok(()) => {
                let pts = frame.timestamp().expect("decoded fixture has no timestamp");
                state
                    .receive(lane, pts, || {
                        Ok(Plane {
                            luma: vec![frame.data(0)[0]],
                            stride: 1,
                            size: Size::new(1, 1),
                            chroma: None,
                            wide: false,
                        })
                    })
                    .unwrap();
            }
            Err(ff::Error::Eof) => return,
            Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return,
            Err(error) => panic!("fixture decode failed: {error}"),
        }
    }
}

#[test]
fn actual_shifted_containers_pair_after_initial_seek_and_repeated_jumps() {
    let fixtures = FixtureDir::new();
    let first = fixtures.write(FIRST, Size::new(32, 32), 30, 0);
    let second = fixtures.write(SECOND, Size::new(32, 32), 30, 60_000);
    let sources = Opened::pair(&first, &second).unwrap();
    let mut state = WalkState::for_sources(&sources, 0.2).unwrap();
    let mut decoders: Vec<_> = sources
        .iter()
        .map(|source| {
            let stream = source.input.stream(source.videos[0].stream).unwrap();
            ff::codec::context::Context::from_parameters(stream.parameters())
                .unwrap()
                .decoder()
                .video()
                .unwrap()
        })
        .collect();
    let mut inputs: Vec<_> = sources.into_iter().map(|source| source.input).collect();
    // Forward, backward and repeat seeks reuse the same containers/decoders.
    // The same production helper performs open and jump's demux seek.
    for (cue, expected_index) in [(0.2, 6), (0.7, 21), (0.0, 0), (0.2, 6)] {
        seek_inputs(&mut inputs, cue).unwrap();
        for decoder in &mut decoders {
            decoder.flush();
        }
        state.cue(cue);
        for (lane, input) in inputs.iter_mut().enumerate() {
            loop {
                let mut packet = ff::Packet::empty();
                match packet.read(input) {
                    Ok(()) => {
                        assert_eq!(packet.stream(), 0);
                        decoders[lane].send_packet(&packet).unwrap();
                        drain_cpu(&mut decoders[lane], &mut state, lane);
                    }
                    Err(ff::Error::Eof) => break,
                    Err(error) => panic!("fixture demux failed: {error}"),
                }
            }
            decoders[lane].send_eof().unwrap();
            drain_cpu(&mut decoders[lane], &mut state, lane);
            state.mark_drained(lane);
        }
        for expected in expected_index..30 {
            let WalkStep::Pair(pair) = state.step() else {
                panic!("missing pair {expected} after cue {cue}");
            };
            assert_eq!(pair.index, expected);
            assert_eq!(pair.at, state.timing.time_of(expected));
            assert_eq!(pair.lenses.len(), 2);
            assert_eq!(pair.lenses[0].luma, pair.lenses[1].luma);
        }
        assert!(matches!(state.step(), WalkStep::End));
    }
}

/// The public delivery routes, including actual VA-API decoder/transfer wiring.
/// Run once per camera with KJERAG_TEST_INSV inside the guarded runtime suite.
#[test]
#[ignore = "needs VA-API and a real capture named by KJERAG_TEST_INSV"]
fn public_walk_and_reader_deliver_the_same_capture_frames() {
    use crate::{Accuracy, Cue, Reader};

    let path = PathBuf::from(std::env::var_os("KJERAG_TEST_INSV").expect("set KJERAG_TEST_INSV"));
    let mut reader = Reader::open(&path).unwrap();
    let mut walk = Walk::over(&reader.paths(), 0.0, Size::new(64, 64)).unwrap();
    assert_eq!(walk.streams(), reader.lenses());

    for index in [0, 6, 21, 0, 6] {
        let cue = Cue::Index(index);
        reader.seek(cue, Accuracy::Exact).unwrap();
        walk.jump(cue.time(reader.timing()).as_secs_f64()).unwrap();
        for offset in 0..3 {
            let gpu = reader.next_frames().unwrap().expect("reader ended early");
            let cpu = walk.next_pair().unwrap().expect("walk ended early");
            assert_eq!(gpu.index, index + offset);
            assert_eq!(cpu.index, gpu.index);
            assert_eq!(cpu.at, gpu.timestamp);
            assert_eq!(cpu.lenses.len(), gpu.lenses.len());
            assert!(cpu.lenses.iter().all(|plane| !plane.luma.is_empty()));
        }
    }
}
