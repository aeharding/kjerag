use super::*;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

const GPU_TEST: &str = "KJERAG_WGPU_PREFLIGHT_TEST";

#[derive(Debug, Default)]
struct Counts {
    prepared: [AtomicUsize; 2],
    checked: [AtomicUsize; 2],
    drawn: [AtomicUsize; 2],
}

#[derive(Debug)]
struct TestPipeline;

impl primitive::Pipeline for TestPipeline {
    fn new(
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _format: wgpu::TextureFormat,
    ) -> Self {
        Self
    }
}

#[derive(Debug)]
struct TestPrimitive {
    slot: usize,
    ready: Arc<AtomicBool>,
    counts: Arc<Counts>,
    schedules_retry: bool,
    requests_redraw: bool,
}

impl Primitive for TestPrimitive {
    type Pipeline = TestPipeline;

    fn prepare(
        &self,
        _pipeline: &mut Self::Pipeline,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        let _ = self.counts.prepared[self.slot].fetch_add(1, Ordering::SeqCst);
    }

    fn is_presentable(&self, _pipeline: &Self::Pipeline) -> bool {
        let _ = self.counts.checked[self.slot].fetch_add(1, Ordering::SeqCst);
        self.ready.load(Ordering::SeqCst)
    }

    fn schedules_retry(&self, _pipeline: &Self::Pipeline) -> bool {
        self.schedules_retry
    }

    fn requests_redraw_after_prepare(
        &self,
        _pipeline: &Self::Pipeline,
    ) -> bool {
        self.requests_redraw
    }

    fn draw(
        &self,
        _pipeline: &Self::Pipeline,
        _render_pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let _ = self.counts.drawn[self.slot].fetch_add(1, Ordering::SeqCst);
        true
    }
}

#[derive(Clone)]
struct Redraws(Arc<AtomicUsize>);

impl graphics::shell::Notifier for Redraws {
    fn request_redraw(&self) {
        let _ = self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn invalidate_layout(&self) {}
}

struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                std::thread::park_timeout(Duration::from_millis(10))
            }
        }
    }
}

fn test_renderer()
-> Option<(Renderer, wgpu::Device, wgpu::Queue, Arc<AtomicUsize>)> {
    if std::env::var_os(GPU_TEST).is_none() {
        return None;
    }

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        ..Default::default()
    });
    let adapter =
        block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .expect(
            "preflight GPU test requested but no Vulkan adapter is available",
        );
    eprintln!("preflight renderer adapter: {:?}", adapter.get_info());
    let (device, queue) =
        block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("iced_wgpu preflight test"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("preflight GPU test could not create its device");
    let redraws = Arc::new(AtomicUsize::new(0));
    let engine = Engine::new(
        &adapter,
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8Unorm,
        None,
        Shell::new(Redraws(Arc::clone(&redraws))),
    );
    let renderer = Renderer::new(engine, Font::DEFAULT, Pixels(16.0));
    Some((renderer, device, queue, redraws))
}

fn target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("iced_wgpu preflight target"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn primitive(
    slot: usize,
    ready: &Arc<AtomicBool>,
    counts: &Arc<Counts>,
) -> TestPrimitive {
    TestPrimitive {
        slot,
        ready: Arc::clone(ready),
        counts: Arc::clone(counts),
        schedules_retry: false,
        requests_redraw: false,
    }
}

#[test]
fn presentable_due_work_can_request_followup_without_deferring_the_picture() {
    let Some((mut renderer, device, _queue, redraws)) = test_renderer() else {
        eprintln!("skipping preflight GPU test; set {GPU_TEST}=1 to run it");
        return;
    };
    let viewport = Viewport::with_physical_size(Size::new(64, 64), 1.0);
    let frame = Rectangle::with_size(Size::new(64.0, 64.0));
    let ready = Arc::new(AtomicBool::new(true));
    let counts = Arc::new(Counts::default());
    let mut waiting = primitive(0, &ready, &counts);
    waiting.requests_redraw = true;
    primitive::Renderer::draw_primitive(&mut renderer, frame, waiting);
    let encoder = renderer.prepare_window(&viewport).unwrap();
    assert_eq!(redraws.load(Ordering::SeqCst), 1);
    assert_eq!(renderer.prepared_ui_count, 1);
    let (_texture, view) = target(&device);
    let submission = renderer.present_prepared(encoder, &view, &viewport, None);
    let _ = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    assert_eq!(renderer.submit_count, 1);
    assert_eq!(counts.drawn[0].load(Ordering::SeqCst), 1);

    // A fresh ready primitive with no due work neither inherits nor issues
    // another request; the previous request is not a persistent mode.
    core::Renderer::reset(&mut renderer, frame);
    primitive::Renderer::draw_primitive(
        &mut renderer,
        frame,
        primitive(0, &ready, &counts),
    );
    let encoder = renderer.prepare_window(&viewport).unwrap();
    renderer.discard_prepared(encoder);
    assert_eq!(redraws.load(Ordering::SeqCst), 1);

    // A clipped primitive cannot wake the window, and offscreen rendering
    // must remain independent of native follow-up scheduling.
    core::Renderer::reset(&mut renderer, frame);
    let mut clipped = primitive(0, &ready, &counts);
    clipped.requests_redraw = true;
    primitive::Renderer::draw_primitive(
        &mut renderer,
        Rectangle::new(Point::new(1000.0, 1000.0), Size::new(10.0, 10.0)),
        clipped,
    );
    let encoder = renderer.prepare_window(&viewport).unwrap();
    renderer.discard_prepared(encoder);
    assert_eq!(redraws.load(Ordering::SeqCst), 1);

    core::Renderer::reset(&mut renderer, frame);
    let mut waiting = primitive(0, &ready, &counts);
    waiting.requests_redraw = true;
    primitive::Renderer::draw_primitive(&mut renderer, frame, waiting);
    let submission = renderer.present(
        None,
        wgpu::TextureFormat::Rgba8Unorm,
        &view,
        &viewport,
    );
    let _ = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    assert_eq!(redraws.load(Ordering::SeqCst), 1);
}

#[test]
fn scheduled_refusal_gets_one_bridge_and_unscheduled_refusal_keeps_waking() {
    let Some((mut renderer, device, _queue, redraws)) = test_renderer() else {
        eprintln!("skipping preflight GPU test; set {GPU_TEST}=1 to run it");
        return;
    };
    let viewport = Viewport::with_physical_size(Size::new(64, 64), 1.0);
    let frame = Rectangle::with_size(Size::new(64.0, 64.0));
    let blocked = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(true));
    let counts = Arc::new(Counts::default());
    let mut timed = primitive(0, &blocked, &counts);
    timed.schedules_retry = true;
    primitive::Renderer::draw_primitive(&mut renderer, frame, timed);
    // A later ready instance must not reset an earlier refusal's episode.
    primitive::Renderer::draw_primitive(
        &mut renderer,
        frame,
        primitive(1, &ready, &counts),
    );
    assert!(renderer.prepare_window(&viewport).is_none());
    assert!(renderer.prepare_window(&viewport).is_none());
    assert_eq!(redraws.load(Ordering::SeqCst), 1);
    assert_eq!(counts.prepared[0].load(Ordering::SeqCst), 2);
    assert_eq!(counts.prepared[1].load(Ordering::SeqCst), 2);
    assert_eq!(renderer.prepared_ui_count, 0);
    assert_eq!(renderer.submit_count, 0);

    blocked.store(true, Ordering::SeqCst);
    let encoder = renderer.prepare_window(&viewport).unwrap();
    let (_texture, view) = target(&device);
    let submission = renderer.present_prepared(encoder, &view, &viewport, None);
    let _ = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    assert!(!renderer.deferred_retry_started);
    blocked.store(false, Ordering::SeqCst);
    assert!(renderer.prepare_window(&viewport).is_none());
    assert!(renderer.prepare_window(&viewport).is_none());
    assert_eq!(redraws.load(Ordering::SeqCst), 2);

    // Default primitives cannot promise a widget timer. Any such refusal
    // keeps the renderer's original per-attempt wake behavior, even alongside
    // a self-scheduled primitive whose episode is already active.
    ready.store(false, Ordering::SeqCst);
    assert!(renderer.prepare_window(&viewport).is_none());
    assert!(renderer.prepare_window(&viewport).is_none());
    assert_eq!(redraws.load(Ordering::SeqCst), 4);
    ready.store(true, Ordering::SeqCst);
    assert!(renderer.prepare_window(&viewport).is_none());
    assert!(renderer.prepare_window(&viewport).is_none());
    assert_eq!(redraws.load(Ordering::SeqCst), 5);

    core::Renderer::reset(&mut renderer, frame);
    let encoder = renderer.prepare_window(&viewport).unwrap();
    renderer.discard_prepared(encoder);
    assert!(!renderer.deferred_retry_started);
    let _ = device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

#[test]
fn native_preflight_refuses_before_ui_then_ready_and_offscreen_prepare_once() {
    let Some((mut renderer, device, _queue, redraws)) = test_renderer() else {
        eprintln!("skipping preflight GPU test; set {GPU_TEST}=1 to run it");
        return;
    };
    let viewport = Viewport::with_physical_size(Size::new(64, 64), 1.0);
    let frame = Rectangle::with_size(Size::new(64.0, 64.0));
    let ready = Arc::new(AtomicBool::new(false));
    let counts = Arc::new(Counts::default());

    core::Renderer::fill_quad(
        &mut renderer,
        core::renderer::Quad {
            bounds: frame,
            ..Default::default()
        },
        Color::BLACK,
    );
    primitive::Renderer::draw_primitive(
        &mut renderer,
        frame,
        primitive(0, &ready, &counts),
    );
    primitive::Renderer::draw_primitive(
        &mut renderer,
        frame,
        primitive(1, &ready, &counts),
    );

    assert!(renderer.prepare_window(&viewport).is_none());
    assert_eq!(renderer.prepared_ui_count, 0);
    assert_eq!(renderer.submit_count, 0);
    assert_eq!(counts.prepared[0].load(Ordering::SeqCst), 1);
    assert_eq!(counts.prepared[1].load(Ordering::SeqCst), 1);
    assert_eq!(counts.checked[0].load(Ordering::SeqCst), 1);
    assert_eq!(counts.checked[1].load(Ordering::SeqCst), 1);
    assert_eq!(counts.drawn[0].load(Ordering::SeqCst), 0);
    assert_eq!(redraws.load(Ordering::SeqCst), 1);

    ready.store(true, Ordering::SeqCst);
    let encoder = renderer
        .prepare_window(&viewport)
        .expect("ready retry was still refused");
    assert_eq!(renderer.prepared_ui_count, 1);
    assert_eq!(renderer.submit_count, 0);
    assert_eq!(counts.prepared[0].load(Ordering::SeqCst), 2);
    assert_eq!(counts.prepared[1].load(Ordering::SeqCst), 2);
    let (_texture, view) = target(&device);
    let submission = renderer.present_prepared(
        encoder,
        &view,
        &viewport,
        Some(Color::BLACK),
    );
    let _ = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    assert_eq!(counts.drawn[0].load(Ordering::SeqCst), 1);
    assert_eq!(counts.drawn[1].load(Ordering::SeqCst), 1);
    assert_eq!(renderer.submit_count, 1);
    assert_eq!(redraws.load(Ordering::SeqCst), 1);

    core::Renderer::reset(&mut renderer, frame);
    ready.store(false, Ordering::SeqCst);
    primitive::Renderer::draw_primitive(
        &mut renderer,
        Rectangle::new(Point::new(1000.0, 1000.0), Size::new(10.0, 10.0)),
        primitive(0, &ready, &counts),
    );
    let encoder = renderer
        .prepare_window(&viewport)
        .expect("an invisible refusing primitive blocked presentation");
    assert_eq!(counts.prepared[0].load(Ordering::SeqCst), 3);
    assert_eq!(counts.checked[0].load(Ordering::SeqCst), 2);
    let (_texture, view) = target(&device);
    let submission = renderer.present_prepared(encoder, &view, &viewport, None);
    let _ = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    assert_eq!(counts.drawn[0].load(Ordering::SeqCst), 1);

    core::Renderer::reset(&mut renderer, frame);
    primitive::Renderer::draw_primitive(
        &mut renderer,
        frame,
        primitive(0, &ready, &counts),
    );
    let (_texture, view) = target(&device);
    let submission = renderer.present(
        Some(Color::BLACK),
        wgpu::TextureFormat::Rgba8Unorm,
        &view,
        &viewport,
    );
    let _ = device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    assert_eq!(counts.prepared[0].load(Ordering::SeqCst), 4);
    assert_eq!(counts.checked[0].load(Ordering::SeqCst), 3);
    assert_eq!(counts.drawn[0].load(Ordering::SeqCst), 2);
    assert_eq!(redraws.load(Ordering::SeqCst), 1);
}
