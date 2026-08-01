//! A pointer for the headless harness: move to a place in the window, and
//! optionally press the left button there.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin pointer -- 1280 720 640 360
//! cargo run --release -p kjerag-spike --bin pointer -- 1280 720 1252 696 click
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
use std::time::Duration;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [width, height, x, y] = place(&args)?;
    let click = args.get(4).is_some_and(|arg| arg == "click");

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

    if click {
        sleep(HELD);
        pointer.button(at(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        queue.roundtrip(&mut found)?;
        sleep(HELD);
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
    read.try_into()
        .map_err(|_| "usage: pointer <width> <height> <x> <y> [click]".to_owned())
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
