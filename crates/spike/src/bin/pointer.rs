//! A pointer for the headless harness: move to a place in the window, and
//! optionally press the left button there.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin pointer -- 1280 720 640 360
//! cargo run --release -p kjerag-spike --bin pointer -- 1280 720 1252 696 click
//! cargo run --release -p kjerag-spike --bin pointer -- 1280 720 350 696 drag 858 696
//! cargo run --release -p kjerag-spike --bin pointer -- 2256 1504 1128 730 pan 12 1000 160
//! ```
//!
//! The four numbers are the output's width and height and the place in it,
//! all in pixels, because `zwlr_virtual_pointer_v1` takes absolute motion as
//! a place and an extent rather than as a coordinate (`motion_absolute`).
//!
//! `scripts/uitest.sh` drives keys with `wtype` and needed a pointer for the
//! same reason: some of this app is only reachable with one. `wlrctl pointer`
//! is the packaged tool for it and it cannot do this job. cage advertises the
//! seat's pointer capability only while a pointer device exists
//! (`seat.c`, `update_capabilities`), a one-shot client's device lives for
//! about a millisecond, and a client cannot bind `wl_pointer` and have the
//! binding reach the compositor in that time: measured 2026-08-01 against
//! this app under cage, a `wlrctl pointer scroll` that should have zoomed the
//! view did nothing at all, twenty of them in a row did nothing, and the same
//! zoom off the keyboard worked every time. So this holds the device open for
//! [`SETTLE`] before it moves anything, which is the whole of what it does
//! differently, and the same clicks then land.
//!
//! It is a spike binary because it is an instrument, and it is in this crate
//! because that is where the instruments are. Nothing in the app or in its
//! layers depends on it.

use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1 as manager, zwlr_virtual_pointer_v1 as pointer,
};

/// How long the pointer exists before it is used, and again before it goes.
/// The window the client needs to notice the seat grew a pointer, bind one,
/// and have the compositor see the binding; and afterwards, the window the
/// events it sent need to be delivered in.
const SETTLE: Duration = Duration::from_millis(500);

/// How long the button is held. A press and a release in the same instant is
/// a click no toolkit would miss, but a click a person could not make either.
const HELD: Duration = Duration::from_millis(120);

/// Linux's own code for the left button, which is what the protocol asks for.
const BTN_LEFT: u32 = 0x110;

/// The horizontal pan completes one smooth out-and-back cycle per second.
const PAN_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Move,
    Click,
    Drag { x: u32, y: u32 },
    Pan(Pan),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pan {
    duration: Duration,
    event_hz: u32,
    amplitude: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [width, height, x, y] = place(&args)?;
    let action = action(&args, width, height, x, y)?;

    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());

    let mut found = Found::default();
    queue.roundtrip(&mut found)?;
    let manager = found
        .manager
        .clone()
        .ok_or("this compositor serves no zwlr_virtual_pointer_manager_v1")?;

    // No seat: with one seat there is nothing to choose between, and the
    // compositor picks it (`create_virtual_pointer`, the null case).
    let pointer = manager.create_virtual_pointer(None, &handle, ());
    queue.roundtrip(&mut found)?;
    sleep(SETTLE);

    pointer.motion_absolute(at(), x, y, width, height);
    pointer.frame();
    queue.roundtrip(&mut found)?;

    if action != Action::Move {
        sleep(HELD);
        pointer.button(at(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        queue.roundtrip(&mut found)?;
        sleep(HELD);

        match action {
            Action::Move | Action::Click => {}
            Action::Drag { x: to_x, y: to_y } => {
                // Multiple pointer updates while held exercise the real slider's
                // keyframe previews before the exact release, not a seek API.
                for step in 1..=20i64 {
                    let blend = |from: u32, to: u32| {
                        (i64::from(from) + (i64::from(to) - i64::from(from)) * step / 20) as u32
                    };
                    pointer.motion_absolute(at(), blend(x, to_x), blend(y, to_y), width, height);
                    pointer.frame();
                    queue.roundtrip(&mut found)?;
                    sleep(Duration::from_millis(25));
                }
            }
            Action::Pan(pan) => {
                run_pan(&connection, &pointer, x, y, width, height, pan)?;
            }
        }

        pointer.button(at(), BTN_LEFT, wl_pointer::ButtonState::Released);
        pointer.frame();
        queue.roundtrip(&mut found)?;
    }

    sleep(SETTLE);
    pointer.destroy();
    queue.roundtrip(&mut found)?;
    Ok(())
}

/// The four numbers, or a line saying what was wanted.
fn place(args: &[String]) -> Result<[u32; 4], String> {
    let read: Vec<u32> = args
        .iter()
        .take(4)
        .filter_map(|arg| arg.parse().ok())
        .collect();
    read.try_into().map_err(|_| usage())
}

fn action(args: &[String], width: u32, height: u32, x: u32, y: u32) -> Result<Action, String> {
    match args.get(4).map(String::as_str) {
        None if args.len() == 4 => Ok(Action::Move),
        Some("") if args.len() == 5 => Ok(Action::Move),
        Some("click") if args.len() == 5 => Ok(Action::Click),
        Some("drag") if args.len() == 7 => Ok(Action::Drag {
            x: args[5]
                .parse()
                .map_err(|_| "drag destination x must be an unsigned integer".to_owned())?,
            y: args[6]
                .parse()
                .map_err(|_| "drag destination y must be an unsigned integer".to_owned())?,
        }),
        Some("pan") if args.len() == 8 => {
            let duration_seconds = args[5].parse::<u64>().map_err(|_| {
                "pan duration must be an integer from 1 through 60 seconds".to_owned()
            })?;
            let event_hz = args[6].parse::<u32>().map_err(|_| {
                "pan event rate must be an integer from 1 through 2000 Hz".to_owned()
            })?;
            let amplitude = args[7]
                .parse::<u32>()
                .map_err(|_| "pan amplitude must be an unsigned integer".to_owned())?;
            if !(1..=60).contains(&duration_seconds) {
                return Err("pan duration must be from 1 through 60 seconds".to_owned());
            }
            if !(1..=2000).contains(&event_hz) {
                return Err("pan event rate must be from 1 through 2000 Hz".to_owned());
            }
            if width == 0 || height == 0 || x >= width || y >= height {
                return Err("pan start must be inside the output".to_owned());
            }
            if amplitude == 0 || amplitude > x.min(width - 1 - x) {
                return Err(
                    "pan amplitude must keep every horizontal position inside the output"
                        .to_owned(),
                );
            }
            Ok(Action::Pan(Pan {
                duration: Duration::from_secs(duration_seconds),
                event_hz,
                amplitude,
            }))
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: pointer <width> <height> <x> <y> [click | drag <x> <y> | pan <seconds> <event-hz> <amplitude-px>]".to_owned()
}

fn run_pan(
    connection: &Connection,
    pointer: &pointer::ZwlrVirtualPointerV1,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    pan: Pan,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let started_monotonic_ns = monotonic_ns()?;
    let started_unix_ms = unix_ms()?;
    let event_count = pan.duration.as_secs() * u64::from(pan.event_hz);

    for event in 1..=event_count {
        let deadline =
            started + Duration::from_nanos(event * 1_000_000_000 / u64::from(pan.event_hz));
        if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            sleep(remaining);
        }
        pointer.motion_absolute(
            at(),
            pan_x(x, pan.amplitude, started.elapsed()),
            y,
            width,
            height,
        );
        pointer.frame();
        connection.flush()?;
    }

    let elapsed = started.elapsed();
    let ended_monotonic_ns = monotonic_ns()?;
    let ended_unix_ms = unix_ms()?;
    println!(
        "{{\"event\":\"pointer_pan\",\"sent\":{event_count},\"requested_duration_ms\":{},\"actual_elapsed_ms\":{},\"event_hz\":{},\"amplitude_px\":{},\"start_monotonic_ns\":{started_monotonic_ns},\"end_monotonic_ns\":{ended_monotonic_ns},\"start_unix_ms\":{started_unix_ms},\"end_unix_ms\":{ended_unix_ms}}}",
        pan.duration.as_millis(),
        elapsed.as_millis(),
        pan.event_hz,
        pan.amplitude,
    );
    Ok(())
}

fn pan_x(center: u32, amplitude: u32, elapsed: Duration) -> u32 {
    let phase = elapsed.as_secs_f64() / PAN_PERIOD.as_secs_f64();
    let offset = (phase * std::f64::consts::TAU).sin() * f64::from(amplitude);
    (f64::from(center) + offset).round() as u32
}

fn monotonic_ns() -> Result<u64, std::io::Error> {
    let mut timestamp = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `timestamp` points to initialized writable storage for one
    // `timespec`, and `CLOCK_MONOTONIC` requires no other caller-owned state.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut timestamp) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(u64::try_from(timestamp.tv_sec).unwrap_or(0) * 1_000_000_000
        + u64::try_from(timestamp.tv_nsec).unwrap_or(0))
}

fn unix_ms() -> Result<u128, std::time::SystemTimeError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
}

/// A timestamp for an event. The protocol wants milliseconds on a monotonic
/// clock and nothing here reads them back, so this is a clock of its own
/// rather than the compositor's.
fn at() -> u32 {
    static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    STARTED
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u32
}

/// What the registry turned up.
#[derive(Default)]
struct Found {
    manager: Option<manager::ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Found {
    fn event(
        found: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        if interface == manager::ZwlrVirtualPointerManagerV1::interface().name {
            found.manager = Some(registry.bind(name, version.min(2), handle, ()));
        }
    }
}

// Neither of these two interfaces sends the client anything.
impl Dispatch<manager::ZwlrVirtualPointerManagerV1, ()> for Found {
    fn event(
        _: &mut Self,
        _: &manager::ZwlrVirtualPointerManagerV1,
        _: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<pointer::ZwlrVirtualPointerV1, ()> for Found {
    fn event(
        _: &mut Self,
        _: &pointer::ZwlrVirtualPointerV1,
        _: pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn existing_actions_still_parse() {
        let move_args = args(&["1280", "720", "640", "360"]);
        let harness_move_args = args(&["1280", "720", "640", "360", ""]);
        let click_args = args(&["1280", "720", "1252", "696", "click"]);
        let drag_args = args(&["1280", "720", "350", "696", "drag", "858", "696"]);
        assert_eq!(action(&move_args, 1280, 720, 640, 360), Ok(Action::Move));
        assert_eq!(
            action(&harness_move_args, 1280, 720, 640, 360),
            Ok(Action::Move)
        );
        assert_eq!(action(&click_args, 1280, 720, 1252, 696), Ok(Action::Click));
        assert_eq!(
            action(&drag_args, 1280, 720, 350, 696),
            Ok(Action::Drag { x: 858, y: 696 })
        );
        assert!(
            action(
                &args(&["1280", "720", "640", "360", "move"]),
                1280,
                720,
                640,
                360
            )
            .is_err()
        );
    }

    #[test]
    fn pan_limits_are_checked() {
        let valid = args(&["2256", "1504", "1128", "730", "pan", "12", "1000", "160"]);
        assert_eq!(
            action(&valid, 2256, 1504, 1128, 730),
            Ok(Action::Pan(Pan {
                duration: Duration::from_secs(12),
                event_hz: 1000,
                amplitude: 160,
            }))
        );

        for invalid in [
            ["pan", "0", "1000", "160"],
            ["pan", "61", "1000", "160"],
            ["pan", "12", "0", "160"],
            ["pan", "12", "2001", "160"],
            ["pan", "12", "1000", "1129"],
        ] {
            let mut words = vec!["2256", "1504", "1128", "730"];
            words.extend(invalid);
            assert!(action(&args(&words), 2256, 1504, 1128, 730).is_err());
        }

        assert!(action(&valid, 2256, 1504, 2256, 730).is_err());
        assert!(action(&valid, 2256, 1504, 1128, 1504).is_err());
    }

    #[test]
    fn pan_path_stays_at_the_center_and_extrema() {
        let center = 1128;
        let amplitude = 160;
        assert_eq!(pan_x(center, amplitude, Duration::ZERO), center);
        assert_eq!(
            pan_x(center, amplitude, Duration::from_millis(250)),
            center + amplitude
        );
        assert_eq!(
            pan_x(center, amplitude, Duration::from_millis(750)),
            center - amplitude
        );
        assert_eq!(pan_x(center, amplitude, Duration::from_secs(1)), center);
    }
}
