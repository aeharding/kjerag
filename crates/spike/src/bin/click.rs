//! Clicks where it is told in a headless session, because the harness cannot
//! (issue #123).
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin click -- <x> <y> [width] [height]
//! ```
//!
//! `wtype` presses keys, and in a cage session it delivers character keys and
//! nothing else: `Return`, `BackSpace` and the arrows never reach a GTK client
//! here, measured against a real file chooser on 2026-08-01, while `Ctrl+L`
//! and the path typed after it arrive intact. A dialog whose only answer is a
//! button therefore cannot be answered from the keyboard at all, which is why
//! the chooser had no coverage on either side of a sandbox.
//!
//! This borrows the same wlr virtual pointer the drag instrument uses. It maps
//! no window of its own: it warps the shared pointer and presses the button,
//! and whatever is under that point receives it, which for a headless session
//! is the one window cage has.
//!
//! The wait after the pointer is created is not decoration. The seat had no
//! pointer at all until this ran, and a client creates its `wl_pointer` on
//! hearing the capability change, which is a round trip in somebody else's
//! process; a button pressed before that is a button nobody heard
//! (`dragsource` waits the same way, for the same reason).

use std::env;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;

const BTN_LEFT: u32 = 0x110;
/// How long the client is given to notice it has a pointer at all. A window
/// that was already up before this ran has to handle the seat gaining a
/// capability it never had, and a dialog opened by a portal is always in that
/// position: nothing in a headless session has a pointer until this makes one.
const SETTLE: Duration = Duration::from_millis(1500);
/// Between the warp and the press, and between the press and the release.
const BEAT: Duration = Duration::from_millis(400);
/// A cage session is one output, and this is its size unless told otherwise.
const OUTPUT: (u32, u32) = (1280, 720);

#[derive(Default)]
struct State {
    seat: Option<WlSeat>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_seat" => state.seat = Some(registry.bind(name, version.min(7), qh, ())),
            "zwlr_virtual_pointer_manager_v1" => {
                state.manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);

fn stamp() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u32)
}

fn parse(args: &[String], index: usize, name: &str) -> Result<u32, String> {
    args.get(index)
        .ok_or_else(|| format!("{name} is missing"))?
        .parse()
        .map_err(|_| format!("{name} is not a number"))
}

fn press(args: &[String]) -> Result<(), String> {
    let x = parse(args, 0, "x")?;
    let y = parse(args, 1, "y")?;
    let width = match args.len() > 2 {
        true => parse(args, 2, "width")?,
        false => OUTPUT.0,
    };
    let height = match args.len() > 3 {
        true => parse(args, 3, "height")?,
        false => OUTPUT.1,
    };

    let conn = Connection::connect_to_env().map_err(|e| format!("no wayland display: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State::default();
    let mut roundtrip = |state: &mut State| -> Result<(), String> {
        queue
            .roundtrip(state)
            .map(|_| ())
            .map_err(|e| format!("the display went away: {e}"))
    };
    roundtrip(&mut state)?;

    let manager = state
        .manager
        .clone()
        .ok_or("this compositor offers no virtual pointer")?;
    let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
    roundtrip(&mut state)?;
    sleep(SETTLE);

    // Twice: the first motion is what makes the compositor give the surface
    // under it a pointer enter, and a client that bound its pointer late has
    // missed it.
    for _ in 0..2 {
        pointer.motion_absolute(stamp(), x, y, width, height);
        pointer.frame();
        roundtrip(&mut state)?;
        sleep(BEAT);
    }

    pointer.button(stamp(), BTN_LEFT, ButtonState::Pressed);
    pointer.frame();
    roundtrip(&mut state)?;
    sleep(BEAT);

    pointer.button(stamp(), BTN_LEFT, ButtonState::Released);
    pointer.frame();
    roundtrip(&mut state)?;
    sleep(BEAT);

    println!("click:  {x},{y} of {width}x{height}");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match press(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("click: {why}");
            ExitCode::from(2)
        }
    }
}
