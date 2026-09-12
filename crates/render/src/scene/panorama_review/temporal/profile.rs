//! Opt-in GPU timestamps for the offline temporal diagnostic.

use std::sync::mpsc;

const ENV: &str = "KJERAG_PANORAMA_GPU_TIMING";
const CAPACITY: u32 = 8;
const LABELS: [&str; 7] = [
    "history", "refine", "motion", "fuse", "color", "project", "readback",
];

pub(super) struct Profile {
    active: Option<Active>,
}

struct Active {
    device: wgpu::Device,
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    labels: Vec<&'static str>,
    used: u32,
}

impl Profile {
    pub(super) fn begin(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) -> Self {
        if std::env::var_os(ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
            return Self { active: None };
        }
        let required =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        assert!(
            device.features().contains(required),
            "panorama temporal GPU timing needs encoder timestamp queries"
        );
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("panorama temporal GPU timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: CAPACITY,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panorama temporal GPU timestamp resolve"),
            size: u64::from(CAPACITY) * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panorama temporal GPU timestamp readback"),
            size: u64::from(CAPACITY) * 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.write_timestamp(&queries, 0);
        Self {
            active: Some(Active {
                device: device.clone(),
                queries,
                resolve,
                readback,
                labels: Vec::with_capacity(LABELS.len()),
                used: 1,
            }),
        }
    }

    pub(super) fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        let Some(active) = &mut self.active else {
            return;
        };
        assert!(
            active.used < CAPACITY,
            "panorama temporal GPU timing wrote too many markers"
        );
        encoder.write_timestamp(&active.queries, active.used);
        active.used += 1;
        active.labels.push(label);
    }

    pub(super) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(active) = &self.active else {
            return;
        };
        assert_eq!(
            active.used, CAPACITY,
            "panorama temporal GPU timing marker count"
        );
        assert_eq!(
            active.labels, LABELS,
            "panorama temporal GPU timing marker order"
        );
        let bytes = u64::from(active.used) * 8;
        encoder.resolve_query_set(&active.queries, 0..active.used, &active.resolve, 0);
        encoder.copy_buffer_to_buffer(&active.resolve, 0, &active.readback, 0, bytes);
    }

    pub(super) fn report(self, device: &wgpu::Device, queue: &wgpu::Queue, center: u64) {
        let Some(active) = self.active else {
            return;
        };
        assert_eq!(
            active.device, *device,
            "panorama temporal GPU timing changed devices"
        );
        assert_eq!(
            active.used, CAPACITY,
            "panorama temporal GPU timing marker count"
        );
        assert_eq!(
            active.labels, LABELS,
            "panorama temporal GPU timing marker order"
        );
        let slice = active.readback.slice(..u64::from(active.used) * 8);
        let (send, receive) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap_or_else(|error| panic!("{error}"));
        receive
            .recv()
            .expect("panorama temporal GPU timing callback disappeared")
            .unwrap_or_else(|error| panic!("{error}"));
        let mapped = slice.get_mapped_range();
        assert_eq!(mapped.len(), active.used as usize * 8);
        let ticks: Vec<u64> = mapped
            .chunks_exact(8)
            .map(|bytes| u64::from_ne_bytes(bytes.try_into().unwrap()))
            .collect();
        assert_eq!(ticks.len(), CAPACITY as usize);
        assert!(
            ticks.windows(2).all(|pair| pair[0] <= pair[1]),
            "panorama temporal GPU timestamps moved backwards"
        );
        let period = f64::from(queue.get_timestamp_period()) / 1_000_000.0;
        let elapsed = |index: usize| (ticks[index + 1] - ticks[index]) as f64 * period;
        let values: Vec<f64> = (0..LABELS.len()).map(elapsed).collect();
        let total = (ticks[CAPACITY as usize - 1] - ticks[0]) as f64 * period;
        drop(mapped);
        active.readback.unmap();
        eprintln!(
            "panorama-temporal-gpu: center {center} history_ms={:.6} refine_ms={:.6} motion_ms={:.6} fuse_ms={:.6} color_ms={:.6} project_ms={:.6} readback_ms={:.6} total_ms={total:.6}",
            values[0], values[1], values[2], values[3], values[4], values[5], values[6],
        );
    }
}
