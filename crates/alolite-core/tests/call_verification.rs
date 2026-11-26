//! Integration and adversarial verification suite for 1-1 ZRTC audio calls (Issue #91).
//!
//! Covers:
//! - `INV-7`: single-call concurrency race, duplicate `401` deduplication, busy rejection.
//! - `INV-8`: `srtpMode: 1` default, `Debug` redaction, and Privacy Guard log verification.
//! - `INV-9`: non-blocking `CoreEngine::session` mutex while call media loop is active.
//! - `ADV-1..ADV-5`: forged/tampered SRTP tags, replay/truncation, malformed ZRTP/P2P `0x7f`,
//!   spoofed `callId`/`senderRole`, malformed JSON signaling, and idempotent teardown.
//! - `EDGE-1..EDGE-5`: multi-frame Opus `carry_pcm`, `NetEq` overflow `next_idx` advance,
//!   FEC hole recovery, SRTP ROC wraparound + backward handover reset, and headless guard.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use alolite_core::common::clock::now_ms;
use alolite_core::core::events::{CallPhase, CoreEvent, CoreEventSink};
use alolite_core::core::CoreEngine;
use alolite_core::protocol::call::CallSignalWireData;
use alolite_core::protocol::messaging::{CallOutcome, MessageKind};
use alolite_core::protocol::session::Session;
use alolite_core::storage::MessageStore;
use serde_json::json;

struct CollectSink(Arc<StdMutex<Vec<CoreEvent>>>);

impl CoreEventSink for CollectSink {
    fn emit(&self, event: CoreEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn make_signal(cmd: u16, call_id: u32, caller_id: &str) -> CallSignalWireData {
    CallSignalWireData::from_value(&json!({
        "cmd": cmd,
        "callId": call_id,
        "sessId": format!("sess-{call_id}"),
        "callerId": caller_id,
        "calleeId": "self-1",
        "isVideo": 0,
        "extendData": "{\"srtpMode\":1,\"maxFT\":60}"
    }))
}

#[tokio::test]
async fn concurrent_incoming_401_invites_enforce_single_call_invariant() {
    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_call_timeouts(
        session,
        ":memory:",
        sink,
        Duration::from_secs(60),
        Duration::from_secs(15),
        Duration::from_millis(50),
    );
    let s_snap = engine.session_arc().lock().await.clone();

    // INV-7 / ADV-2: race 8 concurrent incoming 401 invites with distinct callIds.
    let mut handles = Vec::new();
    for i in 0..8u32 {
        let eng = Arc::clone(&engine);
        let sess = s_snap.clone();
        handles.push(tokio::spawn(async move {
            let cid = 10_000 + i;
            eng.handle_voip_signal(&sess, make_signal(401, cid, &format!("peer-{i}")))
                .await;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let snap = engine.call_snapshot();
    assert_eq!(snap.phase, CallPhase::IncomingRinging); // (INV-7)
    let winning_id = snap.call_id.expect("exactly one call must win");
    assert!((10_000..10_008).contains(&winning_id));

    let ringing_events: Vec<u32> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            CoreEvent::CallStateChanged {
                phase: CallPhase::IncomingRinging,
                call_id: Some(cid),
                ..
            } => Some(*cid),
            _ => None,
        })
        .collect();
    assert_eq!(ringing_events, vec![winning_id]); // (INV-7)

    engine.end_call().await.unwrap();
    assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
}

#[tokio::test]
async fn duplicate_401_delivery_deduplicates_without_resetting_ring_state() {
    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_db_path(session, ":memory:", sink);
    let s_snap = engine.session_arc().lock().await.clone();

    // ADV-1: WS and long-poll deliver the exact same 401 invite twice.
    engine
        .handle_voip_signal(&s_snap, make_signal(401, 20_001, "peer-dup"))
        .await;
    engine
        .handle_voip_signal(&s_snap, make_signal(401, 20_001, "peer-dup"))
        .await;

    let snap = engine.call_snapshot();
    assert_eq!(snap.phase, CallPhase::IncomingRinging);
    assert_eq!(snap.call_id, Some(20_001));

    let ring_count = events
        .lock()
        .unwrap()
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                CoreEvent::CallStateChanged {
                    phase: CallPhase::IncomingRinging,
                    call_id: Some(20_001),
                    ..
                }
            )
        })
        .count();
    assert_eq!(ring_count, 1); // (ADV-1)

    engine.end_call().await.unwrap();
}

#[tokio::test]
async fn start_call_while_call_active_is_rejected_by_inv7() {
    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_db_path(session, ":memory:", sink);
    let s_snap = engine.session_arc().lock().await.clone();

    engine
        .handle_voip_signal(&s_snap, make_signal(401, 30_001, "peer-1"))
        .await;
    assert_eq!(engine.call_snapshot().phase, CallPhase::IncomingRinging);

    // INV-7: starting an outgoing call while IncomingRinging must fail before touching network.
    let err = engine
        .start_call("peer-2".to_string(), None, None)
        .await
        .expect_err("second call must be rejected");
    assert!(!err.to_string().is_empty());
    assert_eq!(engine.call_snapshot().call_id, Some(30_001)); // (INV-7)

    engine.end_call().await.unwrap();
}

#[tokio::test]
async fn simultaneous_local_end_and_peer_409_teardown_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("alolite_call_adv4_{}", now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("history.db");
    let db_str = db_path.to_str().unwrap().to_string();

    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_call_timeouts(
        session,
        &db_str,
        sink,
        Duration::from_secs(60),
        Duration::from_secs(15),
        Duration::from_millis(50),
    );
    let s_snap = engine.session_arc().lock().await.clone();

    engine
        .handle_voip_signal(&s_snap, make_signal(401, 40_001, "peer-adv4"))
        .await;
    engine.answer_call(true).await.unwrap();

    // ADV-4: race local end_call() against incoming peer 409 end signal.
    let eng_a = Arc::clone(&engine);
    let eng_b = Arc::clone(&engine);
    let sess_b = s_snap.clone();
    let (res_a, _) = tokio::join!(
        eng_a.end_call(),
        eng_b.handle_voip_signal(&sess_b, make_signal(409, 40_001, "peer-adv4"))
    );
    res_a.unwrap();
    assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);

    // History persistence must record at most one call entry for call_id 40_001.
    let history_events = events
        .lock()
        .unwrap()
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                CoreEvent::CallHistoryRecorded { msg, .. }
                    if msg.id == "call_40001"
                        && msg.kind == MessageKind::Call
                        && matches!(msg.call_outcome, Some(CallOutcome::Completed | CallOutcome::Missed))
            )
        })
        .count();
    assert_eq!(history_events, 1);

    let store = MessageStore::open(&db_str).unwrap();
    let saved = store.messages_for("peer-adv4", 10).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].msg_id, "call_40001");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn active_call_media_loop_never_blocks_session_mutex() {
    use alolite_core::protocol::call::{P2pKind, ZRTPPacket};
    use tokio::net::UdpSocket;

    let peer_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer_addr = peer_sock.local_addr().unwrap();

    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_call_timeouts(
        session,
        ":memory:",
        sink,
        Duration::from_secs(60),
        Duration::from_secs(15),
        Duration::from_millis(20),
    );
    let s_snap = engine.session_arc().lock().await.clone();

    let sig = CallSignalWireData::from_value(&json!({
        "cmd": 401,
        "callId": 50_001,
        "sessId": "sess-50001",
        "callerId": "peer-inv9",
        "calleeId": "self-1",
        "isVideo": 0,
        "extendData": {
            "srtpMode": 1,
            "maxFT": 60,
            "extraInfo": { "p2pAddr": format!("{}|{}", peer_addr.ip(), peer_addr.port()) }
        }
    }));
    engine.handle_voip_signal(&s_snap, sig).await;
    engine.answer_call(true).await.unwrap();

    // INV-9: hold CoreEngine::session mutex and verify call_snapshot, set_call_devices,
    // and the active UDP media loop continue without blocking.
    let _session_guard = engine.session_arc().lock().await;
    let snap = tokio::time::timeout(Duration::from_millis(500), async { engine.call_snapshot() })
        .await
        .expect("call_snapshot must not block on CoreEngine::session");
    assert_eq!(snap.call_id, Some(50_001));

    tokio::time::timeout(
        Duration::from_millis(500),
        engine.set_call_devices(Some("mic-1".to_string()), Some("spk-1".to_string())),
    )
    .await
    .expect("set_call_devices must not block on CoreEngine::session")
    .unwrap();

    let mut buf = [0u8; 256];
    let (n, engine_addr) =
        tokio::time::timeout(Duration::from_secs(2), peer_sock.recv_from(&mut buf))
            .await
            .expect("media loop must send P2P.BIND while session mutex is held")
            .unwrap();
    let pkt = ZRTPPacket::decode(&buf[..n]).unwrap();
    assert!(matches!(
        pkt,
        ZRTPPacket::P2p {
            call_id: 50_001,
            kind: P2pKind::Bind,
            ..
        }
    ));

    let bind_peer = ZRTPPacket::p2p_bind_peer(
        0,
        50_001,
        1,
        0,
        format!("{}|{}", engine_addr.ip(), engine_addr.port()),
    );
    peer_sock
        .send_to(&bind_peer.encode(), engine_addr)
        .await
        .unwrap();

    let (n2, _) = tokio::time::timeout(Duration::from_secs(2), peer_sock.recv_from(&mut buf))
        .await
        .expect("media loop must reply P2P.BIND_ANSWER while session mutex is held")
        .unwrap();
    let ans = ZRTPPacket::decode(&buf[..n2]).unwrap();
    assert!(matches!(
        ans,
        ZRTPPacket::P2p {
            call_id: 50_001,
            kind: P2pKind::BindAnswer,
            ..
        }
    ));

    drop(_session_guard);
    engine.end_call().await.unwrap();
    assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
}

#[tokio::test]
async fn headless_mode_ignores_incoming_401_and_rejects_answer() {
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::headless(session, ":memory:");
    let s_snap = engine.session_arc().lock().await.clone();

    // EDGE-5: headless mode must ignore incoming 401 and stay Idle.
    engine
        .handle_voip_signal(&s_snap, make_signal(401, 60_001, "peer-headless"))
        .await;
    assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
    assert_eq!(engine.call_snapshot().call_id, None);
    assert!(engine.answer_call(true).await.is_err());
}

fn build_rtp_packet(seq: u16, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = vec![0u8; 12 + payload.len()];
    pkt[0] = 0x80; // V=2, P=0, X=0, CC=0
    pkt[1] = 111; // PT=111 (Opus)
    pkt[2..4].copy_from_slice(&seq.to_be_bytes());
    pkt[4..8].copy_from_slice(&(u32::from(seq) * 320).to_be_bytes());
    pkt[8..12].copy_from_slice(&ssrc.to_be_bytes());
    pkt[12..].copy_from_slice(payload);
    pkt
}

#[test]
fn srtp_rejects_forged_tag_and_payload_tampering_without_mutating_roc() {
    use alolite_core::protocol::call::SrtpContext;

    let key = [0x11u8; 16];
    let salt = [0x22u8; 14];
    let mut tx = SrtpContext::new(key, salt);
    let mut rx = SrtpContext::new(key, salt);

    let rtp_ok = build_rtp_packet(42, 0xdead_beef, &[0xaa, 0xbb, 0xcc, 0xdd]);
    let srtp_ok = tx.protect_rtp(&rtp_ok).unwrap();
    assert_eq!(rx.unprotect_rtp(&srtp_ok).unwrap(), rtp_ok);
    assert_eq!(rx.rx_roc().last_seq(), Some(42));

    // ADV-3: forged auth tag must fail before mutating rx_roc.
    let rtp_43 = build_rtp_packet(43, 0xdead_beef, &[0x01, 0x02, 0x03, 0x04]);
    let srtp_43 = tx.protect_rtp(&rtp_43).unwrap();

    let mut forged_tag = srtp_43.clone();
    let last = forged_tag.len() - 1;
    forged_tag[last] ^= 0x01;
    assert!(rx.unprotect_rtp(&forged_tag).is_err());
    assert_eq!(rx.rx_roc().last_seq(), Some(42));

    // ADV-3: tampered encrypted payload must also fail HMAC-SHA1-80 without mutating rx_roc.
    let mut tampered_payload = srtp_43;
    tampered_payload[12] ^= 0x80;
    assert!(rx.unprotect_rtp(&tampered_payload).is_err());
    assert_eq!(rx.rx_roc().last_seq(), Some(42));
}

#[test]
fn srtp_rejects_truncated_and_wrong_key_packets() {
    use alolite_core::protocol::call::SrtpContext;

    let mut tx_other = SrtpContext::new([0x99u8; 16], [0x88u8; 14]);
    let mut rx = SrtpContext::new([0x11u8; 16], [0x22u8; 14]);

    // Truncated packets (< 12 + 10 bytes).
    assert!(rx.unprotect_rtp(&[]).is_err());
    assert!(rx.unprotect_rtp(&[0x80; 21]).is_err());

    // Malformed RTP header claiming CC=15 (72-byte header) inside a 24-byte buffer.
    let mut bad_cc = vec![0u8; 24];
    bad_cc[0] = 0x8f;
    assert!(rx.unprotect_rtp(&bad_cc).is_err());

    // Packet encrypted/authenticated with wrong key.
    let rtp = build_rtp_packet(1, 0x1234_5678, &[0xde, 0xad]);
    let wrong_srtp = tx_other.protect_rtp(&rtp).unwrap();
    assert!(rx.unprotect_rtp(&wrong_srtp).is_err());
    assert_eq!(rx.rx_roc().last_seq(), None);
}

#[test]
fn srtp_handles_roc_wraparound_at_65535_and_backward_handover_reset() {
    use alolite_core::protocol::call::SrtpContext;

    let key = [0x33u8; 16];
    let salt = [0x44u8; 14];
    let mut tx = SrtpContext::new(key, salt);
    let mut rx = SrtpContext::new(key, salt);

    // EDGE-4: sequence wraparound 65534 -> 65535 -> 0 -> 1 increments ROC from 0 to 1.
    for seq in [65_534u16, 65_535, 0, 1] {
        let rtp = build_rtp_packet(seq, 0x5566_7788, &[0x01, 0x02, 0x03]);
        let srtp = tx.protect_rtp(&rtp).unwrap();
        let dec = rx.unprotect_rtp(&srtp).unwrap();
        assert_eq!(dec, rtp);
    }
    assert_eq!(tx.tx_roc().roc(), 1);
    assert_eq!(rx.rx_roc().roc(), 1);
    assert_eq!(rx.rx_roc().last_seq(), Some(1));

    // EDGE-4: backward reset on handover (fresh sender at ROC=0, seq=100).
    let mut tx_handover = SrtpContext::new(key, salt);
    let rtp_reset = build_rtp_packet(100, 0x5566_7788, &[0xfe, 0xed]);
    let srtp_reset = tx_handover.protect_rtp(&rtp_reset).unwrap();
    let dec_reset = rx.unprotect_rtp(&srtp_reset).unwrap();
    assert_eq!(dec_reset, rtp_reset);
    assert_eq!(rx.rx_roc().roc(), 0);
    assert_eq!(rx.rx_roc().last_seq(), Some(100));
}

#[test]
fn zrtp_decode_rejects_truncated_oversized_and_malformed_packets() {
    use alolite_core::protocol::call::ZRTPPacket;

    // Empty and 1-byte inputs.
    assert!(ZRTPPacket::decode(&[]).is_err());
    assert!(ZRTPPacket::decode(&[0x01]).is_err());
    assert!(ZRTPPacket::decode(&[0x7f]).is_err());

    // Truncated 0x01 Control (< 21 bytes) and 0x02 ServerReply (< 21 bytes).
    assert!(ZRTPPacket::decode(&[0x01; 20]).is_err());
    assert!(ZRTPPacket::decode(&[0x02; 20]).is_err());

    // Truncated 0x7f P2P (< 9 bytes) and truncated Bind body.
    assert!(ZRTPPacket::decode(&[0x7f; 8]).is_err());
    assert!(ZRTPPacket::decode(&[0x7f, 0, 1, 0, 0, 0, 0, 1, 0]).is_err());

    // Unknown outer command and unknown 0x7f kind.
    assert!(ZRTPPacket::decode(&[0xfe; 32]).is_err());
    let mut unknown_p2p_kind = vec![0u8; 16];
    unknown_p2p_kind[0] = 0x7f;
    unknown_p2p_kind[7..9].copy_from_slice(&999u16.to_le_bytes());
    assert!(ZRTPPacket::decode(&unknown_p2p_kind).is_err());

    // Oversized 64 KiB payload for REQ_FORWARD subCmd=10 (expects 24 bytes) and Ping with
    // out-of-bounds string length must fail cleanly without panic.
    let mut oversized_heartbeat = vec![0x01u8; 65_536];
    oversized_heartbeat[18..20].copy_from_slice(&32u16.to_le_bytes()); // ZrtpCmd::ReqForward
    oversized_heartbeat[20] = 10; // ReqForwardSubCmd::LinkHeartbeat
    assert!(ZRTPPacket::decode(&oversized_heartbeat).is_err());

    let mut bad_ping_len = vec![0x01u8; 32];
    bad_ping_len[18..20].copy_from_slice(&2u16.to_le_bytes()); // ZrtpCmd::Ping
    bad_ping_len[21..23].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(ZRTPPacket::decode(&bad_ping_len).is_err());

    assert!(ZRTPPacket::decode(&vec![0xfe; 65_536]).is_err());
}

#[test]
fn p2p_controller_drops_spoofed_call_id_and_own_sender_role() {
    use alolite_core::protocol::call::zrtp::P2pBody;
    use alolite_core::protocol::call::{P2pController, P2pKind, P2pRxOutcome, P2pState};

    let mut p2p = P2pController::new(true, 777, 1001, 55, "sess-777");
    let peer: std::net::SocketAddr = "127.0.0.1:45000".parse().unwrap();

    // ADV-3: spoofed call_id (888 != 777) must be dropped before state mutation.
    let (outcome, out) = p2p.handle_p2p_packet(
        peer,
        1,
        888,
        P2pKind::Bind,
        P2pBody::Bind {
            seq: 1,
            cand_index: 0,
        },
        100,
    );
    assert_eq!(outcome, P2pRxOutcome::Drop);
    assert!(out.is_empty());

    // ADV-3: own sender_role (0 == caller own_role) must be dropped.
    let (outcome2, out2) = p2p.handle_p2p_packet(
        peer,
        0,
        777,
        P2pKind::Audio,
        P2pBody::Media(vec![0x80; 24]),
        110,
    );
    assert_eq!(outcome2, P2pRxOutcome::Drop);
    assert!(out2.is_empty());

    assert_eq!(p2p.dropped_packets(), 2);
    assert_eq!(p2p.state(), P2pState::Idle);
    assert_eq!(p2p.active_peer(), None);
}

#[tokio::test]
async fn media_loop_drops_malformed_udp_and_invalid_srtp_without_mutating_call_state() {
    use alolite_core::protocol::call::{P2pKind, SrtpContext, ZRTPPacket};
    use base64::Engine as _;
    use tokio::net::UdpSocket;

    let peer_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer_addr = peer_sock.local_addr().unwrap();
    let master_raw = [0x5au8; 30];
    let master_b64 = base64::engine::general_purpose::STANDARD.encode(master_raw);

    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_call_timeouts(
        session,
        ":memory:",
        sink,
        Duration::from_secs(60),
        Duration::from_millis(800),
        Duration::from_millis(20),
    );
    let s_snap = engine.session_arc().lock().await.clone();

    let sig = CallSignalWireData::from_value(&json!({
        "cmd": 401,
        "callId": 70_001,
        "sessId": "sess-70001",
        "callerId": "peer-adv3",
        "calleeId": "self-1",
        "isVideo": 0,
        "srtpKey": master_b64,
        "extendData": {
            "srtpMode": 1,
            "maxFT": 60,
            "extraInfo": { "p2pAddr": format!("{}|{}", peer_addr.ip(), peer_addr.port()) }
        }
    }));
    engine.handle_voip_signal(&s_snap, sig).await;
    engine.answer_call(true).await.unwrap();

    let mut buf = [0u8; 256];
    let (n, engine_addr) =
        tokio::time::timeout(Duration::from_secs(2), peer_sock.recv_from(&mut buf))
            .await
            .expect("timed out waiting for P2P.BIND")
            .unwrap();
    assert!(matches!(
        ZRTPPacket::decode(&buf[..n]).unwrap(),
        ZRTPPacket::P2p {
            kind: P2pKind::Bind,
            ..
        }
    ));

    // ADV-3: send malformed UDP garbage, spoofed call_id 0x7f, own-role 0x7f, and tampered SRTP.
    peer_sock
        .send_to(&[0xff, 0x00, 0x13, 0x37], engine_addr)
        .await
        .unwrap();
    peer_sock
        .send_to(
            &ZRTPPacket::p2p_bind(0, 999_999, 1, 0).encode(),
            engine_addr,
        )
        .await
        .unwrap();
    peer_sock
        .send_to(&ZRTPPacket::p2p_bind(1, 70_001, 1, 0).encode(), engine_addr)
        .await
        .unwrap();

    let mut wrong_srtp = SrtpContext::new([0x01u8; 16], [0x02u8; 14]);
    let forged_audio = wrong_srtp
        .protect_rtp(&build_rtp_packet(1, 0x1111, &[0xf8, 0xff, 0xfe]))
        .unwrap();
    peer_sock
        .send_to(
            &ZRTPPacket::p2p_audio(0, 70_001, forged_audio).encode(),
            engine_addr,
        )
        .await
        .unwrap();

    // Valid BindPeer still succeeds and call remains alive.
    let bind_peer = ZRTPPacket::p2p_bind_peer(
        0,
        70_001,
        1,
        0,
        format!("{}|{}", engine_addr.ip(), engine_addr.port()),
    );
    peer_sock
        .send_to(&bind_peer.encode(), engine_addr)
        .await
        .unwrap();
    let (n2, _) = tokio::time::timeout(Duration::from_secs(2), peer_sock.recv_from(&mut buf))
        .await
        .expect("valid BindPeer must still receive BindAnswer after adversarial packets")
        .unwrap();
    assert!(matches!(
        ZRTPPacket::decode(&buf[..n2]).unwrap(),
        ZRTPPacket::P2p {
            kind: P2pKind::BindAnswer,
            ..
        }
    ));

    engine.end_call().await.unwrap();
    assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
}

#[tokio::test]
async fn malformed_json_signaling_and_extend_data_fail_safely_without_panic() {
    use alolite_core::protocol::call::ExtendData;

    // ADV-5: malformed JSON / wrong field types must not panic.
    let empty = CallSignalWireData::from_value(&json!({}));
    assert_eq!(empty.call_id_u32(), None);
    assert_eq!(empty.parsed_extend_data(), ExtendData::default());
    assert_eq!(empty.parsed_extend_data().srtp_mode, 1);

    let bad_types = CallSignalWireData::from_value(&json!({
        "cmd": null,
        "callId": "not-a-number",
        "callerId": 12345,
        "extendData": "{malformed-json"
    }));
    assert_eq!(bad_types.call_id_u32(), None);
    assert_eq!(bad_types.parsed_extend_data().srtp_mode, 1); // (INV-8)

    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::headless(session, ":memory:");
    let s_snap = engine.session_arc().lock().await.clone();

    let malformed_401 = CallSignalWireData::from_value(&json!({
        "cmd": 401,
        "callId": "invalid",
        "extendData": "{broken"
    }));
    engine.handle_voip_signal(&s_snap, malformed_401).await;
    assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
}

#[test]
fn debug_output_redacts_srtp_keys_salts_relay_and_peer_ips() {
    use alolite_core::protocol::call::zrtp::P2pBody;
    use alolite_core::protocol::call::{P2pController, P2pKind, RelayServerAddr, SrtpContext};
    use base64::Engine as _;

    let master_raw = [0xabu8; 30];
    let master_b64 = base64::engine::general_purpose::STANDARD.encode(master_raw);

    let sig = CallSignalWireData::from_value(&json!({
        "cmd": 401,
        "callId": 80_001,
        "sessId": "sess-80001",
        "callerId": "peer-priv",
        "calleeId": "self-1",
        "rtpIp": "203.0.113.42",
        "srtpKey": master_b64,
        "extendData": {
            "srtpMode": 1,
            "relayServers": [{ "ip": "198.51.100.7", "port": 3478 }],
            "extraInfo": { "p2pAddr": "203.0.113.42|54321" }
        }
    }));

    // INV-8: CallSignalWireData Debug must redact rtp_ip and srtp_key.
    let sig_dbg = format!("{sig:?}");
    assert!(!sig_dbg.contains("203.0.113.42"));
    assert!(!sig_dbg.contains(&master_b64));

    // INV-8: SrtpKeyMaterial Debug must redact key and salt.
    let key_mat = sig.srtp_key_material().expect("valid 30-byte srtpKey");
    let mat_dbg = format!("{key_mat:?}");
    assert!(mat_dbg.contains("REDACTED"));
    assert!(!mat_dbg.contains("171")); // 0xab decimal
    assert!(!mat_dbg.contains("ab, ab"));

    // INV-8: SrtpContext Debug must redact derived keys.
    let srtp_ctx = SrtpContext::from_combined(key_mat.as_bytes()).unwrap();
    let ctx_dbg = format!("{srtp_ctx:?}");
    assert!(ctx_dbg.contains("REDACTED"));
    assert!(!ctx_dbg.contains("171"));

    // INV-8: RelayServerAddr Debug must redact relay IP.
    let relay = RelayServerAddr {
        ip: "198.51.100.7".to_string(),
        port: 3478,
        server_type: 1,
    };
    let relay_dbg = format!("{relay:?}");
    assert!(!relay_dbg.contains("198.51.100.7"));
    assert!(relay_dbg.contains("<redacted-ip>"));

    // INV-8: P2pController Debug must redact peer IP even after candidate/active transition.
    let mut p2p = P2pController::new(true, 80_001, 100, 1, "sess-80001");
    let peer_addr: std::net::SocketAddr = "203.0.113.42:54321".parse().unwrap();
    let _ = p2p.add_candidates(vec![peer_addr]);
    let _ = p2p.handle_p2p_packet(
        peer_addr,
        1,
        80_001,
        P2pKind::BindAnswer,
        P2pBody::Bind {
            seq: 1,
            cand_index: 0,
        },
        100,
    );
    let p2p_dbg = format!("{p2p:?}");
    assert!(!p2p_dbg.contains("203.0.113.42"));
}

#[tokio::test]
async fn call_lifecycle_logs_contain_zero_raw_srtp_keys_or_unredacted_ips() {
    use alolite_core::common::logger::{self, FindingLevel};
    use base64::Engine as _;

    logger::clear_entries();
    let master_b64 = base64::engine::general_purpose::STANDARD.encode([0xcd; 30]);

    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::new(CollectSink(Arc::clone(&events)));
    let session = Session::new(false, false, false).unwrap();
    let engine = CoreEngine::with_call_timeouts(
        session,
        ":memory:",
        sink,
        Duration::from_secs(60),
        Duration::from_secs(15),
        Duration::from_millis(20),
    );
    let s_snap = engine.session_arc().lock().await.clone();

    let sig = CallSignalWireData::from_value(&json!({
        "cmd": 401,
        "callId": 80_002,
        "sessId": "sess-80002",
        "callerId": "peer-log-check",
        "calleeId": "self-1",
        "rtpIp": "203.0.113.99",
        "srtpKey": master_b64,
        "extendData": {
            "srtpMode": 1,
            "maxFT": 60,
            "extraInfo": { "p2pAddr": "203.0.113.99|40000" }
        }
    }));
    engine.handle_voip_signal(&s_snap, sig).await;
    engine.answer_call(true).await.unwrap();
    engine.end_call().await.unwrap();

    // INV-8: neither logger entries nor scan_logs may expose raw SRTP keys or peer IPs.
    for entry in logger::get_entries() {
        let joined = serde_json::to_string(&entry).unwrap();
        assert!(
            !joined.contains(&master_b64),
            "raw srtpKey leaked into logger"
        );
        assert!(
            !joined.contains("203.0.113.99"),
            "raw peer IP leaked into logger"
        );
    }
    let findings = logger::scan_logs();
    assert!(
        !findings
            .iter()
            .any(|f| matches!(f.level, FindingLevel::Critical)),
        "unexpected critical findings in logger::scan_logs(): {findings:?}"
    );
}

struct MockOpusDecoder;

impl alolite_core::services::call_media::OpusDecoderBackend for MockOpusDecoder {
    fn decode(
        &mut self,
        packet: &[u8],
        out: &mut [i16],
        decode_fec: bool,
    ) -> anyhow::Result<usize> {
        if decode_fec {
            out[..320].fill(77);
            return Ok(320);
        }
        // Marker byte `0x60` encodes a 60 ms (960-sample) multi-frame packet; otherwise 20 ms (320).
        if packet.first() == Some(&0x60) {
            for (i, sample) in out[..960].iter_mut().enumerate() {
                *sample = (i as i16).wrapping_add(1);
            }
            return Ok(960);
        }
        let tag = i16::from(packet.first().copied().unwrap_or(1));
        out[..320].fill(tag);
        Ok(320)
    }

    fn decode_plc(&mut self, out: &mut [i16], frame_samples: usize) -> anyhow::Result<usize> {
        let n = frame_samples.min(out.len());
        out[..n].fill(-1);
        Ok(n)
    }
}

#[test]
fn neteq_drains_60ms_multiframe_opus_into_three_20ms_ticks() {
    use alolite_core::services::call_media::{NetEq, NetEqConfig, PopOutcome, FRAME_SAMPLES};

    let mut neteq = NetEq::new(
        MockOpusDecoder,
        NetEqConfig {
            max_depth: 10,
            min_delay_packets: 1,
        },
    );
    neteq.push(1, vec![0x60]);

    let mut out = [0i16; FRAME_SAMPLES];
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 960 }
    ); // (EDGE-1)
    assert_eq!(out[0], 1);
    assert_eq!(neteq.carry_samples(), 640);

    assert_eq!(neteq.pop_frame(&mut out).unwrap(), PopOutcome::Carry); // (EDGE-1)
    assert_eq!(out[0], 321);
    assert_eq!(neteq.carry_samples(), 320);

    assert_eq!(neteq.pop_frame(&mut out).unwrap(), PopOutcome::Carry); // (EDGE-1)
    assert_eq!(out[0], 641);
    assert_eq!(neteq.carry_samples(), 0);
}

#[test]
fn neteq_overflow_prunes_oldest_and_advances_next_idx_without_plc_starvation() {
    use alolite_core::services::call_media::{NetEq, NetEqConfig, PopOutcome, FRAME_SAMPLES};

    let mut neteq = NetEq::new(
        MockOpusDecoder,
        NetEqConfig {
            max_depth: 5,
            min_delay_packets: 1,
        },
    );
    neteq.push(1, vec![1]);
    let initial_idx = neteq.next_idx().expect("initial next_idx");
    for seq in 2..=15u16 {
        neteq.push(seq, vec![seq as u8]);
    }
    assert_eq!(neteq.queued_packets(), 5);
    assert_eq!(neteq.next_idx(), Some(initial_idx + 10)); // (EDGE-2)

    let mut out = [0i16; FRAME_SAMPLES];
    for expected in 11..=15i16 {
        assert_eq!(
            neteq.pop_frame(&mut out).unwrap(),
            PopOutcome::Decoded { samples: 320 }
        );
        assert_eq!(out[0], expected);
    }
}

#[test]
fn neteq_recovers_single_packet_hole_via_fec_from_next_packet() {
    use alolite_core::services::call_media::{NetEq, NetEqConfig, PopOutcome, FRAME_SAMPLES};

    let mut neteq = NetEq::new(
        MockOpusDecoder,
        NetEqConfig {
            max_depth: 10,
            min_delay_packets: 1,
        },
    );
    neteq.push(1, vec![1]);
    neteq.push(2, vec![2]);
    // Packet 3 missing; packet 4 present for in-band FEC recovery.
    neteq.push(4, vec![4]);

    let mut out = [0i16; FRAME_SAMPLES];
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 320 }
    );
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 320 }
    );
    assert_eq!(neteq.pop_frame(&mut out).unwrap(), PopOutcome::FecRecovered); // (EDGE-3)
    assert_eq!(out[0], 77);
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 320 }
    );
    assert_eq!(out[0], 4);
}

#[test]
fn neteq_drops_replayed_late_rtp_packets_after_playout() {
    use alolite_core::services::call_media::{NetEq, NetEqConfig, PopOutcome, FRAME_SAMPLES};

    let mut neteq = NetEq::new(
        MockOpusDecoder,
        NetEqConfig {
            max_depth: 10,
            min_delay_packets: 1,
        },
    );
    neteq.push(1, vec![1]);
    neteq.push(2, vec![2]);

    let mut out = [0i16; FRAME_SAMPLES];
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 320 }
    );
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 320 }
    );
    assert_eq!(neteq.queued_packets(), 0);

    // Replayed seq 1 and seq 2 after playout has moved past them must be dropped.
    neteq.push(1, vec![99]);
    neteq.push(2, vec![99]);
    assert_eq!(neteq.queued_packets(), 0);
}

#[cfg(feature = "call")]
#[test]
fn libopus_60ms_multiframe_roundtrip_through_neteq() {
    use alolite_core::services::call_media::{
        LibOpusDecoder, LibOpusEncoder, NetEq, NetEqConfig, OpusEncoderBackend, PopOutcome,
        FRAME_SAMPLES,
    };

    let mut enc = LibOpusEncoder::new(24_000).expect("libopus encoder");
    let dec = LibOpusDecoder::new().expect("libopus decoder");
    let mut neteq = NetEq::new(
        dec,
        NetEqConfig {
            max_depth: 10,
            min_delay_packets: 1,
        },
    );

    // 60 ms = 960 samples at 16 kHz mono (EDGE-1).
    let pcm_60ms = vec![0i16; 960];
    let mut opus_buf = [0u8; 4000];
    let n = enc.encode(&pcm_60ms, &mut opus_buf).unwrap();
    neteq.push(1, opus_buf[..n].to_vec());

    let mut out = [0i16; FRAME_SAMPLES];
    assert_eq!(
        neteq.pop_frame(&mut out).unwrap(),
        PopOutcome::Decoded { samples: 960 }
    );
    assert_eq!(neteq.pop_frame(&mut out).unwrap(), PopOutcome::Carry);
    assert_eq!(neteq.pop_frame(&mut out).unwrap(), PopOutcome::Carry);
    assert_eq!(neteq.carry_samples(), 0);
}
