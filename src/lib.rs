//! Kyerag internals, split by the layers in docs/ARCHITECTURE.md.
//!
//! `media` decodes and `meta` reads the trailer; neither knows the shell
//! exists. `render` owns wgpu, `app` is the libcosmic shell, and the
//! binaries (`kyerag`, `spike`) are thin.

pub mod app;
pub mod media;
pub mod meta;
pub mod render;

/// Errors cross thread boundaries here because iced's shader primitives are
/// `Send + Sync`, so the plain `Box<dyn Error>` a binary would use will not do.
pub type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
