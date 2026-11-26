//! Audio I/O backend abstraction for ZRTC call media.
//!
//! Linux uses `libpulse-simple` with `tlength = 1920` (60 ms) so `pa_simple_write`
//! blocks on the soundcard's hardware sample clock (`re-notes/16` Section 6.4).
//! Headless and default builds use [`NullAudioBackend`].

use anyhow::Result;

use super::FRAME_SAMPLES;
#[cfg(all(feature = "call", target_os = "linux"))]
use super::{CHANNELS, PLAYBACK_TLENGTH_BYTES, SAMPLE_RATE_HZ};

/// Synchronous 20 ms frame-based audio I/O backend.
///
/// Implementations must run on a dedicated audio thread, never on the UI thread (`INV-3`, `INV-9`).
pub trait AudioBackend: Send {
    /// Human-readable backend identifier (e.g. `"pulse-simple"`, `"null"`).
    fn name(&self) -> &'static str;

    /// Read one 20 ms mono PCM frame (`320` samples at 16 kHz) from the microphone.
    ///
    /// # Errors
    /// Returns an error if `pcm.len() != FRAME_SAMPLES` or the underlying audio stream fails.
    fn read_frame(&mut self, pcm: &mut [i16]) -> Result<()>;

    /// Write one 20 ms mono PCM frame (`320` samples at 16 kHz) to the speaker.
    ///
    /// # Errors
    /// Returns an error if `pcm.len() != FRAME_SAMPLES` or the underlying audio stream fails.
    fn write_frame(&mut self, pcm: &[i16]) -> Result<()>;
}

/// No-op audio backend for headless (`--mcp` / `--api-only`) and default non-`call` builds.
#[derive(Debug, Default)]
pub struct NullAudioBackend {
    frames_written: u64,
    frames_read: u64,
}

impl NullAudioBackend {
    /// Create a new silent/discarding audio backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of 20 ms frames written via [`AudioBackend::write_frame`].
    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Number of 20 ms frames read via [`AudioBackend::read_frame`].
    pub fn frames_read(&self) -> u64 {
        self.frames_read
    }
}

impl AudioBackend for NullAudioBackend {
    fn name(&self) -> &'static str {
        "null"
    }

    fn read_frame(&mut self, pcm: &mut [i16]) -> Result<()> {
        if pcm.len() != FRAME_SAMPLES {
            anyhow::bail!(
                "expected {FRAME_SAMPLES} PCM samples for capture, got {}",
                pcm.len()
            );
        }
        pcm.fill(0);
        self.frames_read += 1;
        Ok(())
    }

    fn write_frame(&mut self, pcm: &[i16]) -> Result<()> {
        if pcm.len() != FRAME_SAMPLES {
            anyhow::bail!(
                "expected {FRAME_SAMPLES} PCM samples for playback, got {}",
                pcm.len()
            );
        }
        self.frames_written += 1;
        Ok(())
    }
}

#[cfg(all(feature = "call", target_os = "linux"))]
mod pulse {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_void};

    #[repr(C)]
    struct PaSimple {
        _private: [u8; 0],
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct PaSampleSpec {
        pub format: c_int,
        pub rate: u32,
        pub channels: u8,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct PaBufferAttr {
        pub maxlength: u32,
        pub tlength: u32,
        pub prebuf: u32,
        pub minreq: u32,
        pub fragsize: u32,
    }

    const PA_SAMPLE_S16LE: c_int = 3;
    const PA_STREAM_PLAYBACK: c_int = 1;
    const PA_STREAM_RECORD: c_int = 2;

    extern "C" {
        fn pa_simple_new(
            server: *const c_char,
            name: *const c_char,
            dir: c_int,
            dev: *const c_char,
            stream_name: *const c_char,
            ss: *const PaSampleSpec,
            map: *const c_void,
            attr: *const PaBufferAttr,
            error: *mut c_int,
        ) -> *mut PaSimple;
        fn pa_simple_free(s: *mut PaSimple);
        fn pa_simple_write(
            s: *mut PaSimple,
            data: *const c_void,
            bytes: usize,
            error: *mut c_int,
        ) -> c_int;
        fn pa_simple_read(
            s: *mut PaSimple,
            data: *mut c_void,
            bytes: usize,
            error: *mut c_int,
        ) -> c_int;
        fn pa_strerror(error: c_int) -> *const c_char;
    }

    fn pa_error_string(err: c_int) -> String {
        // SAFETY: `pa_strerror` returns a static NUL-terminated string for any error code.
        let ptr = unsafe { pa_strerror(err) };
        if ptr.is_null() {
            return format!("PulseAudio error {err}");
        }
        // SAFETY: `ptr` is non-null and NUL-terminated per libpulse contract.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    /// Linux `libpulse-simple` duplex audio backend paced by the hardware sample clock.
    pub struct PulseAudioBackend {
        playback: *mut PaSimple,
        capture: *mut PaSimple,
    }

    // SAFETY: `pa_simple` handles are owned exclusively by `PulseAudioBackend` and
    // moved to the single dedicated audio thread (`INV-9`).
    unsafe impl Send for PulseAudioBackend {}

    impl PulseAudioBackend {
        /// Build the `pa_sample_spec` for 16 kHz mono signed 16-bit LE PCM.
        pub(super) fn sample_spec() -> PaSampleSpec {
            PaSampleSpec {
                format: PA_SAMPLE_S16LE,
                rate: SAMPLE_RATE_HZ,
                channels: CHANNELS as u8,
            }
        }

        /// Build the playback `pa_buffer_attr` with `tlength = 1920` (60 ms) so
        /// `pa_simple_write` blocks on the hardware sample clock.
        pub(super) fn playback_buffer_attr() -> PaBufferAttr {
            PaBufferAttr {
                maxlength: u32::MAX,
                tlength: PLAYBACK_TLENGTH_BYTES,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: u32::MAX,
            }
        }

        /// Open duplex PulseAudio/PipeWire playback and capture streams.
        ///
        /// # Errors
        /// Returns an error if the PulseAudio daemon is unreachable or rejects the stream parameters,
        /// or if `dev` contains an interior NUL byte.
        pub fn open(playback_dev: Option<&str>, capture_dev: Option<&str>) -> Result<Self> {
            let app_name = CString::new("AloLite").expect("static string");
            let pb_stream = CString::new("ZRTC Playback").expect("static string");
            let cap_stream = CString::new("ZRTC Capture").expect("static string");
            let pb_dev_c = playback_dev.map(CString::new).transpose()?;
            let cap_dev_c = capture_dev.map(CString::new).transpose()?;

            let ss = Self::sample_spec();
            let pb_attr = Self::playback_buffer_attr();

            let mut err: c_int = 0;
            // SAFETY: All pointers are valid for the duration of `pa_simple_new`, and `err` is initialized.
            let playback = unsafe {
                pa_simple_new(
                    std::ptr::null(),
                    app_name.as_ptr(),
                    PA_STREAM_PLAYBACK,
                    pb_dev_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                    pb_stream.as_ptr(),
                    &ss,
                    std::ptr::null(),
                    &pb_attr,
                    &mut err,
                )
            };
            if playback.is_null() {
                anyhow::bail!(
                    "failed to open PulseAudio playback stream: {}",
                    pa_error_string(err)
                );
            }

            // SAFETY: All pointers are valid for the duration of `pa_simple_new`, and `err` is initialized.
            let capture = unsafe {
                pa_simple_new(
                    std::ptr::null(),
                    app_name.as_ptr(),
                    PA_STREAM_RECORD,
                    cap_dev_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                    cap_stream.as_ptr(),
                    &ss,
                    std::ptr::null(),
                    std::ptr::null(),
                    &mut err,
                )
            };
            if capture.is_null() {
                // SAFETY: `playback` was successfully created above and must not leak on failure.
                unsafe { pa_simple_free(playback) };
                anyhow::bail!(
                    "failed to open PulseAudio capture stream: {}",
                    pa_error_string(err)
                );
            }

            Ok(Self { playback, capture })
        }
    }

    impl Drop for PulseAudioBackend {
        fn drop(&mut self) {
            if !self.playback.is_null() {
                // SAFETY: `self.playback` is a valid non-null `pa_simple` handle owned by `self`.
                unsafe { pa_simple_free(self.playback) };
            }
            if !self.capture.is_null() {
                // SAFETY: `self.capture` is a valid non-null `pa_simple` handle owned by `self`.
                unsafe { pa_simple_free(self.capture) };
            }
        }
    }

    impl AudioBackend for PulseAudioBackend {
        fn name(&self) -> &'static str {
            "pulse-simple"
        }

        fn read_frame(&mut self, pcm: &mut [i16]) -> Result<()> {
            if pcm.len() != FRAME_SAMPLES {
                anyhow::bail!(
                    "expected {FRAME_SAMPLES} PCM samples for capture, got {}",
                    pcm.len()
                );
            }
            let bytes = std::mem::size_of_val(pcm);
            let mut err: c_int = 0;
            // SAFETY: `self.capture` is valid and `pcm` is writable for `bytes` bytes.
            let rc = unsafe {
                pa_simple_read(
                    self.capture,
                    pcm.as_mut_ptr().cast::<c_void>(),
                    bytes,
                    &mut err,
                )
            };
            if rc < 0 {
                anyhow::bail!("PulseAudio read failed: {}", pa_error_string(err));
            }
            Ok(())
        }

        fn write_frame(&mut self, pcm: &[i16]) -> Result<()> {
            if pcm.len() != FRAME_SAMPLES {
                anyhow::bail!(
                    "expected {FRAME_SAMPLES} PCM samples for playback, got {}",
                    pcm.len()
                );
            }
            let bytes = std::mem::size_of_val(pcm);
            let mut err: c_int = 0;
            // SAFETY: `self.playback` is valid and `pcm` is readable for `bytes` bytes.
            let rc = unsafe {
                pa_simple_write(
                    self.playback,
                    pcm.as_ptr().cast::<c_void>(),
                    bytes,
                    &mut err,
                )
            };
            if rc < 0 {
                anyhow::bail!("PulseAudio write failed: {}", pa_error_string(err));
            }
            Ok(())
        }
    }
}

#[cfg(all(feature = "call", target_os = "linux"))]
pub use pulse::PulseAudioBackend;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_audio_backend_reads_silence_and_tracks_writes() {
        let mut backend = NullAudioBackend::new();
        assert_eq!(backend.name(), "null");

        let mut capture = [42i16; FRAME_SAMPLES];
        backend
            .read_frame(&mut capture)
            .expect("read_frame succeeds");
        assert!(capture.iter().all(|&s| s == 0));
        assert_eq!(backend.frames_read(), 1);

        let playback = [100i16; FRAME_SAMPLES];
        backend
            .write_frame(&playback)
            .expect("write_frame succeeds");
        assert_eq!(backend.frames_written(), 1);
    }

    #[test]
    fn null_audio_backend_rejects_wrong_frame_length() {
        let mut backend = NullAudioBackend::new();
        let mut short = [0i16; 160];
        assert!(backend.read_frame(&mut short).is_err());
        assert!(backend.write_frame(&short).is_err());
    }

    #[cfg(all(feature = "call", target_os = "linux"))]
    #[test]
    fn pulse_playback_attr_uses_hardware_clock_tlength_1920() {
        let attr = PulseAudioBackend::playback_buffer_attr();
        assert_eq!(attr.tlength, 1920);
        assert_eq!(attr.maxlength, u32::MAX);
        assert_eq!(attr.prebuf, u32::MAX);
        assert_eq!(attr.minreq, u32::MAX);
        assert_eq!(attr.fragsize, u32::MAX);

        let ss = PulseAudioBackend::sample_spec();
        assert_eq!(ss.rate, 16_000);
        assert_eq!(ss.channels, 1);
    }
}
