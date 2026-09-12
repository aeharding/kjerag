//! Draw custom primitives.
use crate::core::{self, Rectangle};
use crate::graphics::Viewport;
use crate::graphics::futures::{MaybeSend, MaybeSync};

use rustc_hash::FxHashMap;
use std::any::{Any, TypeId};
use std::fmt::Debug;

/// A batch of primitives.
pub type Batch = Vec<Instance>;

/// A set of methods which allows a [`Primitive`] to be rendered.
pub trait Primitive: Debug + MaybeSend + MaybeSync + 'static {
    /// The shared renderer of this [`Primitive`].
    ///
    /// Normally, this will contain a bunch of [`wgpu`] state; like
    /// a rendering pipeline, buffers, and textures.
    ///
    /// All instances of this [`Primitive`] type will share the same
    /// [`Renderer`].
    type Pipeline: Pipeline + MaybeSend + MaybeSync;

    /// Processes the [`Primitive`], allowing for GPU buffer allocation.
    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    );

    /// Returns whether this [`Primitive`] is ready to be presented.
    ///
    /// This is queried immediately after [`prepare`](Self::prepare). Window
    /// renderers may defer presentation while a visible primitive is not
    /// ready, without changing offscreen rendering behavior.
    fn is_presentable(&self, _pipeline: &Self::Pipeline) -> bool {
        true
    }

    /// Whether this primitive schedules further retries after the renderer
    /// requests the first redraw of an unavailable frame.
    ///
    /// Opt in only when its widget guarantees future redraws even while
    /// paused. Otherwise the renderer continues requesting every retry.
    /// This local bridge supports one self-scheduled widget per window;
    /// multiple independent retry owners must leave this disabled.
    fn schedules_retry(&self, _pipeline: &Self::Pipeline) -> bool {
        false
    }

    /// Request a follow-up window redraw after this preparation.
    ///
    /// A video may keep its old picture presentable while its currently due
    /// picture is still being prepared. This lets its widget sleep until the
    /// video deadline and request another tick only if that due work is not
    /// ready, instead of redrawing merely to poll future work. Offscreen
    /// preparation ignores this request. This does not defer presentation.
    fn requests_redraw_after_prepare(
        &self,
        _pipeline: &Self::Pipeline,
    ) -> bool {
        false
    }

    /// Draws the [`Primitive`] in the given [`wgpu::RenderPass`].
    ///
    /// When possible, this should be implemented over [`render`](Self::render)
    /// since reusing the existing render pass should be considerably more
    /// efficient than issuing a new one.
    ///
    /// The viewport and scissor rect of the render pass provided is set
    /// to the bounds and clip bounds of the [`Primitive`], respectively.
    ///
    /// If you have complex composition needs, then you can leverage
    /// [`render`](Self::render) by returning `false` here.
    ///
    /// By default, it does nothing and returns `false`.
    fn draw(
        &self,
        _pipeline: &Self::Pipeline,
        _render_pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        false
    }

    /// Renders the [`Primitive`], using the given [`wgpu::CommandEncoder`].
    ///
    /// This will only be called if [`draw`](Self::draw) returns `false`.
    ///
    /// By default, it does nothing.
    fn render(
        &self,
        _pipeline: &Self::Pipeline,
        _encoder: &mut wgpu::CommandEncoder,
        _target: &wgpu::TextureView,
        _clip_bounds: &Rectangle<u32>,
    ) {
    }
}

/// The pipeline of a graphics [`Primitive`].
pub trait Pipeline: Any + MaybeSend + MaybeSync {
    /// Creates the [`Pipeline`] of a [`Primitive`].
    ///
    /// This will only be called once, when the first [`Primitive`] with this kind
    /// of [`Pipeline`] is encountered.
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self
    where
        Self: Sized;

    /// Trims any cached data in the [`Pipeline`].
    ///
    /// This will normally be called at the end of a frame.
    fn trim(&mut self) {}
}

pub(crate) trait Stored:
    Debug + MaybeSend + MaybeSync + 'static
{
    fn prepare(
        &self,
        storage: &mut Storage,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        bounds: &Rectangle,
        viewport: &Viewport,
    );

    fn draw(
        &self,
        storage: &Storage,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> bool;

    fn is_presentable(&self, storage: &Storage) -> bool;

    fn schedules_retry(&self, storage: &Storage) -> bool;

    fn requests_redraw_after_prepare(&self, storage: &Storage) -> bool;

    fn render(
        &self,
        storage: &Storage,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    );
}

#[derive(Debug)]
struct BlackBox<P: Primitive> {
    primitive: P,
}

impl<P: Primitive> Stored for BlackBox<P> {
    fn prepare(
        &self,
        storage: &mut Storage,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        if !storage.has::<P>() {
            storage.store::<P, _>(P::Pipeline::new(device, queue, format));
        }

        let renderer = storage
            .get_mut::<P>()
            .expect("renderer should be initialized")
            .downcast_mut::<P::Pipeline>()
            .expect("renderer should have the proper type");

        self.primitive
            .prepare(renderer, device, queue, bounds, viewport);
    }

    fn is_presentable(&self, storage: &Storage) -> bool {
        let renderer = storage
            .get::<P>()
            .expect("renderer should be initialized")
            .downcast_ref::<P::Pipeline>()
            .expect("renderer should have the proper type");

        self.primitive.is_presentable(renderer)
    }

    fn schedules_retry(&self, storage: &Storage) -> bool {
        let renderer = storage
            .get::<P>()
            .expect("renderer should be initialized")
            .downcast_ref::<P::Pipeline>()
            .expect("renderer should have the proper type");

        self.primitive.schedules_retry(renderer)
    }

    fn requests_redraw_after_prepare(&self, storage: &Storage) -> bool {
        let renderer = storage
            .get::<P>()
            .expect("renderer should be initialized")
            .downcast_ref::<P::Pipeline>()
            .expect("renderer should have the proper type");

        self.primitive.requests_redraw_after_prepare(renderer)
    }

    fn draw(
        &self,
        storage: &Storage,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let renderer = storage
            .get::<P>()
            .expect("renderer should be initialized")
            .downcast_ref::<P::Pipeline>()
            .expect("renderer should have the proper type");

        self.primitive.draw(renderer, render_pass)
    }

    fn render(
        &self,
        storage: &Storage,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let renderer = storage
            .get::<P>()
            .expect("renderer should be initialized")
            .downcast_ref::<P::Pipeline>()
            .expect("renderer should have the proper type");

        self.primitive
            .render(renderer, encoder, target, clip_bounds);
    }
}

#[derive(Debug)]
/// An instance of a specific [`Primitive`].
pub struct Instance {
    /// The bounds of the [`Instance`].
    pub(crate) bounds: Rectangle,

    /// The [`Primitive`] to render.
    pub(crate) primitive: Box<dyn Stored>,
}

impl Instance {
    /// Creates a new [`Instance`] with the given [`Primitive`].
    pub fn new(bounds: Rectangle, primitive: impl Primitive) -> Self {
        Instance {
            bounds,
            primitive: Box::new(BlackBox { primitive }),
        }
    }
}

/// A renderer than can draw custom primitives.
pub trait Renderer: core::Renderer {
    /// Draws a custom primitive.
    fn draw_primitive(&mut self, bounds: Rectangle, primitive: impl Primitive);
}

/// Stores custom, user-provided types.
#[derive(Default)]
pub struct Storage {
    pipelines: FxHashMap<TypeId, Box<dyn Pipeline>>,
}

impl Storage {
    /// Returns `true` if `Storage` contains a type `T`.
    pub fn has<T: 'static>(&self) -> bool {
        self.pipelines.contains_key(&TypeId::of::<T>())
    }

    /// Inserts the data `T` in to [`Storage`].
    pub fn store<T: 'static, P: Pipeline>(&mut self, pipeline: P) {
        let _ = self.pipelines.insert(TypeId::of::<T>(), Box::new(pipeline));
    }

    /// Returns a reference to the data with type `T` if it exists in [`Storage`].
    pub fn get<T: 'static>(&self) -> Option<&dyn Any> {
        self.pipelines
            .get(&TypeId::of::<T>())
            .map(|pipeline| pipeline.as_ref() as &dyn Any)
    }

    /// Returns a mutable reference to the data with type `T` if it exists in [`Storage`].
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut dyn Any> {
        self.pipelines
            .get_mut(&TypeId::of::<T>())
            .map(|pipeline| pipeline.as_mut() as &mut dyn Any)
    }

    /// Trims the cache of all the pipelines in the [`Storage`].
    pub fn trim(&mut self) {
        for pipeline in self.pipelines.values_mut() {
            pipeline.trim();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestPipeline;

    impl Pipeline for TestPipeline {
        fn new(
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _format: wgpu::TextureFormat,
        ) -> Self {
            Self
        }
    }

    #[derive(Debug)]
    struct ReadyByDefault;

    impl Primitive for ReadyByDefault {
        type Pipeline = TestPipeline;

        fn prepare(
            &self,
            _pipeline: &mut Self::Pipeline,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _bounds: &Rectangle,
            _viewport: &Viewport,
        ) {
        }
    }

    #[derive(Debug)]
    struct NotReady;

    impl Primitive for NotReady {
        type Pipeline = TestPipeline;

        fn prepare(
            &self,
            _pipeline: &mut Self::Pipeline,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _bounds: &Rectangle,
            _viewport: &Viewport,
        ) {
        }

        fn is_presentable(&self, _pipeline: &Self::Pipeline) -> bool {
            false
        }
    }

    #[test]
    fn stored_primitive_forwards_default_and_false_presentability() {
        let default = BlackBox {
            primitive: ReadyByDefault,
        };
        let not_ready = BlackBox {
            primitive: NotReady,
        };
        let mut storage = Storage::default();
        storage.store::<ReadyByDefault, _>(TestPipeline);
        storage.store::<NotReady, _>(TestPipeline);

        assert!(default.is_presentable(&storage));
        assert!(!not_ready.is_presentable(&storage));
        assert!(!default.requests_redraw_after_prepare(&storage));
        assert!(!not_ready.requests_redraw_after_prepare(&storage));
    }
}
