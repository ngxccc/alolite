//! Adaptive `NetEq` jitter buffer for 20/40/60 ms ZRTC Opus frames.
//!
//! Implements:
//! - `EDGE-1`: 1920-sample decode scratch buffer + `carry_pcm` FIFO draining 320-sample (20 ms) ticks.
//! - `EDGE-2`: Overflow pruning that advances `next_idx` to the new oldest key.
//! - `EDGE-3`: Single-packet hole recovery via `decode_fec = 1` (`frame_size = 320`) on `cur_idx + 1`.

use std::collections::{BTreeMap, VecDeque};

use anyhow::Result;

use super::opus_codec::OpusDecoderBackend;
use super::{FRAME_SAMPLES, MAX_OPUS_FRAME_SAMPLES};

/// Configuration parameters for [`NetEq`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetEqConfig {
    /// Maximum number of queued packets before oldest packets are pruned (`EDGE-2`).
    pub max_depth: usize,
    /// Minimum queued packets required before playout starts from an unprimed state.
    pub min_delay_packets: usize,
}

impl Default for NetEqConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            min_delay_packets: 2,
        }
    }
}

/// Outcome of a single 20 ms [`NetEq::pop_frame`] tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopOutcome {
    /// Still pre-buffering (`queue.len() < min_delay_packets`); output filled with silence.
    Buffering,
    /// Drained 320 samples from `carry_pcm` (excess from a prior 40/60 ms packet, `EDGE-1`).
    Carry,
    /// Decoded a queued packet into `1920`-sample scratch buffer (`samples` total samples).
    Decoded {
        /// Total PCM samples decoded from the Opus packet (e.g. 320, 640, or 960).
        samples: usize,
    },
    /// Reconstructed a single missing packet using in-band FEC from `cur_idx + 1` (`EDGE-3`).
    FecRecovered,
    /// Synthesized a 20 ms Packet Loss Concealment (PLC) frame.
    Plc,
}

/// Adaptive jitter buffer decoupling network packet arrival from the 20 ms hardware clock.
pub struct NetEq<D: OpusDecoderBackend> {
    decoder: D,
    config: NetEqConfig,
    queue: BTreeMap<u64, Vec<u8>>,
    carry_pcm: VecDeque<i16>,
    scratch_pcm: [i16; MAX_OPUS_FRAME_SAMPLES],
    next_idx: Option<u64>,
    last_rx_idx: Option<u64>,
    primed: bool,
    consecutive_plc: usize,
}

impl<D: OpusDecoderBackend> NetEq<D> {
    /// Create a new `NetEq` jitter buffer with the given decoder and configuration.
    pub fn new(decoder: D, config: NetEqConfig) -> Self {
        Self {
            decoder,
            config: NetEqConfig {
                max_depth: config.max_depth.max(1),
                min_delay_packets: config.min_delay_packets.max(1),
            },
            queue: BTreeMap::new(),
            carry_pcm: VecDeque::with_capacity(MAX_OPUS_FRAME_SAMPLES),
            scratch_pcm: [0; MAX_OPUS_FRAME_SAMPLES],
            next_idx: None,
            last_rx_idx: None,
            primed: false,
            consecutive_plc: 0,
        }
    }

    /// Number of encoded Opus packets currently buffered.
    pub fn queued_packets(&self) -> usize {
        self.queue.len()
    }

    /// Number of excess PCM samples currently waiting in `carry_pcm`.
    pub fn carry_samples(&self) -> usize {
        self.carry_pcm.len()
    }

    /// Current extended playout sequence index, if initialized.
    pub fn next_idx(&self) -> Option<u64> {
        self.next_idx
    }

    /// Reset all buffered packets, carry samples, and sequence tracking (e.g. on handover).
    pub fn reset(&mut self) {
        self.queue.clear();
        self.carry_pcm.clear();
        self.next_idx = None;
        self.last_rx_idx = None;
        self.primed = false;
        self.consecutive_plc = 0;
    }

    /// Enqueue an incoming Opus packet with 16-bit RTP sequence number `seq`.
    pub fn push(&mut self, seq: u16, payload: Vec<u8>) {
        let idx = self.unwrap_seq(seq);
        if self.next_idx.is_some_and(|next| idx < next) {
            // Packet arrived after its playout slot already passed.
            return;
        }
        self.queue.insert(idx, payload);

        // EDGE-2: Prune oldest packets on overflow and advance `next_idx` so playout
        // does not target a pruned key and spiral into PLC starvation.
        while self.queue.len() > self.config.max_depth {
            if let Some((oldest_key, _)) = self.queue.pop_first() {
                if let Some(&new_oldest) = self.queue.keys().next() {
                    if self.next_idx.is_some_and(|next| next <= oldest_key) {
                        self.next_idx = Some(new_oldest);
                    }
                }
            }
        }
    }

    /// Produce one 20 ms (`320`-sample) mono PCM frame for playout.
    ///
    /// # Errors
    /// Returns an error if the underlying Opus decoder fails.
    pub fn pop_frame(&mut self, out: &mut [i16; FRAME_SAMPLES]) -> Result<PopOutcome> {
        // EDGE-1: Drain excess samples from prior 40/60 ms packets before popping `next_idx`.
        if self.carry_pcm.len() >= FRAME_SAMPLES {
            for sample in out.iter_mut() {
                *sample = self.carry_pcm.pop_front().unwrap_or(0);
            }
            self.consecutive_plc = 0;
            return Ok(PopOutcome::Carry);
        }

        if !self.primed {
            if self.queue.len() < self.config.min_delay_packets {
                out.fill(0);
                return Ok(PopOutcome::Buffering);
            }
            self.primed = true;
            if let Some(&oldest) = self.queue.keys().next() {
                if self.next_idx.is_none_or(|next| next < oldest) {
                    self.next_idx = Some(oldest);
                }
            }
        }

        let Some(cur_idx) = self.next_idx else {
            out.fill(0);
            return Ok(PopOutcome::Buffering);
        };

        if let Some(packet) = self.queue.remove(&cur_idx) {
            let decoded = self.decoder.decode(&packet, &mut self.scratch_pcm, false)?;
            if decoded >= FRAME_SAMPLES {
                out.copy_from_slice(&self.scratch_pcm[..FRAME_SAMPLES]);
                self.carry_pcm
                    .extend(&self.scratch_pcm[FRAME_SAMPLES..decoded]);
            } else {
                out[..decoded].copy_from_slice(&self.scratch_pcm[..decoded]);
                out[decoded..].fill(0);
            }
            self.next_idx = Some(cur_idx + 1);
            self.consecutive_plc = 0;
            return Ok(PopOutcome::Decoded { samples: decoded });
        }

        // EDGE-3: Single-packet hole with `cur_idx + 1` present -> decode in-band FEC
        // with `frame_size = 320` while keeping `cur_idx + 1` queued for the next tick.
        if let Some(next_packet) = self.queue.get(&(cur_idx + 1)) {
            let decoded =
                self.decoder
                    .decode(next_packet, &mut self.scratch_pcm[..FRAME_SAMPLES], true)?;
            let copy_len = decoded.min(FRAME_SAMPLES);
            out[..copy_len].copy_from_slice(&self.scratch_pcm[..copy_len]);
            if copy_len < FRAME_SAMPLES {
                out[copy_len..].fill(0);
            }
            self.next_idx = Some(cur_idx + 1);
            self.consecutive_plc = 0;
            return Ok(PopOutcome::FecRecovered);
        }

        let plc_len = self
            .decoder
            .decode_plc(&mut self.scratch_pcm[..FRAME_SAMPLES], FRAME_SAMPLES)?;
        let copy_len = plc_len.min(FRAME_SAMPLES);
        out[..copy_len].copy_from_slice(&self.scratch_pcm[..copy_len]);
        if copy_len < FRAME_SAMPLES {
            out[copy_len..].fill(0);
        }
        self.next_idx = Some(cur_idx + 1);
        self.consecutive_plc += 1;

        // Avoid infinite PLC loops after prolonged silence/starvation; re-arm pre-buffering.
        if self.consecutive_plc >= 5 && self.queue.is_empty() {
            self.primed = false;
            self.next_idx = None;
        }
        Ok(PopOutcome::Plc)
    }

    fn unwrap_seq(&mut self, seq: u16) -> u64 {
        let Some(last_idx) = self.last_rx_idx else {
            // Start at epoch 1 (`65536`) so small negative out-of-order deltas never underflow `0`.
            let initial = (1u64 << 16) + u64::from(seq);
            self.last_rx_idx = Some(initial);
            self.next_idx = Some(initial);
            return initial;
        };

        let last_seq = (last_idx & 0xFFFF) as u16;
        let delta = seq.wrapping_sub(last_seq) as i16;
        let idx = last_idx.saturating_add_signed(i64::from(delta));
        if idx > last_idx {
            self.last_rx_idx = Some(idx);
        }
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DecodeCall {
        payload: Vec<u8>,
        buf_len: usize,
        decode_fec: bool,
    }

    struct FakeOpusDecoder {
        calls: Vec<DecodeCall>,
        plc_calls: usize,
    }

    impl FakeOpusDecoder {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                plc_calls: 0,
            }
        }
    }

    impl OpusDecoderBackend for FakeOpusDecoder {
        fn decode(&mut self, packet: &[u8], pcm: &mut [i16], decode_fec: bool) -> Result<usize> {
            self.calls.push(DecodeCall {
                payload: packet.to_vec(),
                buf_len: pcm.len(),
                decode_fec,
            });
            if decode_fec {
                // Reconstruct 320 samples tagged with `packet[0] + 100`.
                let fill = i16::from(packet[0]) + 100;
                pcm[..FRAME_SAMPLES].fill(fill);
                return Ok(FRAME_SAMPLES);
            }
            // Payload encoding for test: `[tag, frame_count_20ms]`
            let tag = i16::from(packet[0]);
            let frames = usize::from(packet.get(1).copied().unwrap_or(1));
            let total = frames * FRAME_SAMPLES;
            assert!(
                pcm.len() >= total,
                "decoder must receive 1920-sample scratch buffer"
            );
            for (i, sample) in pcm[..total].iter_mut().enumerate() {
                *sample = tag * 10 + (i / FRAME_SAMPLES) as i16;
            }
            Ok(total)
        }

        fn decode_plc(&mut self, pcm: &mut [i16], frame_samples: usize) -> Result<usize> {
            self.plc_calls += 1;
            pcm[..frame_samples].fill(-1);
            Ok(frame_samples)
        }
    }

    #[test]
    fn neteq_decodes_60ms_packet_and_drains_carry_pcm_without_advancing_next_idx() {
        let mut neteq = NetEq::new(
            FakeOpusDecoder::new(),
            NetEqConfig {
                max_depth: 10,
                min_delay_packets: 1,
            },
        );
        // Packet seq=10 carries 60 ms (3 x 320 = 960 samples), tagged with 7.
        neteq.push(10, vec![7, 3]);
        neteq.push(11, vec![8, 1]);

        let mut out = [0i16; FRAME_SAMPLES];
        let o1 = neteq.pop_frame(&mut out).expect("tick 1");
        assert_eq!(o1, PopOutcome::Decoded { samples: 960 });
        assert!(out.iter().all(|&s| s == 70));
        assert_eq!(neteq.carry_samples(), 640);
        let idx_after_decode = neteq.next_idx();

        // Tick 2 & 3 must drain carry_pcm without popping seq=11 or advancing next_idx.
        let o2 = neteq.pop_frame(&mut out).expect("tick 2");
        assert_eq!(o2, PopOutcome::Carry);
        assert!(out.iter().all(|&s| s == 71));
        assert_eq!(neteq.next_idx(), idx_after_decode);

        let o3 = neteq.pop_frame(&mut out).expect("tick 3");
        assert_eq!(o3, PopOutcome::Carry);
        assert!(out.iter().all(|&s| s == 72));
        assert_eq!(neteq.carry_samples(), 0);
        assert_eq!(neteq.next_idx(), idx_after_decode);

        // Tick 4 pops seq=11.
        let o4 = neteq.pop_frame(&mut out).expect("tick 4");
        assert_eq!(o4, PopOutcome::Decoded { samples: 320 });
        assert!(out.iter().all(|&s| s == 80));
    }

    #[test]
    fn neteq_decodes_40ms_packet_via_carry_pcm() {
        let mut neteq = NetEq::new(
            FakeOpusDecoder::new(),
            NetEqConfig {
                max_depth: 10,
                min_delay_packets: 1,
            },
        );
        neteq.push(1, vec![5, 2]);
        let mut out = [0i16; FRAME_SAMPLES];

        assert_eq!(
            neteq.pop_frame(&mut out).expect("tick 1"),
            PopOutcome::Decoded { samples: 640 }
        );
        assert!(out.iter().all(|&s| s == 50));
        assert_eq!(neteq.carry_samples(), 320);

        assert_eq!(
            neteq.pop_frame(&mut out).expect("tick 2"),
            PopOutcome::Carry
        );
        assert!(out.iter().all(|&s| s == 51));
        assert_eq!(neteq.carry_samples(), 0);
    }

    #[test]
    fn neteq_overflow_pruning_advances_next_idx_to_new_oldest_key() {
        let mut neteq = NetEq::new(
            FakeOpusDecoder::new(),
            NetEqConfig {
                max_depth: 3,
                min_delay_packets: 1,
            },
        );
        // Push 5 packets into a max_depth=3 buffer -> seq 1 and 2 are pruned; oldest becomes seq 3.
        for seq in 1..=5u16 {
            neteq.push(seq, vec![seq as u8, 1]);
        }
        assert_eq!(neteq.queued_packets(), 3);
        assert_eq!(neteq.next_idx(), Some((1u64 << 16) + 3));

        let mut out = [0i16; FRAME_SAMPLES];
        let o = neteq.pop_frame(&mut out).expect("pop oldest surviving");
        assert_eq!(o, PopOutcome::Decoded { samples: 320 });
        assert!(out.iter().all(|&s| s == 30));
    }

    #[test]
    fn neteq_single_packet_hole_invokes_decode_fec_then_decodes_next_normally() {
        let mut neteq = NetEq::new(
            FakeOpusDecoder::new(),
            NetEqConfig {
                max_depth: 10,
                min_delay_packets: 2,
            },
        );
        neteq.push(10, vec![1, 1]);
        // seq 11 is missing (single-packet hole); seq 12 is present.
        neteq.push(12, vec![3, 1]);

        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(
            neteq.pop_frame(&mut out).expect("seq 10"),
            PopOutcome::Decoded { samples: 320 }
        );
        assert!(out.iter().all(|&s| s == 10));

        // Missing seq 11 -> FEC from seq 12 with 320-sample buffer.
        assert_eq!(
            neteq.pop_frame(&mut out).expect("seq 11 FEC"),
            PopOutcome::FecRecovered
        );
        assert!(out.iter().all(|&s| s == 103));
        assert!(neteq.decoder.calls.last().unwrap().decode_fec);
        assert_eq!(neteq.decoder.calls.last().unwrap().buf_len, FRAME_SAMPLES);

        // Next tick decodes seq 12 normally.
        assert_eq!(
            neteq.pop_frame(&mut out).expect("seq 12 normal"),
            PopOutcome::Decoded { samples: 320 }
        );
        assert!(out.iter().all(|&s| s == 30));
        assert!(!neteq.decoder.calls.last().unwrap().decode_fec);
    }

    #[test]
    fn neteq_handles_u16_wraparound_and_plc_starvation_reset() {
        let mut neteq = NetEq::new(
            FakeOpusDecoder::new(),
            NetEqConfig {
                max_depth: 10,
                min_delay_packets: 2,
            },
        );
        neteq.push(65534, vec![1, 1]);
        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(
            neteq.pop_frame(&mut out).expect("pre-buffering"),
            PopOutcome::Buffering
        );

        neteq.push(65535, vec![2, 1]);
        neteq.push(0, vec![3, 1]);
        neteq.push(1, vec![4, 1]);

        for expected_tag in [10i16, 20, 30, 40] {
            assert_eq!(
                neteq.pop_frame(&mut out).expect("pop"),
                PopOutcome::Decoded { samples: 320 }
            );
            assert!(out.iter().all(|&s| s == expected_tag));
        }

        // 5 consecutive PLC ticks with empty queue reset primed state to Buffering.
        for _ in 0..5 {
            assert_eq!(neteq.pop_frame(&mut out).expect("plc"), PopOutcome::Plc);
        }
        assert_eq!(
            neteq.pop_frame(&mut out).expect("re-buffering"),
            PopOutcome::Buffering
        );
    }

    #[cfg(feature = "call")]
    #[test]
    fn neteq_with_libopus_handles_60ms_carry_and_single_packet_fec() {
        use super::super::opus_codec::{LibOpusDecoder, LibOpusEncoder, OpusEncoderBackend};

        let mut enc = LibOpusEncoder::new(24_000).expect("encoder");
        let dec = LibOpusDecoder::new().expect("decoder");
        let mut neteq = NetEq::new(
            dec,
            NetEqConfig {
                max_depth: 10,
                min_delay_packets: 2,
            },
        );

        let tone = |len: usize| -> Vec<i16> {
            (0..len)
                .map(|i| ((i as f32 * 0.2).sin() * 6_000.0) as i16)
                .collect()
        };

        let mut pkt0 = vec![0u8; 1500];
        let n0 = enc.encode(&tone(960), &mut pkt0).expect("encode 60ms");
        pkt0.truncate(n0);

        let mut pkt1 = vec![0u8; 1500];
        let n1 = enc.encode(&tone(320), &mut pkt1).expect("encode 20ms");
        pkt1.truncate(n1);

        let mut pkt2 = vec![0u8; 1500];
        let n2 = enc.encode(&tone(320), &mut pkt2).expect("encode 20ms");
        pkt2.truncate(n2);

        neteq.push(0, pkt0);
        // Drop seq 1; push seq 2.
        neteq.push(2, pkt2);

        let mut out = [0i16; FRAME_SAMPLES];
        assert_eq!(
            neteq.pop_frame(&mut out).expect("seq 0"),
            PopOutcome::Decoded { samples: 960 }
        );
        assert_eq!(
            neteq.pop_frame(&mut out).expect("carry 1"),
            PopOutcome::Carry
        );
        assert_eq!(
            neteq.pop_frame(&mut out).expect("carry 2"),
            PopOutcome::Carry
        );
        assert_eq!(
            neteq.pop_frame(&mut out).expect("seq 1 FEC"),
            PopOutcome::FecRecovered
        );
        assert_eq!(
            neteq.pop_frame(&mut out).expect("seq 2"),
            PopOutcome::Decoded { samples: 320 }
        );
    }
}
