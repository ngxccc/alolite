//! Feature-gated ZRTC call media pipeline (`NetEq` jitter buffer, Opus codec, and audio I/O).
//!
//! System `libopus` and Linux `libpulse-simple` bindings are compiled only when the
//! `call` Cargo feature is enabled (`docs/adr/0008-pure-rust-zrtc-audio-call-engine.md`).
//! Default builds provide [`NullAudioBackend`] and [`NullOpusCodec`] so headless and
//! non-call builds stay green without native audio headers.

pub mod audio_backend;
pub mod neteq;
pub mod opus_codec;

/// Audio sample rate negotiated for ZRTC 1-1 calls (16 kHz wideband).
pub const SAMPLE_RATE_HZ: u32 = 16_000;

/// Mono channel count for ZRTC 1-1 voice calls.
pub const CHANNELS: usize = 1;

/// Samples per 20 ms playout/capture frame at 16 kHz mono (`16000 * 0.02 = 320`).
pub const FRAME_SAMPLES: usize = 320;

/// Maximum Opus packet duration in samples per channel (`120 ms` at 16 kHz = `1920`).
pub const MAX_OPUS_FRAME_SAMPLES: usize = 1_920;

/// PulseAudio playback `tlength` in bytes (`60 ms` = 3 frames of 320 `i16` samples = 1920 bytes).
pub const PLAYBACK_TLENGTH_BYTES: u32 = 1_920;

#[cfg(all(feature = "call", target_os = "linux"))]
pub use audio_backend::PulseAudioBackend;
pub use audio_backend::{AudioBackend, NullAudioBackend};
pub use neteq::{NetEq, NetEqConfig, PopOutcome};
#[cfg(feature = "call")]
pub use opus_codec::{LibOpusDecoder, LibOpusEncoder};
pub use opus_codec::{NullOpusDecoder, NullOpusEncoder, OpusDecoderBackend, OpusEncoderBackend};
