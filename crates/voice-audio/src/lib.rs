//! Microphone capture: the `Recorder` port on top of `cpal`.
//!
//! The sound maths ([`dsp`], [`capture`]) and the recording lifecycle ([`recorder`]) are
//! hardware-free and fully tested; only [`hardware`] talks to the real audio devices.

pub mod capture;
pub mod dsp;
mod hardware;
pub mod recorder;

pub use recorder::CpalRecorder;
