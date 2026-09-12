//! Kjerag: a 360 video player for the COSMIC desktop.
//!
//! `src/main.rs` is the binary and documents the command line; the shell
//! itself is docs/UI.md's, which is the design this crate implements and
//! which cites a first-party COSMIC app for every call it makes.
//!
//! The modules are `pub` so `src/main.rs` can drive them; nothing outside this
//! crate reads them. The headless instruments once did, through the saved seam
//! pool `crates/spike`'s `seam=pool` read, but that per-capture fit was the
//! non-parity mechanism and was removed (2026-08-15): the player draws the
//! factory calibration, which needs no saved state and nothing to read it out.

pub mod app;
pub mod args;
pub mod config;
#[cfg(test)]
mod controls_tree_tests;
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
