//! Kjerag: a 360 video player for the COSMIC desktop.
//!
//! `src/main.rs` is the binary and documents the command line; the shell
//! itself is docs/UI.md's, which is the design this crate implements and
//! which cites a first-party COSMIC app for every call it makes.
//!
//! **Why the shell has a library face at all**: so the headless instruments
//! can draw with the app's own saved seam pool rather than with a copy of it.
//! `crates/spike`'s `seam=pool` reads [`config::state`] and applies
//! [`config::SeamPool::answer`], and a second reader of that file would be a
//! second answer to the question "what does the app draw this camera with",
//! which is the question an acceptance line exists to hold still
//! (docs/research/reference-views.md).

pub mod app;
pub mod args;
pub mod config;
mod dnd;
mod fail;
mod key_bind;
mod menu;
mod shot;
mod strings;

/// What the desktop and both cosmic-config directories call this app.
///
/// Here rather than only on the `Application` impl because the instruments
/// read the same state directory and a second spelling of this string would
/// point them at a pool nothing writes.
pub const APP_ID: &str = "dev.harding.Kjerag";
