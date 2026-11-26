//! Opus encoder and decoder traits with `libopus` FFI (`call` feature) and null fallbacks.
//!
//! ZRTC negotiates 16 kHz mono Opus with `maxFT: 60` (`EDGE-1`), so decoders must
//! accept a 1920-sample scratch buffer (`120 ms` max Opus packet duration) and support
//! `decode_fec = 1` with `frame_size = 320` on single-packet holes (`EDGE-3`).

use anyhow::Result;

use super::FRAME_SAMPLES;

/// Opus decoder abstraction used by [`super::NetEq`].
pub trait OpusDecoderBackend: Send {
    /// Decode an Opus `packet` into `pcm` (capacity up to `1920` samples for normal decode,
    /// or `320` samples when `decode_fec` is `true`).
    ///
    /// Returns the number of decoded samples per channel.
    ///
    /// # Errors
    /// Returns an error if `pcm` is too small or `libopus` rejects the packet.
    fn decode(&mut self, packet: &[u8], pcm: &mut [i16], decode_fec: bool) -> Result<usize>;

    /// Synthesize a Packet Loss Concealment (PLC) frame of `frame_samples` into `pcm`.
    ///
    /// # Errors
    /// Returns an error if `pcm.len() < frame_samples` or `libopus` fails.
    fn decode_plc(&mut self, pcm: &mut [i16], frame_samples: usize) -> Result<usize>;
}

/// Opus encoder abstraction for outbound 16 kHz mono ZRTC frames.
pub trait OpusEncoderBackend: Send {
    /// Encode a mono PCM frame (`320` / `640` / `960` samples) into `out`.
    ///
    /// Returns the encoded byte length.
    ///
    /// # Errors
    /// Returns an error if `pcm` length is invalid or `out` is too small.
    fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize>;
}

/// No-op Opus decoder for headless and default non-`call` builds.
#[derive(Debug, Default)]
pub struct NullOpusDecoder;

impl OpusDecoderBackend for NullOpusDecoder {
    fn decode(&mut self, _packet: &[u8], pcm: &mut [i16], decode_fec: bool) -> Result<usize> {
        let len = if decode_fec {
            FRAME_SAMPLES
        } else {
            FRAME_SAMPLES.min(pcm.len())
        };
        if pcm.len() < len {
            anyhow::bail!("PCM buffer too small: {} < {len}", pcm.len());
        }
        pcm[..len].fill(0);
        Ok(len)
    }

    fn decode_plc(&mut self, pcm: &mut [i16], frame_samples: usize) -> Result<usize> {
        if pcm.len() < frame_samples {
            anyhow::bail!("PCM buffer too small: {} < {frame_samples}", pcm.len());
        }
        pcm[..frame_samples].fill(0);
        Ok(frame_samples)
    }
}

/// No-op Opus encoder for headless and default non-`call` builds.
#[derive(Debug, Default)]
pub struct NullOpusEncoder;

impl OpusEncoderBackend for NullOpusEncoder {
    fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize> {
        if pcm.is_empty() || out.is_empty() {
            anyhow::bail!("empty PCM or output buffer");
        }
        out[0] = 0;
        Ok(1)
    }
}

#[cfg(feature = "call")]
mod libopus {
    use super::*;
    use crate::services::call_media::{CHANNELS, SAMPLE_RATE_HZ};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int, c_uchar};

    #[repr(C)]
    struct OpusDecoder {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct OpusEncoder {
        _private: [u8; 0],
    }

    const OPUS_OK: c_int = 0;
    const OPUS_APPLICATION_VOIP: c_int = 2048;
    const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
    const OPUS_SET_INBAND_FEC_REQUEST: c_int = 4012;
    const OPUS_SET_PACKET_LOSS_PERC_REQUEST: c_int = 4014;

    extern "C" {
        fn opus_decoder_create(Fs: i32, channels: c_int, error: *mut c_int) -> *mut OpusDecoder;
        fn opus_decoder_destroy(st: *mut OpusDecoder);
        fn opus_decode(
            st: *mut OpusDecoder,
            data: *const c_uchar,
            len: i32,
            pcm: *mut i16,
            frame_size: c_int,
            decode_fec: c_int,
        ) -> c_int;

        fn opus_encoder_create(
            Fs: i32,
            channels: c_int,
            application: c_int,
            error: *mut c_int,
        ) -> *mut OpusEncoder;
        fn opus_encoder_destroy(st: *mut OpusEncoder);
        fn opus_encode(
            st: *mut OpusEncoder,
            pcm: *const i16,
            frame_size: c_int,
            data: *mut c_uchar,
            max_data_bytes: i32,
        ) -> i32;
        fn opus_encoder_ctl(st: *mut OpusEncoder, request: c_int, ...) -> c_int;
        fn opus_strerror(error: c_int) -> *const c_char;
    }

    fn opus_err_str(code: c_int) -> String {
        // SAFETY: `opus_strerror` returns a static NUL-terminated string for any error code.
        let ptr = unsafe { opus_strerror(code) };
        if ptr.is_null() {
            return format!("Opus error {code}");
        }
        // SAFETY: `ptr` is non-null and NUL-terminated per libopus contract.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    /// System `libopus` 16 kHz mono decoder.
    pub struct LibOpusDecoder {
        raw: *mut OpusDecoder,
    }

    // SAFETY: `OpusDecoder` state is owned exclusively by `LibOpusDecoder` and
    // accessed from one NetEq/audio thread at a time.
    unsafe impl Send for LibOpusDecoder {}

    impl LibOpusDecoder {
        /// Create a new 16 kHz mono `libopus` decoder.
        ///
        /// # Errors
        /// Returns an error if `opus_decoder_create` fails.
        pub fn new() -> Result<Self> {
            let mut err: c_int = 0;
            // SAFETY: `err` is a valid out-pointer; `SAMPLE_RATE_HZ` (16000) and `CHANNELS` (1) are valid.
            let raw =
                unsafe { opus_decoder_create(SAMPLE_RATE_HZ as i32, CHANNELS as c_int, &mut err) };
            if raw.is_null() || err != OPUS_OK {
                if !raw.is_null() {
                    // SAFETY: `raw` was allocated by `opus_decoder_create`.
                    unsafe { opus_decoder_destroy(raw) };
                }
                anyhow::bail!("opus_decoder_create failed: {}", opus_err_str(err));
            }
            Ok(Self { raw })
        }
    }

    impl Drop for LibOpusDecoder {
        fn drop(&mut self) {
            if !self.raw.is_null() {
                // SAFETY: `self.raw` is a valid decoder owned by `self`.
                unsafe { opus_decoder_destroy(self.raw) };
            }
        }
    }

    impl OpusDecoderBackend for LibOpusDecoder {
        fn decode(&mut self, packet: &[u8], pcm: &mut [i16], decode_fec: bool) -> Result<usize> {
            if packet.is_empty() {
                anyhow::bail!("cannot decode empty Opus packet");
            }
            let frame_size = if decode_fec {
                if pcm.len() < FRAME_SAMPLES {
                    anyhow::bail!(
                        "PCM buffer too small for FEC decode: {} < {FRAME_SAMPLES}",
                        pcm.len()
                    );
                }
                FRAME_SAMPLES as c_int
            } else {
                pcm.len() as c_int
            };

            // SAFETY: `self.raw` is valid; `packet` and `pcm` pointers are valid for their lengths.
            let decoded = unsafe {
                opus_decode(
                    self.raw,
                    packet.as_ptr(),
                    packet.len() as i32,
                    pcm.as_mut_ptr(),
                    frame_size,
                    c_int::from(decode_fec),
                )
            };
            if decoded < 0 {
                anyhow::bail!("opus_decode failed: {}", opus_err_str(decoded));
            }
            Ok(decoded as usize)
        }

        fn decode_plc(&mut self, pcm: &mut [i16], frame_samples: usize) -> Result<usize> {
            if pcm.len() < frame_samples {
                anyhow::bail!(
                    "PCM buffer too small for PLC: {} < {frame_samples}",
                    pcm.len()
                );
            }
            // SAFETY: Passing `NULL` data and `len = 0` triggers Opus PLC for `frame_samples`.
            let decoded = unsafe {
                opus_decode(
                    self.raw,
                    std::ptr::null(),
                    0,
                    pcm.as_mut_ptr(),
                    frame_samples as c_int,
                    0,
                )
            };
            if decoded < 0 {
                anyhow::bail!("opus_decode PLC failed: {}", opus_err_str(decoded));
            }
            Ok(decoded as usize)
        }
    }

    /// System `libopus` 16 kHz mono VoIP encoder with in-band FEC enabled.
    pub struct LibOpusEncoder {
        raw: *mut OpusEncoder,
    }

    // SAFETY: `OpusEncoder` state is owned exclusively by `LibOpusEncoder` and
    // accessed from one capture/encode thread at a time.
    unsafe impl Send for LibOpusEncoder {}

    impl LibOpusEncoder {
        /// Create a new 16 kHz mono VoIP `libopus` encoder with in-band FEC enabled.
        ///
        /// # Errors
        /// Returns an error if `opus_encoder_create` or CTL configuration fails.
        pub fn new(bitrate_bps: i32) -> Result<Self> {
            let mut err: c_int = 0;
            // SAFETY: `err` is a valid out-pointer; 16 kHz mono VOIP is a valid configuration.
            let raw = unsafe {
                opus_encoder_create(
                    SAMPLE_RATE_HZ as i32,
                    CHANNELS as c_int,
                    OPUS_APPLICATION_VOIP,
                    &mut err,
                )
            };
            if raw.is_null() || err != OPUS_OK {
                if !raw.is_null() {
                    // SAFETY: `raw` was allocated by `opus_encoder_create`.
                    unsafe { opus_encoder_destroy(raw) };
                }
                anyhow::bail!("opus_encoder_create failed: {}", opus_err_str(err));
            }

            let enc = Self { raw };
            enc.set_ctl(OPUS_SET_BITRATE_REQUEST, bitrate_bps)?;
            // Enable in-band FEC and advertise non-zero expected packet loss so Opus embeds LBRR.
            enc.set_ctl(OPUS_SET_INBAND_FEC_REQUEST, 1)?;
            enc.set_ctl(OPUS_SET_PACKET_LOSS_PERC_REQUEST, 10)?;
            Ok(enc)
        }

        fn set_ctl(&self, request: c_int, value: i32) -> Result<()> {
            // SAFETY: `self.raw` is valid and `request` takes a single `opus_int32` (`i32`) argument.
            let rc = unsafe { opus_encoder_ctl(self.raw, request, value as c_int) };
            if rc != OPUS_OK {
                anyhow::bail!("opus_encoder_ctl({request}) failed: {}", opus_err_str(rc));
            }
            Ok(())
        }
    }

    impl Drop for LibOpusEncoder {
        fn drop(&mut self) {
            if !self.raw.is_null() {
                // SAFETY: `self.raw` is a valid encoder owned by `self`.
                unsafe { opus_encoder_destroy(self.raw) };
            }
        }
    }

    impl OpusEncoderBackend for LibOpusEncoder {
        fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize> {
            if pcm.is_empty() || out.is_empty() {
                anyhow::bail!("cannot encode empty PCM or output buffer");
            }
            // SAFETY: `self.raw` is valid; `pcm` and `out` pointers are valid for their lengths.
            let encoded = unsafe {
                opus_encode(
                    self.raw,
                    pcm.as_ptr(),
                    pcm.len() as c_int,
                    out.as_mut_ptr(),
                    out.len() as i32,
                )
            };
            if encoded < 0 {
                anyhow::bail!("opus_encode failed: {}", opus_err_str(encoded));
            }
            Ok(encoded as usize)
        }
    }
}

#[cfg(feature = "call")]
pub use libopus::{LibOpusDecoder, LibOpusEncoder};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_opus_codec_decodes_silence_and_plc() {
        let mut dec = NullOpusDecoder;
        let mut pcm = [99i16; FRAME_SAMPLES];
        let n = dec.decode(&[1, 2, 3], &mut pcm, false).expect("decode");
        assert_eq!(n, FRAME_SAMPLES);
        assert!(pcm.iter().all(|&s| s == 0));

        let mut plc = [77i16; FRAME_SAMPLES];
        let n_plc = dec.decode_plc(&mut plc, FRAME_SAMPLES).expect("plc");
        assert_eq!(n_plc, FRAME_SAMPLES);
        assert!(plc.iter().all(|&s| s == 0));
    }

    #[cfg(feature = "call")]
    #[test]
    fn libopus_encodes_and_decodes_20ms_40ms_60ms_frames() {
        use crate::services::call_media::MAX_OPUS_FRAME_SAMPLES;

        let mut enc = LibOpusEncoder::new(24_000).expect("encoder");
        let mut dec = LibOpusDecoder::new().expect("decoder");

        for frame_samples in [320usize, 640, 960] {
            let pcm_in: Vec<i16> = (0..frame_samples)
                .map(|i| ((i as f32 * 0.15).sin() * 8_000.0) as i16)
                .collect();
            let mut pkt = vec![0u8; 1500];
            let pkt_len = enc.encode(&pcm_in, &mut pkt).expect("encode");
            assert!(pkt_len > 0);

            let mut pcm_out = [0i16; MAX_OPUS_FRAME_SAMPLES];
            let decoded = dec
                .decode(&pkt[..pkt_len], &mut pcm_out, false)
                .expect("decode");
            assert_eq!(decoded, frame_samples);
        }
    }
}
