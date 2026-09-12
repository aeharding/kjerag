//! Exact graphics-device identity for the selected ONE X2 transaction.
//!
//! iced owns the device and queue. This private cloneable handle keeps that
//! exact pair together while resident resources move through independently
//! constructed pipelines. Identity is structural: a recreated renderer
//! pipeline receiving clones of the same pair remains compatible, while a
//! separately requested pair cannot inherit resources from the old one. wgpu
//! exposes the one queue returned with a requested device, not a constructor
//! for a second independent queue on that same device, but both handles are
//! still checked so the pair remains the unit of ownership.

use std::sync::{Arc, OnceLock};
use std::thread::ThreadId;

use crate::Fallible;

/// The one device and queue on which a resident ONE X2 transaction is valid.
#[derive(Clone)]
pub(crate) struct OneXsGpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    worker_thread: Option<Arc<OnceLock<ThreadId>>>,
    #[cfg(test)]
    worker_l1_submissions: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl OneXsGpuContext {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            worker_thread: None,
            #[cfg(test)]
            worker_l1_submissions: None,
        }
    }

    /// Clone the exact device/queue pair with a fresh worker identity.
    pub(crate) fn with_worker(&self) -> Self {
        Self {
            device: self.device.clone(),
            queue: self.queue.clone(),
            worker_thread: Some(Arc::new(OnceLock::new())),
            #[cfg(test)]
            worker_l1_submissions: Some(Arc::new(std::sync::atomic::AtomicUsize::new(0))),
        }
    }

    pub(crate) fn register_worker_thread(&self, thread: ThreadId) -> Fallible<()> {
        self.worker_thread
            .as_ref()
            .ok_or("ONE X2 GPU context has no worker marker")?
            .set(thread)
            .map_err(|_| "ONE X2 GPU worker thread was already registered".into())
    }

    pub(crate) fn is_worker_thread(&self) -> bool {
        self.worker_thread
            .as_ref()
            .and_then(|worker| worker.get())
            .is_some_and(|worker| *worker == std::thread::current().id())
    }

    #[cfg(test)]
    pub(crate) fn note_worker_l1_submission(&self) {
        if self.is_worker_thread()
            && let Some(submissions) = &self.worker_l1_submissions
        {
            submissions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[cfg(test)]
    pub(crate) fn worker_l1_submissions(&self) -> usize {
        self.worker_l1_submissions
            .as_ref()
            .map_or(0, |submissions| {
                submissions.load(std::sync::atomic::Ordering::SeqCst)
            })
    }

    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub(crate) fn ensure_same(&self, other: &Self) -> Fallible<()> {
        if self.device == other.device && self.queue == other.queue {
            Ok(())
        } else {
            Err("ONE X2 GPU submission crossed a different device or queue".into())
        }
    }
}
