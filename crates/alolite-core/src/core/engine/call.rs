//! 1-1 ZRTC audio call coordinator (`CallManager`) for [`CoreEngine`].
//!
//! Enforces `INV-7` (single active `CallSession`), `INV-8` (SRTP + redacted logs),
//! `INV-9` (`CallManager` lives outside `CoreEngine::session` mutex), and `EDGE-5`
//! (headless mode ignores `401` invites and never opens local audio devices).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::CoreEngine;
use crate::clock::{now_ms, unix_now};
use crate::core::events::{CallPhase, CoreEvent};
use crate::protocol::call::p2p::{P2pController, P2pOutbound, P2pRxOutcome};
use crate::protocol::call::signaling::{
    answer_call, cancel_call, end_call as send_end_call_http, hold_request, hold_response,
    log_end_call, request_call, send_answer_ack, send_conf, send_request, send_ring_ring,
    unhold_request, unhold_response, CallSignalCode, CallSignalWireData, ExtendData,
    RelayServerAddr, SrtpKeyMaterial,
};
use crate::protocol::call::srtp::SrtpContext;
use crate::protocol::call::zrtp::{
    ControlBody, ReqForwardSubCmd, ZRTPPacket, ZrtpCmd, MSG_AUDIO_RTP_DOWN,
};
use crate::protocol::call::{
    CallDirection, CallEffect, CallEndReason, CallEvent, CallSession, CallState,
};
use crate::protocol::messaging::{CallOutcome, ChatMessage, MessageKind};
use crate::protocol::session::Session;
use crate::services::call_media::{
    normalize_device_id, NetEq, NetEqConfig, NullAudioBackend, NullOpusDecoder, NullOpusEncoder,
    OpusDecoderBackend, OpusEncoderBackend, FRAME_SAMPLES,
};
use crate::storage::{MessageStore, StoredMessage};

/// Default ring timeout before unanswered incoming/outgoing calls auto-cancel (`60 s`).
pub const DEFAULT_RING_TIMEOUT: Duration = Duration::from_secs(60);

/// Default active media watchdog timeout when zero UDP packets arrive (`15 s`).
pub const DEFAULT_MEDIA_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(15);

/// Default ZRTP Echo probe timeout (`2000 ms`).
pub const DEFAULT_ECHO_PROBE_TIMEOUT: Duration = Duration::from_millis(2000);

/// Snapshot of the current 1-1 call state for IPC and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSnapshot {
    pub phase: CallPhase,
    pub call_id: Option<u32>,
    pub peer_id: Option<String>,
    pub peer_name: Option<String>,
    pub peer_avatar: Option<String>,
    pub is_incoming: bool,
    pub is_muted: bool,
    pub is_held: bool,
    pub started_at_ms: Option<i64>,
    pub end_reason: Option<String>,
}

#[derive(Debug)]
enum MediaLoopCmd {
    Stop {
        send_end_call_pkt: bool,
    },
    SetMuted(bool),
    SetHeld(bool),
    SetDevices {
        mic_id: Option<String>,
        speaker_id: Option<String>,
    },
}

#[derive(Default)]
struct CallManagerState {
    session: CallSession,
    peer_name: Option<String>,
    peer_avatar: Option<String>,
    relay_servers: Vec<RelayServerAddr>,
    p2p_candidates: Vec<SocketAddr>,
    srtp_key: Option<SrtpKeyMaterial>,
    started_at_ms: Option<i64>,
    ring_timer_task: Option<tokio::task::JoinHandle<()>>,
    media_task: Option<tokio::task::JoinHandle<()>>,
    media_cmd_tx: Option<mpsc::UnboundedSender<MediaLoopCmd>>,
    selected_mic_id: Option<String>,
    selected_speaker_id: Option<String>,
    audio_device_opened_count: usize,
}

/// Coordinates the single active 1-1 ZRTC [`CallSession`] outside `CoreEngine::session` (`INV-9`).
pub struct CallManager {
    state: StdMutex<CallManagerState>,
    ring_timeout: Duration,
    media_watchdog_timeout: Duration,
    echo_probe_timeout: Duration,
    force_null_audio: bool,
}

impl Default for CallManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CallManager {
    /// Create a [`CallManager`] with production ZRTC timeouts (`60s` ring, `15s` watchdog, `2000ms` echo).
    pub fn new() -> Self {
        Self {
            state: StdMutex::new(CallManagerState::default()),
            ring_timeout: DEFAULT_RING_TIMEOUT,
            media_watchdog_timeout: DEFAULT_MEDIA_WATCHDOG_TIMEOUT,
            echo_probe_timeout: DEFAULT_ECHO_PROBE_TIMEOUT,
            force_null_audio: false,
        }
    }

    /// Create a [`CallManager`] with custom timeouts and null audio (used by unit/integration tests).
    pub fn with_timeouts(
        ring_timeout: Duration,
        media_watchdog_timeout: Duration,
        echo_probe_timeout: Duration,
    ) -> Self {
        Self {
            state: StdMutex::new(CallManagerState::default()),
            ring_timeout,
            media_watchdog_timeout,
            echo_probe_timeout,
            force_null_audio: true,
        }
    }

    /// Return a UI-safe snapshot of the current call state without touching `CoreEngine::session`.
    pub fn snapshot(&self) -> CallSnapshot {
        let g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        snapshot_from_state(&g, None)
    }

    /// Number of times a real audio device was opened (used to verify `EDGE-5` headless guard).
    pub fn audio_device_opened_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .audio_device_opened_count
    }

    /// Return the currently selected `(mic_id, speaker_id)` preferences (`None` = system default).
    pub fn selected_devices(&self) -> (Option<String>, Option<String>) {
        let g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (g.selected_mic_id.clone(), g.selected_speaker_id.clone())
    }
}

fn to_call_phase(state: CallState) -> CallPhase {
    match state {
        CallState::Idle => CallPhase::Idle,
        CallState::OutgoingRinging => CallPhase::OutgoingRinging,
        CallState::IncomingRinging => CallPhase::IncomingRinging,
        CallState::Connecting => CallPhase::Connecting,
        CallState::Active => CallPhase::Active,
        CallState::Ending => CallPhase::Ending,
    }
}

fn end_reason_label(reason: CallEndReason) -> String {
    match reason {
        CallEndReason::Normal => "normal",
        CallEndReason::Cancelled => "cancelled",
        CallEndReason::Rejected => "rejected",
        CallEndReason::Busy => "busy",
        CallEndReason::RingTimeout => "ring_timeout",
        CallEndReason::MediaTimeout => "media_timeout",
        CallEndReason::PeerEnded => "peer_ended",
        CallEndReason::NetworkError => "network_error",
    }
    .to_string()
}

fn format_call_duration(duration_secs: u64) -> String {
    let mins = duration_secs / 60;
    let secs = duration_secs % 60;
    format!("{mins:02}:{secs:02}")
}

fn resolve_call_outcome(
    reason: CallEndReason,
    is_outgoing: bool,
    was_connected: bool,
) -> CallOutcome {
    if was_connected {
        return CallOutcome::Completed;
    }
    match reason {
        CallEndReason::Cancelled => {
            if is_outgoing {
                CallOutcome::Cancelled
            } else {
                CallOutcome::Missed
            }
        }
        CallEndReason::Rejected | CallEndReason::Busy => CallOutcome::Rejected,
        CallEndReason::RingTimeout => CallOutcome::Missed,
        CallEndReason::Normal
        | CallEndReason::PeerEnded
        | CallEndReason::MediaTimeout
        | CallEndReason::NetworkError => {
            if is_outgoing {
                CallOutcome::Cancelled
            } else {
                CallOutcome::Missed
            }
        }
    }
}

fn build_call_history_message(
    call_id: u32,
    peer_id: &str,
    peer_name: Option<&str>,
    is_outgoing: bool,
    was_connected: bool,
    duration_secs: u64,
    reason: CallEndReason,
    ts_ms: i64,
) -> ChatMessage {
    let outcome = resolve_call_outcome(reason, is_outgoing, was_connected);
    let call_duration_secs = (outcome == CallOutcome::Completed).then_some(duration_secs);
    let text = match outcome {
        CallOutcome::Completed => {
            format!("Cuộc gọi thoại ({})", format_call_duration(duration_secs))
        }
        CallOutcome::Missed => "Cuộc gọi nhỡ".to_string(),
        CallOutcome::Rejected => {
            if is_outgoing {
                "Cuộc gọi bị từ chối".to_string()
            } else {
                "Đã từ chối cuộc gọi".to_string()
            }
        }
        CallOutcome::Cancelled => "Đã hủy cuộc gọi".to_string(),
    };
    let sender = if is_outgoing {
        "Tôi".to_string()
    } else {
        peer_name
            .filter(|n| !n.is_empty())
            .unwrap_or(peer_id)
            .to_string()
    };
    let sender_id = if is_outgoing {
        Some("0".to_string())
    } else {
        Some(peer_id.to_string())
    };
    ChatMessage {
        kind: MessageKind::Call,
        id: format!("call_{call_id}"),
        sender,
        sender_id,
        text,
        time: crate::clock::message_time(ts_ms),
        is_outgoing,
        ts: ts_ms,
        call_outcome: Some(outcome),
        call_duration_secs,
        ..Default::default()
    }
}

fn snapshot_from_state(g: &CallManagerState, end_reason: Option<CallEndReason>) -> CallSnapshot {
    CallSnapshot {
        phase: to_call_phase(g.session.state()),
        call_id: g.session.call_id(),
        peer_id: g.session.peer_id().map(str::to_string),
        peer_name: g.peer_name.clone(),
        peer_avatar: g.peer_avatar.clone(),
        is_incoming: g.session.direction() == Some(CallDirection::Incoming),
        is_muted: g.session.is_muted(),
        is_held: g.session.is_held(),
        started_at_ms: g.started_at_ms,
        end_reason: end_reason.map(end_reason_label),
    }
}

impl CoreEngine {
    /// Return the current 1-1 call snapshot (`INV-9`: never acquires `CoreEngine::session`).
    pub fn call_snapshot(&self) -> CallSnapshot {
        self.call_manager.snapshot()
    }

    /// Start an outgoing 1-1 audio call to `peer_id` (`11300 requestcall`).
    ///
    /// # Errors
    /// Returns an error if another call is already active (`INV-7`), the user is not logged in,
    /// or `requestcall` signaling fails.
    pub async fn start_call(
        self: &Arc<Self>,
        peer_id: String,
        peer_name: Option<String>,
        peer_avatar: Option<String>,
    ) -> Result<()> {
        {
            let g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if g.session.state() != CallState::Idle {
                bail!("Đang có cuộc gọi khác");
            }
        }

        // INV-9: clone Session under a short lock and drop it before any network/media I/O.
        let s = self.session.lock().await.clone();
        let resp = request_call(&s, &peer_id, 0, &ExtendData::default()).await?;
        let call_id = resp
            .call_id_u32()
            .ok_or_else(|| anyhow::anyhow!("Thiếu callId từ máy chủ"))?;
        let sess_id = resp
            .sess_id_str()
            .unwrap_or_else(|| format!("sess-{call_id}"));
        let srtp_mode = resp.parsed_extend_data().srtp_mode;
        let relays = resp.relay_servers();
        let p2p_cands = resp.p2p_candidates();
        let srtp_key = resp.srtp_key_material();

        let (resolved_name, resolved_avatar) =
            self.resolve_call_peer(&peer_id, peer_name, peer_avatar);

        let effects = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.peer_name = resolved_name;
            g.peer_avatar = resolved_avatar;
            g.relay_servers = relays;
            g.p2p_candidates = p2p_cands;
            g.srtp_key = srtp_key;
            g.started_at_ms = None;
            g.session.apply(CallEvent::StartOutgoing {
                call_id,
                sess_id,
                peer_id,
                srtp_mode,
            })?
        };

        self.execute_call_effects(&s, effects).await;
        Ok(())
    }

    /// Accept (`accept = true`) or reject (`accept = false`) the current incoming call (`11303 answer`).
    ///
    /// # Errors
    /// Returns an error if no matching incoming ringing call is active.
    pub async fn answer_call(self: &Arc<Self>, accept: bool) -> Result<()> {
        let s = self.session.lock().await.clone();
        let (call_id, effects) = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let Some(cid) = g.session.call_id() else {
                bail!("Không có cuộc gọi đến nào đang đổ chuông");
            };
            let ev = if accept {
                CallEvent::AcceptIncoming { call_id: cid }
            } else {
                CallEvent::RejectIncoming { call_id: cid }
            };
            (cid, g.session.apply(ev)?)
        };
        let _ = call_id;
        self.execute_call_effects(&s, effects).await;
        Ok(())
    }

    /// End or cancel the current call (`11305 cancel` / `11306 endcall`, `ADV-4` idempotent).
    pub async fn end_call(self: &Arc<Self>) -> Result<()> {
        let effects = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.session.apply(CallEvent::LocalEnd)?
        };
        if effects.is_empty() {
            return Ok(());
        }
        let s = self.session.lock().await.clone();
        self.execute_call_effects(&s, effects).await;
        Ok(())
    }

    /// Mute or unmute local microphone during an active call (`12006 conf`).
    pub async fn set_call_muted(self: &Arc<Self>, muted: bool) -> Result<()> {
        let effects = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.session.apply(CallEvent::SetMuted { muted })?
        };
        if effects.is_empty() {
            return Ok(());
        }
        let s = self.session.lock().await.clone();
        self.execute_call_effects(&s, effects).await;
        Ok(())
    }

    /// Put the active call on hold or resume (`11307 holdreq` / `11309 unholdreq`).
    pub async fn set_call_held(self: &Arc<Self>, held: bool) -> Result<()> {
        let effects = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.session.apply(CallEvent::SetHeld { held })?
        };
        if effects.is_empty() {
            return Ok(());
        }
        let s = self.session.lock().await.clone();
        self.execute_call_effects(&s, effects).await;
        Ok(())
    }

    /// Select microphone and speaker devices for active and future calls (`None` / `"default"` = system default).
    ///
    /// # Errors
    /// Returns `Ok(())` after storing the normalized device selection and notifying any active media loop.
    pub async fn set_call_devices(
        self: &Arc<Self>,
        mic_id: Option<String>,
        speaker_id: Option<String>,
    ) -> Result<()> {
        let norm_mic = normalize_device_id(mic_id.as_deref()).map(ToString::to_string);
        let norm_spk = normalize_device_id(speaker_id.as_deref()).map(ToString::to_string);
        let tx = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.selected_mic_id = norm_mic.clone();
            g.selected_speaker_id = norm_spk.clone();
            g.media_cmd_tx.clone()
        };
        if let Some(tx) = tx {
            let _ = tx.send(MediaLoopCmd::SetDevices {
                mic_id: norm_mic,
                speaker_id: norm_spk,
            });
        }
        Ok(())
    }

    /// Dispatch a parsed VoIP signaling event (`401..409`) from WS `604` or `601..603`.
    pub async fn handle_voip_signal(
        self: &Arc<Self>,
        s_snap: &Session,
        signal: CallSignalWireData,
    ) {
        let Some(code) = signal.signal_code() else {
            return;
        };
        let Some(call_id) = signal.call_id_u32() else {
            return;
        };

        // EDGE-5: Headless mode (`--mcp` / `--api-only`, `NoopEventSink`) ignores 401 invites,
        // never auto-answers, and never opens a microphone or audio device.
        if self.sink.is_headless() {
            if code == CallSignalCode::RequestCall {
                crate::logger::log_error(
                    "call::headless",
                    &format!("ignored incoming 401 invite call_id={call_id}"),
                );
            }
            return;
        }

        let effects = match code {
            CallSignalCode::RequestCall => {
                if signal.is_video_call() {
                    crate::logger::log_error(
                        "call::signal",
                        &format!("ignored unsupported video call invite call_id={call_id}"),
                    );
                    return;
                }
                let peer_id = signal
                    .caller_id_str()
                    .or_else(|| signal.callee_id_str())
                    .unwrap_or_default();
                if peer_id.is_empty() {
                    return;
                }
                let sess_id = signal
                    .sess_id_str()
                    .unwrap_or_else(|| format!("sess-{call_id}"));
                let srtp_mode = signal.parsed_extend_data().srtp_mode;
                let relays = signal.relay_servers();
                let p2p_cands = signal.p2p_candidates();
                let srtp_key = signal.srtp_key_material();
                let (resolved_name, resolved_avatar) = self.resolve_call_peer(&peer_id, None, None);

                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let was_idle = g.session.state() == CallState::Idle;
                let res = g.session.apply(CallEvent::IncomingInvite {
                    call_id,
                    sess_id,
                    peer_id,
                    srtp_mode,
                });
                match res {
                    Ok(effs) => {
                        if was_idle && g.session.state() == CallState::IncomingRinging {
                            g.peer_name = resolved_name;
                            g.peer_avatar = resolved_avatar;
                            g.relay_servers = relays;
                            g.p2p_candidates = p2p_cands;
                            g.srtp_key = srtp_key;
                            g.started_at_ms = None;
                        }
                        effs
                    }
                    Err(_) => return,
                }
            }
            CallSignalCode::RingRing => {
                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.session
                    .apply(CallEvent::PeerRingRing { call_id })
                    .unwrap_or_default()
            }
            CallSignalCode::Answer => {
                let accepted = signal.status_i64().unwrap_or(0) == 0;
                let relays = signal.relay_servers();
                let p2p_cands = signal.p2p_candidates();
                let srtp_key = signal.srtp_key_material();
                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if accepted {
                    if !relays.is_empty() {
                        g.relay_servers = relays;
                    }
                    if !p2p_cands.is_empty() {
                        g.p2p_candidates = p2p_cands;
                    }
                    if srtp_key.is_some() {
                        g.srtp_key = srtp_key;
                    }
                }
                g.session
                    .apply(CallEvent::PeerAnswered { call_id, accepted })
                    .unwrap_or_default()
            }
            CallSignalCode::Cancel => {
                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.session
                    .apply(CallEvent::PeerCancelled { call_id })
                    .unwrap_or_default()
            }
            CallSignalCode::EndCall => {
                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.session
                    .apply(CallEvent::PeerEnded { call_id })
                    .unwrap_or_default()
            }
            CallSignalCode::HoldReq | CallSignalCode::UnholdReq => {
                let ack = {
                    let g = self
                        .call_manager
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if g.session.call_id() == Some(call_id) {
                        Some((
                            g.session.peer_id().unwrap_or_default().to_string(),
                            g.session.sess_id().unwrap_or_default().to_string(),
                        ))
                    } else {
                        None
                    }
                };
                if let Some((peer_id, sess_id)) = ack {
                    let s_clone = s_snap.clone();
                    tokio::spawn(async move {
                        let res = if code == CallSignalCode::HoldReq {
                            hold_response(&s_clone, &peer_id, call_id, &sess_id, 0).await
                        } else {
                            unhold_response(&s_clone, &peer_id, call_id, &sess_id, 0).await
                        };
                        if let Err(e) = res {
                            crate::logger::log_error(
                                "call::signal",
                                &format!("hold/unhold ack failed: {e}"),
                            );
                        }
                    });
                }
                return;
            }
            _ => return,
        };

        if !effects.is_empty() {
            self.execute_call_effects(s_snap, effects).await;
        }
    }

    fn resolve_call_peer(
        &self,
        peer_id: &str,
        explicit_name: Option<String>,
        explicit_avatar: Option<String>,
    ) -> (Option<String>, Option<String>) {
        if explicit_name.is_some() {
            return (explicit_name, explicit_avatar);
        }
        let threads = self.known_threads_snapshot();
        let found_name = threads
            .iter()
            .find(|(id, _, _)| id == peer_id)
            .map(|(_, name, _)| name.clone());
        (found_name, explicit_avatar)
    }

    fn emit_call_snapshot(&self, end_reason: Option<CallEndReason>) {
        let snap = {
            let g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            snapshot_from_state(&g, end_reason)
        };
        self.emit(CoreEvent::CallStateChanged {
            phase: snap.phase,
            call_id: snap.call_id,
            peer_id: snap.peer_id,
            peer_name: snap.peer_name,
            peer_avatar: snap.peer_avatar,
            is_incoming: snap.is_incoming,
            is_muted: snap.is_muted,
            is_held: snap.is_held,
            started_at_ms: snap.started_at_ms,
            end_reason: snap.end_reason,
        });
    }

    async fn execute_call_effects(self: &Arc<Self>, s_snap: &Session, effects: Vec<CallEffect>) {
        let mut needs_teardown_complete = false;
        let mut finished_reason: Option<CallEndReason> = None;

        for eff in effects {
            match eff {
                CallEffect::StateChanged(new_state) => {
                    match new_state {
                        CallState::OutgoingRinging | CallState::IncomingRinging => {
                            self.arm_ring_timer();
                        }
                        CallState::Connecting => {
                            self.cancel_ring_timer();
                        }
                        CallState::Active => {
                            self.cancel_ring_timer();
                            let mut g = self
                                .call_manager
                                .state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            if g.started_at_ms.is_none() {
                                g.started_at_ms = Some(now_ms() as i64);
                            }
                        }
                        CallState::Ending => {
                            self.cancel_ring_timer();
                        }
                        CallState::Idle => {
                            self.cancel_ring_timer();
                        }
                    }
                    self.emit_call_snapshot(finished_reason);
                }
                CallEffect::SendSignal(sig) => {
                    self.dispatch_outbound_signal(s_snap, sig).await;
                }
                CallEffect::RejectBusy {
                    call_id,
                    sess_id,
                    peer_id,
                } => {
                    // INV-7 / ADV-2: reject 2nd incoming call in background without touching active call.
                    let s_clone = s_snap.clone();
                    tokio::spawn(async move {
                        let _ = answer_call(
                            &s_clone,
                            &peer_id,
                            call_id,
                            &sess_id,
                            1,
                            &ExtendData::default(),
                        )
                        .await;
                    });
                }
                CallEffect::StartZrtpAndMedia => {
                    self.start_media_loop(s_snap);
                }
                CallEffect::StopZrtpAndMedia { send_end_call_pkt } => {
                    self.stop_media_loop(send_end_call_pkt).await;
                    needs_teardown_complete = true;
                }
                CallEffect::MuteChanged(muted) => {
                    let tx = {
                        let g = self
                            .call_manager
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        g.media_cmd_tx.clone()
                    };
                    if let Some(tx) = tx {
                        let _ = tx.send(MediaLoopCmd::SetMuted(muted));
                    }
                    self.emit_call_snapshot(None);
                }
                CallEffect::HoldChanged(held) => {
                    let tx = {
                        let g = self
                            .call_manager
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        g.media_cmd_tx.clone()
                    };
                    if let Some(tx) = tx {
                        let _ = tx.send(MediaLoopCmd::SetHeld(held));
                    }
                    self.emit_call_snapshot(None);
                }
                CallEffect::CallFinished { reason, .. } => {
                    finished_reason = Some(reason);
                }
            }
        }

        if needs_teardown_complete {
            let (
                call_id,
                sess_id,
                peer_id,
                peer_name,
                is_outgoing,
                was_connected,
                duration_secs,
                teardown_effects,
            ) = {
                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let cid = g.session.call_id().unwrap_or(0);
                let sid = g.session.sess_id().unwrap_or_default().to_string();
                let pid = g.session.peer_id().unwrap_or_default().to_string();
                let pname = g.peer_name.clone();
                let outgoing = g.session.direction() == Some(CallDirection::Outgoing);
                let connected = g.started_at_ms.is_some();
                let dur = g
                    .started_at_ms
                    .map(|st| (((now_ms() as i64) - st).max(0) / 1000) as u64)
                    .unwrap_or(0);
                let effs = g
                    .session
                    .apply(CallEvent::TeardownComplete)
                    .unwrap_or_default();
                g.peer_name = None;
                g.peer_avatar = None;
                g.relay_servers.clear();
                g.p2p_candidates.clear();
                g.srtp_key = None;
                g.started_at_ms = None;
                (cid, sid, pid, pname, outgoing, connected, dur, effs)
            };

            let mut end_reason_for_idle = None;
            for eff in teardown_effects {
                match eff {
                    CallEffect::SendSignal(CallSignalCode::LogEndCall) => {
                        if call_id != 0 && !peer_id.is_empty() && s_snap.has_live_credentials() {
                            let _ =
                                log_end_call(s_snap, &peer_id, call_id, &sess_id, duration_secs, 0)
                                    .await;
                        }
                    }
                    CallEffect::CallFinished { reason, .. } => {
                        end_reason_for_idle = Some(reason);
                    }
                    CallEffect::StateChanged(CallState::Idle) => {
                        if call_id != 0 && !peer_id.is_empty() {
                            let reason = end_reason_for_idle.unwrap_or(CallEndReason::Normal);
                            let msg = build_call_history_message(
                                call_id,
                                &peer_id,
                                peer_name.as_deref(),
                                is_outgoing,
                                was_connected,
                                duration_secs,
                                reason,
                                now_ms() as i64,
                            );
                            self.persist_call_history_message(&peer_id, &msg).await;
                            self.emit(CoreEvent::CallHistoryRecorded {
                                thread_id: peer_id.clone(),
                                msg,
                            });
                        }
                        self.emit_call_snapshot(end_reason_for_idle);
                    }
                    _ => {}
                }
            }
        }
    }

    async fn persist_call_history_message(&self, thread_id: &str, msg: &ChatMessage) {
        let db_path = self.active_db_path().await;
        let mut sm = StoredMessage::from(msg);
        sm.thread_id = thread_id.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut store) = MessageStore::open(&db_path) {
                if let Err(err) = store.upsert_message(&sm) {
                    crate::logger::log_error("core::call_history_persist", &err.to_string());
                }
            }
        })
        .await;
    }

    async fn dispatch_outbound_signal(&self, s_snap: &Session, sig: CallSignalCode) {
        if !s_snap.has_live_credentials() {
            return;
        }
        let (call_id, sess_id, peer_id, is_muted) = {
            let g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (
                g.session.call_id().unwrap_or(0),
                g.session.sess_id().unwrap_or_default().to_string(),
                g.session.peer_id().unwrap_or_default().to_string(),
                g.session.is_muted(),
            )
        };
        if call_id == 0 || peer_id.is_empty() {
            return;
        }

        let res = match sig {
            CallSignalCode::Request => {
                send_request(s_snap, &peer_id, call_id, &sess_id, &ExtendData::default()).await
            }
            CallSignalCode::RingRing => {
                send_ring_ring(s_snap, &peer_id, call_id, &sess_id, 0).await
            }
            CallSignalCode::Answer => {
                let status = {
                    let g = self
                        .call_manager
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if g.session.state() == CallState::Ending {
                        1
                    } else {
                        0
                    }
                };
                answer_call(
                    s_snap,
                    &peer_id,
                    call_id,
                    &sess_id,
                    status,
                    &ExtendData::default(),
                )
                .await
            }
            CallSignalCode::AnswerAck => send_answer_ack(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::Cancel => cancel_call(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::EndCall => {
                send_end_call_http(s_snap, &peer_id, call_id, &sess_id, 0).await
            }
            CallSignalCode::HoldReq => hold_request(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::UnholdReq => unhold_request(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::Conf => {
                send_conf(s_snap, &peer_id, call_id, false, !is_muted, None).await
            }
            _ => return,
        };

        if let Err(e) = res {
            crate::logger::log_error("call::signal", &format!("signal {sig:?} failed: {e}"));
        }
    }

    fn arm_ring_timer(self: &Arc<Self>) {
        self.cancel_ring_timer();
        let engine = Arc::clone(self);
        let timeout = self.call_manager.ring_timeout;
        let handle = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            let effects = {
                let mut g = engine
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.ring_timer_task = None;
                g.session.apply(CallEvent::RingTimeout).unwrap_or_default()
            };
            if !effects.is_empty() {
                let s = engine.session.lock().await.clone();
                engine.execute_call_effects(&s, effects).await;
            }
        });
        let mut g = self
            .call_manager
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.ring_timer_task = Some(handle);
    }

    fn cancel_ring_timer(&self) {
        let handle = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.ring_timer_task.take()
        };
        if let Some(h) = handle {
            h.abort();
        }
    }

    fn start_media_loop(self: &Arc<Self>, s_snap: &Session) {
        let already_running = {
            let g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            g.media_task.is_some()
        };
        if already_running {
            return;
        }

        let (call_id, sess_id, peer_id, is_caller, relays, p2p_cands, srtp_key, init_mic, init_spk) = {
            let g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (
                g.session.call_id().unwrap_or(0),
                g.session.sess_id().unwrap_or_default().to_string(),
                g.session.peer_id().unwrap_or_default().to_string(),
                g.session.direction() == Some(CallDirection::Outgoing),
                g.relay_servers.clone(),
                g.p2p_candidates.clone(),
                g.srtp_key.clone(),
                g.selected_mic_id.clone(),
                g.selected_speaker_id.clone(),
            )
        };
        let uid: u32 = s_snap
            .login
            .as_ref()
            .and_then(|l| l.uin.parse::<u32>().ok())
            .unwrap_or(0);
        let peer_uid: u32 = peer_id.parse::<u32>().unwrap_or(0);
        let is_headless = self.sink.is_headless();
        let force_null_audio = self.call_manager.force_null_audio;
        let echo_timeout = self.call_manager.echo_probe_timeout;
        let watchdog_timeout = self.call_manager.media_watchdog_timeout;

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<MediaLoopCmd>();
        let engine = Arc::clone(self);
        let s_clone = s_snap.clone();

        let handle = tokio::spawn(async move {
            let socket = match UdpSocket::bind("0.0.0.0:0").await {
                Ok(sock) => sock,
                Err(e) => {
                    crate::logger::log_error("call::udp", &format!("UDP bind failed: {e}"));
                    return;
                }
            };

            let relay_addr: Option<SocketAddr> =
                select_relay_with_echo(&socket, &relays, uid, echo_timeout).await;

            // Send InitZRTP (cmd = 11 caller / 12 callee) when relay is available.
            let mut server_token = 0u32;
            if let Some(target) = relay_addr {
                let init_pkt =
                    ZRTPPacket::init_call(is_caller, 1, uid, call_id, peer_uid, sess_id.clone());
                let _ = socket.send_to(&init_pkt.encode(), target).await;

                // Wait briefly for InitResponse (0x02) before proceeding.
                let mut buf = [0u8; 2048];
                if let Ok(Ok((n, _))) =
                    tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buf))
                        .await
                {
                    if let Ok(ZRTPPacket::Control { token, .. }) = ZRTPPacket::decode(&buf[..n]) {
                        server_token = token;
                    }
                }
            }

            // Transition Connecting -> Active when ZRTP/media is ready.
            let ready_effects = {
                let mut g = engine
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.session
                    .apply(CallEvent::MediaReady { call_id })
                    .unwrap_or_default()
            };
            if !ready_effects.is_empty() {
                engine.execute_call_effects(&s_clone, ready_effects).await;
            }

            // EDGE-5: never open hardware audio devices in headless mode.
            let muted_flag = Arc::new(AtomicBool::new(false));
            let held_flag = Arc::new(AtomicBool::new(false));
            let stop_audio_flag = Arc::new(AtomicBool::new(false));
            let reopen_devices_flag = Arc::new(AtomicBool::new(false));
            let pending_devices = Arc::new(StdMutex::new((init_mic, init_spk)));
            let (tx_pcm_up, mut rx_pcm_up) = mpsc::channel::<[i16; FRAME_SAMPLES]>(16);
            let (tx_pcm_down, rx_pcm_down) = mpsc::channel::<[i16; FRAME_SAMPLES]>(16);

            let _audio_thread = if !is_headless
                && !force_null_audio
                && cfg!(all(feature = "call", target_os = "linux"))
            {
                {
                    let mut g = engine
                        .call_manager
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    g.audio_device_opened_count += 1;
                }
                spawn_audio_thread(
                    Arc::clone(&muted_flag),
                    Arc::clone(&held_flag),
                    Arc::clone(&stop_audio_flag),
                    Arc::clone(&reopen_devices_flag),
                    Arc::clone(&pending_devices),
                    tx_pcm_up,
                    rx_pcm_down,
                )
            } else {
                drop(tx_pcm_up);
                drop(rx_pcm_down);
                None
            };

            let mut srtp_tx = srtp_key
                .as_ref()
                .and_then(|k| SrtpContext::from_combined(k.as_bytes()).ok());
            let mut srtp_rx = srtp_key
                .as_ref()
                .and_then(|k| SrtpContext::from_combined(k.as_bytes()).ok());

            let mut encoder = create_opus_encoder();
            let mut opus_out_buf = [0u8; 512];
            let mut neteq = create_neteq();
            let mut p2p =
                P2pController::new(is_caller, call_id, uid, server_token, sess_id.clone());
            if let Some(ip) = probe_local_ipv4() {
                let _ = p2p.on_local_ip_probe(ip, now_ms());
            }
            if !p2p_cands.is_empty() {
                let probes = p2p.add_candidates(p2p_cands);
                dispatch_p2p_outbound(&socket, relay_addr, probes).await;
            }
            if let Some(target) = relay_addr {
                // Initial REQ_FORWARD subCmd=8 channel prime (`re-notes/16` §3).
                let prime = ZRTPPacket::Control {
                    is_reply: false,
                    magic: 0x01,
                    field02: 0,
                    seq: 1,
                    uid,
                    token: server_token,
                    cmd: ZrtpCmd::ReqForward as u16,
                    sub_cmd: ReqForwardSubCmd::ChannelPrime as u8,
                    body: ControlBody::ForwardPrime([4, 0, 0, 0, 0, 0, 0, 0]),
                };
                let _ = socket.send_to(&prime.encode(), target).await;
            }

            let mut rtp_seq: u16 = 1;
            let mut last_rx_at = Instant::now();
            let mut recv_buf = [0u8; 2048];
            let mut ping_tick = tokio::time::interval(Duration::from_secs(5));
            let mut route_probe_tick = tokio::time::interval(Duration::from_secs(1));
            let mut change_addr_tick = tokio::time::interval(Duration::from_millis(500));
            let mut watchdog_tick = tokio::time::interval(Duration::from_millis(200));

            loop {
                tokio::select! {
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            MediaLoopCmd::SetMuted(m) => {
                                muted_flag.store(m, Ordering::Relaxed);
                            }
                            MediaLoopCmd::SetHeld(h) => {
                                held_flag.store(h, Ordering::Relaxed);
                            }
                            MediaLoopCmd::SetDevices { mic_id, speaker_id } => {
                                {
                                    let mut dev_g = pending_devices
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    *dev_g = (mic_id, speaker_id);
                                }
                                reopen_devices_flag.store(true, Ordering::Release);
                            }
                            MediaLoopCmd::Stop { send_end_call_pkt } => {
                                if send_end_call_pkt {
                                    if let Some(target) = relay_addr {
                                        let end_pkt = ZRTPPacket::end_call(2, uid, server_token, call_id);
                                        let _ = socket.send_to(&end_pkt.encode(), target).await;
                                    }
                                }
                                break;
                            }
                        }
                    }
                    Some(pcm) = rx_pcm_up.recv() => {
                        if (relay_addr.is_some() || p2p.active_peer().is_some())
                            && !muted_flag.load(Ordering::Relaxed)
                            && !held_flag.load(Ordering::Relaxed)
                        {
                            if let Ok(enc_len) = encoder.encode(&pcm, &mut opus_out_buf) {
                                if enc_len > 0 {
                                    let rtp_pkt = build_minimal_rtp(rtp_seq, &opus_out_buf[..enc_len]);
                                    rtp_seq = rtp_seq.wrapping_add(1);
                                    let wire_rtp = if let Some(ctx) = srtp_tx.as_mut() {
                                        ctx.protect_rtp(&rtp_pkt).unwrap_or(rtp_pkt)
                                    } else {
                                        rtp_pkt
                                    };
                                    let outbound = p2p.build_outbound_audio(wire_rtp);
                                    dispatch_p2p_outbound(&socket, relay_addr, vec![outbound]).await;
                                }
                            }
                        }
                    }
                    Ok((n, src)) = socket.recv_from(&mut recv_buf) => {
                        last_rx_at = Instant::now();
                        match ZRTPPacket::decode(&recv_buf[..n]) {
                            Ok(ZRTPPacket::RelayMediaDown { msg_type, payload }) => {
                                if msg_type == MSG_AUDIO_RTP_DOWN
                                    && !held_flag.load(Ordering::Relaxed)
                                    && decode_and_queue_audio(
                                        &payload,
                                        srtp_rx.as_mut(),
                                        &mut neteq,
                                        &tx_pcm_down,
                                    )
                                {
                                    p2p.on_relay_audio_received(now_ms());
                                }
                            }
                            Ok(ZRTPPacket::P2p {
                                sender_role,
                                call_id: pkt_call_id,
                                kind,
                                body,
                                ..
                            }) => {
                                let (outcome, replies) = p2p.handle_p2p_packet(
                                    src,
                                    sender_role,
                                    pkt_call_id,
                                    kind,
                                    body,
                                    now_ms(),
                                );
                                dispatch_p2p_outbound(&socket, relay_addr, replies).await;
                                if let P2pRxOutcome::AudioMedia(payload) = outcome {
                                    if !held_flag.load(Ordering::Relaxed) {
                                        decode_and_queue_audio(
                                            &payload,
                                            srtp_rx.as_mut(),
                                            &mut neteq,
                                            &tx_pcm_down,
                                        );
                                    }
                                }
                            }
                            Ok(ZRTPPacket::Control {
                                is_reply,
                                cmd,
                                sub_cmd,
                                token,
                                ..
                            }) => {
                                if is_reply {
                                    if token != 0 {
                                        server_token = token;
                                    }
                                    p2p.on_server_reply(cmd, token);
                                } else if cmd == ZrtpCmd::ReqForward as u16 {
                                    let replies = p2p.on_req_forward(sub_cmd);
                                    dispatch_p2p_outbound(&socket, relay_addr, replies).await;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ = ping_tick.tick() => {
                        if let Some(target) = relay_addr {
                            let ping = ZRTPPacket::ping(0, uid, server_token, sess_id.clone());
                            let _ = socket.send_to(&ping.encode(), target).await;
                        }
                    }
                    _ = route_probe_tick.tick() => {
                        if let Some(ip) = probe_local_ipv4() {
                            let pkts = p2p.on_local_ip_probe(ip, now_ms());
                            dispatch_p2p_outbound(&socket, relay_addr, pkts).await;
                        }
                    }
                    _ = change_addr_tick.tick() => {
                        let pkts = p2p.on_change_address_tick(now_ms());
                        dispatch_p2p_outbound(&socket, relay_addr, pkts).await;
                    }
                    _ = watchdog_tick.tick() => {
                        if last_rx_at.elapsed() >= watchdog_timeout {
                            let effs = {
                                let mut g = engine
                                    .call_manager
                                    .state
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                g.session.apply(CallEvent::MediaWatchdogTimeout).unwrap_or_default()
                            };
                            if !effs.is_empty() {
                                let eng = Arc::clone(&engine);
                                let sc = s_clone.clone();
                                tokio::spawn(async move {
                                    eng.execute_call_effects(&sc, effs).await;
                                });
                            }
                            break;
                        }
                    }
                }
            }

            stop_audio_flag.store(true, Ordering::Relaxed);
        });

        let mut g = self
            .call_manager
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.media_cmd_tx = Some(cmd_tx);
        g.media_task = Some(handle);
    }

    async fn stop_media_loop(&self, send_end_call_pkt: bool) {
        let (tx, handle) = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (g.media_cmd_tx.take(), g.media_task.take())
        };
        if let Some(tx) = tx {
            let _ = tx.send(MediaLoopCmd::Stop { send_end_call_pkt });
        }
        if let Some(h) = handle {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
    }
}

async fn select_relay_with_echo(
    socket: &UdpSocket,
    relays: &[RelayServerAddr],
    uid: u32,
    timeout: Duration,
) -> Option<SocketAddr> {
    let first_addr: Option<SocketAddr> = relays
        .iter()
        .find_map(|r| format!("{}:{}", r.ip, r.port).parse::<SocketAddr>().ok());
    let fallback = first_addr?;

    let echo_pkt = ZRTPPacket::Control {
        is_reply: false,
        magic: 1,
        field02: 0,
        seq: 0,
        uid,
        token: 0,
        cmd: ZrtpCmd::Echo as u16,
        sub_cmd: 0,
        body: ControlBody::Echo(unix_now().to_string()),
    };
    let encoded = echo_pkt.encode();
    for r in relays {
        if let Ok(addr) = format!("{}:{}", r.ip, r.port).parse::<SocketAddr>() {
            let _ = socket.send_to(&encoded, addr).await;
        }
    }

    let mut buf = [0u8; 512];
    if let Ok(Ok((_n, from))) = tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
        return Some(from);
    }
    Some(fallback)
}

fn build_minimal_rtp(seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut rtp = Vec::with_capacity(12 + payload.len());
    rtp.push(0x80);
    rtp.push(111); // Opus payload type
    rtp.extend_from_slice(&seq.to_be_bytes());
    rtp.extend_from_slice(&((seq as u32) * (FRAME_SAMPLES as u32)).to_be_bytes());
    rtp.extend_from_slice(&1u32.to_be_bytes());
    rtp.extend_from_slice(payload);
    rtp
}

fn decode_and_queue_audio(
    payload: &[u8],
    srtp_rx: Option<&mut SrtpContext>,
    neteq: &mut NetEq<Box<dyn OpusDecoderBackend>>,
    tx_pcm_down: &mpsc::Sender<[i16; FRAME_SAMPLES]>,
) -> bool {
    let plain_rtp = if let Some(ctx) = srtp_rx {
        match ctx.unprotect_rtp(payload) {
            Ok(p) => p,
            Err(_) => return false, // ADV-3: drop bad SRTP HMAC
        }
    } else {
        payload.to_vec()
    };
    if plain_rtp.len() >= 12 {
        let seq = u16::from_be_bytes([plain_rtp[2], plain_rtp[3]]);
        neteq.push(seq, plain_rtp[12..].to_vec());
        let mut out_pcm = [0i16; FRAME_SAMPLES];
        if neteq.pop_frame(&mut out_pcm).is_ok() {
            let _ = tx_pcm_down.try_send(out_pcm);
        }
        true
    } else {
        false
    }
}

async fn dispatch_p2p_outbound(
    socket: &UdpSocket,
    relay_addr: Option<SocketAddr>,
    pkts: Vec<P2pOutbound>,
) {
    for item in pkts {
        match item {
            P2pOutbound::ToPeer(addr, pkt) => {
                let _ = socket.send_to(&pkt.encode(), addr).await;
            }
            P2pOutbound::ToRelay(pkt) => {
                if let Some(relay) = relay_addr {
                    let _ = socket.send_to(&pkt.encode(), relay).await;
                }
            }
        }
    }
}

fn probe_local_ipv4() -> Option<Ipv4Addr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:53").ok()?;
    match sock.local_addr().ok()? {
        SocketAddr::V4(v4) => Some(*v4.ip()),
        SocketAddr::V6(_) => None,
    }
}

fn create_opus_encoder() -> Box<dyn OpusEncoderBackend> {
    #[cfg(feature = "call")]
    if let Ok(enc) = crate::services::call_media::LibOpusEncoder::new(24_000) {
        return Box::new(enc);
    }
    Box::new(NullOpusEncoder)
}

fn create_neteq() -> NetEq<Box<dyn OpusDecoderBackend>> {
    let dec: Box<dyn OpusDecoderBackend> = {
        #[cfg(feature = "call")]
        {
            match crate::services::call_media::LibOpusDecoder::new() {
                Ok(d) => Box::new(d),
                Err(_) => Box::new(NullOpusDecoder),
            }
        }
        #[cfg(not(feature = "call"))]
        {
            Box::new(NullOpusDecoder)
        }
    };
    NetEq::new(dec, NetEqConfig::default())
}

impl OpusDecoderBackend for Box<dyn OpusDecoderBackend> {
    fn decode(&mut self, packet: &[u8], pcm: &mut [i16], decode_fec: bool) -> Result<usize> {
        (**self).decode(packet, pcm, decode_fec)
    }

    fn decode_plc(&mut self, pcm: &mut [i16], frame_samples: usize) -> Result<usize> {
        (**self).decode_plc(pcm, frame_samples)
    }
}

fn spawn_audio_thread(
    muted: Arc<AtomicBool>,
    held: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    reopen_devices: Arc<AtomicBool>,
    pending_devices: Arc<StdMutex<(Option<String>, Option<String>)>>,
    tx_up: mpsc::Sender<[i16; FRAME_SAMPLES]>,
    mut rx_down: mpsc::Receiver<[i16; FRAME_SAMPLES]>,
) -> Option<std::thread::JoinHandle<()>> {
    #[cfg(all(feature = "call", target_os = "linux"))]
    {
        use crate::services::call_media::{AudioBackend, PulseAudioBackend};
        Some(std::thread::spawn(move || {
            let open_backend = |mic: Option<&str>, spk: Option<&str>| -> Box<dyn AudioBackend> {
                match PulseAudioBackend::open(normalize_device_id(spk), normalize_device_id(mic)) {
                    Ok(b) => Box::new(b),
                    Err(e) => {
                        crate::logger::log_error(
                            "call::audio",
                            &format!("PulseAudio open failed: {e}"),
                        );
                        Box::new(NullAudioBackend::new())
                    }
                }
            };

            let (init_mic, init_spk) = pending_devices
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let mut backend = open_backend(init_mic.as_deref(), init_spk.as_deref());
            let mut cap_buf = [0i16; FRAME_SAMPLES];
            let silence = [0i16; FRAME_SAMPLES];
            while !stop.load(Ordering::Relaxed) {
                if reopen_devices.swap(false, Ordering::AcqRel) {
                    let (mic, spk) = pending_devices
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    // Drop current PulseAudio streams before opening replacement devices.
                    drop(backend);
                    backend = open_backend(mic.as_deref(), spk.as_deref());
                }
                let is_held = held.load(Ordering::Relaxed);
                if !is_held
                    && !muted.load(Ordering::Relaxed)
                    && backend.read_frame(&mut cap_buf).is_ok()
                {
                    let _ = tx_up.try_send(cap_buf);
                }
                let play_buf = if is_held {
                    let _ = rx_down.try_recv();
                    silence
                } else {
                    rx_down.try_recv().unwrap_or(silence)
                };
                let _ = backend.write_frame(&play_buf);
            }
        }))
    }
    #[cfg(not(all(feature = "call", target_os = "linux")))]
    {
        let _ = (
            muted,
            held,
            stop,
            reopen_devices,
            pending_devices,
            tx_up,
            &mut rx_down,
            NullAudioBackend::new(),
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::CoreEventSink;
    use serde_json::json;

    struct CollectSink(Arc<StdMutex<Vec<CoreEvent>>>);

    impl CoreEventSink for CollectSink {
        fn emit(&self, ev: CoreEvent) {
            self.0.lock().unwrap().push(ev);
        }
    }

    fn test_signal(cmd: u16, call_id: u32, caller_id: &str) -> CallSignalWireData {
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
    async fn headless_engine_ignores_401_and_never_opens_audio_device() {
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::headless(session, ":memory:");
        let s_snap = engine.session_arc().lock().await.clone();

        // EDGE-5: headless mode must ignore incoming 401 and stay Idle.
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 1001, "peer-1"))
            .await;

        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
        assert_eq!(engine.call_manager.audio_device_opened_count(), 0);
    }

    #[tokio::test]
    async fn incoming_401_emits_call_state_changed_and_enforces_single_call_invariant() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_db_path(session, ":memory:", sink);
        let s_snap = engine.session_arc().lock().await.clone();

        engine
            .handle_voip_signal(&s_snap, test_signal(401, 2001, "peer-1"))
            .await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::IncomingRinging);
        assert_eq!(engine.call_snapshot().call_id, Some(2001));

        // ADV-1: duplicate 401 for same callId is deduplicated.
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 2001, "peer-1"))
            .await;
        assert_eq!(engine.call_snapshot().call_id, Some(2001));

        // ADV-2 / INV-7: second 401 with different callId keeps first call undisturbed.
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 2002, "peer-2"))
            .await;
        assert_eq!(engine.call_snapshot().call_id, Some(2001));
        assert_eq!(engine.call_snapshot().phase, CallPhase::IncomingRinging);

        let captured = events.lock().unwrap();
        assert!(captured.iter().any(|ev| matches!(
            ev,
            CoreEvent::CallStateChanged {
                phase: CallPhase::IncomingRinging,
                call_id: Some(2001),
                is_incoming: true,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn ring_timeout_transitions_to_idle_and_emits_call_state_changed() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_millis(30),
            Duration::from_secs(15),
            Duration::from_millis(50),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        engine
            .handle_voip_signal(&s_snap, test_signal(401, 3001, "peer-1"))
            .await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::IncomingRinging);

        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);

        let captured = events.lock().unwrap();
        assert!(captured.iter().any(|ev| matches!(
            ev,
            CoreEvent::CallStateChanged {
                phase: CallPhase::Idle,
                end_reason: Some(r),
                ..
            } if r == "ring_timeout"
        )));
    }

    #[tokio::test]
    async fn media_watchdog_timeout_tears_down_active_call_to_idle() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_secs(60),
            Duration::from_millis(60),
            Duration::from_millis(20),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        engine
            .handle_voip_signal(&s_snap, test_signal(401, 4001, "peer-1"))
            .await;
        engine.answer_call(true).await.unwrap();

        // Wait for media watchdog to fire (> 60ms) with zero UDP packets arriving.
        tokio::time::sleep(Duration::from_millis(320)).await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);

        let captured = events.lock().unwrap();
        assert!(captured.iter().any(|ev| matches!(
            ev,
            CoreEvent::CallStateChanged {
                phase: CallPhase::Idle,
                end_reason: Some(r),
                ..
            } if r == "media_timeout"
        )));
    }

    #[tokio::test]
    async fn call_manager_is_isolated_from_session_mutex_and_end_is_idempotent() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_db_path(session, ":memory:", sink);
        let s_snap = engine.session_arc().lock().await.clone();

        engine
            .handle_voip_signal(&s_snap, test_signal(401, 5001, "peer-1"))
            .await;

        // INV-9: Holding CoreEngine::session mutex must not block call_snapshot().
        let _session_guard = engine.session_arc().lock().await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::IncomingRinging);
        drop(_session_guard);

        // ADV-4: Concurrent local end + peer 409 end are idempotent.
        engine.end_call().await.unwrap();
        engine.end_call().await.unwrap();
        engine
            .handle_voip_signal(&s_snap, test_signal(409, 5001, "peer-1"))
            .await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
    }

    #[tokio::test]
    async fn media_loop_responds_to_0x7f_p2p_bind_and_accepts_direct_audio() {
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
            Duration::from_millis(500),
            Duration::from_millis(20),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        // Incoming 401 with peer_sock as P2P candidate.
        let sig = CallSignalWireData::from_value(&json!({
            "cmd": 401,
            "callId": 6001,
            "sessId": "sess-6001",
            "callerId": "peer-1",
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

        // Peer receives P2P.BIND (0x7f kind=1) from our media loop.
        let mut buf = [0u8; 256];
        let (n, engine_addr) =
            tokio::time::timeout(Duration::from_secs(2), peer_sock.recv_from(&mut buf))
                .await
                .expect("timed out waiting for P2P.BIND")
                .unwrap();
        let pkt = ZRTPPacket::decode(&buf[..n]).unwrap();
        assert!(matches!(
            pkt,
            ZRTPPacket::P2p {
                call_id: 6001,
                kind: crate::protocol::call::P2pKind::Bind,
                ..
            }
        ));

        // Peer sends BindPeer (kind=2) -> engine responds with BindAnswer (kind=3) + Signal(Start).
        let bind_peer = ZRTPPacket::p2p_bind_peer(
            0,
            6001,
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
            .expect("timed out waiting for P2P.BIND_ANSWER")
            .unwrap();
        let ans = ZRTPPacket::decode(&buf[..n2]).unwrap();
        assert!(matches!(
            ans,
            ZRTPPacket::P2p {
                call_id: 6001,
                kind: crate::protocol::call::P2pKind::BindAnswer,
                ..
            }
        ));

        // Peer sends direct P2P audio (kind=6) -> keeps watchdog alive and does not crash.
        let rtp = build_minimal_rtp(1, &[0xf8, 0xff, 0xfe]);
        let p2p_audio = ZRTPPacket::p2p_audio(0, 6001, rtp);
        peer_sock
            .send_to(&p2p_audio.encode(), engine_addr)
            .await
            .unwrap();

        engine.end_call().await.unwrap();
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
    }

    #[tokio::test]
    async fn call_teardown_persists_message_kind_call_and_emits_event() {
        use crate::protocol::messaging::{CallOutcome, MessageKind};
        use crate::storage::MessageStore;

        let dir = std::env::temp_dir().join(format!("alolite_call_hist_{}", now_ms()));
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
            Duration::from_millis(60),
            Duration::from_millis(60),
            Duration::from_millis(20),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        // 1. Incoming call rejected locally -> CallOutcome::Rejected
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 6001, "peer-hist-1"))
            .await;
        engine.answer_call(false).await.unwrap();
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);

        // 2. Incoming call ring timeout -> CallOutcome::Missed
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 6002, "peer-hist-1"))
            .await;
        tokio::time::sleep(Duration::from_millis(160)).await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);

        {
            let captured = events.lock().unwrap();
            assert!(captured.iter().any(|ev| matches!(
                ev,
                CoreEvent::CallHistoryRecorded { thread_id, msg }
                    if thread_id == "peer-hist-1"
                        && msg.id == "call_6001"
                        && msg.kind == MessageKind::Call
                        && msg.call_outcome == Some(CallOutcome::Rejected)
            )));
            assert!(captured.iter().any(|ev| matches!(
                ev,
                CoreEvent::CallHistoryRecorded { thread_id, msg }
                    if thread_id == "peer-hist-1"
                        && msg.id == "call_6002"
                        && msg.kind == MessageKind::Call
                        && msg.call_outcome == Some(CallOutcome::Missed)
            )));
        }

        let store = MessageStore::open(&db_str).unwrap();
        let saved = store.messages_for("peer-hist-1", 10).unwrap();
        assert_eq!(saved.len(), 2);
        assert!(saved
            .iter()
            .any(|m| m.msg_id == "call_6001" && m.msg_type == "chat.call"));
        assert!(saved
            .iter()
            .any(|m| m.msg_id == "call_6002" && m.msg_type == "chat.call"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn set_call_muted_and_held_update_snapshot_and_emit_events() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_db_path(session, ":memory:", sink);

        {
            let mut g = engine.call_manager.state.lock().unwrap();
            let _ = g
                .session
                .apply(CallEvent::IncomingInvite {
                    call_id: 6001,
                    sess_id: "s-6001".to_string(),
                    peer_id: "peer-6".to_string(),
                    srtp_mode: 1,
                })
                .unwrap();
            let _ = g
                .session
                .apply(CallEvent::AcceptIncoming { call_id: 6001 })
                .unwrap();
            let _ = g
                .session
                .apply(CallEvent::MediaReady { call_id: 6001 })
                .unwrap();
        }

        engine.set_call_muted(true).await.unwrap();
        assert!(engine.call_snapshot().is_muted);

        engine.set_call_held(true).await.unwrap();
        assert!(engine.call_snapshot().is_held);

        engine.set_call_held(false).await.unwrap();
        assert!(!engine.call_snapshot().is_held);

        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|ev| matches!(
            ev,
            CoreEvent::CallStateChanged {
                phase: CallPhase::Active,
                is_muted: true,
                ..
            }
        )));
        assert!(evs.iter().any(|ev| matches!(
            ev,
            CoreEvent::CallStateChanged {
                phase: CallPhase::Active,
                is_held: true,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn set_call_devices_stores_normalized_selection() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_db_path(session, ":memory:", sink);

        engine
            .set_call_devices(
                Some("alsa_input.usb".to_string()),
                Some("default".to_string()),
            )
            .await
            .unwrap();

        let (mic, spk) = engine.call_manager.selected_devices();
        assert_eq!(mic.as_deref(), Some("alsa_input.usb"));
        assert_eq!(spk, None);
    }
}
