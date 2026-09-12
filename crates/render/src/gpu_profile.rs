//! Opt-in encoder-local timestamps for the real filtered Scene source path.

pub(crate) const ENV: &str = "KJERAG_LIVE_CORRECTION_GPU_PROFILE";

pub(crate) struct Profile {
    active: Option<Active>,
}

struct Active {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    scope: &'static str,
    labels: &'static [&'static str],
    frame: u64,
    used: u32,
}

impl Profile {
    pub(crate) fn begin(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scope: &'static str,
        labels: &'static [&'static str],
        frame: u64,
    ) -> Self {
        if std::env::var_os(ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
            return Self { active: None };
        }
        let required =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        assert!(
            device.features().contains(required),
            "live correction GPU profile needs encoder timestamp queries"
        );
        let count = u32::try_from(labels.len() + 1).expect("GPU profile marker count overflowed");
        let bytes = u64::from(count) * 8;
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("live correction GPU timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live correction GPU timestamp resolve"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live correction GPU timestamp readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.write_timestamp(&queries, 0);
        Self {
            active: Some(Active {
                queries,
                resolve,
                readback,
                scope,
                labels,
                frame,
                used: 1,
            }),
        }
    }

    pub(crate) fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        let Some(active) = &mut self.active else {
            return;
        };
        assert!(
            active.used as usize <= active.labels.len(),
            "live correction GPU profile wrote too many markers"
        );
        let expected = active.labels[(active.used - 1) as usize];
        assert_eq!(label, expected, "live correction GPU profile marker order");
        encoder.write_timestamp(&active.queries, active.used);
        active.used += 1;
    }

    /// Resolve into this same command encoder. The final timestamp precedes
    /// query resolve and readback copy, so those diagnostic operations are not
    /// charged to the measured source intervals.
    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(active) = &self.active else {
            return;
        };
        assert_eq!(
            active.used as usize,
            active.labels.len() + 1,
            "live correction GPU profile marker count"
        );
        let bytes = u64::from(active.used) * 8;
        encoder.resolve_query_set(&active.queries, 0..active.used, &active.resolve, 0);
        encoder.copy_buffer_to_buffer(&active.resolve, 0, &active.readback, 0, bytes);
    }

    /// Request an asynchronous map after the encoder's ordinary submission.
    /// Existing Scene/worker device polls deliver the callback; profiling adds
    /// no wait before or between normal source work.
    pub(crate) fn report_after_submit(self, queue: &wgpu::Queue) {
        let Some(active) = self.active else {
            return;
        };
        let bytes = u64::from(active.used) * 8;
        let period = f64::from(queue.get_timestamp_period()) / 1_000_000.0;
        let scope = active.scope;
        let labels = active.labels;
        let frame = active.frame;
        let mapped_readback = active.readback.clone();
        let readback = active.readback.clone();
        mapped_readback
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                result.unwrap_or_else(|error| {
                    panic!("live correction GPU timestamp map failed: {error}")
                });
                let mapped = readback.slice(..bytes).get_mapped_range();
                let ticks = mapped
                    .chunks_exact(8)
                    .map(|value| u64::from_ne_bytes(value.try_into().unwrap()))
                    .collect::<Vec<_>>();
                assert!(
                    ticks.windows(2).all(|pair| pair[0] <= pair[1]),
                    "live correction GPU timestamps moved backwards"
                );
                let values = ticks
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]) as f64 * period)
                    .collect::<Vec<_>>();
                let fields = labels
                    .iter()
                    .zip(values.iter())
                    .map(|(label, value)| format!(" {label}_ms={value:.6}"))
                    .collect::<String>();
                let total = (ticks[ticks.len() - 1] - ticks[0]) as f64 * period;
                drop(mapped);
                readback.unmap();
                eprintln!(
                    "live-correction-gpu: frame={} scope={}{} total_ms={total:.6} instrumentation=timestamp-writes+resolve-copy+async-map no_added_wait=true",
                    frame, scope, fields,
                );
            });
    }
}
