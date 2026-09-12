//! Shared opt-in markers for the native changing-view capacity instrument.

use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{FrameStamp, Reframe};

#[derive(Clone, Copy)]
pub(super) struct DrawMarker {
    view_hash: u64,
}

impl DrawMarker {
    pub(super) fn for_reframe(reframe: &Reframe) -> Option<Self> {
        Self::from_bytes(enabled(), reframe.bytes())
    }

    fn from_bytes(enabled: bool, bytes: &[u8]) -> Option<Self> {
        enabled.then(|| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hash);
            Self {
                view_hash: hash.finish(),
            }
        })
    }

    pub(super) fn record(self, frame: &FrameStamp, pass: &mut wgpu::RenderPass<'_>) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        let start = std::time::Instant::now();
        let unix_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros();
        eprintln!(
            "native-draw: {{\"id\":{ordinal},\"unix_us\":{unix_us},\"source\":{},\"pts_ns\":{},\"view_hash\":{}}}",
            frame.index(),
            frame.timestamp().as_nanos(),
            self.view_hash,
        );
        pass.on_submitted_work_done(move || {
            let unix_us = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros();
            eprintln!(
                "native-draw-done: {{\"id\":{ordinal},\"unix_us\":{unix_us},\"elapsed_ns\":{}}}",
                start.elapsed().as_nanos()
            );
        });
    }
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("KJERAG_NATIVE_CAPACITY_PROBE").is_some())
}

#[cfg(test)]
mod tests {
    use super::DrawMarker;

    #[test]
    fn disabled_marker_skips_view_hash() {
        assert!(DrawMarker::from_bytes(false, b"view").is_none());
    }

    #[test]
    fn view_hash_is_stable_and_changes_with_reframe_bytes() {
        let first = DrawMarker::from_bytes(true, b"first").unwrap();
        let repeat = DrawMarker::from_bytes(true, b"first").unwrap();
        let second = DrawMarker::from_bytes(true, b"second").unwrap();
        assert_eq!(first.view_hash, repeat.view_hash);
        assert_ne!(first.view_hash, second.view_hash);
    }
}
