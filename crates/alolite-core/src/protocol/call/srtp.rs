//! RFC 3711 SRTP (`AES-128-CM + HMAC-SHA1-80`) and `RocTracker` handover fallback (`EDGE-4`).
//!
//! Enforces `INV-8` (keys zeroized on drop and redacted in `Debug`) and constant-time MAC verification.

use std::fmt;

use aes::Aes128;
use anyhow::{bail, Result};
use cipher::{KeyIvInit, StreamCipher};
use ctr::Ctr128BE;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// RFC 3711 AES-128 master key length in bytes.
pub const SRTP_MASTER_KEY_LEN: usize = 16;
/// RFC 3711 master salt length in bytes.
pub const SRTP_MASTER_SALT_LEN: usize = 14;
/// RFC 3711 HMAC-SHA1-80 authentication tag length in bytes.
pub const SRTP_AUTH_TAG_LEN: usize = 10;
/// Minimum RTP header length in bytes.
const RTP_MIN_HEADER_LEN: usize = 12;

type HmacSha1 = Hmac<Sha1>;
type Aes128Ctr = Ctr128BE<Aes128>;

/// Tracks RTP rollover counter (`ROC`) and last authenticated sequence number.
///
/// Supports `EDGE-4` backward sequence resets across network handovers (e.g. `50000 -> 1000`)
/// by testing `[guessed, 0, roc + 1, roc - 1]` against the 80-bit HMAC-SHA1 tag.
#[derive(Debug, Clone, Default)]
pub struct RocTracker {
    roc: u32,
    last_seq: Option<u16>,
}

impl RocTracker {
    /// Create a new [`RocTracker`] starting at `roc = 0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current authenticated rollover counter.
    pub const fn roc(&self) -> u32 {
        self.roc
    }

    /// Last authenticated 16-bit RTP sequence number.
    pub const fn last_seq(&self) -> Option<u16> {
        self.last_seq
    }

    /// Estimate `ROC` for an incoming `seq` using RFC 3711 Appendix A.
    pub fn guess_roc(&self, seq: u16) -> u32 {
        let Some(s_l) = self.last_seq else {
            return 0;
        };
        if s_l < 32768 {
            if seq.wrapping_sub(s_l) > 32768 && seq > s_l {
                self.roc.wrapping_sub(1)
            } else {
                self.roc
            }
        } else if s_l - 32768 > seq {
            self.roc.wrapping_add(1)
        } else {
            self.roc
        }
    }

    /// Return deduplicated candidate `ROC` values in verification priority order:
    /// `[guessed, 0, roc + 1, roc - 1]` (`EDGE-4`).
    pub fn candidate_rocs(&self, seq: u16) -> Vec<u32> {
        let guessed = self.guess_roc(seq);
        let raw = [
            guessed,
            0,
            self.roc.wrapping_add(1),
            self.roc.wrapping_sub(1),
        ];
        let mut out = Vec::with_capacity(4);
        for cand in raw {
            if !out.contains(&cand) {
                out.push(cand);
            }
        }
        out
    }

    /// Commit `(seq, matched_roc)` after cryptographic authentication succeeds.
    ///
    /// Accepts backward sequence resets `> 100` packets once HMAC-SHA1-80 passes (`EDGE-4`).
    pub fn update(&mut self, seq: u16, matched_roc: u32) {
        self.roc = matched_roc;
        self.last_seq = Some(seq);
    }
}

/// RFC 3711 SRTP cipher context (`AES-128-CM` + `HMAC-SHA1-80`).
///
/// Session keys are derived via RFC 3711 §4.3.1 KDF, zeroized on drop, and redacted in `Debug` (`INV-8`).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SrtpContext {
    session_enc_key: [u8; 16],
    session_auth_key: [u8; 20],
    session_salt: [u8; 14],
    #[zeroize(skip)]
    tx_roc: RocTracker,
    #[zeroize(skip)]
    rx_roc: RocTracker,
}

impl fmt::Debug for SrtpContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SrtpContext")
            .field("keys", &"[REDACTED]")
            .field("tx_roc", &self.tx_roc)
            .field("rx_roc", &self.rx_roc)
            .finish()
    }
}

impl SrtpContext {
    /// Derive SRTP session keys from a 16-byte master key and 14-byte master salt (RFC 3711 §4.3.1).
    pub fn new(
        master_key: [u8; SRTP_MASTER_KEY_LEN],
        master_salt: [u8; SRTP_MASTER_SALT_LEN],
    ) -> Self {
        let session_enc_key = derive_session_key::<16>(&master_key, &master_salt, 0x00);
        let session_auth_key = derive_session_key::<20>(&master_key, &master_salt, 0x01);
        let session_salt = derive_session_key::<14>(&master_key, &master_salt, 0x02);
        Self {
            session_enc_key,
            session_auth_key,
            session_salt,
            tx_roc: RocTracker::new(),
            rx_roc: RocTracker::new(),
        }
    }

    /// Construct from a combined 30-byte `(master_key[16] || master_salt[14])` slice.
    ///
    /// # Errors
    /// Returns an error if `key_and_salt.len() != 30`.
    pub fn from_combined(key_and_salt: &[u8]) -> Result<Self> {
        let expected = SRTP_MASTER_KEY_LEN + SRTP_MASTER_SALT_LEN;
        if key_and_salt.len() != expected {
            bail!(
                "invalid SRTP key+salt length: {} (expected {expected})",
                key_and_salt.len()
            );
        }
        let mut master_key = [0u8; SRTP_MASTER_KEY_LEN];
        let mut master_salt = [0u8; SRTP_MASTER_SALT_LEN];
        master_key.copy_from_slice(&key_and_salt[..SRTP_MASTER_KEY_LEN]);
        master_salt.copy_from_slice(&key_and_salt[SRTP_MASTER_KEY_LEN..]);
        Ok(Self::new(master_key, master_salt))
    }

    /// Borrow the outbound [`RocTracker`].
    pub const fn tx_roc(&self) -> &RocTracker {
        &self.tx_roc
    }

    /// Borrow the inbound [`RocTracker`].
    pub const fn rx_roc(&self) -> &RocTracker {
        &self.rx_roc
    }

    /// Encrypt and authenticate a plaintext RTP packet into an SRTP packet (`AES-128-CM` + `HMAC-SHA1-80`).
    ///
    /// # Errors
    /// Returns an error if the RTP header is malformed or truncated.
    pub fn protect_rtp(&mut self, rtp: &[u8]) -> Result<Vec<u8>> {
        let hdr_len = rtp_header_len(rtp)?;
        let seq = u16::from_be_bytes([rtp[2], rtp[3]]);
        let ssrc = u32::from_be_bytes([rtp[8], rtp[9], rtp[10], rtp[11]]);

        let roc = if let Some(last) = self.tx_roc.last_seq() {
            if seq < last && last.wrapping_sub(seq) > 32768 {
                self.tx_roc.roc().wrapping_add(1)
            } else {
                self.tx_roc.roc()
            }
        } else {
            0
        };

        let mut out = Vec::with_capacity(rtp.len() + SRTP_AUTH_TAG_LEN);
        out.extend_from_slice(rtp);
        apply_aes_cm(
            &self.session_enc_key,
            &self.session_salt,
            ssrc,
            roc,
            seq,
            &mut out[hdr_len..],
        );
        let tag = compute_auth_tag(&self.session_auth_key, &out, roc);
        out.extend_from_slice(&tag);

        self.tx_roc.update(seq, roc);
        Ok(out)
    }

    /// Authenticate and decrypt an incoming SRTP packet, returning plaintext RTP.
    ///
    /// Uses `RocTracker` fallback `[guessed, 0, roc + 1, roc - 1]` against the 80-bit HMAC-SHA1 tag
    /// (`EDGE-4`) with constant-time tag comparison.
    ///
    /// # Errors
    /// Returns an error if the packet is shorter than `12 + 10` bytes, has a malformed RTP header,
    /// or fails HMAC-SHA1-80 authentication for all candidate `ROC`s.
    pub fn unprotect_rtp(&mut self, srtp: &[u8]) -> Result<Vec<u8>> {
        if srtp.len() < RTP_MIN_HEADER_LEN + SRTP_AUTH_TAG_LEN {
            bail!("SRTP packet too short: {} bytes", srtp.len());
        }
        let body_len = srtp.len() - SRTP_AUTH_TAG_LEN;
        let auth_part = &srtp[..body_len];
        let recv_tag = &srtp[body_len..];
        let hdr_len = rtp_header_len(auth_part)?;

        let seq = u16::from_be_bytes([auth_part[2], auth_part[3]]);
        let ssrc = u32::from_be_bytes([auth_part[8], auth_part[9], auth_part[10], auth_part[11]]);

        let mut matched_roc = None;
        for cand_roc in self.rx_roc.candidate_rocs(seq) {
            let expected_tag = compute_auth_tag(&self.session_auth_key, auth_part, cand_roc);
            if bool::from(expected_tag.ct_eq(recv_tag)) {
                matched_roc = Some(cand_roc);
                break;
            }
        }

        let roc =
            matched_roc.ok_or_else(|| anyhow::anyhow!("SRTP HMAC-SHA1-80 verification failed"))?;
        let mut rtp = auth_part.to_vec();
        apply_aes_cm(
            &self.session_enc_key,
            &self.session_salt,
            ssrc,
            roc,
            seq,
            &mut rtp[hdr_len..],
        );
        self.rx_roc.update(seq, roc);
        Ok(rtp)
    }
}

fn derive_session_key<const N: usize>(
    master_key: &[u8; SRTP_MASTER_KEY_LEN],
    master_salt: &[u8; SRTP_MASTER_SALT_LEN],
    label: u8,
) -> [u8; N] {
    let mut iv = [0u8; 16];
    iv[..SRTP_MASTER_SALT_LEN].copy_from_slice(master_salt);
    iv[7] ^= label;

    let mut out = [0u8; N];
    let mut cipher = Aes128Ctr::new(master_key.into(), &iv.into());
    cipher.apply_keystream(&mut out);
    out
}

fn apply_aes_cm(
    session_key: &[u8; 16],
    session_salt: &[u8; 14],
    ssrc: u32,
    roc: u32,
    seq: u16,
    payload: &mut [u8],
) {
    let mut iv = [0u8; 16];
    iv[..14].copy_from_slice(session_salt);
    let ssrc_bytes = ssrc.to_be_bytes();
    let roc_bytes = roc.to_be_bytes();
    let seq_bytes = seq.to_be_bytes();
    for i in 0..4 {
        iv[4 + i] ^= ssrc_bytes[i];
        iv[8 + i] ^= roc_bytes[i];
    }
    iv[12] ^= seq_bytes[0];
    iv[13] ^= seq_bytes[1];

    let mut cipher = Aes128Ctr::new(session_key.into(), &iv.into());
    cipher.apply_keystream(payload);
}

fn compute_auth_tag(auth_key: &[u8; 20], data: &[u8], roc: u32) -> [u8; SRTP_AUTH_TAG_LEN] {
    let mut mac =
        <HmacSha1 as KeyInit>::new_from_slice(auth_key).expect("HMAC-SHA1 accepts 20-byte key");
    mac.update(data);
    mac.update(&roc.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let mut tag = [0u8; SRTP_AUTH_TAG_LEN];
    tag.copy_from_slice(&digest[..SRTP_AUTH_TAG_LEN]);
    tag
}

fn rtp_header_len(rtp: &[u8]) -> Result<usize> {
    if rtp.len() < RTP_MIN_HEADER_LEN {
        bail!(
            "RTP packet shorter than 12-byte header ({} bytes)",
            rtp.len()
        );
    }
    let cc = usize::from(rtp[0] & 0x0f);
    let has_ext = (rtp[0] & 0x10) != 0;
    let mut hdr_len = RTP_MIN_HEADER_LEN + cc * 4;
    if hdr_len > rtp.len() {
        bail!("RTP CSRC list exceeds packet length");
    }
    if has_ext {
        if hdr_len + 4 > rtp.len() {
            bail!("RTP extension header truncated");
        }
        let ext_words = usize::from(u16::from_be_bytes([rtp[hdr_len + 2], rtp[hdr_len + 3]]));
        hdr_len += 4 + ext_words * 4;
        if hdr_len > rtp.len() {
            bail!("RTP extension body exceeds packet length");
        }
    }
    Ok(hdr_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_to_bytes<const N: usize>(hex: &str) -> [u8; N] {
        assert_eq!(hex.len(), N * 2);
        let mut out = [0u8; N];
        for i in 0..N {
            out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    fn make_rtp(seq: u16, ssrc: u32, payload: &[u8]) -> Vec<u8> {
        let mut pkt = Vec::with_capacity(12 + payload.len());
        pkt.push(0x80); // V=2
        pkt.push(111); // PT=111 (Opus)
        pkt.extend_from_slice(&seq.to_be_bytes());
        pkt.extend_from_slice(&(u32::from(seq) * 320).to_be_bytes());
        pkt.extend_from_slice(&ssrc.to_be_bytes());
        pkt.extend_from_slice(payload);
        pkt
    }

    #[test]
    fn rfc3711_appendix_b3_key_derivation_known_answer() {
        // Independent ground truth from RFC 3711 Appendix B.3.
        let master_key = hex_to_bytes::<16>("E1F97A0D3E018BE0D64FA32C06DE4139");
        let master_salt = hex_to_bytes::<14>("0EC675AD498AFEEBB6960B3AABE6");
        let ctx = SrtpContext::new(master_key, master_salt);

        assert_eq!(
            ctx.session_enc_key,
            hex_to_bytes::<16>("C61E7A93744F39EE10734AFE3FF7A087")
        );
        assert_eq!(
            ctx.session_auth_key,
            hex_to_bytes::<20>("CEBE321F6FF7716B6FD4AB49AF256A156D38BAA4")
        );
        assert_eq!(
            ctx.session_salt,
            hex_to_bytes::<14>("30CBBC08863D8C85D49DB34A9AE1")
        );
    }

    #[test]
    fn srtp_protect_and_unprotect_roundtrips_and_rejects_tampering() {
        let key_and_salt = [0x42u8; 30];
        let mut tx = SrtpContext::from_combined(&key_and_salt).unwrap();
        let mut rx = SrtpContext::from_combined(&key_and_salt).unwrap();

        let rtp = make_rtp(100, 0x12345678, b"opus-frame-20ms");
        let srtp = tx.protect_rtp(&rtp).unwrap();
        assert_eq!(srtp.len(), rtp.len() + SRTP_AUTH_TAG_LEN);
        assert_ne!(&srtp[12..rtp.len()], &rtp[12..]);

        let recovered = rx.unprotect_rtp(&srtp).unwrap();
        assert_eq!(recovered, rtp);

        // Tamper with auth tag -> must fail.
        let mut tampered = srtp.clone();
        tampered[srtp.len() - 1] ^= 0xff;
        assert!(rx.unprotect_rtp(&tampered).is_err());

        // (INV-8) Debug output must redact keys.
        assert!(!format!("{tx:?}").contains("42"));
    }

    #[test]
    fn roc_tracker_recovers_backward_seq_reset_across_handover() {
        // EDGE-4: peer resets RTP seq from 50000 to 1000 with roc=0 across network handover.
        let key_and_salt = [0x77u8; 30];
        let mut tx_before = SrtpContext::from_combined(&key_and_salt).unwrap();
        let mut rx = SrtpContext::from_combined(&key_and_salt).unwrap();

        let p1 = tx_before
            .protect_rtp(&make_rtp(50000, 0xAABBCCDD, b"before"))
            .unwrap();
        assert_eq!(
            rx.unprotect_rtp(&p1).unwrap(),
            make_rtp(50000, 0xAABBCCDD, b"before")
        );
        assert_eq!(rx.rx_roc().last_seq(), Some(50000));
        assert_eq!(rx.rx_roc().roc(), 0);

        // Fresh sender state after handover starts at seq = 1000, roc = 0.
        // RFC 3711 guess_roc(1000) when last_seq=50000 guesses roc + 1 = 1,
        // so fallback [0, roc+1, roc-1] must recover roc = 0.
        let mut tx_after = SrtpContext::from_combined(&key_and_salt).unwrap();
        let p2 = tx_after
            .protect_rtp(&make_rtp(1000, 0xAABBCCDD, b"after"))
            .unwrap();
        let recovered = rx.unprotect_rtp(&p2).unwrap();
        assert_eq!(recovered, make_rtp(1000, 0xAABBCCDD, b"after"));
        assert_eq!(rx.rx_roc().roc(), 0);
        assert_eq!(rx.rx_roc().last_seq(), Some(1000));
    }
}
