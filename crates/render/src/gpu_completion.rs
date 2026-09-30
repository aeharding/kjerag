//! Completion of one GPU submission, independent of later queue work.
//!
//! The marker is private to this submission and is never reused. Mapping it proves
//! its last GPU use, not whatever happens to be the queue's newest submission
//! when a callback is registered. No image bytes are copied or read by the CPU.

use std::sync::{Arc, Mutex};

use crate::Fallible;

#[derive(Default)]
struct Signal(Mutex<Option<Result<(), String>>>);

impl Signal {
    fn complete(&self, result: Result<(), String>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(result);
    }

    fn ready(&self) -> Fallible<bool> {
        match self
            .0
            .lock()
            .map_err(|_| "GPU completion state is poisoned")?
            .as_ref()
        {
            None => Ok(false),
            Some(Ok(())) => Ok(true),
            Some(Err(error)) => Err(error.clone().into()),
        }
    }
}

#[derive(Clone)]
pub(crate) struct CompletionStatus(Arc<Signal>);

impl CompletionStatus {
    pub(crate) fn ready(&self) -> Fallible<bool> {
        self.0.ready()
    }

    #[cfg(test)]
    pub(crate) fn pending_for_test() -> Self {
        Self(Arc::new(Signal::default()))
    }

    #[cfg(test)]
    pub(crate) fn finish_for_test(&self, result: Result<(), String>) {
        self.0.complete(result);
    }
}

#[derive(Clone)]
pub(crate) struct SubmissionCompletion {
    device: wgpu::Device,
    status: CompletionStatus,
}

impl SubmissionCompletion {
    pub(crate) fn encode(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) -> Self {
        let marker = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("private submission completion marker"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.clear_buffer(&marker, 0, None);
        let signal = Arc::new(Signal::default());
        let callback_signal = Arc::clone(&signal);
        let callback_marker = marker.clone();
        encoder.map_buffer_on_submit(&marker, wgpu::MapMode::Read, 0..4, move |result| {
            let result = result.map_err(|error| error.to_string());
            if result.is_ok() {
                callback_marker.unmap();
            }
            // Publication and waking belong to the worker, not this callback.
            callback_signal.complete(result);
        });
        Self {
            device: device.clone(),
            status: CompletionStatus(signal),
        }
    }

    pub(crate) fn ready(&self) -> Fallible<bool> {
        self.status.ready()
    }

    pub(crate) fn status(&self) -> CompletionStatus {
        self.status.clone()
    }

    /// Only the bounded worker calls this, including when no job or redraw is
    /// arriving. Poll never waits for this output or for the shared queue.
    pub(crate) fn poll(&self) -> Fallible<()> {
        self.device.poll(wgpu::PollType::Poll)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_output_is_not_ready() {
        let signal = Signal::default();
        assert!(!signal.ready().unwrap());
        signal.complete(Ok(()));
        assert!(signal.ready().unwrap());
        assert!(signal.ready().unwrap());
    }

    #[test]
    fn completion_preserves_the_underlying_error() {
        let signal = Signal::default();
        signal.complete(Err("test device lost while mapping output".into()));
        assert_eq!(
            signal.ready().unwrap_err().to_string(),
            "test device lost while mapping output"
        );
    }
}
