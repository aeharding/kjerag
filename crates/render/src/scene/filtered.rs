//! Scene-side scheduling and presentation of complete filtered panoramas.

use super::*;

impl ScenePipeline {
    pub(super) fn prepare_filtered(
        &mut self,
        primitive: &ScenePrimitive,
        aspect: f32,
    ) -> Fallible<()> {
        self.resident_draw = ResidentDrawSelection::None;
        self.flow_draw = FlowDraw::Nothing;
        let shown = primitive.shown.get();

        // A terminal capture performs no more stateful worker operations.
        // Its last completed panorama remains independently owned by Shown.
        if primitive.stalled.stopped() {
            if let Some(shown) = shown.as_ref()
                && let Some(shown_capture) = shown.filtered.as_ref()
                && let Some(panorama) = shown_capture.installed()?
                && panorama.frame() == &shown.frames.stamp()
            {
                let device = self.one_xs_gpu.device().clone();
                let reframe = self.resident_reframe(primitive, shown, aspect);
                let draw = panorama.prepare_view(&device, &reframe, self.format)?;
                if draw.frame() != &shown.frames.stamp() {
                    return Err("filtered draw differs from the shown Scene frame".into());
                }
                self.filtered_draw = Some(draw);
            }
            if let Some(request) = primitive.shutter.take() {
                self.shoot_filtered(primitive, request, aspect);
            }
            return Ok(());
        }

        // This path owns its own upstream stitch session. Retire any raw-path
        // attachment left by an earlier selection without publishing from it.
        if let Some(old) = self.resident_one_xs.take() {
            self.retired_one_xs.push(old);
            self.resident_completed_view = None;
        }
        let mut raw_retirement_pending = false;
        let mut index = 0;
        while index < self.retired_one_xs.len() {
            match self.retired_one_xs[index]
                .1
                .drain_replaced_after_external_poll()?
            {
                ResidentDrain::Drained => {
                    self.retired_one_xs.remove(index);
                }
                ResidentDrain::Pending => {
                    raw_retirement_pending = true;
                    primitive
                        .resident_refresh
                        .store(true, AtomicOrdering::Release);
                    index += 1;
                }
                ResidentDrain::FailClosedRetained => index += 1,
            }
        }

        let capture = primitive.filtered_capture.as_ref();
        if let Some(capture) = capture {
            capture.attach(self.one_xs_gpu.clone())?;
        }
        let offered = primitive.view.as_ref().filter(|view| {
            capture.is_some_and(|capture| {
                view.filtered
                    .as_ref()
                    .is_some_and(|owner| owner.same_capture(capture))
            })
        });
        let due = offered.map(|view| view.frames.stamp());

        let mut installed_due = None;
        if let (Some(capture), Some(due)) = (capture, due.as_ref()) {
            installed_due = capture.install_due(due)?;
        }
        let due_acknowledged = match (capture, due.as_ref()) {
            (Some(capture), Some(due)) => capture.acknowledged(due)?,
            _ => false,
        };
        let shown_is_due = offered.is_some_and(|view| {
            shown.as_ref().is_some_and(|shown| {
                shown.frames.stamp() == view.frames.stamp()
                    && shown.filtered.as_ref().is_some_and(|shown_capture| {
                        capture.is_some_and(|capture| shown_capture.same_capture(capture))
                    })
            })
        });
        let due_unready = due.is_some() && !due_acknowledged && !shown_is_due;
        self.redraw_after_prepare =
            due_redraw_after_prepare(due_unready, primitive.stalled.stopped());
        if self.redraw_after_prepare {
            primitive
                .resident_refresh
                .store(true, AtomicOrdering::Release);
        }

        let device = self.one_xs_gpu.device().clone();
        if let (Some(view), Some(panorama)) = (offered, installed_due.as_ref())
            && primitive
                .resident_target
                .is_none_or(|target| panorama.frame().index() == target)
        {
            if panorama.frame() != &view.frames.stamp() {
                return Err("filtered panorama differs from the offered Scene frame".into());
            }
            let reframe = self.resident_reframe(primitive, view, aspect);
            let draw = panorama.prepare_view(&device, &reframe, self.format)?;
            if draw.frame() != &view.frames.stamp() {
                return Err("filtered draw differs from the offered Scene frame".into());
            }
            self.filtered_draw = Some(draw);
            primitive.shown.keep(view);
        } else if let Some(shown) = shown.as_ref()
            && let Some(shown_capture) = shown.filtered.as_ref()
            && let Some(panorama) = shown_capture.installed()?
            && panorama.frame() == &shown.frames.stamp()
        {
            let reframe = self.resident_reframe(primitive, shown, aspect);
            let draw = panorama.prepare_view(&device, &reframe, self.format)?;
            if draw.frame() != &shown.frames.stamp() {
                return Err("filtered draw differs from the shown Scene frame".into());
            }
            self.filtered_draw = Some(draw);
        }

        let mut admitted = false;
        if !primitive.stalled.stopped()
            && let (Some(capture), Some(offered)) = (capture, offered)
        {
            let accepted = capture.accepted_stamp()?;
            let next = std::iter::once(offered)
                .chain(primitive.filtered_ahead.iter())
                .filter(|view| {
                    view.filtered
                        .as_ref()
                        .is_some_and(|owner| owner.same_capture(capture))
                })
                .find(|view| resident_stamp_follows(accepted.as_ref(), &view.frames.stamp()));
            if let Some(view) = next {
                let reframe = filtered_source_reframe(primitive, view, aspect);
                admitted = capture.try_submit(view.frames.clone(), reframe)?;
                if admitted {
                    primitive.stalled.landed();
                }
            }

            let accepted = capture.accepted_stamp()?;
            let last = std::iter::once(offered)
                .chain(primitive.filtered_ahead.iter())
                .rfind(|view| {
                    view.filtered
                        .as_ref()
                        .is_some_and(|owner| owner.same_capture(capture))
                })
                .map(|view| view.frames.stamp());
            if primitive.filtered_eof
                && accepted.as_ref() == last.as_ref()
                && !capture.is_finished()?
            {
                // A full ready FIFO needs presentation, not another redraw.
                // While paused, retain it without spinning. An unready due
                // source below still gets the normal worker wake/retry.
                let _ = capture.try_finish()?;
            }
        }

        if let Some(request) = primitive.shutter.take() {
            self.shoot_filtered(primitive, request, aspect);
        }

        // Worker completion may replace a renderer retry only when an exact
        // due output is genuinely pending. Missing decode lookahead keeps the
        // ordinary refresh alive so more sources can be prepared and admitted.
        if self.redraw_after_prepare
            && !raw_retirement_pending
            && let (Some(capture), Some(due)) = (capture, due.as_ref())
            && capture.wait_for_frame(due, &primitive.ready_wake)?
        {
            self.redraw_after_prepare = false;
            primitive
                .resident_refresh
                .store(false, AtomicOrdering::Release);
            primitive
                .resident_waiting
                .store(true, AtomicOrdering::Release);
        } else if self.redraw_after_prepare && !admitted {
            primitive
                .resident_refresh
                .store(true, AtomicOrdering::Release);
        }
        Ok(())
    }

    fn shoot_filtered(&self, primitive: &ScenePrimitive, request: Request, aspect: f32) {
        let Some(view) = primitive.shown.get() else {
            if let Some(error) = primitive.stalled.terminal() {
                capture::reject(request, error);
            } else {
                primitive.shutter.arm(request);
                primitive
                    .resident_refresh
                    .store(true, AtomicOrdering::Release);
            }
            return;
        };
        let result = (|| {
            let capture = view
                .filtered
                .as_ref()
                .ok_or("filtered screenshot lost its capture owner")?;
            let panorama = capture
                .installed()?
                .filter(|panorama| panorama.frame() == &view.frames.stamp())
                .ok_or("filtered screenshot output differs from the shown frame")?;
            let reframe = self.resident_reframe(primitive, &view, aspect);
            let draw = panorama.prepare_view(self.one_xs_gpu.device(), &reframe, self.format)?;
            let at = Stamp {
                index: view.frames.index,
                time: view.frames.timestamp,
            };
            self.expose_filtered(request.width, aspect, at, draw)
        })();
        capture::deliver(result, request.then);
    }

    fn expose_filtered(
        &self,
        width: u32,
        aspect: f32,
        at: Stamp,
        draw: PreparedCorrectionDraw,
    ) -> Fallible<Pending> {
        if draw.frame().index() != at.index || draw.frame().timestamp() != at.time {
            return Err("filtered screenshot draw differs from the shown Scene frame".into());
        }
        let device = self.one_xs_gpu.device();
        let queue = self.one_xs_gpu.queue();
        let order = Order::of(self.format)?;
        let size = capture::fitted(width, aspect)?;
        let stride = capture::stride(size.width);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filtered panorama capture"),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filtered panorama capture"),
            size: u64::from(stride) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let texture_view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("filtered panorama capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            draw.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size.height),
                },
            },
            size.extent(),
        );
        Ok(Pending {
            device: device.clone(),
            _texture: texture,
            readback,
            submission: queue.submit([encoder.finish()]),
            size,
            stride,
            order,
            at,
        })
    }
}

/// The worker materializes a gamma-RGB body panorama. Surface transfer is
/// applied only by the later projector draw.
fn filtered_source_reframe(primitive: &ScenePrimitive, view: &View, aspect: f32) -> Reframe {
    Reframe::new(
        &view.lenses,
        view.frames.size,
        primitive.camera,
        view.held,
        aspect,
        false,
        primitive.sampling,
    )
    .with_samples(view.frames.samples)
    .with_table(view.table)
}
