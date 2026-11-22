//! The growing buffer a running stream writes into, and what is derived from it.
use crate::dsp::{downmix_to_mono, level_of, resample};
use voice_core::{rules::SAMPLE_RATE, Recording};

/// How much of the most recent sound the live level looks at.
const LEVEL_WINDOW_MS: u32 = 50;

/// Mono sound collected so far, at the capturing device's own sample rate.
#[derive(Debug)]
pub struct Capture {
    rate: u32,
    samples: Vec<f32>,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            rate: SAMPLE_RATE,
            samples: Vec::new(),
        }
    }
}

impl Capture {
    /// Declares the device's sample rate; call before the first `push_interleaved`.
    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate;
    }

    /// Appends interleaved frames of `channels` samples, mixed down to mono.
    pub fn push_interleaved(&mut self, data: &[f32], channels: usize) {
        self.samples.extend(downmix_to_mono(data, channels));
    }

    /// Level of the most recent ~50 ms, in [0.0, 1.0].
    pub fn level(&self) -> f32 {
        let window = (self.rate as usize * LEVEL_WINDOW_MS as usize / 1000).max(1);
        let start = self.samples.len().saturating_sub(window);
        level_of(&self.samples[start..])
    }

    /// Everything collected, converted to the recording format.
    pub fn take_recording(&mut self) -> Recording {
        let samples = std::mem::take(&mut self.samples);
        Recording::new(resample(&samples, self.rate, SAMPLE_RATE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_input_is_collected_as_mono() {
        let mut capture = Capture::default();
        capture.push_interleaved(&[1.0, 0.0, 0.0, 1.0], 2);
        assert_eq!(capture.take_recording().samples, vec![0.5, 0.5]);
    }

    #[test]
    fn recording_is_delivered_at_sixteen_kilohertz() {
        let mut capture = Capture::default();
        capture.set_rate(48_000);
        capture.push_interleaved(&vec![0.1; 48_000], 1);
        assert_eq!(capture.take_recording().samples.len(), 16_000);
    }

    #[test]
    fn taking_the_recording_empties_the_buffer() {
        let mut capture = Capture::default();
        capture.push_interleaved(&[0.5; 100], 1);
        capture.take_recording();
        assert!(capture.take_recording().samples.is_empty());
    }

    #[test]
    fn level_of_nothing_is_zero() {
        assert_eq!(Capture::default().level(), 0.0);
    }

    #[test]
    fn level_only_looks_at_the_last_fifty_milliseconds() {
        let mut capture = Capture::default();
        capture.set_rate(16_000);
        capture.push_interleaved(&[1.0; 16_000], 1); // loud second
        capture.push_interleaved(&[0.0; 800], 1); // then 50 ms of silence
        assert_eq!(capture.level(), 0.0);
    }

    #[test]
    fn level_reflects_a_loud_tail() {
        let mut capture = Capture::default();
        capture.push_interleaved(&[0.0; 16_000], 1);
        capture.push_interleaved(&[1.0; 800], 1);
        assert_eq!(capture.level(), 1.0);
    }
}
