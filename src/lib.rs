//! Core of casparcg-osc-companion-timer. Nothing in here knows about the GUI.
//!
//! Data flow: CasparCG OSC (UDP) -> `caspar_in` -> `layer_state` -> `binding`
//! (pure rendering + dedupe) -> `companion_out` (gateway to Companion).
//! `engine` ties the pieces together, `worker` runs it on its own thread.

pub mod binding;
pub mod caspar_in;
pub mod companion_out;
pub mod config;
pub mod engine;
pub mod layer_state;
pub mod worker;
