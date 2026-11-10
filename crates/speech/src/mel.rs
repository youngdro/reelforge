//! The mel filter bank Whisper was trained with (librosa's Slaney-style filters, area-normalised),
//! computed here so no filter file has to ship beside the model.

/// Whisper's sample rate.
pub const SAMPLE_RATE: u32 = 16_000;
/// FFT size.
pub const N_FFT: usize = 400;

fn hz_to_mel(f: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    let log_step = 6.4f64.ln() / 27.0;
    if f >= 1000.0 { 15.0 + (f / 1000.0).ln() / log_step } else { f / F_SP }
}

fn mel_to_hz(m: f64) -> f64 {
    const F_SP: f64 = 200.0 / 3.0;
    let log_step = 6.4f64.ln() / 27.0;
    if m >= 15.0 { 1000.0 * ((m - 15.0) * log_step).exp() } else { m * F_SP }
}

/// `n_mels × (N_FFT / 2 + 1)` weights, row by row, from 0 Hz to half the sample rate.
pub fn filters(n_mels: usize) -> Vec<f32> {
    let bins = N_FFT / 2 + 1;
    let nyquist = SAMPLE_RATE as f64 / 2.0;
    let fft_hz: Vec<f64> = (0..bins).map(|k| k as f64 * nyquist / (bins - 1) as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(nyquist));
    let mel_hz: Vec<f64> =
        (0..n_mels + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64)).collect();
    let mut out = vec![0f32; n_mels * bins];
    for m in 0..n_mels {
        let (left, centre, right) = (mel_hz[m], mel_hz[m + 1], mel_hz[m + 2]);
        let norm = 2.0 / (right - left);
        for (k, &f) in fft_hz.iter().enumerate() {
            let up = (f - left) / (centre - left);
            let down = (right - f) / (right - centre);
            out[m * bins + k] = (up.min(down).max(0.0) * norm) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_match_whisper() {
        let f = filters(80);
        assert_eq!(f.len(), 80 * 201);
        // values from whisper's mel_filters.npz (librosa.filters.mel(sr=16000, n_fft=400, n_mels=80))
        assert!((f[1] - 0.024_862_595).abs() < 1e-6, "{}", f[1]);
        assert!((f[201 + 2] - 0.022_871_772).abs() < 1e-6, "{}", f[203]);
        let peak = |f: &[f32], m: usize| f[m * 201..(m + 1) * 201].iter().copied().fold(0f32, f32::max);
        assert!((peak(&f, 40) - 0.014_735_566).abs() < 1e-6);
        let big = filters(128);
        assert!((peak(&big, 127) - 0.005_041_601_6).abs() < 1e-6);
        assert!((peak(&big, 64) - 0.018_091_518).abs() < 1e-6);
        // every filter has weight and the bank covers the band
        for m in 0..80 {
            assert!(f[m * 201..(m + 1) * 201].iter().any(|&w| w > 0.0), "filter {m} is empty");
        }
    }
}
