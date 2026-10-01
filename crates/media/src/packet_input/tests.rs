use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::task::Wake;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Counter(AtomicUsize);

impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Counter {
    fn wait(&self) {
        let deadline = Instant::now() + WAIT;
        while self.0.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "refill notification was lost");
            std::thread::yield_now();
        }
    }
}

/// A real packet consumer whose demuxer blocks with less than one second read.
/// Player tests use this observer without constructing decoded/GPU surfaces.
pub(crate) fn blocked_read_ahead() -> (PacketInput, ReadAhead, Sender<()>, Receiver<()>) {
    let (source, control) = source(200, 1, Some(6));
    let mut input = PacketInput::spawn(source, Limits::VIDEO).unwrap();
    input.read().unwrap().unwrap();
    control.entered.recv_timeout(WAIT).unwrap();
    let observer = input.read_ahead(vec![0], ff::Rational(1, 30), 0);
    (input, observer, control.release, control.dropped)
}

#[test]
fn refill_wait_wakes_on_actual_packet_arrival_without_consuming_input() {
    let (mut input, observer, release, dropped) = blocked_read_ahead();
    let wake = Arc::new(Counter::default());
    assert!(
        !observer
            .buffered_or_wait(Duration::from_secs(1), Waker::from(wake.clone()))
            .unwrap()
    );
    assert_eq!(wake.0.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    wait_for(&input.0, |state| state.terminal.is_some());
    wake.wait();
    assert_eq!(wake.0.load(Ordering::SeqCst), 1);
    assert!(
        observer
            .buffered_or_wait(Duration::from_secs(1), Waker::noop().clone())
            .unwrap()
    );
    for at in 1..200 {
        assert_eq!(input.read().unwrap().unwrap().pts(), Some(at));
    }
    drop(input);
    dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn refill_checks_every_lens_and_normalizes_the_files_own_clock() {
    let (input, observer, release, dropped) = blocked_read_ahead();
    let mut observer = observer;
    observer.streams = vec![0, 2];
    observer.start = 900;
    let packet = |stream, at| {
        let mut packet = ff::Packet::copy(&[1]);
        packet.set_stream(stream);
        packet.set_dts(Some(at));
        packet.set_pts(Some(at + 3));
        packet.set_duration(1);
        packet
    };
    {
        let mut state = input.0.state.lock().unwrap();
        state.packets.clear();
        state.bytes = 2;
        state.packets.extend([packet(0, 929), packet(2, 905)]);
    }
    assert!(
        !observer
            .buffered_or_wait(Duration::from_secs(1), Waker::noop().clone())
            .unwrap(),
        "one prepared lens cannot stand in for its partner"
    );
    input
        .0
        .state
        .lock()
        .unwrap()
        .packets
        .push_back(packet(2, 929));
    assert!(
        observer
            .buffered_or_wait(Duration::from_secs(1), Waker::noop().clone())
            .unwrap()
    );
    assert!(
        !observer
            .buffered_or_wait(Duration::from_secs(2), Waker::noop().clone())
            .unwrap()
    );
    observer.cancel();
    drop(input);
    release.send(()).unwrap();
    dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn refill_does_not_wait_past_eof_or_existing_video_and_audio_bounds() {
    let (input, observer, release, dropped) = blocked_read_ahead();
    for bound in 0..5 {
        let mut state = input.0.state.lock().unwrap();
        state.bytes = 0;
        state.packets.clear();
        state.audio = AudioQueue::default();
        state.terminal = None;
        match bound {
            0 => state.bytes = Limits::VIDEO.bytes,
            1 => state
                .packets
                .extend((0..Limits::VIDEO.packets).map(|_| ff::Packet::empty())),
            2 => {
                state.audio.active = true;
                state.audio.bytes = Limits::AUDIO.bytes;
            }
            3 => {
                state.audio.active = true;
                state
                    .audio
                    .packets
                    .extend((0..Limits::AUDIO.packets).map(|_| ff::Packet::empty()));
            }
            _ => state.terminal = Some(Ok(())),
        }
        drop(state);
        assert!(
            observer
                .buffered_or_wait(Duration::from_secs(100), Waker::noop().clone())
                .unwrap()
        );
    }
    drop(input);
    release.send(()).unwrap();
    dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn seek_revokes_previous_input_lead_and_cancel_removes_its_wake() {
    let (source, control) = source(200, 1, Some(0));
    let mut input = PacketInput::spawn(source, Limits::VIDEO).unwrap();
    let observer = input.read_ahead(vec![0], ff::Rational(1, 30), 0);
    input.seek(70).unwrap();
    wait_for(&input.0, |state| state.terminal.is_some());
    assert!(
        observer
            .buffered_or_wait(Duration::from_secs(4), Waker::noop().clone())
            .unwrap()
    );
    input.seek(0).unwrap();
    control.entered.recv_timeout(WAIT).unwrap();
    let wake = Arc::new(Counter::default());
    assert!(
        !observer
            .buffered_or_wait(Duration::from_secs(1), Waker::from(wake.clone()))
            .unwrap()
    );
    observer.cancel();
    control.release.send(()).unwrap();
    wait_for(&input.0, |state| state.terminal.is_some());
    assert_eq!(wake.0.load(Ordering::SeqCst), 0);
    drop(input);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn input_close_wakes_refill_and_underlying_failure_is_not_hidden_by_a_full_cache() {
    let (input, observer, release, dropped) = blocked_read_ahead();
    let wake = Arc::new(Counter::default());
    assert!(
        !observer
            .buffered_or_wait(Duration::from_secs(1), Waker::from(wake.clone()))
            .unwrap()
    );
    {
        let mut state = input.0.state.lock().unwrap();
        state.bytes = Limits::VIDEO.bytes;
        state.terminal = Some(Err("fixture input failed".into()));
    }
    assert_eq!(
        observer
            .buffered_or_wait(Duration::ZERO, Waker::noop().clone())
            .unwrap_err()
            .to_string(),
        "fixture input failed"
    );
    drop(input);
    assert_eq!(wake.0.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
    dropped.recv_timeout(WAIT).unwrap();
}

fn wait_for(shared: &Shared, predicate: impl Fn(&State) -> bool) {
    let deadline = Instant::now() + WAIT;
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    while !predicate(&state) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "input worker did not reach the expected state"
        );
        state = shared
            .changed
            .wait_timeout(state, remaining)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

struct Source {
    at: u64,
    end: u64,
    packet_size: usize,
    audio_every: Option<u64>,
    block_at: Option<u64>,
    entered: Sender<()>,
    release: Receiver<()>,
    dropped: Sender<()>,
}

impl Demux for Source {
    fn read(&mut self) -> Result<Option<ff::Packet>, String> {
        if self.block_at == Some(self.at) {
            self.block_at = None;
            self.entered.send(()).unwrap();
            self.release.recv_timeout(WAIT).unwrap();
        }
        if self.at == self.end {
            return Ok(None);
        }
        let mut packet = ff::Packet::copy(&vec![self.at as u8; self.packet_size]);
        packet.set_pts(Some(self.at as i64));
        packet.set_duration(1);
        if let Some(every) = self.audio_every {
            packet.set_stream(usize::from(self.at % every == every - 1));
        }
        self.at += 1;
        Ok(Some(packet))
    }

    fn seek(&mut self, to: i64) -> Result<(), String> {
        if to < 0 {
            return Err("fixture seek failed".into());
        }
        self.at = to as u64;
        Ok(())
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.dropped.send(());
    }
}

struct Control {
    entered: Receiver<()>,
    release: Sender<()>,
    dropped: Receiver<()>,
}

fn source(end: u64, packet_size: usize, block_at: Option<u64>) -> (Source, Control) {
    let (entered, waiting) = mpsc::channel();
    let (release, permit) = mpsc::channel();
    let (dropped, closed) = mpsc::channel();
    (
        Source {
            at: 0,
            end,
            packet_size,
            audio_every: None,
            block_at,
            entered,
            release: permit,
            dropped,
        },
        Control {
            entered: waiting,
            release,
            dropped: closed,
        },
    )
}

fn small_limits() -> Limits {
    Limits {
        bytes: 32,
        packets: 4,
    }
}

#[test]
fn input_prefetches_without_decoded_delivery_and_serves_a_read_stall_from_cache() {
    let (source, control) = source(20, 1, Some(5));
    let mut input = PacketInput::spawn(source, small_limits()).unwrap();
    assert_eq!(input.read().unwrap().unwrap().pts(), Some(0));
    wait_for(&input.0, |state| state.packets.len() == 4);
    assert_eq!(input.read().unwrap().unwrap().pts(), Some(1));
    control.entered.recv_timeout(WAIT).unwrap();

    // File reading is now blocked. Previously prefetched compressed packets
    // remain usable without waiting for that read or requiring GPU surfaces.
    for at in 2..=4 {
        assert_eq!(input.read().unwrap().unwrap().pts(), Some(at));
    }
    control.release.send(()).unwrap();
    assert_eq!(input.read().unwrap().unwrap().pts(), Some(5));
    drop(input);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn byte_limit_and_count_limit_both_stop_prefetch() {
    for (packet_size, expected_count) in [(10, 4), (100, 1), (0, 4)] {
        let (source, control) = source(20, packet_size, None);
        let mut input = PacketInput::spawn(source, small_limits()).unwrap();
        input.read().unwrap().unwrap();
        wait_for(&input.0, |state| state.packets.len() == expected_count);
        let state = input.0.state.lock().unwrap();
        assert_eq!(state.bytes, expected_count * packet_size);
        assert!(state.bytes >= 32 || state.packets.len() == 4);
        drop(state);
        drop(input);
        control.dropped.recv_timeout(WAIT).unwrap();
    }
}

#[test]
fn seek_invalidates_an_in_flight_packet_and_keeps_the_new_packet_order() {
    let (source, control) = source(100, 1, Some(1));
    let mut input = PacketInput::spawn(source, small_limits()).unwrap();
    assert_eq!(input.read().unwrap().unwrap().pts(), Some(0));
    control.entered.recv_timeout(WAIT).unwrap();
    let shared = input.0.clone();
    let (finished, result) = mpsc::channel();
    std::thread::spawn(move || {
        input.seek(70).unwrap();
        finished.send(input).unwrap();
    });
    wait_for(&shared, |state| state.seek == Some(70));
    control.release.send(()).unwrap();
    let mut input = result.recv_timeout(WAIT).unwrap();
    for at in 70..=74 {
        assert_eq!(input.read().unwrap().unwrap().pts(), Some(at));
    }
    drop(input);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn seek_failure_is_raw_and_a_later_valid_seek_can_recover() {
    let (source, control) = source(10, 1, None);
    let mut input = PacketInput::spawn(source, small_limits()).unwrap();
    assert_eq!(
        input.seek(-1).unwrap_err().to_string(),
        "fixture seek failed"
    );
    assert_eq!(
        input
            .read()
            .err()
            .expect("seek failure is sticky")
            .to_string(),
        "fixture seek failed"
    );
    input.seek(5).unwrap();
    assert_eq!(input.read().unwrap().unwrap().pts(), Some(5));
    drop(input);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn closing_does_not_wait_for_an_in_flight_filesystem_read() {
    let (source, control) = source(10, 1, Some(1));
    let mut input = PacketInput::spawn(source, small_limits()).unwrap();
    input.read().unwrap().unwrap();
    control.entered.recv_timeout(WAIT).unwrap();
    drop(input);
    assert!(control.dropped.try_recv().is_err(), "read is still blocked");
    control.release.send(()).unwrap();
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn real_demux_packets_and_seek_results_are_unchanged_by_prefetch() {
    let fixture = crate::capture_fixture::FixtureDir::new();
    let path = fixture.write("packet-input.insv", crate::Size::new(32, 16), 129, 0);
    let mut reference = ff::format::input(&path).unwrap();
    let input = crate::capture::open_input(&path).unwrap();
    let mut cached = PacketInput::new(input, small_limits()).unwrap();
    for target in [None, Some(1_500_000), Some(0)] {
        if let Some(to) = target {
            reference.seek(to, ..to).unwrap();
            cached.seek(to).unwrap();
        }
        loop {
            let mut packet = ff::Packet::empty();
            let expected = match packet.read(&mut reference) {
                Ok(()) => Some(packet),
                Err(ff::Error::Eof) => None,
                Err(error) => panic!("reference packet read failed: {error}"),
            };
            let actual = cached.read().unwrap();
            match (expected, actual) {
                (Some(expected), Some(actual)) => {
                    assert_eq!(actual.data(), expected.data());
                    assert_eq!(actual.stream(), expected.stream());
                    assert_eq!(actual.pts(), expected.pts());
                    assert_eq!(actual.dts(), expected.dts());
                    assert_eq!(actual.duration(), expected.duration());
                    assert_eq!(actual.position(), expected.position());
                    assert_eq!(actual.flags(), expected.flags());
                }
                (None, None) => break,
                _ => panic!("prefetch changed the packet count"),
            }
        }
        assert!(cached.read().unwrap().is_none(), "EOF remains observable");
    }
}

#[test]
fn producer_panic_wakes_read_and_does_not_leave_seek_waiting_forever() {
    struct Panics;
    impl Demux for Panics {
        fn read(&mut self) -> Result<Option<ff::Packet>, String> {
            panic!("fixture input reader panic");
        }
        fn seek(&mut self, _: i64) -> Result<(), String> {
            Ok(())
        }
    }
    let mut input = PacketInput::spawn(Panics, small_limits()).unwrap();
    assert_eq!(
        input
            .read()
            .err()
            .expect("producer panic wakes read")
            .to_string(),
        "fixture input reader panic"
    );
    assert_eq!(
        input.seek(0).unwrap_err().to_string(),
        "fixture input reader panic"
    );
}

fn timeline() -> AudioTimeline {
    AudioTimeline {
        stream: 1,
        time_base: ff::Rational(1, 1_000_000),
        start: 0,
    }
}

#[test]
fn audio_reads_past_the_camera_sized_interleave_without_video_delivery() {
    let (mut source, control) = source(68, 1024 * 1024, None);
    source.audio_every = Some(68);
    let mut video = PacketInput::spawn_routed(source, Limits::VIDEO, Some(timeline())).unwrap();
    let mut audio = video.audio_reader().unwrap();
    // Do not decode or consume a single picture. The previous surface-paced
    // demuxer could not reach this audio chunk before its playback deadline.
    assert_eq!(audio.read().unwrap().unwrap().pts(), Some(67));
    for at in 0..67 {
        assert_eq!(video.read().unwrap().unwrap().pts(), Some(at));
    }
    assert!(video.read().unwrap().is_none());
    assert!(audio.read().unwrap().is_none());
    drop(video);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn audio_packet_bounds_backpressure_without_discarding_either_stream() {
    let (mut source, control) = source(800, 3000, None);
    source.audio_every = Some(2);
    let mut video = PacketInput::spawn_routed(source, Limits::VIDEO, Some(timeline())).unwrap();
    let mut audio = video.audio_reader().unwrap();
    assert_eq!(video.read().unwrap().unwrap().pts(), Some(0));
    wait_for(&video.0, |state| state.audio.bytes >= Limits::AUDIO.bytes);
    let state = video.0.state.lock().unwrap();
    assert!(state.audio.bytes < Limits::AUDIO.bytes + 3000);
    assert!(state.audio.packets.len() <= Limits::AUDIO.packets);
    drop(state);
    for at in (1..800).step_by(2) {
        assert_eq!(audio.read().unwrap().unwrap().pts(), Some(at));
        if at + 1 < 800 {
            assert_eq!(video.read().unwrap().unwrap().pts(), Some(at + 1));
        }
    }
    assert!(audio.read().unwrap().is_none());
    assert!(video.read().unwrap().is_none());
    drop(video);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn zero_byte_audio_packets_still_obey_the_count_bound() {
    let (mut source, control) = source(400, 0, None);
    source.audio_every = Some(2);
    let mut video = PacketInput::spawn_routed(source, Limits::VIDEO, Some(timeline())).unwrap();
    let audio = video.audio_reader().unwrap();
    video.read().unwrap().unwrap();
    wait_for(&video.0, |state| {
        state.audio.packets.len() == Limits::AUDIO.packets
    });
    assert_eq!(video.0.state.lock().unwrap().audio.bytes, 0);
    drop(audio);
    for at in (2..400).step_by(2) {
        assert_eq!(video.read().unwrap().unwrap().pts(), Some(at));
    }
    assert!(video.read().unwrap().is_none());
    drop(video);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn a_canceled_audio_read_releases_control_while_file_io_is_blocked() {
    let (mut source, control) = source(20, 1, Some(1));
    source.audio_every = Some(2);
    let mut video = PacketInput::spawn_routed(source, small_limits(), Some(timeline())).unwrap();
    let mut audio = video.audio_reader().unwrap();
    video.read().unwrap().unwrap();
    control.entered.recv_timeout(WAIT).unwrap();
    let pipe = crate::audio::Pipe::new(48_000, 2, Duration::from_millis(500));
    let epoch = pipe.epoch();
    let testing = pipe.clone();
    let (checked, observing) = mpsc::channel();
    let (finished, result) = mpsc::channel();
    std::thread::spawn(move || {
        assert!(
            audio
                .read_while(|| {
                    let _ = checked.send(());
                    testing.is_current(&epoch)
                })
                .unwrap()
                .is_none()
        );
        finished.send(audio).unwrap();
    });
    observing.recv_timeout(WAIT).unwrap();
    pipe.invalidate();
    let mut audio = result.recv_timeout(WAIT).unwrap();
    // The canceled turn neither joins the blocked read nor loses its packet.
    control.release.send(()).unwrap();
    assert_eq!(audio.read().unwrap().unwrap().pts(), Some(1));
    drop(video);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn dropping_audio_keeps_video_running_and_dropping_video_wakes_audio() {
    let (mut source, control) = source(20, 1, None);
    source.audio_every = Some(2);
    let mut video = PacketInput::spawn_routed(source, small_limits(), Some(timeline())).unwrap();
    let audio = video.audio_reader().unwrap();
    drop(audio);
    for at in (0..20).step_by(2) {
        assert_eq!(video.read().unwrap().unwrap().pts(), Some(at));
    }
    assert!(video.read().unwrap().is_none());
    drop(video);
    control.dropped.recv_timeout(WAIT).unwrap();

    let (mut source, control) = self::source(20, 1, Some(1));
    source.audio_every = Some(2);
    let mut video = PacketInput::spawn_routed(source, small_limits(), Some(timeline())).unwrap();
    let mut audio = video.audio_reader().unwrap();
    video.read().unwrap().unwrap();
    control.entered.recv_timeout(WAIT).unwrap();
    drop(video);
    assert!(audio.read().unwrap().is_none());
    control.release.send(()).unwrap();
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn replay_gate_drops_only_pre_target_audio_and_resets_on_backward_seek() {
    let (mut source, control) = source(100, 1, None);
    source.audio_every = Some(2);
    let mut video = PacketInput::spawn_routed(source, small_limits(), Some(timeline())).unwrap();
    let mut audio = video.audio_reader().unwrap();
    video
        .seek_with_audio(0, Some(Duration::from_micros(60)))
        .unwrap();
    for at in (0..60).step_by(2) {
        assert_eq!(video.read().unwrap().unwrap().pts(), Some(at));
    }
    assert_eq!(audio.read().unwrap().unwrap().pts(), Some(59));
    video.seek(0).unwrap();
    // No old 60-microsecond gate can discard the first backward-seek audio.
    assert_eq!(audio.read().unwrap().unwrap().pts(), Some(1));
    audio.seek(4).unwrap();
    assert_eq!(audio.read().unwrap().unwrap().pts(), Some(3));
    assert_eq!(video.read().unwrap().unwrap().pts(), Some(0));
    drop(video);
    control.dropped.recv_timeout(WAIT).unwrap();
}

#[test]
fn real_mov_fanout_preserves_packets_and_repeated_seek_landings() {
    let fixture = crate::capture_fixture::FixtureDir::new();
    let path = fixture.write_audio("routed.mov");
    let input = crate::capture::open_input(&path).unwrap();
    let spec = crate::track::AudioSpec::inspect(&input).unwrap();
    let timeline = spec.timeline();
    let mut video = PacketInput::with_audio(input, Limits::VIDEO, Some(timeline)).unwrap();
    let mut audio = video.audio_reader().unwrap();
    let mut reference = ff::format::input(&path).unwrap();
    for target in [None, Some(1_500_000), Some(0)] {
        if let Some(to) = target {
            reference.seek(to, ..to).unwrap();
            video.seek(to).unwrap();
        }
        let cutoff = target.map(|to| Duration::from_micros(to as u64));
        let mut expected_video = Vec::new();
        let mut expected_audio = Vec::new();
        loop {
            let mut packet = ff::Packet::empty();
            match packet.read(&mut reference) {
                Ok(()) if packet.stream() == timeline.stream => {
                    if timeline.needed(&packet, cutoff) {
                        expected_audio.push(packet);
                    }
                }
                Ok(()) => expected_video.push(packet),
                Err(ff::Error::Eof) => break,
                Err(error) => panic!("reference read failed: {error}"),
            }
        }
        // Both consumers progress independently; a full bounded audio queue
        // must not make this test deadlock while video waits for final EOF.
        let checking_audio = std::thread::spawn(move || {
            for expected in expected_audio {
                assert_packet(&expected, &audio.read().unwrap().unwrap());
            }
            assert!(audio.read().unwrap().is_none());
            audio
        });
        for expected in expected_video {
            assert_packet(&expected, &video.read().unwrap().unwrap());
        }
        assert!(video.read().unwrap().is_none());
        audio = checking_audio.join().unwrap();
    }
}

fn assert_packet(expected: &ff::Packet, actual: &ff::Packet) {
    assert_eq!(actual.data(), expected.data());
    assert_eq!(actual.stream(), expected.stream());
    assert_eq!(actual.pts(), expected.pts());
    assert_eq!(actual.dts(), expected.dts());
    assert_eq!(actual.duration(), expected.duration());
    assert_eq!(actual.position(), expected.position());
    assert_eq!(actual.flags(), expected.flags());
}
