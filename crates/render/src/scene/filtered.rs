//! Scene-side scheduling and presentation of complete filtered panoramas.

use super::*;

fn filtered_progress_deadline(now: Instant, due: Option<Instant>, retirement_full: bool) -> Next {
    if retirement_full {
        Next::At(due.map_or(now + DRAW_RETIREMENT_RETRY, |due| {
            due.min(now + DRAW_RETIREMENT_RETRY)
        }))
    } else {
        due.map_or(Next::Never, Next::At)
    }
}

fn selected_filtered_picture<T: Clone>(
    stopped: bool,
    offered: Option<&T>,
    shown: Option<T>,
) -> Option<T> {
    if stopped {
        shown
    } else {
        offered.cloned().or(shown)
    }
}

impl Scene {
    /// Selected filtered playback is driven by shell events, not compositor
    /// redraw callbacks. Other projection paths retain their existing policy.
    pub fn event_playback(&self) -> bool {
        self.show
            .as_ref()
            .is_some_and(|show| show.filtered.is_some())
    }

    /// Absolute next media deadline. A replaced subscription cancels the old
    /// timer on pause, seek, capture replacement or a changed due source.
    pub fn playback_deadline(&self) -> Option<Instant> {
        self.event_playback()
            .then(|| self.playback_deadline.get())
            .flatten()
    }

    pub fn progress(&self, now: Instant) -> Next {
        if !self.event_playback() {
            return self.pump(now);
        }
        self.playback_deadline.set(None);
        let result = self.progress_filtered(now);
        match result {
            Ok(next) => {
                if let Next::At(deadline) = next {
                    self.playback_deadline.set(Some(deadline));
                }
                next
            }
            Err(error) => {
                self.stalled.fail_now(error);
                self.pump_inner(now)
            }
        }
    }

    fn progress_filtered(&self, now: Instant) -> Fallible<Next> {
        let show = self.show.as_ref().expect("selected filtered capture");
        let capture = show.filtered.as_ref().expect("selected filtered facade");
        if self.stalled.stopped() {
            return Ok(self.pump_inner(now));
        }
        capture.set_progress_wake(&self.ready_wake)?;
        let held = Holding {
            horizon: self.horizon.get(),
            clock: self.clock.get(),
            forced: self.forced.get(),
            readout: self.readout.get(),
        };

        // The bound is the existing completed-picture FIFO, not a new source
        // refresh cadence. A subsequent coalesced event continues catch-up.
        for _ in 0..FilteredCaptureFacade::READY_CAPACITY {
            let before = self.frame_stamp();
            let was_ready = before
                .as_ref()
                .map(|stamp| capture.acknowledged(stamp))
                .transpose()?
                .unwrap_or(false);
            // Pumping promotes at most one real source and retains startup,
            // exact-seek and autoplay clock gates.
            let next = self.pump_inner(now);
            if matches!(next, Next::Stopped(_)) {
                return Ok(next);
            }
            if !capture.is_attached()? {
                self.wait_for_filtered_decode();
                return Ok(Next::Never);
            }

            let Some(current) = show.view(held) else {
                self.wait_for_filtered_decode();
                return Ok(Next::Never);
            };
            let sources = std::iter::once(current.clone())
                .chain((0..6).filter_map(|ahead| show.prepared_view(held, ahead)))
                .collect::<Vec<_>>();
            for source in &sources {
                let accepted = capture.accepted_stamp()?;
                if resident_stamp_follows(accepted.as_ref(), &source.frames.stamp())
                    && (!capture.wants_source()?
                        || !capture.try_submit(
                            source.frames.clone(),
                            filtered_source_reframe(source, self.sampling.get()),
                        )?)
                {
                    break;
                }
            }
            let exhausted = matches!(&show.playing.borrow().source,
                Source::Live(player) if player.is_input_exhausted());
            if exhausted
                && capture.accepted_stamp()? == sources.last().map(|view| view.frames.stamp())
                && !capture.is_finished()?
            {
                let _ = capture.try_finish()?;
            }

            if let Some(picture) = capture.install_due(&current.frames.stamp())? {
                let target = show
                    .replay
                    .borrow()
                    .as_ref()
                    .filter(|replay| replay.accuracy == Accuracy::Exact)
                    .map(|replay| replay.target);
                if target.is_none_or(|target| picture.frame().index() == target) {
                    let mut complete = current;
                    complete.complete = Some(CompleteFiltered(picture));
                    self.filtered_display.keep(&complete);
                }
            } else {
                self.wait_for_filtered_decode();
                return Ok(filtered_progress_deadline(
                    now,
                    None,
                    capture.source_retirement_full()?,
                ));
            }

            // A completed frame may be logically consumed without submitting
            // an obsolete screen update. Every real source still traverses
            // the exact source, map, color and temporal transactions.
            let due = match &show.playing.borrow().source {
                Source::Live(player) => player.next_due(),
                Source::Stepped(_) => None,
            };
            let replaying = show.replay.borrow().is_some();
            if !replaying && due.is_none_or(|due| due > now) {
                self.wait_for_filtered_decode();
                return Ok(filtered_progress_deadline(
                    now,
                    due,
                    capture.source_retirement_full()?,
                ));
            }
            if was_ready && self.frame_stamp() == before && !replaying {
                self.wait_for_filtered_decode();
                return Ok(Next::Never);
            }
        }
        self.ready_wake.notify();
        Ok(Next::Never)
    }

    fn wait_for_filtered_decode(&self) {
        if self.ready_wake.listening()
            && let Some(show) = self.show.as_ref()
            && let Source::Live(player) = &show.playing.borrow().source
        {
            let _ = player.wait_for_prepared_decode(std::task::Waker::from(Arc::new(
                self.ready_wake.clone(),
            )));
        }
    }
}

impl ScenePipeline {
    pub(super) fn prepare_filtered(
        &mut self,
        primitive: &ScenePrimitive,
        aspect: f32,
    ) -> Fallible<()> {
        self.resident_draw = ResidentDrawSelection::None;
        self.flow_draw = FlowDraw::Nothing;
        // Renderer attachment is the sole bootstrap exception. Source admission
        // and FIFO publication belong to Scene::progress, not surface callbacks.
        if !primitive.stalled.stopped()
            && let Some(capture) = primitive.filtered_capture.as_ref()
        {
            capture.set_progress_wake(&primitive.ready_wake)?;
            if !capture.is_attached()? {
                capture.attach(self.one_xs_gpu.clone())?;
                primitive.ready_wake.notify();
            }
        }
        if let Some(old) = self.resident_one_xs.take() {
            self.retired_one_xs.push(old);
            self.resident_completed_view = None;
        }
        let mut index = 0;
        while index < self.retired_one_xs.len() {
            match self.retired_one_xs[index]
                .1
                .drain_replaced_after_external_poll()?
            {
                ResidentDrain::Drained => {
                    self.retired_one_xs.remove(index);
                }
                ResidentDrain::Pending | ResidentDrain::FailClosedRetained => index += 1,
            }
        }
        let selected = selected_filtered_picture(
            primitive.stalled.stopped(),
            primitive
                .view
                .as_ref()
                .filter(|view| view.complete.is_some()),
            primitive.shown.get(),
        );
        if let Some(view) = selected
            && let Some(picture) = view.complete.as_ref()
        {
            if picture.0.frame() != &view.frames.stamp() {
                return Err("filtered picture differs from its Scene view".into());
            }
            let reframe = self.resident_reframe(primitive, &view, aspect);
            self.filtered_draw = Some(picture.0.prepare_view(
                self.one_xs_gpu.device(),
                &reframe,
                self.format,
            )?);
            primitive.shown.keep(&view);
        }
        if let Some(request) = primitive.shutter.take() {
            self.shoot_filtered(primitive, request, aspect);
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
            let panorama = view
                .complete
                .as_ref()
                .filter(|picture| picture.0.frame() == &view.frames.stamp())
                .ok_or("filtered screenshot output differs from the shown frame")?;
            let reframe = self.resident_reframe(primitive, &view, aspect);
            let draw = panorama
                .0
                .prepare_view(self.one_xs_gpu.device(), &reframe, self.format)?;
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

/// The worker materializes a gamma-RGB canonical world panorama. Its chart is
/// source-owned: display camera, window shape and horizon policy cannot move
/// it or advance a different temporal history. Surface transfer is applied
/// only by the later projector draw.
pub(super) fn filtered_source_reframe(view: &View, sampling: Sampling) -> Reframe {
    Reframe::new(
        &view.lenses,
        view.frames.size,
        Camera::default(),
        Held {
            body_from_world: view.body_from_world,
            rolling: view.held.rolling,
        },
        2.0,
        false,
        sampling,
    )
    .with_samples(view.frames.samples)
    .with_table(view.table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn paused_complete_pipeline_has_no_progress_timer() {
        assert_eq!(
            filtered_progress_deadline(Instant::now(), None, false),
            Next::Never
        );
    }

    #[test]
    fn media_deadline_is_absolute_and_only_retirement_full_adds_a_poll() {
        let now = Instant::now();
        let due = now + Duration::from_millis(33);
        assert_eq!(
            filtered_progress_deadline(now, Some(due), false),
            Next::At(due)
        );
        assert_eq!(
            filtered_progress_deadline(now, Some(due), true),
            Next::At(now + DRAW_RETIREMENT_RETRY)
        );
        assert_eq!(
            filtered_progress_deadline(now, None, true),
            Next::At(now + DRAW_RETIREMENT_RETRY)
        );
    }

    #[test]
    fn terminal_draw_retains_exact_shown_owner_after_later_logical_completion() {
        let shown = Arc::new(7);
        let later = Arc::new(11);
        let selected = selected_filtered_picture(true, Some(&later), Some(shown.clone())).unwrap();
        assert!(Arc::ptr_eq(&selected, &shown));
        assert!(!Arc::ptr_eq(&selected, &later));
        let active = selected_filtered_picture(false, Some(&later), Some(shown)).unwrap();
        assert!(Arc::ptr_eq(&active, &later));
    }

    fn view(body_from_world: Quat, held: Held) -> View {
        let stamp = FrameStamp::for_test(7, Duration::from_millis(233), None);
        View {
            lenses: Arc::from([]),
            table: Table::REST,
            frames: Arc::new(Frames::empty_for_test(
                stamp,
                Size {
                    width: 1920,
                    height: 960,
                },
            )),
            held,
            body_from_world,
            one_xs: None,
            resident_one_xs: None,
            filtered: None,
            complete: None,
            one_xs_profile: None,
        }
    }

    #[test]
    fn display_camera_aspect_and_horizon_do_not_change_the_source_reframe() {
        let source_held = Held {
            body_from_world: Quat::from_rotation_vector([0.19, -0.31, 0.47]).conjugate(),
            rolling: Some(Rolling {
                turn: [0.003, -0.007, 0.011],
                axis: [0.0, 1.0],
            }),
        };
        let locked = view(source_held.body_from_world, source_held);
        let free = view(
            source_held.body_from_world,
            Held {
                body_from_world: Quat::IDENTITY,
                rolling: source_held.rolling,
            },
        );
        let locked_camera = Camera {
            yaw: -1.4,
            pitch: 0.6,
            fov: 1.1,
        };
        let free_camera = Camera {
            yaw: 2.2,
            pitch: -0.3,
            fov: 2.0,
        };

        let locked_display = Reframe::new(
            &locked.lenses,
            locked.frames.size,
            locked_camera,
            locked.held,
            16.0 / 9.0,
            false,
            Sampling::default(),
        );
        let free_display = Reframe::new(
            &free.lenses,
            free.frames.size,
            free_camera,
            free.held,
            1.0,
            false,
            Sampling::default(),
        );
        assert_ne!(locked_display.bytes(), free_display.bytes());

        let locked_source = filtered_source_reframe(&locked, Sampling::default());
        let free_source = filtered_source_reframe(&free, Sampling::default());
        assert_eq!(locked_source.bytes(), free_source.bytes());
    }

    #[test]
    fn a_free_display_retains_the_nonidentity_canonical_source_pose() {
        let pose = Quat::from_rotation_vector([-0.23, 0.41, 0.17]).conjugate();
        let source_held = Held {
            body_from_world: pose,
            rolling: None,
        };
        let free = view(
            pose,
            Held {
                body_from_world: Quat::IDENTITY,
                rolling: None,
            },
        );

        assert_ne!(free.body_from_world, Quat::IDENTITY);
        assert_eq!(free.body_from_world, pose);
        let source = filtered_source_reframe(&free, Sampling::default());
        let expected = Reframe::new(
            &free.lenses,
            free.frames.size,
            Camera::default(),
            source_held,
            2.0,
            false,
            Sampling::default(),
        )
        .with_samples(free.frames.samples)
        .with_table(free.table);
        assert_eq!(source.bytes(), expected.bytes());
    }
}
