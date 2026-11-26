//! Pure-Rust ZRTC 1-1 audio call protocol: signaling, ZRTP framing, SRTP cipher, and state machine.
//!
//! Enforces `INV-5` (zero `rusqlite` or audio device imports), `INV-7` (single active `CallSession`),
//! and `INV-8` (`srtpMode: 1` default + redacted secrets/IPs in `Debug`).

pub mod session;
pub mod signaling;
pub mod srtp;
pub mod zrtp;

pub use session::{CallDirection, CallEffect, CallEndReason, CallEvent, CallSession, CallState};
pub use signaling::{
    CallSignalCode, CallSignalWireData, ExtendData, RelayServerAddr, SrtpKeyMaterial,
};
pub use srtp::{
    RocTracker, SrtpContext, SRTP_AUTH_TAG_LEN, SRTP_MASTER_KEY_LEN, SRTP_MASTER_SALT_LEN,
};
pub use zrtp::{P2pKind, P2pSignalSubCmd, ReqForwardSubCmd, ZRTPPacket, ZrtpCmd};
