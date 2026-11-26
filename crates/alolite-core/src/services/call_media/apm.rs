//! Embedded WebRTC Audio Processing Module (AEC, Noise Suppression, High Pass Filter).
//!
//! Provides in-process acoustic echo cancellation and background noise suppression
//! when `webrtc-audio-processing-1` is present at build time.

use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicI32, Ordering};

extern "C" {
    fn alolite_apm_create(
        sample_rate_hz: c_int,
        channels: c_int,
        aec_enabled: c_int,
        ns_enabled: c_int,
        hpf_enabled: c_int,
    ) -> *mut c_void;

    fn alolite_apm_process_render(
        handle: *mut c_void,
        pcm: *const i16,
        num_samples: c_int,
    ) -> c_int;

    fn alolite_apm_process_capture(
        handle: *mut c_void,
        pcm: *mut i16,
        num_samples: c_int,
        delay_ms: c_int,
    ) -> c_int;

    fn alolite_apm_destroy(handle: *mut c_void);
}

/// Safe wrapper around `webrtc::AudioProcessing` for 16 kHz mono AEC + NS + HPF.
pub struct Apm {
    raw: *mut c_void,
    delay_ms: AtomicI32,
}

// SAFETY: `Apm` owns a raw C handle that is not thread-bound; it is moved into
// the single dedicated audio thread (`INV-9`).
unsafe impl Send for Apm {}

impl Apm {
    /// Instantiate a WebRTC AudioProcessing instance.
    ///
    /// # Errors
    /// Returns an error if `webrtc-audio-processing-1` was unavailable at build time
    /// or failed to initialize.
    pub fn new(
        sample_rate_hz: i32,
        channels: i32,
        aec_enabled: bool,
        ns_enabled: bool,
        hpf_enabled: bool,
    ) -> Result<Self, String> {
        // SAFETY: `alolite_apm_create` takes only scalars and returns null on failure.
        let raw = unsafe {
            alolite_apm_create(
                sample_rate_hz as c_int,
                channels as c_int,
                i32::from(aec_enabled),
                i32::from(ns_enabled),
                i32::from(hpf_enabled),
            )
        };

        if raw.is_null() {
            return Err("WebRTC AudioProcessing unavailable".to_string());
        }

        Ok(Self {
            raw,
            delay_ms: AtomicI32::new(60),
        })
    }

    /// Feed speaker render audio as the echo cancellation reference.
    pub fn process_render(&self, pcm: &[i16]) {
        if self.raw.is_null() || pcm.is_empty() {
            return;
        }
        // SAFETY: `self.raw` is valid until `Drop` and `pcm` is readable for `pcm.len()` samples.
        unsafe {
            alolite_apm_process_render(
                self.raw,
                pcm.as_ptr(),
                i32::try_from(pcm.len()).unwrap_or(c_int::MAX),
            );
        }
    }

    /// Process microphone capture audio in-place through AEC, NS, and HPF.
    pub fn process_capture(&self, pcm: &mut [i16]) {
        if self.raw.is_null() || pcm.is_empty() {
            return;
        }
        let delay = self.delay_ms.load(Ordering::Relaxed);
        // SAFETY: `self.raw` is valid until `Drop` and `pcm` is exclusively borrowed for `pcm.len()`.
        unsafe {
            alolite_apm_process_capture(
                self.raw,
                pcm.as_mut_ptr(),
                i32::try_from(pcm.len()).unwrap_or(c_int::MAX),
                delay as c_int,
            );
        }
    }

    /// Update the estimated render-to-capture acoustic delay in milliseconds.
    pub fn set_delay_ms(&self, delay_ms: i32) {
        self.delay_ms.store(delay_ms, Ordering::Relaxed);
    }
}

impl Drop for Apm {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: `raw` was returned by `alolite_apm_create` and is destroyed once here.
            unsafe {
                alolite_apm_destroy(self.raw);
            }
            self.raw = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apm_lifecycle_or_graceful_fallback() {
        match Apm::new(16_000, 1, true, true, true) {
            Ok(apm) => {
                let render = vec![500i16; 320];
                apm.process_render(&render);
                let mut capture = vec![500i16; 320];
                apm.process_capture(&mut capture);
                assert_eq!(capture.len(), 320);
            }
            Err(err) => {
                assert!(err.contains("WebRTC AudioProcessing"));
            }
        }
    }
}
