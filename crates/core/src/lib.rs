//! Platform-independent agent logic. Never calls OS APIs or HTTP directly.
//!
//! - [`agent`]: the Claude computer-use loop.
//! - [`screen`]: screenshot scaling/encoding and coordinate mapping.
//! - [`keys`]: key-combo parsing.
//! - [`motion`]: smooth, cancellable mouse movement.
//! - [`state`], [`tool`]: agent states and the custom-tool interface.

#![forbid(unsafe_code)]

pub mod agent;
pub mod keys;
pub mod motion;
pub mod risk;
pub mod screen;
pub mod state;
pub mod tool;
