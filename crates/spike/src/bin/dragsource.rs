//! Drags a file onto whatever window is already up, so that a drop can be
//! tested at all (issue #118).
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin dragsource -- <file.insv> \
//!   [offer=uri-list|portal|both] [linger=secs]
//! ```
//!
//! The harness presses keys with `wtype` and nothing in it can drag, so
//! until this existed the drop path had no coverage anywhere, on either side
//! of a sandbox. This is a second Wayland client: it maps a small window of
//! its own in the session, borrows the pointer it has no input device for
//! from the wlr virtual pointer protocol, presses the button over its own
//! window, hands the compositor a `wl_data_source` and lets go over the app.
//! Nothing about the app is special-cased and nothing is injected into it:
//! what it receives is a drag from another client, the same as from a file
//! manager.
//!
//! What `offer=` chooses is the thing issue #118 is about, and both halves
//! are real behaviour of real sources:
//!
//! - `uri-list` is a file manager that hands over paths, which is every
//!   unsandboxed source, cosmic-files included (`src/clipboard.rs`, which
//!   offers `text/uri-list`, `text/plain` and `x-special/gnome-copied-files`
//!   and nothing else).
//! - `portal` is what GTK does for the same drag: the file is registered
//!   with the document portal and what travels is the transfer key, which
//!   the target exchanges for a path it can open. It is the only one of the
//!   two a sandboxed target can read, because a path off another app's
//!   filesystem means nothing inside the sandbox.
//!
//! The source is the honest end to measure from: the `send` event names the
//! mime type the target asked for, so this prints what the app read rather
//! than what it was offered.
//!
//! Exit: 0 the target read one of the offers and took the drop, 1 the drop
//! landed and nothing was read, 2 the drag could not be performed at all.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use url::Url;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_pointer::{self, ButtonState, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

const URI_LIST: &str = "text/uri-list";
const FILE_TRANSFER: &str = "application/vnd.portal.filetransfer";

/// Linux's `BTN_LEFT`, which is what the virtual pointer protocol asks for.
const BTN_LEFT: u32 = 0x110;

/// The drag window. Small on purpose: cage lays every window out over the
/// whole output, so a full sized one would be the surface under the pointer
/// for the whole drag and the app would never see it. The buffer is left at
/// zeroes, which is transparent, so a capture taken during a drag is the
/// app's own window and not this.
const WINDOW: i32 = 96;

/// How far apart the probes that hunt for our own window are. Under half its
/// width, so no placement can fall between two of them.
const PROBE_STEP: u32 = 40;

/// How many motions the drag is walked over. A destination is entered on
/// motion, so a single jump from the press to the drop would be a drop on a
/// window the app was never told the pointer had reached.
const STEPS: i64 = 8;

/// A drag the compositor never answers is a broken session rather than a
/// failed check, and every wait here is one the app cannot lengthen.
const REPLY: Duration = Duration::from_secs(5);

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("dragsource: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let Args {
        file,
        offer,
        linger,
    } = Args::parse(std::env::args().skip(1))?;
    let payloads = payloads(&file, offer)?;
    println!(
        "dragsource: offering {} for {}",
        payloads
            .iter()
            .map(|(mime, _)| mime.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        file.display()
    );

    let conn = Connection::connect_to_env().map_err(|e| format!("no wayland session: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut state = State::new(payloads);
    queue.roundtrip(&mut state).map_err(reply)?;
    state.bind(&registry, &qh)?;
    queue.roundtrip(&mut state).map_err(reply)?;

    state.map_window(&qh)?;
    pump(&mut queue, &mut state, REPLY, |s| s.configured)?;
    state.paint(&qh)?;

    let pointer = state.virtual_pointer(&qh)?;
    pump(&mut queue, &mut state, REPLY, |s| s.pointer.is_some())?;

    let window = state.find_window(&mut queue, &pointer)?;
    println!(
        "dragsource: our window is at {},{} and the output is {}x{}",
        window.0, window.1, state.output.0, state.output.1
    );

    let (grab_x, grab_y) = (window.0 + WINDOW as u32 / 2, window.1 + WINDOW as u32 / 2);
    state.warp(&pointer, grab_x, grab_y);
    pointer.button(state.stamp(), BTN_LEFT, ButtonState::Pressed);
    pointer.frame();
    pump(&mut queue, &mut state, REPLY, |s| s.grab.is_some())?;
    let grab = state.grab.take().expect("pumped until the button landed");

    state.start_drag(&qh, grab)?;
    println!("dragsource: the drag is up, moving to the drop");

    let (drop_x, drop_y) = state.away_from(window);
    for step in 1..=STEPS {
        let at = |from: u32, to: u32| {
            (i64::from(from) + (i64::from(to) - i64::from(from)) * step / STEPS) as u32
        };
        state.warp(&pointer, at(grab_x, drop_x), at(grab_y, drop_y));
        queue.roundtrip(&mut state).map_err(reply)?;
    }
    pointer.button(state.stamp(), BTN_LEFT, ButtonState::Released);
    pointer.frame();
    println!("dragsource: dropped at {drop_x},{drop_y}");

    // The drop, the read and the finish are three round trips through the
    // target, and a target that takes the drop and reads nothing is exactly
    // the failure this instrument exists to catch, so none of them is
    // required for the wait to end.
    let _ = pump(&mut queue, &mut state, REPLY, |s| s.finished);
    let verdict = state.verdict();

    // The transfer is held open for as long as this process is, and the app
    // is still opening the file it points at: leaving now would pull the path
    // out from under a read that has already started. The caller ends this by
    // killing us once the app says it opened something, which is the only
    // signal there is.
    let _ = pump(&mut queue, &mut state, linger, |_| false);
    Ok(verdict)
}

struct Args {
    file: PathBuf,
    offer: Offer,
    linger: Duration,
}

#[derive(Clone, Copy, PartialEq)]
enum Offer {
    UriList,
    Portal,
    Both,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut file = None;
        let mut offer = Offer::Both;
        let mut linger = Duration::from_secs(20);
        for arg in args {
            match arg.split_once('=') {
                Some(("offer", "uri-list")) => offer = Offer::UriList,
                Some(("offer", "portal")) => offer = Offer::Portal,
                Some(("offer", "both")) => offer = Offer::Both,
                Some(("linger", secs)) => {
                    linger = Duration::from_secs(secs.parse().map_err(|_| format!("{arg}?"))?);
                }
                Some(_) => return Err(format!("{arg}?")),
                None => file = Some(PathBuf::from(arg)),
            }
        }
        let file = file.ok_or("a file to drag is the one argument")?;
        match file.is_file() {
            true => Ok(Self {
                file,
                offer,
                linger,
            }),
            false => Err(format!("no file at {}", file.display())),
        }
    }
}

/// What each offered mime type hands over when the target asks for it.
fn payloads(file: &Path, offer: Offer) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut payloads = Vec::new();
    if offer != Offer::UriList {
        // NUL terminated because that is how the key is read back: libcosmic
        // drops the last byte of this mime type's payload before it uses it
        // (`src/widget/dnd_destination.rs:517-524`), which is GTK's own
        // convention for it.
        let mut key = transfer_key(file)?.into_bytes();
        key.push(0);
        payloads.push((FILE_TRANSFER.to_owned(), key));
    }
    if offer != Offer::Portal {
        let url = Url::from_file_path(file).map_err(|()| "the file has no absolute path")?;
        payloads.push((URI_LIST.to_owned(), format!("{url}\r\n").into_bytes()));
    }
    Ok(payloads)
}

/// Registers the file with the document portal, which is what makes a drop
/// readable inside a sandbox, and answers with the key that stands for it.
fn transfer_key(file: &Path) -> Result<String, String> {
    let open = File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no runtime for the portal call: {e}"))?;
    runtime.block_on(async {
        let portal = ashpd::documents::FileTransfer::new()
            .await
            .map_err(|e| format!("no file transfer portal: {e}"))?;
        let key = portal
            .start_transfer(false, true)
            .await
            .map_err(|e| format!("StartTransfer: {e}"))?;
        portal
            .add_files(&key, &[&open.as_fd()])
            .await
            .map_err(|e| format!("AddFiles: {e}"))?;
        Ok(key)
    })
}

struct State {
    // The globals, all of which have to be there for a drag to be possible.
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    seat: Option<WlSeat>,
    wm_base: Option<XdgWmBase>,
    devices: Option<WlDataDeviceManager>,
    pointers: Option<ZwlrVirtualPointerManagerV1>,
    globals: Vec<(u32, String, u32)>,

    surface: Option<WlSurface>,
    device: Option<WlDataDevice>,
    pointer: Option<WlPointer>,
    output: (u32, u32),
    configured: bool,

    /// The serial of a press over our own window, which is the one thing a
    /// compositor will start a drag from.
    grab: Option<u32>,
    /// Where the pointer was when it entered our own window, and where it
    /// was in the output at the time.
    entered: Option<((f64, f64), (u32, u32))>,
    at: (u32, u32),
    clock: Instant,

    payloads: Vec<(String, Vec<u8>)>,
    read: Vec<String>,
    dropped: bool,
    finished: bool,
    cancelled: bool,
}

impl State {
    fn new(payloads: Vec<(String, Vec<u8>)>) -> Self {
        Self {
            compositor: None,
            shm: None,
            seat: None,
            wm_base: None,
            devices: None,
            pointers: None,
            globals: Vec::new(),
            surface: None,
            device: None,
            pointer: None,
            output: (1280, 720),
            configured: false,
            grab: None,
            entered: None,
            at: (0, 0),
            clock: Instant::now(),
            payloads,
            read: Vec::new(),
            dropped: false,
            finished: false,
            cancelled: false,
        }
    }

    fn bind(&mut self, registry: &WlRegistry, qh: &QueueHandle<Self>) -> Result<(), String> {
        for (name, interface, version) in std::mem::take(&mut self.globals) {
            match interface.as_str() {
                "wl_compositor" => {
                    self.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" => self.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" => self.seat = Some(registry.bind(name, version.min(5), qh, ())),
                "xdg_wm_base" => self.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
                // Version 3 is the one with drag actions in it, and a source
                // with no actions is a drop the destination discards without
                // reading (smithay-clipboard `src/dnd/state.rs:186-190`).
                "wl_data_device_manager" => {
                    self.devices = Some(registry.bind(name, version.min(3), qh, ()));
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    self.pointers = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let _: WlOutput = registry.bind(name, version.min(2), qh, ());
                }
                _ => {}
            }
        }
        let missing = [
            ("wl_compositor", self.compositor.is_none()),
            ("wl_shm", self.shm.is_none()),
            ("wl_seat", self.seat.is_none()),
            ("xdg_wm_base", self.wm_base.is_none()),
            ("wl_data_device_manager", self.devices.is_none()),
            ("zwlr_virtual_pointer_manager_v1", self.pointers.is_none()),
        ];
        match missing.iter().find(|(_, missing)| *missing) {
            Some((name, _)) => Err(format!(
                "the compositor has no {name}, so no drag is possible"
            )),
            None => Ok(()),
        }
    }

    fn map_window(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        let compositor = self.compositor.as_ref().ok_or("no compositor")?;
        let wm_base = self.wm_base.as_ref().ok_or("no xdg_wm_base")?;
        let surface = compositor.create_surface(qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg.get_toplevel(qh, ());
        toplevel.set_title("dragsource".to_owned());
        toplevel.set_app_id("dev.harding.Kjerag.dragsource".to_owned());
        surface.commit();
        self.surface = Some(surface);
        Ok(())
    }

    /// A buffer of zeroes: the window has to be mapped to be pressed on, and
    /// nothing about it has to be seen.
    fn paint(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        let shm = self.shm.as_ref().ok_or("no wl_shm")?;
        let surface = self.surface.as_ref().ok_or("no surface")?;
        let size = (WINDOW * WINDOW * 4) as u64;
        let file = shm_file(size)?;
        let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(0, WINDOW, WINDOW, WINDOW * 4, Format::Argb8888, qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, WINDOW, WINDOW);
        surface.commit();
        Ok(())
    }

    fn virtual_pointer(&mut self, qh: &QueueHandle<Self>) -> Result<ZwlrVirtualPointerV1, String> {
        let seat = self.seat.as_ref().ok_or("no seat")?;
        let devices = self.devices.as_ref().ok_or("no data device manager")?;
        self.device = Some(devices.get_data_device(seat, qh, ()));
        let pointers = self.pointers.as_ref().ok_or("no virtual pointer manager")?;
        // The seat has no pointer capability until this exists: a headless
        // session has no input devices, and the capability is what the
        // wl_pointer below is asked for on.
        Ok(pointers.create_virtual_pointer(Some(seat), qh, ()))
    }

    fn stamp(&self) -> u32 {
        self.clock.elapsed().as_millis() as u32
    }

    fn warp(&mut self, pointer: &ZwlrVirtualPointerV1, x: u32, y: u32) {
        let (w, h) = self.output;
        pointer.motion_absolute(self.stamp(), x.min(w - 1), y.min(h - 1), w, h);
        pointer.frame();
        self.at = (x, y);
    }

    /// Where our own window ended up, found by walking the pointer over the
    /// output until it enters our surface. The compositor places it and
    /// never says where, and every position after this is measured from it.
    fn find_window(
        &mut self,
        queue: &mut EventQueue<Self>,
        pointer: &ZwlrVirtualPointerV1,
    ) -> Result<(u32, u32), String> {
        let (w, h) = self.output;
        for y in (PROBE_STEP / 2..h).step_by(PROBE_STEP as usize) {
            for x in (PROBE_STEP / 2..w).step_by(PROBE_STEP as usize) {
                self.warp(pointer, x, y);
                queue.roundtrip(self).map_err(reply)?;
                if let Some(((sx, sy), (px, py))) = self.entered {
                    return Ok((px.saturating_sub(sx as u32), py.saturating_sub(sy as u32)));
                }
            }
        }
        Err("the pointer never entered our own window, so nothing can be dragged".to_owned())
    }

    /// A point in the output that our own window is not over, which is where
    /// the app is.
    fn away_from(&self, window: (u32, u32)) -> (u32, u32) {
        let (w, h) = self.output;
        let clear = |(x, y): &(u32, u32)| {
            x.abs_diff(window.0) > WINDOW as u32 && y.abs_diff(window.1) > WINDOW as u32
        };
        let corners = [
            (w / 2, h / 2),
            (w / 4, h / 4),
            (w * 3 / 4, h * 3 / 4),
            (w / 4, h * 3 / 4),
            (w * 3 / 4, h / 4),
        ];
        corners.into_iter().find(clear).unwrap_or((w / 2, h / 2))
    }

    fn start_drag(&mut self, qh: &QueueHandle<Self>, grab: u32) -> Result<(), String> {
        let devices = self.devices.as_ref().ok_or("no data device manager")?;
        let device = self.device.as_ref().ok_or("no data device")?;
        let surface = self.surface.as_ref().ok_or("no surface")?;
        let source = devices.create_data_source(qh, ());
        for (mime, _) in &self.payloads {
            source.offer(mime.clone());
        }
        source.set_actions(DndAction::Copy | DndAction::Move);
        device.start_drag(Some(&source), surface, None, grab);
        Ok(())
    }

    fn verdict(&self) -> ExitCode {
        for mime in &self.read {
            println!("dragsource: the target read it as {mime}");
        }
        match (self.read.is_empty(), self.dropped, self.cancelled) {
            (false, _, _) => ExitCode::SUCCESS,
            (true, true, _) => {
                println!("dragsource: the drop landed and nothing was read from it");
                ExitCode::from(1)
            }
            (true, _, true) => {
                println!("dragsource: the drag was cancelled, so nothing was over a destination");
                ExitCode::from(1)
            }
            _ => {
                println!("dragsource: the drag ended without a drop");
                ExitCode::from(1)
            }
        }
    }
}

/// The shared memory a buffer is carved out of. Unlinked at once: the file is
/// the fd, and nothing else ever has to find it.
fn shm_file(size: u64) -> Result<File, String> {
    let dir = std::env::var("XDG_RUNTIME_DIR").map_err(|_| "no XDG_RUNTIME_DIR")?;
    let path = PathBuf::from(dir).join(format!("dragsource-{}", std::process::id()));
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.set_len(size).map_err(|e| e.to_string())?;
    Ok(file)
}

fn reply(e: impl std::fmt::Display) -> String {
    format!("the compositor stopped answering: {e}")
}

/// Dispatches until the session says what is being waited for, or the wait
/// runs out. `poll` rather than a blocking dispatch: a check that hangs is
/// worse than a check that fails, and every wait here has an answer that may
/// legitimately never come.
fn pump(
    queue: &mut EventQueue<State>,
    state: &mut State,
    within: Duration,
    done: impl Fn(&State) -> bool,
) -> Result<(), String> {
    let until = Instant::now() + within;
    loop {
        queue.dispatch_pending(state).map_err(reply)?;
        if done(state) {
            return Ok(());
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the session did not answer in time".to_owned());
        }
        queue.flush().map_err(reply)?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut poll = libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, left.as_millis() as i32) };
        match ready {
            1 => {
                guard.read().map_err(reply)?;
            }
            _ => drop(guard),
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            state.globals.push((name, interface, version));
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let has_pointer = capabilities
            .into_result()
            .is_ok_and(|c| c.contains(Capability::Pointer));
        if has_pointer && state.pointer.is_none() {
            state.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // Only our own surface is ever entered: the pointer is never told
            // about anyone else's.
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => state.entered = Some(((surface_x, surface_y), state.at)),
            wl_pointer::Event::Leave { .. } => state.entered = None,
            wl_pointer::Event::Button {
                serial,
                state: WEnum::Value(ButtonState::Pressed),
                ..
            } => state.grab = Some(serial),
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataSource,
        event: wl_data_source::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // The one measurement this instrument exists for: what the target
            // asked for, out of everything it was offered.
            wl_data_source::Event::Send { mime_type, fd } => {
                state.read.push(mime_type.clone());
                if let Err(e) = write_payload(&state.payloads, &mime_type, fd) {
                    eprintln!("dragsource: {mime_type}: {e}");
                }
            }
            wl_data_source::Event::DndDropPerformed => state.dropped = true,
            wl_data_source::Event::DndFinished => state.finished = true,
            wl_data_source::Event::Cancelled => state.cancelled = true,
            _ => {}
        }
    }
}

fn write_payload(
    payloads: &[(String, Vec<u8>)],
    mime: &str,
    fd: OwnedFd,
) -> Result<(), std::io::Error> {
    let mut pipe = File::from(fd);
    match payloads.iter().find(|(offered, _)| offered == mime) {
        Some((_, bytes)) => pipe.write_all(bytes),
        None => Ok(()),
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg: &XdgSurface,
        event: xdg_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { width, height, .. } = event {
            state.output = (width as u32, height as u32);
        }
    }
}

/// A drag makes this client a destination as well, and it is not one: the
/// offers it is told about are the ones it is making itself.
impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDevice,
        _: wl_data_device::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore WlDataDeviceManager);
delegate_noop!(State: ignore WlDataOffer);
delegate_noop!(State: ignore XdgToplevel);
delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);
