use crate::rules::SAMPLE_RATE;
use std::time::Duration;

/// Captured audio of one dictation: mono, `SAMPLE_RATE` Hz, samples in [-1.0, 1.0].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recording {
    pub samples: Vec<f32>,
}

impl Recording {
    pub fn new(samples: Vec<f32>) -> Self {
        Self { samples }
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / SAMPLE_RATE as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_follows_the_sample_count() {
        let recording = Recording::new(vec![0.0; SAMPLE_RATE as usize * 3 / 2]);

        assert_eq!(recording.duration(), Duration::from_millis(1500));
    }
}
