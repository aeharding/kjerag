//! Kyerag internals, split by the layers in docs/ARCHITECTURE.md.
//!
//! The binary is a thin shell over this library; everything below the
//! shell is here, and none of it knows the shell exists.

pub mod meta;
