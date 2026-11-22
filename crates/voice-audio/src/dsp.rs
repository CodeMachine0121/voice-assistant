//! Pure sound maths: channel downmix, sample-rate conversion and level metering.

/// How much the raw RMS is boosted before the square-root curve, so ordinary speech
/// (RMS around 0.05–0.2) lands in the upper half of the meter.
const LEVEL_GAIN: f32 = 3.0;

/// Averages interleaved frames of `channels` samples down to one mono sample each.
/// A trailing partial frame is ignored. `channels == 0` yields nothing.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Converts mono `input` from `from_rate` to `to_rate`.
///
/// Downsampling averages the input span each output sample covers (a cheap low-pass that
/// avoids aliasing); upsampling interpolates linearly. Equal rates return a copy; empty
/// input or a zero rate yields nothing.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return input.to_vec();
    }
    let step = from_rate as f64 / to_rate as f64;
    let out_len = (input.len() as f64 / step).floor() as usize;
    if step > 1.0 {
        (0..out_len)
            .map(|i| average_span(input, i as f64 * step, step))
            .collect()
    } else {
        (0..out_len)
            .map(|i| interpolate(input, i as f64 * step))
            .collect()
    }
}

/// Weighted mean of the input over `[start, start + len)`, clipped to the input's end.
fn average_span(input: &[f32], start: f64, len: f64) -> f32 {
    let end = (start + len).min(input.len() as f64);
    let first = start.floor() as usize;
    let mut sum = 0.0f64;
    let mut weight = 0.0f64;
    for (j, &sample) in input.iter().enumerate().skip(first) {
        let lo = (j as f64).max(start);
        let hi = ((j + 1) as f64).min(end);
        if hi <= lo {
            break;
        }
        sum += sample as f64 * (hi - lo);
        weight += hi - lo;
    }
    (sum / weight) as f32
}

fn interpolate(input: &[f32], pos: f64) -> f32 {
    let i = pos.floor() as usize;
    let frac = (pos - i as f64) as f32;
    let a = input[i];
    let b = input.get(i + 1).copied().unwrap_or(a);
    a + (b - a) * frac
}

/// Maps a window of samples to a meter value in [0.0, 1.0]: RMS, boosted and
/// square-root scaled so quiet speech is still visible. An empty window is 0.0.
pub fn level_of(window: &[f32]) -> f32 {
    if window.is_empty() {
        return 0.0;
    }
    let mean_square = window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32;
    (mean_square.sqrt() * LEVEL_GAIN).sqrt().clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn downmix_averages_stereo_frames() {
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, -0.5], 2), vec![0.5, 0.0]);
    }

    #[test]
    fn downmix_of_mono_is_unchanged() {
        assert_eq!(downmix_to_mono(&[0.1, 0.2, 0.3], 1), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn downmix_ignores_trailing_partial_frame() {
        assert_eq!(
            downmix_to_mono(&[1.0, 1.0, 1.0, 1.0, 9.0], 2),
            vec![1.0, 1.0]
        );
    }

    #[test]
    fn downmix_with_zero_channels_is_empty() {
        assert!(downmix_to_mono(&[1.0, 2.0], 0).is_empty());
    }

    #[test]
    fn resample_same_rate_is_identity() {
        assert_eq!(
            resample(&[0.1, 0.2, 0.3], 16_000, 16_000),
            vec![0.1, 0.2, 0.3]
        );
    }

    #[test]
    fn resample_empty_or_zero_rate_is_empty() {
        assert!(resample(&[], 48_000, 16_000).is_empty());
        assert!(resample(&[1.0], 0, 16_000).is_empty());
        assert!(resample(&[1.0], 48_000, 0).is_empty());
    }

    #[test]
    fn downsample_by_three_averages_each_group() {
        let out = resample(&[3.0, 6.0, 9.0, 0.0, 0.0, 3.0], 48_000, 16_000);
        assert_eq!(out.len(), 2);
        assert!(approx(out[0], 6.0) && approx(out[1], 1.0));
    }

    #[test]
    fn downsample_drops_an_incomplete_trailing_group() {
        assert_eq!(resample(&[0.5; 10], 48_000, 16_000).len(), 3);
    }

    #[test]
    fn downsample_one_second_yields_16000_samples() {
        assert_eq!(resample(&vec![0.25; 44_100], 44_100, 16_000).len(), 16_000);
    }

    #[test]
    fn downsample_non_integer_ratio_keeps_constant_signal() {
        let out = resample(&vec![0.25; 4_410], 44_100, 16_000);
        assert!(out.iter().all(|&s| approx(s, 0.25)));
    }

    #[test]
    fn downsample_attenuates_tone_above_new_nyquist() {
        // A 12 kHz tone at 48 kHz is above the 8 kHz Nyquist of 16 kHz output; a 3-sample
        // average passes about a third of its amplitude, so the output is at most half.
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        let tone: Vec<f32> = (0..4_800)
            .map(|i| (2.0 * std::f32::consts::PI * 12_000.0 * i as f32 / 48_000.0).sin())
            .collect();
        let out = resample(&tone, 48_000, 16_000);
        assert!(rms(&out) < 0.5 * rms(&tone));
    }

    #[test]
    fn upsample_interpolates_midpoints() {
        let out = resample(&[0.0, 1.0, 0.0], 8_000, 16_000);
        assert_eq!(out.len(), 6);
        assert!(approx(out[0], 0.0) && approx(out[1], 0.5) && approx(out[2], 1.0));
        assert!(approx(out[3], 0.5) && approx(out[4], 0.0));
    }

    #[test]
    fn level_of_silence_is_zero() {
        assert_eq!(level_of(&[0.0; 800]), 0.0);
        assert_eq!(level_of(&[]), 0.0);
    }

    #[test]
    fn level_of_moderate_signal_is_scaled_by_square_root() {
        // RMS 1/12 -> sqrt(3 * 1/12) = 0.5
        assert!(approx(level_of(&[1.0 / 12.0; 800]), 0.5));
    }

    #[test]
    fn level_of_full_scale_is_clamped_to_one() {
        assert_eq!(level_of(&[1.0; 800]), 1.0);
        assert_eq!(level_of(&[-1.0; 800]), 1.0);
    }

    #[test]
    fn level_grows_with_loudness() {
        assert!(level_of(&[0.02; 100]) < level_of(&[0.2; 100]));
    }
}
