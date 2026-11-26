//! Pure-Rust ZRTC direct P2P (`0x7f`) candidate binding, media routing, and Network Handover state machine.
//!
//! Enforces `INV-5` (zero `rusqlite`, audio devices, or network sockets), `INV-8` (redacted IPs in `Debug`),
//! and `ADV-3` (rejects `0x7f` packets with mismatched `callId` or `senderRole == ownRole`).
//! Protocol details documented in `re-notes/16-zrtc-network-handover-p2p-and-audio-engine.md`.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};

use super::zrtp::{
    P2pBody, P2pKind, P2pSignalSubCmd, ReqForwardSubCmd, ZRTPPacket, ZrtpCmd, MSG_AUDIO_RTCP_UP,
    MSG_AUDIO_RTP_UP,
};

/// Consecutive relay audio packets (`25 * 20 ms = 500 ms`) that trigger automatic P2P teardown.
pub const RELAY_STREAK_TEARDOWN_THRESHOLD: u32 = 25;

/// Minimum elapsed time (`5000 ms`) after P2P activation before relay streak teardown is armed.
pub const P2P_MIN_UPTIME_FOR_STREAK_MS: u64 = 5_000;

/// Retry interval (`3000 ms`) for unacknowledged `CHANGE_ADDRESS` (`cmd = 14`).
pub const CHANGE_ADDRESS_RETRY_MS: u64 = 3_000;

/// Bounded retry attempts for `CHANGE_ADDRESS` (`5 * 3000 ms = 15 s`, matching media watchdog).
pub const MAX_CHANGE_ADDRESS_ATTEMPTS: u32 = 5;

/// Redundant `P2P.SIGNAL` / `REQ_FORWARD` stop packets sent on handover (`seq = 0..3`).
pub const HANDOVER_STOP_REDUNDANCY: u32 = 3;

/// Direct P2P path state (`re-notes/16` §5.2).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum P2pState {
    /// No P2P binding attempted yet.
    Idle,
    /// `P2P.BIND` probes sent; awaiting `BIND_PEER` / `BIND_ANSWER`.
    Negotiating,
    /// Direct P2P media path active (`p2pState == 6` in `zcall_mac.node`).
    Active {
        peer_addr: SocketAddr,
        cand_index: u32,
        started_at_ms: u64,
    },
    /// P2P torn down (`p2pState == 7`); audio routes through Relay Server.
    Stopped,
}

impl fmt::Debug for P2pState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("P2pState::Idle"),
            Self::Negotiating => f.write_str("P2pState::Negotiating"),
            Self::Active {
                cand_index,
                started_at_ms,
                ..
            } => f
                .debug_struct("P2pState::Active")
                .field("peer_addr", &"<redacted-ip>")
                .field("cand_index", cand_index)
                .field("started_at_ms", started_at_ms)
                .finish(),
            Self::Stopped => f.write_str("P2pState::Stopped"),
        }
    }
}

/// Outbound datagram emitted by [`P2pController`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2pOutbound {
    /// Send directly to a peer UDP `SocketAddr`.
    ToPeer(SocketAddr, ZRTPPacket),
    /// Send to the active ZRTC Relay Server.
    ToRelay(ZRTPPacket),
}

/// Classification of an incoming `0x7f` packet processed by [`P2pController`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2pRxOutcome {
    /// Control/signaling packet handled internally (or non-audio media ignored).
    Handled,
    /// Validated `P2pKind::Audio` (`kind = 6`) SRTP payload ready for `SrtpContext::unprotect_rtp`.
    AudioMedia(Vec<u8>),
    /// Validated `P2pKind::AudioRtcp` (`kind = 7`) compound RTCP payload.
    AudioRtcp(Vec<u8>),
    /// Rejected by `ADV-3` (`callId` mismatch or `senderRole == ownRole`).
    Drop,
}

/// Parse `"ip|port"` or `"ip:port"` candidate strings into a [`SocketAddr`].
pub fn parse_p2p_addr(raw: &str) -> Option<SocketAddr> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((ip, port)) = trimmed.rsplit_once('|') {
        return format!("{ip}:{port}").parse::<SocketAddr>().ok();
    }
    trimmed.parse::<SocketAddr>().ok()
}

/// Format a [`SocketAddr`] as `"ip|port"` for `BIND_PEER` / `BIND_ANSWER` payloads.
pub fn format_p2p_addr(addr: SocketAddr) -> String {
    format!("{}|{}", addr.ip(), addr.port())
}

/// Pure ZRTC P2P & Network Handover state machine (`INV-5`).
pub struct P2pController {
    own_role: u8,
    call_id: u32,
    uid: u32,
    server_token: u32,
    sess_id: String,
    state: P2pState,
    candidates: Vec<SocketAddr>,
    bind_seq: u16,
    ctrl_seq: u32,
    current_local_ip: Option<Ipv4Addr>,
    consecutive_relay_audio: u32,
    changing_address: bool,
    change_address_attempts: u32,
    last_change_address_sent_ms: u64,
    dropped_packets: u64,
}

impl fmt::Debug for P2pController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("P2pController")
            .field("own_role", &self.own_role)
            .field("call_id", &self.call_id)
            .field("state", &self.state)
            .field("candidate_count", &self.candidates.len())
            .field("consecutive_relay_audio", &self.consecutive_relay_audio)
            .field("changing_address", &self.changing_address)
            .field("dropped_packets", &self.dropped_packets)
            .finish()
    }
}

impl P2pController {
    /// Create a new [`P2pController`] for a 1-1 call (`is_caller = true` -> `own_role = 0`).
    pub fn new(
        is_caller: bool,
        call_id: u32,
        uid: u32,
        server_token: u32,
        sess_id: impl Into<String>,
    ) -> Self {
        Self {
            own_role: if is_caller { 0 } else { 1 },
            call_id,
            uid,
            server_token,
            sess_id: sess_id.into(),
            state: P2pState::Idle,
            candidates: Vec::new(),
            bind_seq: 1,
            ctrl_seq: 10,
            current_local_ip: None,
            consecutive_relay_audio: 0,
            changing_address: false,
            change_address_attempts: 0,
            last_change_address_sent_ms: 0,
            dropped_packets: 0,
        }
    }

    /// Update the assigned relay `serverToken` (e.g. after `InitResponse`).
    pub fn set_server_token(&mut self, token: u32) {
        if token != 0 {
            self.server_token = token;
        }
    }

    /// Current [`P2pState`].
    pub const fn state(&self) -> P2pState {
        self.state
    }

    /// Active peer P2P [`SocketAddr`], if `state` is [`P2pState::Active`].
    pub const fn active_peer(&self) -> Option<SocketAddr> {
        match self.state {
            P2pState::Active { peer_addr, .. } => Some(peer_addr),
            _ => None,
        }
    }

    /// Number of `0x7f` packets dropped by `ADV-3` validation.
    pub const fn dropped_packets(&self) -> u64 {
        self.dropped_packets
    }

    /// Whether a `CHANGE_ADDRESS` (`cmd = 14`) handover is awaiting `SERVER_REPLY`.
    pub const fn is_changing_address(&self) -> bool {
        self.changing_address
    }

    /// Register peer candidate addresses and emit `P2P.BIND` (`kind = 1`) probes.
    pub fn add_candidates(&mut self, addrs: Vec<SocketAddr>) -> Vec<P2pOutbound> {
        let mut out = Vec::new();
        for addr in addrs {
            if !self.candidates.contains(&addr) {
                self.candidates.push(addr);
            }
        }
        if matches!(self.state, P2pState::Idle | P2pState::Negotiating)
            && !self.candidates.is_empty()
        {
            self.state = P2pState::Negotiating;
            out.extend(self.emit_bind_probes());
        }
        out
    }

    /// Process an incoming `0x7f` (`P2P_EXT`) datagram from `src`.
    ///
    /// Enforces `ADV-3`: drops any packet whose `call_id` differs from the active call
    /// or whose `sender_role` equals `own_role`.
    pub fn handle_p2p_packet(
        &mut self,
        src: SocketAddr,
        sender_role: u8,
        call_id: u32,
        kind: P2pKind,
        body: P2pBody,
        now_ms: u64,
    ) -> (P2pRxOutcome, Vec<P2pOutbound>) {
        // ADV-3: reject spoofed/cross-call or echoed own-role P2P packets before any mutation.
        if call_id != self.call_id || sender_role == self.own_role {
            self.dropped_packets += 1;
            return (P2pRxOutcome::Drop, Vec::new());
        }

        let mut out = Vec::new();
        match (kind, body) {
            (P2pKind::Bind, P2pBody::Bind { seq, cand_index }) => {
                if !self.candidates.contains(&src) {
                    self.candidates.push(src);
                }
                if self.state == P2pState::Idle {
                    self.state = P2pState::Negotiating;
                }
                out.push(P2pOutbound::ToPeer(
                    src,
                    ZRTPPacket::p2p_bind_peer(
                        self.own_role,
                        self.call_id,
                        seq,
                        cand_index,
                        format_p2p_addr(src),
                    ),
                ));
                (P2pRxOutcome::Handled, out)
            }
            (
                P2pKind::BindPeer,
                P2pBody::BindPeer {
                    seq,
                    cand_index,
                    addr,
                },
            ) => {
                out.push(P2pOutbound::ToPeer(
                    src,
                    ZRTPPacket::p2p_bind_answer(self.own_role, self.call_id, seq, cand_index, addr),
                ));
                if self.state != P2pState::Stopped {
                    self.activate_p2p(src, cand_index, now_ms, &mut out);
                }
                (P2pRxOutcome::Handled, out)
            }
            (P2pKind::BindAnswer, P2pBody::BindAnswer { cand_index, .. }) => {
                if self.state != P2pState::Stopped && !matches!(self.state, P2pState::Active { .. })
                {
                    self.activate_p2p(src, cand_index, now_ms, &mut out);
                }
                (P2pRxOutcome::Handled, out)
            }
            (
                P2pKind::Keepalive,
                P2pBody::Keepalive {
                    sub_cmd,
                    seq,
                    payload,
                },
            ) => {
                if sub_cmd == 1 {
                    out.push(P2pOutbound::ToPeer(
                        src,
                        ZRTPPacket::p2p_keepalive(self.own_role, self.call_id, 2, seq, payload),
                    ));
                }
                (P2pRxOutcome::Handled, out)
            }
            (P2pKind::Signal, P2pBody::Signal { sub_cmd, .. }) => {
                match P2pSignalSubCmd::from_u8(sub_cmd) {
                    Some(P2pSignalSubCmd::Start) => {
                        if self.state != P2pState::Stopped
                            && !matches!(self.state, P2pState::Active { .. })
                        {
                            self.state = P2pState::Active {
                                peer_addr: src,
                                cand_index: 0,
                                started_at_ms: now_ms,
                            };
                            self.consecutive_relay_audio = 0;
                        }
                    }
                    Some(P2pSignalSubCmd::Stop) => {
                        self.stop_p2p(false, &mut out);
                    }
                    None => {}
                }
                (P2pRxOutcome::Handled, out)
            }
            (P2pKind::Audio, P2pBody::Media(bytes)) => {
                self.consecutive_relay_audio = 0;
                (P2pRxOutcome::AudioMedia(bytes), out)
            }
            (P2pKind::AudioRtcp, P2pBody::Media(bytes)) => (P2pRxOutcome::AudioRtcp(bytes), out),
            (P2pKind::Video | P2pKind::VideoRtcp, _) => (P2pRxOutcome::Handled, out),
            _ => (P2pRxOutcome::Handled, out),
        }
    }

    /// Build an outbound audio SRTP datagram routed via P2P (`kind = 6`) when active, or Relay (`0x03`).
    pub fn build_outbound_audio(&self, srtp_payload: Vec<u8>) -> P2pOutbound {
        if let P2pState::Active { peer_addr, .. } = self.state {
            P2pOutbound::ToPeer(
                peer_addr,
                ZRTPPacket::p2p_audio(self.own_role, self.call_id, srtp_payload),
            )
        } else {
            P2pOutbound::ToRelay(ZRTPPacket::RelayMediaUp {
                msg_type: MSG_AUDIO_RTP_UP,
                server_token: self.server_token,
                payload: srtp_payload,
            })
        }
    }

    /// Build an outbound RTCP datagram routed via P2P (`kind = 7`) when active, or Relay (`0x05`).
    pub fn build_outbound_rtcp(&self, rtcp_payload: Vec<u8>) -> P2pOutbound {
        if let P2pState::Active { peer_addr, .. } = self.state {
            P2pOutbound::ToPeer(
                peer_addr,
                ZRTPPacket::p2p_audio_rtcp(self.own_role, self.call_id, rtcp_payload),
            )
        } else {
            P2pOutbound::ToRelay(ZRTPPacket::RelayMediaUp {
                msg_type: MSG_AUDIO_RTCP_UP,
                server_token: self.server_token,
                payload: rtcp_payload,
            })
        }
    }

    /// Record an authenticated audio packet arriving from the Relay Server (`0x04`).
    ///
    /// Automatically tears down P2P (`stop_p2p(false)`) after [`RELAY_STREAK_TEARDOWN_THRESHOLD`] (`25`)
    /// consecutive relay audio packets when `state == Active` and at least `5000 ms` have elapsed.
    pub fn on_relay_audio_received(&mut self, now_ms: u64) {
        self.consecutive_relay_audio = self.consecutive_relay_audio.saturating_add(1);
        if let P2pState::Active { started_at_ms, .. } = self.state {
            if self.consecutive_relay_audio >= RELAY_STREAK_TEARDOWN_THRESHOLD
                && now_ms.saturating_sub(started_at_ms) >= P2P_MIN_UPTIME_FOR_STREAK_MS
            {
                let mut sink = Vec::new();
                self.stop_p2p(false, &mut sink);
            }
        }
    }

    /// Handle a 1 Hz local IPv4 route probe result (`connect("8.8.8.8:53") + getsockname()`).
    ///
    /// On IP change:
    /// 1. If `p2pState == Active`, tears down P2P and emits `3x` `P2P.SIGNAL (subCmd=2)` + `REQ_FORWARD (subCmd=2)`.
    /// 2. Sends `CHANGE_ADDRESS` (`cmd = 14`) to the Relay Server and arms the `3000 ms` retry timer.
    pub fn on_local_ip_probe(&mut self, local_ip: Ipv4Addr, now_ms: u64) -> Vec<P2pOutbound> {
        let Some(prev) = self.current_local_ip else {
            self.current_local_ip = Some(local_ip);
            return Vec::new();
        };
        if prev == local_ip {
            return Vec::new();
        }
        self.current_local_ip = Some(local_ip);

        let mut out = Vec::new();
        if matches!(self.state, P2pState::Active { .. }) {
            self.stop_p2p(true, &mut out);
        }

        self.changing_address = true;
        self.change_address_attempts = 1;
        self.last_change_address_sent_ms = now_ms;
        let seq = self.next_ctrl_seq();
        out.push(P2pOutbound::ToRelay(ZRTPPacket::change_address(
            seq,
            self.uid,
            self.server_token,
            self.sess_id.clone(),
        )));
        out
    }

    /// Poll the `3000 ms` `CHANGE_ADDRESS` retry timer.
    pub fn on_change_address_tick(&mut self, now_ms: u64) -> Vec<P2pOutbound> {
        if !self.changing_address {
            return Vec::new();
        }
        if now_ms.saturating_sub(self.last_change_address_sent_ms) < CHANGE_ADDRESS_RETRY_MS {
            return Vec::new();
        }
        if self.change_address_attempts >= MAX_CHANGE_ADDRESS_ATTEMPTS {
            self.changing_address = false;
            return Vec::new();
        }

        self.change_address_attempts += 1;
        self.last_change_address_sent_ms = now_ms;
        let seq = self.next_ctrl_seq();
        vec![P2pOutbound::ToRelay(ZRTPPacket::change_address(
            seq,
            self.uid,
            self.server_token,
            self.sess_id.clone(),
        ))]
    }

    /// Handle an incoming `SERVER_REPLY` (`0x02`) from the Relay Server.
    pub fn on_server_reply(&mut self, cmd: u16, token: u32) {
        self.set_server_token(token);
        if cmd == ZrtpCmd::ChangeAddress as u16 {
            self.changing_address = false;
            self.change_address_attempts = 0;
        }
    }

    /// Handle an incoming `REQ_FORWARD` (`cmd = 32`) relayed from the remote peer.
    pub fn on_req_forward(&mut self, sub_cmd: u8) -> Vec<P2pOutbound> {
        let mut out = Vec::new();
        match ReqForwardSubCmd::from_u8(sub_cmd) {
            Some(ReqForwardSubCmd::StopP2p) => {
                self.stop_p2p(false, &mut out);
            }
            Some(ReqForwardSubCmd::RestartP2p)
                if self.state == P2pState::Stopped && !self.candidates.is_empty() =>
            {
                self.state = P2pState::Negotiating;
                out.extend(self.emit_bind_probes());
            }
            _ => {}
        }
        out
    }

    fn activate_p2p(
        &mut self,
        peer_addr: SocketAddr,
        cand_index: u32,
        now_ms: u64,
        out: &mut Vec<P2pOutbound>,
    ) {
        self.state = P2pState::Active {
            peer_addr,
            cand_index,
            started_at_ms: now_ms,
        };
        self.consecutive_relay_audio = 0;
        let seq = self.next_ctrl_seq();
        out.push(P2pOutbound::ToPeer(
            peer_addr,
            ZRTPPacket::p2p_signal(self.own_role, self.call_id, P2pSignalSubCmd::Start, seq),
        ));
    }

    fn stop_p2p(&mut self, notify_peer: bool, out: &mut Vec<P2pOutbound>) {
        let prev_peer = self.active_peer();
        self.state = P2pState::Stopped;
        self.consecutive_relay_audio = 0;
        if notify_peer {
            for seq in 0..HANDOVER_STOP_REDUNDANCY {
                if let Some(peer_addr) = prev_peer {
                    out.push(P2pOutbound::ToPeer(
                        peer_addr,
                        ZRTPPacket::p2p_signal(
                            self.own_role,
                            self.call_id,
                            P2pSignalSubCmd::Stop,
                            seq,
                        ),
                    ));
                }
                out.push(P2pOutbound::ToRelay(ZRTPPacket::req_forward_empty(
                    seq,
                    self.uid,
                    self.server_token,
                    ReqForwardSubCmd::StopP2p,
                )));
            }
        }
    }

    fn emit_bind_probes(&mut self) -> Vec<P2pOutbound> {
        let mut out = Vec::with_capacity(self.candidates.len());
        for (idx, addr) in self.candidates.iter().copied().enumerate() {
            let seq = self.bind_seq;
            self.bind_seq = self.bind_seq.wrapping_add(1);
            out.push(P2pOutbound::ToPeer(
                addr,
                ZRTPPacket::p2p_bind(self.own_role, self.call_id, seq, idx as u32),
            ));
        }
        out
    }

    fn next_ctrl_seq(&mut self) -> u32 {
        let s = self.ctrl_seq;
        self.ctrl_seq = self.ctrl_seq.wrapping_add(1);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::call::zrtp::MSG_P2P_EXT;

    fn peer_addr() -> SocketAddr {
        "198.51.100.10:50000".parse().unwrap()
    }

    #[test]
    fn p2p_candidate_binding_0x7f_kinds_1_to_5_activates_direct_path() {
        let mut caller = P2pController::new(true, 464564007, 100001, 777, "sess-1");
        let mut callee = P2pController::new(false, 464564007, 100002, 888, "sess-1");
        let caller_addr: SocketAddr = "198.51.100.1:40000".parse().unwrap();
        let callee_addr = peer_addr();

        // Caller sends Bind (kind=1) to callee candidate.
        let probes = caller.add_candidates(vec![callee_addr]);
        assert_eq!(probes.len(), 1);
        let P2pOutbound::ToPeer(dst, bind_pkt) = &probes[0] else {
            panic!("expected ToPeer Bind");
        };
        assert_eq!(*dst, callee_addr);

        // Callee receives Bind (kind=1) -> replies BindPeer (kind=2).
        let ZRTPPacket::P2p {
            sender_role,
            call_id,
            kind,
            body,
            ..
        } = bind_pkt.clone()
        else {
            panic!("expected P2p");
        };
        let (outcome, reply) =
            callee.handle_p2p_packet(caller_addr, sender_role, call_id, kind, body, 1_000);
        assert_eq!(outcome, P2pRxOutcome::Handled);
        assert_eq!(reply.len(), 1);
        let P2pOutbound::ToPeer(_, bind_peer_pkt) = &reply[0] else {
            panic!("expected BindPeer");
        };

        // Caller receives BindPeer (kind=2) -> replies BindAnswer (kind=3) + Signal(Start, kind=5) and becomes Active.
        let ZRTPPacket::P2p {
            sender_role,
            call_id,
            kind,
            body,
            ..
        } = bind_peer_pkt.clone()
        else {
            panic!("expected P2p");
        };
        let (_, caller_replies) =
            caller.handle_p2p_packet(callee_addr, sender_role, call_id, kind, body, 1_020);
        assert_eq!(caller.active_peer(), Some(callee_addr));
        assert!(caller_replies.iter().any(|o| matches!(
            o,
            P2pOutbound::ToPeer(
                _,
                ZRTPPacket::P2p {
                    kind: P2pKind::BindAnswer,
                    ..
                }
            )
        )));

        // Callee receives BindAnswer (kind=3) -> becomes Active too.
        let P2pOutbound::ToPeer(_, bind_ans_pkt) = &caller_replies[0] else {
            panic!("expected BindAnswer");
        };
        let ZRTPPacket::P2p {
            sender_role,
            call_id,
            kind,
            body,
            ..
        } = bind_ans_pkt.clone()
        else {
            panic!("expected P2p");
        };
        let _ = callee.handle_p2p_packet(caller_addr, sender_role, call_id, kind, body, 1_040);
        assert_eq!(callee.active_peer(), Some(caller_addr));

        // Keepalive ping (kind=4, subCmd=1) -> pong (subCmd=2).
        let (_, pong) = callee.handle_p2p_packet(
            caller_addr,
            0,
            464564007,
            P2pKind::Keepalive,
            P2pBody::Keepalive {
                sub_cmd: 1,
                seq: 9,
                payload: vec![0xaa],
            },
            1_100,
        );
        assert!(matches!(
            pong.as_slice(),
            [P2pOutbound::ToPeer(
                _,
                ZRTPPacket::P2p {
                    kind: P2pKind::Keepalive,
                    body: P2pBody::Keepalive {
                        sub_cmd: 2,
                        seq: 9,
                        ..
                    },
                    ..
                }
            )]
        ));
    }

    #[test]
    fn p2p_rejects_wrong_call_id_and_own_sender_role_adv3() {
        let mut ctrl = P2pController::new(true, 100, 1, 2, "sess-1");

        // ADV-3: wrong call_id must be dropped.
        let (out1, pkts1) = ctrl.handle_p2p_packet(
            peer_addr(),
            1,
            999,
            P2pKind::Audio,
            P2pBody::Media(vec![1, 2, 3]),
            1_000,
        );
        assert_eq!(out1, P2pRxOutcome::Drop);
        assert!(pkts1.is_empty());

        // ADV-3: sender_role == own_role (0 == 0) must be dropped.
        let (out2, pkts2) = ctrl.handle_p2p_packet(
            peer_addr(),
            0,
            100,
            P2pKind::Bind,
            P2pBody::Bind {
                seq: 1,
                cand_index: 0,
            },
            1_000,
        );
        assert_eq!(out2, P2pRxOutcome::Drop);
        assert!(pkts2.is_empty());
        assert_eq!(ctrl.dropped_packets(), 2);
    }

    #[test]
    fn p2p_direct_audio_and_rtcp_kinds_6_and_7_route_to_peer_when_active() {
        let mut ctrl = P2pController::new(true, 100, 1, 2, "sess-1");
        let p = peer_addr();

        // Before P2P active -> routes via Relay (0x03 / 0x05).
        assert!(matches!(
            ctrl.build_outbound_audio(vec![1, 2, 3]),
            P2pOutbound::ToRelay(ZRTPPacket::RelayMediaUp {
                msg_type: MSG_AUDIO_RTP_UP,
                server_token: 2,
                ..
            })
        ));

        // Activate via BindPeer.
        let _ = ctrl.handle_p2p_packet(
            p,
            1,
            100,
            P2pKind::BindPeer,
            P2pBody::BindPeer {
                seq: 1,
                cand_index: 0,
                addr: "198.51.100.1|40000".to_string(),
            },
            1_000,
        );

        // Active -> routes via 0x7f kind=6 (Audio) and kind=7 (AudioRtcp).
        let out_audio = ctrl.build_outbound_audio(vec![0xaa, 0xbb]);
        let P2pOutbound::ToPeer(dst, pkt) = out_audio else {
            panic!("expected ToPeer");
        };
        assert_eq!(dst, p);
        assert_eq!(pkt.encode()[0], MSG_P2P_EXT);

        assert!(matches!(
            ctrl.build_outbound_rtcp(vec![0xcc]),
            P2pOutbound::ToPeer(
                _,
                ZRTPPacket::P2p {
                    kind: P2pKind::AudioRtcp,
                    ..
                }
            )
        ));

        // Incoming kind=6 and kind=7 yield AudioMedia / AudioRtcp.
        let (rx_a, _) = ctrl.handle_p2p_packet(
            p,
            1,
            100,
            P2pKind::Audio,
            P2pBody::Media(vec![9, 8, 7]),
            1_020,
        );
        assert_eq!(rx_a, P2pRxOutcome::AudioMedia(vec![9, 8, 7]));

        let (rx_r, _) = ctrl.handle_p2p_packet(
            p,
            1,
            100,
            P2pKind::AudioRtcp,
            P2pBody::Media(vec![6, 5]),
            1_040,
        );
        assert_eq!(rx_r, P2pRxOutcome::AudioRtcp(vec![6, 5]));
    }

    #[test]
    fn p2p_handover_on_local_ip_change_sends_3x_stop_and_retries_change_address_every_3000ms() {
        let mut ctrl = P2pController::new(true, 100, 1, 55, "sess-handover");
        let p = peer_addr();
        let _ = ctrl.handle_p2p_packet(
            p,
            1,
            100,
            P2pKind::BindPeer,
            P2pBody::BindPeer {
                seq: 1,
                cand_index: 0,
                addr: "198.51.100.1|40000".to_string(),
            },
            1_000,
        );
        assert_eq!(ctrl.active_peer(), Some(p));

        // Baseline IP probe does not trigger handover.
        assert!(ctrl
            .on_local_ip_probe(Ipv4Addr::new(192, 168, 1, 10), 2_000)
            .is_empty());
        assert!(ctrl
            .on_local_ip_probe(Ipv4Addr::new(192, 168, 1, 10), 3_000)
            .is_empty());

        // IP changes -> 3x P2P.SIGNAL(subCmd=2) + 3x REQ_FORWARD(subCmd=2) + 1x CHANGE_ADDRESS(cmd=14).
        let pkts = ctrl.on_local_ip_probe(Ipv4Addr::new(10, 0, 0, 5), 4_000);
        assert_eq!(ctrl.state(), P2pState::Stopped);
        assert!(ctrl.is_changing_address());

        let p2p_stops = pkts
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    P2pOutbound::ToPeer(
                        _,
                        ZRTPPacket::P2p {
                            kind: P2pKind::Signal,
                            body: P2pBody::Signal { sub_cmd: 2, .. },
                            ..
                        }
                    )
                )
            })
            .count();
        let req_fwd_stops = pkts
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    P2pOutbound::ToRelay(ZRTPPacket::Control {
                        cmd: 32,
                        sub_cmd: 2,
                        ..
                    })
                )
            })
            .count();
        let change_addrs = pkts
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    P2pOutbound::ToRelay(ZRTPPacket::Control {
                        cmd: 14,
                        sub_cmd: 1,
                        ..
                    })
                )
            })
            .count();
        assert_eq!(p2p_stops, 3);
        assert_eq!(req_fwd_stops, 3);
        assert_eq!(change_addrs, 1);

        // Before 3000ms -> no retry.
        assert!(ctrl.on_change_address_tick(6_999).is_empty());
        // At >= 3000ms -> retries CHANGE_ADDRESS (cmd=14).
        let retry = ctrl.on_change_address_tick(7_000);
        assert_eq!(retry.len(), 1);
        assert!(matches!(
            retry[0],
            P2pOutbound::ToRelay(ZRTPPacket::Control { cmd: 14, .. })
        ));

        // SERVER_REPLY (cmd=14) clears changing_address flag.
        ctrl.on_server_reply(ZrtpCmd::ChangeAddress as u16, 99);
        assert!(!ctrl.is_changing_address());
        assert!(ctrl.on_change_address_tick(15_000).is_empty());
    }

    #[test]
    fn p2p_auto_teardown_after_25_consecutive_relay_packets_after_5000ms() {
        let mut ctrl = P2pController::new(true, 100, 1, 55, "sess-streak");
        let p = peer_addr();
        let _ = ctrl.handle_p2p_packet(
            p,
            1,
            100,
            P2pKind::BindPeer,
            P2pBody::BindPeer {
                seq: 1,
                cand_index: 0,
                addr: "198.51.100.1|40000".to_string(),
            },
            1_000,
        );
        assert!(matches!(ctrl.state(), P2pState::Active { .. }));

        // Before 5000ms uptime, even 30 relay packets do not stop P2P.
        for _ in 0..30 {
            ctrl.on_relay_audio_received(5_999);
        }
        assert!(matches!(ctrl.state(), P2pState::Active { .. }));

        // A P2P audio packet resets the relay streak to 0.
        let _ = ctrl.handle_p2p_packet(p, 1, 100, P2pKind::Audio, P2pBody::Media(vec![1]), 6_000);

        // 24 consecutive relay packets after >= 5000ms keep P2P Active.
        for _ in 0..24 {
            ctrl.on_relay_audio_received(6_100);
        }
        assert!(matches!(ctrl.state(), P2pState::Active { .. }));

        // 25th consecutive relay packet triggers automatic P2P teardown.
        ctrl.on_relay_audio_received(6_120);
        assert_eq!(ctrl.state(), P2pState::Stopped);
    }

    #[test]
    fn p2p_req_forward_stop_and_restart_control_path() {
        let mut ctrl = P2pController::new(false, 100, 2, 55, "sess-fwd");
        let p = peer_addr();
        let _ = ctrl.add_candidates(vec![p]);
        let _ = ctrl.handle_p2p_packet(
            p,
            0,
            100,
            P2pKind::BindPeer,
            P2pBody::BindPeer {
                seq: 1,
                cand_index: 0,
                addr: "198.51.100.1|40000".to_string(),
            },
            1_000,
        );
        assert!(matches!(ctrl.state(), P2pState::Active { .. }));

        // REQ_FORWARD subCmd=2 stops P2P.
        assert!(ctrl
            .on_req_forward(ReqForwardSubCmd::StopP2p as u8)
            .is_empty());
        assert_eq!(ctrl.state(), P2pState::Stopped);

        // REQ_FORWARD subCmd=3 restarts P2P negotiation by probing known candidates.
        let probes = ctrl.on_req_forward(ReqForwardSubCmd::RestartP2p as u8);
        assert_eq!(ctrl.state(), P2pState::Negotiating);
        assert_eq!(probes.len(), 1);
    }
}
