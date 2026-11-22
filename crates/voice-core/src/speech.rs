use crate::rules::SAMPLE_RATE;
use crate::Recording;

/// Length of one analysis frame.
pub const FRAME_MILLIS: usize = 30;
/// Number of samples in one analysis frame at `SAMPLE_RATE`.
pub const FRAME_SAMPLES: usize = SAMPLE_RATE as usize * FRAME_MILLIS / 1000;
/// A frame whose RMS energy reaches this level counts as voiced.
pub const VOICED_RMS: f32 = 0.01;
/// A recording contains speech when its voiced frames add up to at least this long.
pub const MIN_VOICED_MILLIS: usize = 250;

/// Decides whether a recording contains any speech at all (energy based),
/// so silence never reaches the speech model.
pub fn contains_speech(recording: &Recording) -> bool {
    let voiced_frames = recording
        .samples
        .chunks(FRAME_SAMPLES)
        .filter(|frame| rms(frame) >= f64::from(VOICED_RMS))
        .count();
    voiced_frames * FRAME_MILLIS >= MIN_VOICED_MILLIS
}

/// Root mean square of a frame, accumulated in `f64` to limit rounding over many samples.
fn rms(frame: &[f32]) -> f64 {
    let sum_of_squares: f64 = frame.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum_of_squares / frame.len() as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `frames` full frames of a constant-amplitude square wave (RMS == amplitude).
    fn tone(frames: usize, amplitude: f32) -> Vec<f32> {
        (0..frames * FRAME_SAMPLES)
            .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
            .collect()
    }

    fn silence(frames: usize) -> Vec<f32> {
        vec![0.0; frames * FRAME_SAMPLES]
    }

    #[test]
    fn frame_is_480_samples() {
        assert_eq!(FRAME_SAMPLES, 480);
    }

    #[test]
    fn silence_is_not_speech() {
        assert!(!contains_speech(&Recording::new(silence(100))));
    }

    #[test]
    fn empty_recording_is_not_speech() {
        assert!(!contains_speech(&Recording::default()));
    }

    #[test]
    fn nine_voiced_frames_reach_250_ms_and_count_as_speech() {
        // 9 × 30 ms = 270 ms ≥ 250 ms
        let mut samples = silence(10);
        samples.extend(tone(9, 0.2));
        samples.extend(silence(10));

        assert!(contains_speech(&Recording::new(samples)));
    }

    #[test]
    fn eight_voiced_frames_are_too_short() {
        // 8 × 30 ms = 240 ms < 250 ms
        let mut samples = tone(8, 0.2);
        samples.extend(silence(50));

        assert!(!contains_speech(&Recording::new(samples)));
    }

    #[test]
    fn voiced_time_adds_up_across_separate_bursts() {
        let mut samples = tone(5, 0.2);
        samples.extend(silence(20));
        samples.extend(tone(4, 0.2));

        assert!(contains_speech(&Recording::new(samples)));
    }

    #[test]
    fn rms_just_above_threshold_counts_as_voiced() {
        assert!(contains_speech(&Recording::new(tone(9, 0.0101))));
    }

    #[test]
    fn rms_just_below_threshold_is_not_voiced() {
        assert!(!contains_speech(&Recording::new(tone(100, 0.0099))));
    }

    #[test]
    fn a_loud_trailing_partial_frame_counts_as_a_frame() {
        let mut samples = tone(8, 0.2);
        samples.extend([0.5_f32; 10]);

        assert!(contains_speech(&Recording::new(samples)));
    }
}
