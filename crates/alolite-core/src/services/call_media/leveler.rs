//! Adaptive software AGC (`AudioLeveler`) with noise-floor learning, speech-triggered
//! gain adaptation, far-end hold, smooth per-sample ramping, and hard peak limiting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const NOISE_CEIL_DB: f32 = -38.0;
const SPEECH_OVER_NOISE_DB: f32 = 9.0;
const FLOOR_DB: f32 = -60.0;
const LEARN_FRAMES: usize = 20;
const UP_DB: f32 = 0.5;
const DOWN_DB: f32 = 1.0;
const LEVEL_WINDOW: usize = 250;

/// Configuration for [`AudioLeveler`] (microphone AGC or speaker loudness normalization).
#[derive(Debug, Clone, Copy)]
pub struct LevelerConfig {
    /// Target speech RMS level in dBFS.
    pub target_db: f32,
    /// Minimum gain (negative values attenuate overly hot signals).
    pub min_gain_db: f32,
    /// Maximum make-up gain in dB.
    pub max_gain_db: f32,
}

impl LevelerConfig {
    /// Preset for local microphone capture (`-22 dBFS` target, `-6..+24 dB`).
    #[must_use]
    pub const fn mic() -> Self {
        Self {
            target_db: -22.0,
            min_gain_db: -6.0,
            max_gain_db: 24.0,
        }
    }

    /// Preset for remote speaker playout (`-20 dBFS` target, `-18..+6 dB`) to prevent clipping.
    #[must_use]
    pub const fn speaker() -> Self {
        Self {
            target_db: -20.0,
            min_gain_db: -18.0,
            max_gain_db: 6.0,
        }
    }
}

/// Adaptive RMS leveler with peak limiter for 16 kHz mono PCM frames.
pub struct AudioLeveler {
    config: LevelerConfig,
    gain_db: f32,
    g_prev: f32,
    noise_db: f32,
    levels: Vec<f32>,
    level_idx: usize,
    level_count: usize,
    n: usize,
    far_active: Option<Arc<AtomicBool>>,
}

impl AudioLeveler {
    /// Create a new [`AudioLeveler`] with optional `far_active` flag to freeze mic adaptation
    /// while the remote peer is speaking.
    #[must_use]
    pub fn new(config: LevelerConfig, far_active: Option<Arc<AtomicBool>>) -> Self {
        Self {
            config,
            gain_db: 0.0,
            g_prev: 1.0,
            noise_db: -70.0,
            levels: vec![0.0; LEVEL_WINDOW],
            level_idx: 0,
            level_count: 0,
            n: 0,
            far_active,
        }
    }

    /// Compute frame RMS level in dBFS.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "audio dBFS level; precision below 2^52 is irrelevant"
    )]
    pub fn compute_level_db(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return -120.0;
        }
        let sum_sq: f64 = samples.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
        let rms = (sum_sq / samples.len() as f64).sqrt().max(1.0);
        20.0 * (rms / 32768.0).log10() as f32
    }

    /// Adapt gain and apply smooth per-sample scaling + peak limiting in-place.
    #[allow(
        clippy::cast_precision_loss,
        reason = "audio sample scaling; precision below 2^52 is irrelevant"
    )]
    pub fn process(&mut self, samples: &mut [i16]) {
        if samples.is_empty() {
            return;
        }

        let level = Self::compute_level_db(samples);

        self.levels[self.level_idx] = level;
        self.level_idx = (self.level_idx + 1) % LEVEL_WINDOW;
        if self.level_count < LEVEL_WINDOW {
            self.level_count += 1;
        }
        self.n += 1;

        if self.level_count >= LEARN_FRAMES
            && (self.n.is_multiple_of(25) || self.level_count == LEARN_FRAMES)
        {
            let mut sorted = self.levels[..self.level_count].to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            self.noise_db = sorted[self.level_count / 10];
        }

        if self.level_count >= LEARN_FRAMES
            && level > FLOOR_DB
            && level > (self.noise_db + SPEECH_OVER_NOISE_DB)
        {
            let far_is_talking = self
                .far_active
                .as_ref()
                .is_some_and(|f| f.load(Ordering::Relaxed));

            if !far_is_talking {
                let ceiling = self
                    .config
                    .max_gain_db
                    .min((NOISE_CEIL_DB - self.noise_db).max(0.0));
                let want = (self.config.target_db - level).clamp(self.config.min_gain_db, ceiling);
                let step = (want - self.gain_db).clamp(-DOWN_DB, UP_DB);
                self.gain_db += step;
            }
        }

        let mut g = 10.0f32.powf(self.gain_db / 20.0);
        let mut g0 = self.g_prev;
        let peak = samples
            .iter()
            .map(|&v| i32::from(v).abs())
            .max()
            .unwrap_or(0);

        if peak > 0 {
            let lim = 32000.0 / peak as f32;
            g = g.min(lim);
            g0 = g0.min(lim);
        }

        self.g_prev = g;

        if (g - 1.0).abs() < 0.01 && (g0 - 1.0).abs() < 0.01 {
            return;
        }

        let step = (g - g0) / samples.len() as f32;
        for (i, s) in samples.iter_mut().enumerate() {
            let factor = g0 + step * (i + 1) as f32;
            let val = (f32::from(*s) * factor).round() as i32;
            *s = val.clamp(-32768, 32767) as i16;
        }
    }

    /// Current adaptive gain expressed as a percentage (`10..=150`) for UI sync.
    #[must_use]
    pub fn gain_pct(&self) -> u8 {
        let pct = (10.0f32.powf(self.gain_db / 20.0) * 100.0).round();
        pct.clamp(10.0, 150.0) as u8
    }

    #[cfg(test)]
    pub(crate) fn gain_db(&self) -> f32 {
        self.gain_db
    }

    #[cfg(test)]
    pub(crate) fn noise_db(&self) -> f32 {
        self.noise_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leveler_boosts_quiet_speech_and_holds_during_far_end() {
        let far_active = Arc::new(AtomicBool::new(false));
        let mut leveler = AudioLeveler::new(LevelerConfig::mic(), Some(Arc::clone(&far_active)));

        let noise = vec![103i16; 320];
        for _ in 0..25 {
            let mut f = noise.clone();
            leveler.process(&mut f);
        }
        assert!(leveler.noise_db() > -60.0 && leveler.noise_db() < -45.0);

        let mut speech = vec![1036i16; 320];
        let before = leveler.gain_db();
        leveler.process(&mut speech);
        assert!(leveler.gain_db() > before);

        far_active.store(true, Ordering::Relaxed);
        let frozen = leveler.gain_db();
        let mut speech2 = vec![1036i16; 320];
        leveler.process(&mut speech2);
        assert!((leveler.gain_db() - frozen).abs() < f32::EPSILON);
    }

    #[test]
    fn speaker_leveler_attenuates_clipping_audio_and_limits_peaks() {
        let mut leveler = AudioLeveler::new(LevelerConfig::speaker(), None);
        let silence = vec![50i16; 320];
        for _ in 0..25 {
            let mut f = silence.clone();
            leveler.process(&mut f);
        }

        let mut loud = vec![32_767i16; 320];
        leveler.process(&mut loud);
        assert!(leveler.gain_db() < 0.0);
        for &s in &loud {
            assert!((-32_000..=32_000).contains(&s));
        }
    }
}
