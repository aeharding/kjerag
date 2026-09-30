use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);

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
    let input = Input::open(&path).unwrap();
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
