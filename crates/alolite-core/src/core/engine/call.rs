//! 1-1 ZRTC audio call coordinator (`CallManager`) for [`CoreEngine`].
//!
//! Enforces `INV-7` (single active `CallSession`), `INV-8` (SRTP + redacted logs),
//! `INV-9` (`CallManager` lives outside `CoreEngine::session` mutex), and `EDGE-5`
//! (headless mode ignores `401` invites and never opens local audio devices).

use std::net::{Ipv4Addr, SocketAddr};
#[cfg(all(feature = "call", target_os = "linux"))]
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
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
use crate::protocol::call::srtp::{extract_rtp_payload, SrtpContext};
use crate::protocol::call::zrtp::{
    ControlBody, ReqForwardSubCmd, ZRTPPacket, ZrtpCmd, MSG_AUDIO_RTP_DOWN,
};
use crate::protocol::call::{
    CallDirection, CallEffect, CallEndReason, CallEvent, CallSession, CallState,
};
use crate::protocol::messaging::{CallOutcome, ChatMessage, MessageKind};
use crate::protocol::session::Session;
use crate::services::call_media::{
    normalize_device_id, NetEq, NetEqConfig, NullOpusDecoder, NullOpusEncoder, OpusDecoderBackend,
    OpusEncoderBackend, FRAME_SAMPLES,
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
    pub peer_muted: bool,
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

const ENDED_CALL_TTL: Duration = Duration::from_secs(90);
const MAX_ENDED_CALLS: usize = 64;

#[derive(Default)]
struct CallManagerState {
    session: CallSession,
    peer_name: Option<String>,
    peer_avatar: Option<String>,
    local_media_uid: Option<u32>,
    peer_media_uid: Option<u32>,
    relay_servers: Vec<RelayServerAddr>,
    selected_relay: Option<RelayServerAddr>,
    public_p2p_addr: Option<String>,
    p2p_candidates: Vec<SocketAddr>,
    srtp_key: Option<SrtpKeyMaterial>,
    started_at_ms: Option<i64>,
    ring_timer_task: Option<tokio::task::JoinHandle<()>>,
    media_task: Option<tokio::task::JoinHandle<()>>,
    media_cmd_tx: Option<mpsc::UnboundedSender<MediaLoopCmd>>,
    stop_audio_flag: Option<Arc<AtomicBool>>,
    audio_threads: Vec<std::thread::JoinHandle<()>>,
    selected_mic_id: Option<String>,
    selected_speaker_id: Option<String>,
    peer_muted: bool,
    mic_test_stop: Option<Arc<AtomicBool>>,
    mic_test_thread: Option<std::thread::JoinHandle<()>>,
    audio_device_opened_count: usize,
    ended_call_ids: std::collections::VecDeque<(u32, Instant)>,
}

impl CallManagerState {
    fn prune_ended_calls(&mut self, now: Instant) {
        while let Some((_, ts)) = self.ended_call_ids.front() {
            if now.duration_since(*ts) > ENDED_CALL_TTL {
                self.ended_call_ids.pop_front();
            } else {
                break;
            }
        }
    }

    fn remember_ended_call(&mut self, call_id: u32) {
        if call_id == 0 {
            return;
        }
        let now = Instant::now();
        self.prune_ended_calls(now);
        self.ended_call_ids.retain(|(id, _)| *id != call_id);
        self.ended_call_ids.push_back((call_id, now));
        while self.ended_call_ids.len() > MAX_ENDED_CALLS {
            self.ended_call_ids.pop_front();
        }
    }

    fn is_ended_call(&mut self, call_id: u32) -> bool {
        if call_id == 0 {
            return false;
        }
        self.prune_ended_calls(Instant::now());
        self.ended_call_ids.iter().any(|(id, _)| *id == call_id)
    }
}

/// Coordinates the single active 1-1 ZRTC [`CallSession`] outside `CoreEngine::session` (`INV-9`).
pub struct CallManager {
    state: StdMutex<CallManagerState>,
    ring_timeout: Duration,
    media_watchdog_timeout: Duration,
    echo_probe_timeout: Duration,
    force_null_audio: bool,
    srtp_drop_count: AtomicU64,
    mic_volume_pct: Arc<AtomicU8>,
    speaker_volume_pct: Arc<AtomicU8>,
    auto_mic_agc: Arc<AtomicBool>,
    auto_speaker_agc: Arc<AtomicBool>,
    live_mic_level: Arc<AtomicU8>,
    live_spk_level: Arc<AtomicU8>,
    live_mic_gain_pct: Arc<AtomicU8>,
    live_spk_gain_pct: Arc<AtomicU8>,
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
            srtp_drop_count: AtomicU64::new(0),
            mic_volume_pct: Arc::new(AtomicU8::new(100)),
            speaker_volume_pct: Arc::new(AtomicU8::new(100)),
            auto_mic_agc: Arc::new(AtomicBool::new(true)),
            auto_speaker_agc: Arc::new(AtomicBool::new(true)),
            live_mic_level: Arc::new(AtomicU8::new(0)),
            live_spk_level: Arc::new(AtomicU8::new(0)),
            live_mic_gain_pct: Arc::new(AtomicU8::new(100)),
            live_spk_gain_pct: Arc::new(AtomicU8::new(100)),
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
            srtp_drop_count: AtomicU64::new(0),
            mic_volume_pct: Arc::new(AtomicU8::new(100)),
            speaker_volume_pct: Arc::new(AtomicU8::new(100)),
            auto_mic_agc: Arc::new(AtomicBool::new(true)),
            auto_speaker_agc: Arc::new(AtomicBool::new(true)),
            live_mic_level: Arc::new(AtomicU8::new(0)),
            live_spk_level: Arc::new(AtomicU8::new(0)),
            live_mic_gain_pct: Arc::new(AtomicU8::new(100)),
            live_spk_gain_pct: Arc::new(AtomicU8::new(100)),
        }
    }

    /// Number of SRTP packets dropped due to authentication failure (`ADV-3`).
    pub fn srtp_drop_count(&self) -> u64 {
        self.srtp_drop_count.load(Ordering::Relaxed)
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
        CallEndReason::HandledElsewhere => "handled_elsewhere",
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
        | CallEndReason::HandledElsewhere
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
        peer_muted: g.peer_muted,
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
        let resp = match request_call(&s, &peer_id, 0, &ExtendData::default()).await {
            Ok(r) => r,
            Err(err) => {
                eprintln!("[alolite::call] start_call failed: {err:#}");
                crate::logger::log_error("call::signal", &format!("start_call failed: {err}"));
                return Err(err);
            }
        };
        let call_id = match resp.call_id_u32() {
            Some(id) => id,
            None => {
                eprintln!(
                    "[alolite::call] start_call response missing callId (cmd={:?})",
                    resp.cmd
                );
                crate::logger::log_error(
                    "call::signal",
                    "start_call response missing numeric callId",
                );
                bail!("Thiếu callId từ máy chủ");
            }
        };
        let sess_id = resp
            .sess_id_str()
            .unwrap_or_else(|| format!("sess-{call_id}"));
        let srtp_mode = resp.parsed_extend_data().srtp_mode;
        let relays = resp.relay_servers();
        let p2p_cands = resp.p2p_candidates();
        let srtp_key = resp.srtp_key_material();
        // Outgoing 11300 returns 32-bit VoIP media UIDs: `fromId` (caller) and `toId` (callee).
        // Verified in ZaloCall ZCallInfo::_prepareCallConfig (0x1000d4552).
        let local_media_uid = resp.from_media_uid_u32();
        let peer_media_uid = resp.to_media_uid_u32();
        crate::logger::call_debug(&format!(
            "[DEBUG-call] 11300_ok call_id={call_id} sess_len={} relays={} p2p={} has_srtp={} has_raw_rtp_ip={} has_from_uid={} has_to_uid={}",
            sess_id.len(),
            relays.len(),
            p2p_cands.len(),
            srtp_key.is_some(),
            resp.rtp_ip.is_some(),
            local_media_uid.is_some(),
            peer_media_uid.is_some(),
        ));

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
            g.local_media_uid = local_media_uid;
            g.peer_media_uid = peer_media_uid;
            g.relay_servers = relays;
            g.selected_relay = None;
            g.public_p2p_addr = None;
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
        self.persist_call_audio_settings();
        Ok(())
    }

    /// Adjust live software microphone and speaker volume percentages (`0..=150`).
    pub fn set_call_volume(&self, mic_pct: u8, speaker_pct: u8) {
        self.call_manager
            .mic_volume_pct
            .store(mic_pct.min(150), Ordering::Relaxed);
        self.call_manager
            .speaker_volume_pct
            .store(speaker_pct.min(150), Ordering::Relaxed);
    }

    /// Configure Auto (AGC) vs Manual mode and volume percentages (`0..=150`) for mic and speaker.
    pub fn set_call_audio_config(
        &self,
        auto_mic: bool,
        auto_speaker: bool,
        mic_pct: u8,
        speaker_pct: u8,
    ) {
        self.call_manager
            .auto_mic_agc
            .store(auto_mic, Ordering::Relaxed);
        self.call_manager
            .auto_speaker_agc
            .store(auto_speaker, Ordering::Relaxed);
        self.set_call_volume(mic_pct, speaker_pct);
        self.persist_call_audio_settings();
    }

    /// Synchronize persistent [`super::settings::AppSettings`] into runtime `CallManager` state.
    pub fn apply_call_audio_settings(&self, settings: &crate::core::settings::AppSettings) {
        let norm_mic = normalize_device_id(Some(settings.selected_mic_device.as_str()))
            .map(ToString::to_string);
        let norm_spk = normalize_device_id(Some(settings.selected_speaker_device.as_str()))
            .map(ToString::to_string);
        let mic_pct = u8::try_from(settings.mic_gain_percent.min(150)).unwrap_or(100);
        let spk_pct = u8::try_from(settings.speaker_gain_percent.min(150)).unwrap_or(100);

        self.call_manager
            .auto_mic_agc
            .store(settings.auto_mic_agc, Ordering::Relaxed);
        self.call_manager
            .auto_speaker_agc
            .store(settings.auto_speaker_agc, Ordering::Relaxed);
        self.set_call_volume(mic_pct, spk_pct);

        let tx = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let changed = g.selected_mic_id != norm_mic || g.selected_speaker_id != norm_spk;
            g.selected_mic_id = norm_mic.clone();
            g.selected_speaker_id = norm_spk.clone();
            changed.then(|| g.media_cmd_tx.clone()).flatten()
        };
        if let Some(tx) = tx {
            let _ = tx.send(MediaLoopCmd::SetDevices {
                mic_id: norm_mic,
                speaker_id: norm_spk,
            });
        }
    }

    fn persist_call_audio_settings(&self) {
        if self.sink.is_headless() || self.call_manager.force_null_audio {
            return;
        }
        let (mic_id, spk_id) = self.call_manager.selected_devices();
        let mut settings = crate::core::settings::AppSettings::load();
        settings.selected_mic_device = mic_id.unwrap_or_default();
        settings.selected_speaker_device = spk_id.unwrap_or_default();
        settings.auto_mic_agc = self.call_manager.auto_mic_agc.load(Ordering::Relaxed);
        settings.auto_speaker_agc = self.call_manager.auto_speaker_agc.load(Ordering::Relaxed);
        settings.mic_gain_percent =
            u32::from(self.call_manager.mic_volume_pct.load(Ordering::Relaxed));
        settings.speaker_gain_percent =
            u32::from(self.call_manager.speaker_volume_pct.load(Ordering::Relaxed));
        settings.save();
    }

    /// Start a pre-call local microphone test that emits [`CoreEvent::CallAudioLevels`]
    /// every 100 ms while `CallPhase::Idle`.
    ///
    /// # Errors
    /// Returns an error if a call is currently active.
    pub fn start_call_mic_test(
        self: &Arc<Self>,
        mic_id: Option<String>,
        auto_mic: bool,
        mic_pct: u8,
    ) -> Result<()> {
        self.stop_call_mic_test();
        let norm_mic = normalize_device_id(mic_id.as_deref()).map(ToString::to_string);
        {
            let g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if g.session.state() != CallState::Idle {
                anyhow::bail!("Không thể thử micro khi đang có cuộc gọi");
            }
        }
        self.call_manager
            .auto_mic_agc
            .store(auto_mic, Ordering::Relaxed);
        self.call_manager
            .mic_volume_pct
            .store(mic_pct.min(150), Ordering::Relaxed);

        #[cfg(all(feature = "call", target_os = "linux"))]
        if !self.sink.is_headless() && !self.call_manager.force_null_audio {
            use crate::services::call_media::{
                Apm, AudioLeveler, LevelerConfig, PulseCapture, CHANNELS, SAMPLE_RATE_HZ,
            };
            let stop_flag = Arc::new(AtomicBool::new(false));
            let stop_worker = Arc::clone(&stop_flag);
            let engine = Arc::clone(self);
            let auto_flag = Arc::clone(&self.call_manager.auto_mic_agc);
            let vol_pct = Arc::clone(&self.call_manager.mic_volume_pct);
            let handle = std::thread::Builder::new()
                .name("alolite-mic-test".to_string())
                .spawn(move || {
                    let mut cap = match PulseCapture::open(norm_mic.as_deref()) {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    let apm =
                        Apm::new(SAMPLE_RATE_HZ as i32, CHANNELS as i32, false, true, true).ok();
                    let mut leveler = AudioLeveler::new(LevelerConfig::mic(), None);
                    let mut buf = [0i16; FRAME_SAMPLES];
                    let mut last_emit = std::time::Instant::now();
                    while !stop_worker.load(Ordering::Relaxed) {
                        if cap.read_frame(&mut buf).is_err() {
                            std::thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        if let Some(ref a) = apm {
                            a.process_capture(&mut buf);
                        }
                        let gain_pct = if auto_flag.load(Ordering::Relaxed) {
                            leveler.process(&mut buf);
                            leveler.gain_pct()
                        } else {
                            let pct = vol_pct.load(Ordering::Relaxed);
                            scale_pcm_pct(&mut buf, pct);
                            pct
                        };
                        if last_emit.elapsed() >= Duration::from_millis(100) {
                            last_emit = std::time::Instant::now();
                            engine.emit(CoreEvent::CallAudioLevels {
                                mic_level: pcm_level_pct(&buf),
                                speaker_level: 0,
                                mic_gain_pct: gain_pct,
                                speaker_gain_pct: engine
                                    .call_manager
                                    .live_spk_gain_pct
                                    .load(Ordering::Relaxed),
                            });
                        }
                    }
                })
                .ok();
            if let Some(jh) = handle {
                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.mic_test_stop = Some(stop_flag);
                g.mic_test_thread = Some(jh);
            }
        }
        #[cfg(not(all(feature = "call", target_os = "linux")))]
        let _ = norm_mic;

        Ok(())
    }

    /// Stop any active pre-call microphone test and reset the live mic meter to 0.
    pub fn stop_call_mic_test(&self) {
        let (stop, thread) = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (g.mic_test_stop.take(), g.mic_test_thread.take())
        };
        if let Some(flag) = stop {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(jh) = thread {
            let _ = jh.join();
        }
        self.emit(CoreEvent::CallAudioLevels {
            mic_level: 0,
            speaker_level: 0,
            mic_gain_pct: self.call_manager.live_mic_gain_pct.load(Ordering::Relaxed),
            speaker_gain_pct: self.call_manager.live_spk_gain_pct.load(Ordering::Relaxed),
        });
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
                // Incoming invite (`act: "request"` / `401`) carries `uidTo` (callee's local media UID)
                // and `uidFrom` (caller's peer media UID). Verified in ZaloCall
                // ZCallInfo::_prepareCallConfig(ZIncomingCallInfo) (0x1000d63ba: `pshufd $0xe1`).
                let local_media_uid = signal.to_media_uid_u32();
                let peer_media_uid = signal.from_media_uid_u32();
                let (resolved_name, resolved_avatar) = self.resolve_call_peer(
                    &peer_id,
                    signal.peer_display_name(),
                    signal.peer_avatar_url(),
                );

                let mut g = self
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if g.is_ended_call(call_id) {
                    crate::logger::call_debug(&format!(
                        "[DEBUG-call] ignored late 401 for ended call_id={call_id}"
                    ));
                    return;
                }
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
                            g.local_media_uid = local_media_uid;
                            g.peer_media_uid = peer_media_uid;
                            g.relay_servers = relays;
                            g.selected_relay = None;
                            g.public_p2p_addr = None;
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
                if !accepted || g.session.state() == CallState::IncomingRinging {
                    g.remember_ended_call(call_id);
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
                g.remember_ended_call(call_id);
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
                g.remember_ended_call(call_id);
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
            CallSignalCode::Conf => {
                if let Some(peer_muted) = signal.peer_muted_update() {
                    let changed = {
                        let mut g = self
                            .call_manager
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if g.session.call_id() == Some(call_id) && g.peer_muted != peer_muted {
                            g.peer_muted = peer_muted;
                            true
                        } else {
                            false
                        }
                    };
                    if changed {
                        self.emit_call_snapshot(None);
                    }
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
            peer_muted: snap.peer_muted,
            started_at_ms: snap.started_at_ms,
            end_reason: snap.end_reason,
        });
    }

    async fn execute_call_effects(self: &Arc<Self>, s_snap: &Session, effects: Vec<CallEffect>) {
        crate::logger::call_debug(&format!(
            "[DEBUG-call] execute_effects count={} effects={effects:?}",
            effects.len()
        ));
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
                        if let Err(err) = answer_call(
                            &s_clone,
                            &peer_id,
                            call_id,
                            &sess_id,
                            1,
                            &ExtendData::default(),
                        )
                        .await
                        {
                            crate::logger::log_error(
                                "call::signal",
                                &format!(
                                    "busy reject answer_call failed (call_id={call_id}): {err}"
                                ),
                            );
                        }
                    });
                }
                CallEffect::StartZrtpAndMedia => {
                    self.stop_call_mic_test();
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
                        // Best-effort notify media loop; loop may already be exiting.
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
                        // Best-effort notify media loop; loop may already be exiting.
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
                g.remember_ended_call(cid);
                g.peer_muted = false;
                g.peer_name = None;
                g.peer_avatar = None;
                g.local_media_uid = None;
                g.peer_media_uid = None;
                g.relay_servers.clear();
                g.selected_relay = None;
                g.public_p2p_addr = None;
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
                            if let Err(err) =
                                log_end_call(s_snap, &peer_id, call_id, &sess_id, duration_secs, 0)
                                    .await
                            {
                                crate::logger::log_error(
                                    "call::signal",
                                    &format!("log_end_call failed (call_id={call_id}): {err}"),
                                );
                            }
                        }
                    }
                    CallEffect::CallFinished { reason, .. } => {
                        end_reason_for_idle = Some(reason);
                    }
                    CallEffect::StateChanged(CallState::Idle) => {
                        if call_id != 0
                            && !peer_id.is_empty()
                            && end_reason_for_idle != Some(CallEndReason::HandledElsewhere)
                        {
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
        if let Err(err) = tokio::task::spawn_blocking(move || match MessageStore::open(&db_path) {
            Ok(mut store) => {
                if let Err(err) = store.upsert_message(&sm) {
                    crate::logger::log_error("core::call_history_persist", &err.to_string());
                }
            }
            Err(err) => {
                crate::logger::log_error(
                    "core::call_history_persist",
                    &format!("MessageStore::open failed: {err}"),
                );
            }
        })
        .await
        {
            crate::logger::log_error(
                "core::call_history_persist",
                &format!("spawn_blocking join failed: {err}"),
            );
        }
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
        crate::logger::call_debug(&format!(
            "[DEBUG-call] send_signal sig={sig:?} call_id={call_id}"
        ));

        let res = match sig {
            CallSignalCode::Request => {
                let (server_addr, extra_info) = {
                    let g = self
                        .call_manager
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    build_signal_relay_and_p2p_fields(
                        g.selected_relay.as_ref(),
                        &g.relay_servers,
                        g.public_p2p_addr.as_deref(),
                    )
                };
                let ext = ExtendData {
                    server_addr,
                    extra_info,
                    ..ExtendData::default()
                };
                send_request(s_snap, &peer_id, call_id, &sess_id, &ext).await
            }
            CallSignalCode::RingRing => {
                send_ring_ring(s_snap, &peer_id, call_id, &sess_id, 0).await
            }
            CallSignalCode::Answer => {
                let (status, server_addr, extra_info) = {
                    let g = self
                        .call_manager
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let st = if g.session.state() == CallState::Ending {
                        1
                    } else {
                        0
                    };
                    let (srv, p2p_info) = build_signal_relay_and_p2p_fields(
                        g.selected_relay.as_ref(),
                        &g.relay_servers,
                        g.public_p2p_addr.as_deref(),
                    );
                    (st, srv, p2p_info)
                };
                let ext = ExtendData {
                    server_addr,
                    extra_info,
                    ..ExtendData::default()
                };
                answer_call(s_snap, &peer_id, call_id, &sess_id, status, &ext).await
            }
            CallSignalCode::AnswerAck => send_answer_ack(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::Cancel => cancel_call(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::EndCall => {
                send_end_call_http(s_snap, &peer_id, call_id, &sess_id, 0).await
            }
            CallSignalCode::HoldReq => hold_request(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::UnholdReq => unhold_request(s_snap, &peer_id, call_id, &sess_id).await,
            CallSignalCode::Conf => {
                // In Zalo's 12006 `/api/voicecall/conf` contract, `isOnMic: 1` means mic is muted
                // on the peer's UI and `0` clears the mute indicator.
                send_conf(s_snap, &peer_id, call_id, false, is_muted, None).await
            }
            _ => return,
        };

        crate::logger::call_debug(&format!(
            "[DEBUG-call] send_signal_done sig={sig:?} ok={}",
            res.is_ok()
        ));
        if let Err(e) = res {
            crate::logger::call_debug(&format!("[DEBUG-call] send_signal_err sig={sig:?} err={e}"));
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

        let (
            call_id,
            sess_id,
            peer_id,
            is_caller,
            local_media_uid,
            peer_media_uid,
            relays,
            p2p_cands,
            srtp_key,
            init_mic,
            init_spk,
        ) = {
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
                g.local_media_uid,
                g.peer_media_uid,
                g.relay_servers.clone(),
                g.p2p_candidates.clone(),
                g.srtp_key.clone(),
                g.selected_mic_id.clone(),
                g.selected_speaker_id.clone(),
            )
        };
        // Use the 32-bit VoIP media UIDs (`fromId`/`toId` or `uidTo`/`uidFrom`) assigned by Zalo's
        // signaling server (`_sendRequestInitZRTPToServer` @ `0x10019780e`). Fall back to parsed
        // account IDs only in synthetic unit tests where media UIDs are omitted.
        let uid: u32 = local_media_uid.unwrap_or_else(|| {
            s_snap
                .login
                .as_ref()
                .and_then(|l| l.uin.parse::<u64>().ok())
                .unwrap_or(0) as u32
        });
        let peer_uid: u32 =
            peer_media_uid.unwrap_or_else(|| peer_id.parse::<u64>().unwrap_or(0) as u32);
        let is_headless = self.sink.is_headless();
        let force_null_audio = self.call_manager.force_null_audio;
        let echo_timeout = self.call_manager.echo_probe_timeout;
        let watchdog_timeout = self.call_manager.media_watchdog_timeout;
        crate::logger::call_debug(&format!(
            "[DEBUG-call] media_loop_start call_id={call_id} is_caller={is_caller} relays={} has_local_media_uid={} has_peer_media_uid={} has_srtp={} init_mic={:?} init_spk={:?}",
            relays.len(),
            local_media_uid.is_some(),
            peer_media_uid.is_some(),
            srtp_key.is_some(),
            init_mic,
            init_spk,
        ));

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<MediaLoopCmd>();
        let stop_audio_flag = Arc::new(AtomicBool::new(false));
        let stop_audio_for_loop = Arc::clone(&stop_audio_flag);
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

            let relay_addr: Option<SocketAddr> = if relays.is_empty() {
                None
            } else {
                select_relay_with_echo(&socket, &relays, uid, echo_timeout).await
            };
            crate::logger::call_debug(&format!(
                "[DEBUG-call] echo_done call_id={call_id} relay_found={} relay_addr={:?}",
                relay_addr.is_some(),
                relay_addr,
            ));

            // Send InitZRTP (cmd = 11 caller / 12 callee) when relay is available.
            let mut server_token = 0u32;
            let mut zrtp_ready = false;
            if let Some(target) = relay_addr {
                let init_pkt =
                    ZRTPPacket::init_call(is_caller, 0, uid, call_id, peer_uid, sess_id.clone());
                // Best-effort initial ZRTP handshake packet; timeout handles loss.
                let _ = socket.send_to(&init_pkt.encode(), target).await;

                // Wait for InitResponse (0x02, cmd = 11/12) and verify res == 0 before marking media ready.
                let mut buf = [0u8; 2048];
                let deadline = tokio::time::Instant::now() + echo_timeout;
                while let Ok(Ok((n, _))) =
                    tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await
                {
                    let decoded = ZRTPPacket::decode(&buf[..n]);
                    crate::logger::call_debug(&format!(
                        "[DEBUG-call] init_rx n={n} decoded_ok={} pkt={:?}",
                        decoded.is_ok(),
                        decoded.as_ref().ok().map(|p| match p {
                            ZRTPPacket::Control {
                                is_reply,
                                cmd,
                                sub_cmd,
                                ..
                            } => format!("Control(reply={is_reply},cmd={cmd},sub={sub_cmd})"),
                            _ => "Other".to_string(),
                        })
                    ));
                    if let Ok(ZRTPPacket::Control {
                        is_reply: true,
                        cmd,
                        token,
                        body: ControlBody::Raw(bytes),
                        ..
                    }) = decoded
                    {
                        if cmd != ZrtpCmd::InitCallCaller as u16
                            && cmd != ZrtpCmd::InitCallCallee as u16
                        {
                            // Ignore late Echo replies (`cmd = 5`) from slower candidate relays.
                            continue;
                        }
                        if is_init_response_ok(&bytes) {
                            server_token = if bytes.len() >= 12 {
                                u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]])
                            } else {
                                token
                            };
                            let public_addr = parse_init_response_public_addr(&bytes);
                            let matched_relay = relays
                                .iter()
                                .find(|r| {
                                    r.ip == target.ip().to_string() && r.port == target.port()
                                })
                                .cloned()
                                .unwrap_or_else(|| RelayServerAddr {
                                    ip: target.ip().to_string(),
                                    port: target.port(),
                                    server_type: 1,
                                    rtcp_addr: None,
                                    rtp_ipv6: None,
                                    rtcp_ipv6: None,
                                });
                            {
                                let mut g = engine
                                    .call_manager
                                    .state
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                g.selected_relay = Some(matched_relay);
                                g.public_p2p_addr = public_addr;
                            }
                            zrtp_ready = true;
                        } else {
                            crate::logger::call_debug(&format!(
                                "[DEBUG-call] init_resp_non_zero len={} prefix={:02x?}",
                                bytes.len(),
                                &bytes[..bytes.len().min(8)]
                            ));
                            crate::logger::log_error(
                                "call::zrtp",
                                "InitResponse returned non-zero status",
                            );
                        }
                        break;
                    }
                }
            }
            crate::logger::call_debug(&format!(
                "[DEBUG-call] zrtp_done call_id={call_id} zrtp_ready={zrtp_ready}"
            ));

            if !relays.is_empty() && !zrtp_ready {
                crate::logger::log_error(
                    "call::zrtp",
                    "relay Echo or ZRTP InitResponse failed/timed out; ending call",
                );
                let effs = {
                    let mut g = engine
                        .call_manager
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    g.session
                        .apply(CallEvent::MediaWatchdogTimeout)
                        .unwrap_or_default()
                };
                if !effs.is_empty() {
                    let eng = Arc::clone(&engine);
                    let sc = s_clone.clone();
                    tokio::spawn(async move {
                        eng.execute_call_effects(&sc, effs).await;
                    });
                }
                return;
            }

            // Emit MediaReady only after ZRTP InitResponse (res == 0) or when running relay-less P2P in tests.
            if zrtp_ready || (relays.is_empty() && !p2p_cands.is_empty()) {
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
            }

            // EDGE-5: never open hardware audio devices in headless mode.
            let muted_flag = Arc::new(AtomicBool::new(false));
            let held_flag = Arc::new(AtomicBool::new(false));
            let reopen_mic_flag = Arc::new(AtomicBool::new(false));
            let reopen_spk_flag = Arc::new(AtomicBool::new(false));
            let pending_devices = Arc::new(StdMutex::new((init_mic, init_spk)));
            let (tx_pcm_up, mut rx_pcm_up) = mpsc::channel::<[i16; FRAME_SAMPLES]>(16);
            let neteq = Arc::new(StdMutex::new(create_neteq()));

            let spawned_threads = if !is_headless
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
                spawn_audio_threads(
                    call_id,
                    Arc::clone(&muted_flag),
                    Arc::clone(&held_flag),
                    Arc::clone(&stop_audio_for_loop),
                    Arc::clone(&reopen_mic_flag),
                    Arc::clone(&reopen_spk_flag),
                    Arc::clone(&pending_devices),
                    Arc::clone(&engine.call_manager.mic_volume_pct),
                    Arc::clone(&engine.call_manager.speaker_volume_pct),
                    Arc::clone(&engine.call_manager.auto_mic_agc),
                    Arc::clone(&engine.call_manager.auto_speaker_agc),
                    Arc::clone(&engine.call_manager.live_mic_level),
                    Arc::clone(&engine.call_manager.live_spk_level),
                    Arc::clone(&engine.call_manager.live_mic_gain_pct),
                    Arc::clone(&engine.call_manager.live_spk_gain_pct),
                    tx_pcm_up,
                    Arc::clone(&neteq),
                )
            } else {
                drop(tx_pcm_up);
                Vec::new()
            };
            if !spawned_threads.is_empty() {
                let mut g = engine
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.audio_threads = spawned_threads;
            }

            let mut srtp_tx = srtp_key
                .as_ref()
                .and_then(|k| SrtpContext::from_combined(k.as_bytes()).ok());
            let mut srtp_rx = srtp_key
                .as_ref()
                .and_then(|k| SrtpContext::from_combined(k.as_bytes()).ok());

            let mut encoder = create_opus_encoder();
            let mut opus_out_buf = [0u8; 512];
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
                // Best-effort REQ_FORWARD subCmd=8 channel prime (`re-notes/16` §3).
                let _ = socket.send_to(&prime.encode(), target).await;
            }

            let mut rtp_seq: u16 = 1;
            let mut tx_octets: u32 = 0;
            let mut last_rx_at = Instant::now();
            let mut recv_buf = [0u8; 2048];
            let mut ping_tick = tokio::time::interval(Duration::from_secs(5));
            let mut route_probe_tick = tokio::time::interval(Duration::from_secs(1));
            let mut change_addr_tick = tokio::time::interval(Duration::from_millis(500));
            let mut watchdog_tick = tokio::time::interval(Duration::from_millis(200));
            let mut levels_tick = tokio::time::interval(Duration::from_millis(100));
            let mut stats_tick = tokio::time::interval(Duration::from_secs(1));
            let mut tx_rtp: u64 = 0;
            let mut tx_enc_err: u64 = 0;
            let mut tx_srtp_err: u64 = 0;
            let mut rx_udp: u64 = 0;
            let mut rx_relay_audio: u64 = 0;
            let mut rx_relay_other: u64 = 0;
            let mut rx_p2p_audio: u64 = 0;
            let mut rx_decode_ok: u64 = 0;

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
                                let (mic_changed, spk_changed) = {
                                    let mut dev_g = pending_devices
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner());
                                    let mc = dev_g.0 != mic_id;
                                    let sc = dev_g.1 != speaker_id;
                                    *dev_g = (mic_id, speaker_id);
                                    (mc, sc)
                                };
                                if mic_changed {
                                    reopen_mic_flag.store(true, Ordering::Release);
                                }
                                if spk_changed {
                                    reopen_spk_flag.store(true, Ordering::Release);
                                }
                            }
                            MediaLoopCmd::Stop { send_end_call_pkt } => {
                                if send_end_call_pkt {
                                    if let Some(target) = relay_addr {
                                        let end_pkt = ZRTPPacket::end_call(2, uid, server_token, call_id);
                                        // Best-effort UDP END_CALL packet during teardown.
                                        let _ = socket.send_to(&end_pkt.encode(), target).await;
                                    }
                                }
                                break;
                            }
                        }
                    }
                    Some(pcm) = rx_pcm_up.recv() => {
                        if (relay_addr.is_some() || p2p.active_peer().is_some())
                            && !held_flag.load(Ordering::Relaxed)
                        {
                            // Send silent Opus frames while muted so the peer's media watchdog
                            // does not treat a muted mic as a dropped connection.
                            let effective_pcm = if muted_flag.load(Ordering::Relaxed) {
                                [0i16; FRAME_SAMPLES]
                            } else {
                                pcm
                            };
                            match encoder.encode(&effective_pcm, &mut opus_out_buf) {
                                Ok(enc_len) if enc_len > 0 => {
                                    let rtp_pkt =
                                        build_minimal_rtp(rtp_seq, uid, &opus_out_buf[..enc_len]);
                                    rtp_seq = rtp_seq.wrapping_add(1);
                                    let wire_rtp = if let Some(ctx) = srtp_tx.as_mut() {
                                        match ctx.protect_rtp(&rtp_pkt) {
                                            Ok(pkt) => pkt,
                                            Err(e) => {
                                                tx_srtp_err += 1;
                                                crate::logger::log_error(
                                                    "call::srtp",
                                                    &format!("SRTP protect failed, dropping RTP packet: {e}"),
                                                );
                                                continue;
                                            }
                                        }
                                    } else {
                                        rtp_pkt
                                    };
                                    tx_rtp += 1;
                                    tx_octets = tx_octets.wrapping_add(enc_len as u32);
                                    if tx_rtp == 1 {
                                        crate::logger::call_debug(&format!(
                                            "[DEBUG-call] first_tx_rtp seq={rtp_seq} ssrc={uid} has_token={} opus_len={enc_len} wire_len={}",
                                            server_token != 0,
                                            wire_rtp.len(),
                                        ));
                                    }
                                    let outbound = p2p.build_outbound_audio(wire_rtp);
                                    dispatch_p2p_outbound(&socket, relay_addr, vec![outbound]).await;
                                }
                                Ok(_) => {}
                                Err(_) => {
                                    tx_enc_err += 1;
                                }
                            }
                        }
                    }
                    Ok((n, src)) = socket.recv_from(&mut recv_buf) => {
                        rx_udp += 1;
                        match ZRTPPacket::decode(&recv_buf[..n]) {
                            Ok(ZRTPPacket::RelayMediaDown { msg_type, payload }) => {
                                if msg_type == MSG_AUDIO_RTP_DOWN {
                                    rx_relay_audio += 1;
                                    if !held_flag.load(Ordering::Relaxed)
                                        && decode_and_queue_audio(
                                            &payload,
                                            srtp_rx.as_mut(),
                                            &neteq,
                                            &engine.call_manager.srtp_drop_count,
                                        )
                                    {
                                        rx_decode_ok += 1;
                                        last_rx_at = Instant::now();
                                        p2p.on_relay_audio_received(now_ms());
                                    }
                                } else {
                                    rx_relay_other += 1;
                                    if rx_relay_other <= 3 {
                                        crate::logger::call_debug(&format!(
                                            "[DEBUG-call] rx_relay_other msg_type={msg_type} len={}",
                                            payload.len()
                                        ));
                                    }
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
                                match outcome {
                                    P2pRxOutcome::AudioMedia(payload) => {
                                        rx_p2p_audio += 1;
                                        if !held_flag.load(Ordering::Relaxed)
                                            && decode_and_queue_audio(
                                                &payload,
                                                srtp_rx.as_mut(),
                                                &neteq,
                                                &engine.call_manager.srtp_drop_count,
                                            )
                                        {
                                            rx_decode_ok += 1;
                                            last_rx_at = Instant::now();
                                        }
                                    }
                                    P2pRxOutcome::Handled | P2pRxOutcome::AudioRtcp(_) => {
                                        last_rx_at = Instant::now();
                                    }
                                    P2pRxOutcome::Drop => {}
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
                                    last_rx_at = Instant::now();
                                    if token != 0 {
                                        server_token = token;
                                    }
                                    p2p.on_server_reply(cmd, token);
                                } else if cmd == ZrtpCmd::ReqForward as u16 {
                                    last_rx_at = Instant::now();
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
                            // Best-effort keepalive ping to relay server.
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
                    _ = levels_tick.tick() => {
                        if !is_headless {
                            engine.sink.emit(CoreEvent::CallAudioLevels {
                                mic_level: engine.call_manager.live_mic_level.load(Ordering::Relaxed),
                                speaker_level: engine.call_manager.live_spk_level.load(Ordering::Relaxed),
                                mic_gain_pct: engine.call_manager.live_mic_gain_pct.load(Ordering::Relaxed),
                                speaker_gain_pct: engine.call_manager.live_spk_gain_pct.load(Ordering::Relaxed),
                            });
                        }
                    }
                    _ = stats_tick.tick() => {
                        if tx_rtp > 0 && (relay_addr.is_some() || p2p.active_peer().is_some()) {
                            let rtp_ts = u32::from(rtp_seq).wrapping_mul(FRAME_SAMPLES as u32);
                            let rtcp = build_audio_rtcp_sr_sdes(
                                uid,
                                now_ms(),
                                rtp_ts,
                                tx_rtp as u32,
                                tx_octets,
                            );
                            let outbound = p2p.build_outbound_rtcp(rtcp);
                            dispatch_p2p_outbound(&socket, relay_addr, vec![outbound]).await;
                        }
                        let srtp_drop = engine.call_manager.srtp_drop_count.load(Ordering::Relaxed);
                        let (nq, ncarry) = {
                            let g = neteq.lock().unwrap_or_else(|e| e.into_inner());
                            (g.queued_packets(), g.carry_samples())
                        };
                        crate::logger::call_debug(&format!(
                            "[DEBUG-call] media_stats tx_rtp={tx_rtp} tx_enc_err={tx_enc_err} tx_srtp_err={tx_srtp_err} rx_udp={rx_udp} rx_relay_aud={rx_relay_audio} rx_relay_oth={rx_relay_other} rx_p2p_aud={rx_p2p_audio} rx_dec_ok={rx_decode_ok} srtp_drop={srtp_drop} neteq_q={nq} neteq_carry={ncarry} p2p_active={} srtp_tx={} srtp_rx={}",
                            p2p.active_peer().is_some(),
                            srtp_tx.is_some(),
                            srtp_rx.is_some(),
                        ));
                    }
                    _ = watchdog_tick.tick() => {
                        // Do not fire MediaWatchdogTimeout while the call is intentionally held,
                        // since both endpoints may pause audio RTP during hold.
                        if held_flag.load(Ordering::Relaxed) {
                            last_rx_at = Instant::now();
                            continue;
                        }
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

            stop_audio_for_loop.store(true, Ordering::Relaxed);
            let remaining_audio = {
                let mut g = engine
                    .call_manager
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                std::mem::take(&mut g.audio_threads)
            };
            if !remaining_audio.is_empty() {
                // Best-effort join on blocking pool so Tokio worker thread is never blocked.
                let _ = tokio::task::spawn_blocking(move || {
                    for jh in remaining_audio {
                        if jh.join().is_err() {
                            crate::logger::log_error("call::audio", "audio thread panicked");
                        }
                    }
                })
                .await;
            }
        });

        let mut g = self
            .call_manager
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.media_cmd_tx = Some(cmd_tx);
        g.media_task = Some(handle);
        g.stop_audio_flag = Some(stop_audio_flag);
    }

    async fn stop_media_loop(&self, send_end_call_pkt: bool) {
        let (tx, handle, stop_flag, audio_threads) = {
            let mut g = self
                .call_manager
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (
                g.media_cmd_tx.take(),
                g.media_task.take(),
                g.stop_audio_flag.take(),
                std::mem::take(&mut g.audio_threads),
            )
        };
        if let Some(tx) = tx {
            // Best-effort signal media loop to stop; task may already have exited.
            let _ = tx.send(MediaLoopCmd::Stop { send_end_call_pkt });
        }
        if let Some(h) = handle {
            let abort_handle = h.abort_handle();
            if tokio::time::timeout(Duration::from_secs(2), h)
                .await
                .is_err()
            {
                crate::logger::log_error(
                    "call::media",
                    "media loop teardown timed out; aborting task",
                );
                abort_handle.abort();
            }
        }
        if let Some(flag) = stop_flag {
            flag.store(true, Ordering::Relaxed);
        }
        if !audio_threads.is_empty() {
            // Best-effort join on blocking pool so Tokio worker thread is never blocked.
            let _ = tokio::task::spawn_blocking(move || {
                for jh in audio_threads {
                    if jh.join().is_err() {
                        crate::logger::log_error("call::audio", "audio thread panicked");
                    }
                }
            })
            .await;
        }
    }
}

fn is_init_response_ok(payload: &[u8]) -> bool {
    if payload.len() >= 4 {
        i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) == 0
    } else if !payload.is_empty() {
        payload.iter().all(|&b| b == 0)
    } else {
        false
    }
}

/// Extract the optional `u16le`-prefixed public reflex address (`"ip|port"`) from `InitResponse`
/// (`ZRTPPacket::_parsePacketInternal` @ `0x10016c32a`: packet offset `+0x21` = body offset `+12`).
fn parse_init_response_public_addr(payload: &[u8]) -> Option<String> {
    if payload.len() < 14 {
        return None;
    }
    let len = u16::from_le_bytes([payload[12], payload[13]]) as usize;
    if len == 0 || payload.len() < 14 + len {
        return None;
    }
    std::str::from_utf8(&payload[14..14 + len])
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn build_signal_relay_and_p2p_fields(
    selected_relay: Option<&RelayServerAddr>,
    relays: &[RelayServerAddr],
    public_p2p_addr: Option<&str>,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    let mut ordered: Vec<&RelayServerAddr> = Vec::with_capacity(relays.len() + 1);
    if let Some(sel) = selected_relay {
        ordered.push(sel);
    }
    for r in relays {
        if !ordered
            .iter()
            .any(|existing| existing.ip == r.ip && existing.port == r.port)
        {
            ordered.push(r);
        }
    }
    let server_addr = if ordered.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(
            ordered
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "rtp": r.rtp_host_port(),
                        "rtcp": r.rtcp_host_port(),
                        "rtpIPv6": r.rtp_ipv6.as_deref().unwrap_or(""),
                        "rtcpIPv6": r.rtcp_ipv6.as_deref().unwrap_or(""),
                        "tpType": 0,
                    })
                })
                .collect(),
        ))
    };
    let extra_info = public_p2p_addr
        .filter(|s| !s.is_empty())
        .map(|addr| serde_json::json!({ "p2pAddr": addr }));
    (server_addr, extra_info)
}

async fn select_relay_with_echo(
    socket: &UdpSocket,
    relays: &[RelayServerAddr],
    _uid: u32,
    timeout: Duration,
) -> Option<SocketAddr> {
    if relays.is_empty() {
        return None;
    }

    // Zalo relay Echo (cmd=5) requires magic fromId=0x003E9765 and callId=0x012F833C.
    let echo_pkt = ZRTPPacket::Control {
        is_reply: false,
        magic: 1,
        field02: 0,
        seq: 0,
        uid: 0x003E_9765,
        token: 0x012F_833C,
        cmd: ZrtpCmd::Echo as u16,
        sub_cmd: 0,
        body: ControlBody::Echo(unix_now().to_string()),
    };
    let encoded = echo_pkt.encode();
    for r in relays {
        if let Ok(addr) = format!("{}:{}", r.ip, r.port).parse::<SocketAddr>() {
            // Best-effort Echo probe to candidate relay server.
            let _ = socket.send_to(&encoded, addr).await;
        }
    }

    let mut buf = [0u8; 512];
    if let Ok(Ok((_n, from))) = tokio::time::timeout(timeout, socket.recv_from(&mut buf)).await {
        return Some(from);
    }
    None
}

fn build_minimal_rtp(seq: u16, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    let effective_ssrc = if ssrc != 0 { ssrc } else { 1 };
    let mut rtp = Vec::with_capacity(20 + payload.len());
    // Match ZaloCall 1-1 audio RTP header: V=2, X=1 (0x90), PT=112 (marker on first packet),
    // 16 kHz timestamp (+320/frame), and RFC 8285 1-byte TWCC extension (id=5, len=2 bytes).
    rtp.push(0x90);
    rtp.push(if seq == 1 { 0x80 | 112 } else { 112 });
    rtp.extend_from_slice(&seq.to_be_bytes());
    rtp.extend_from_slice(&((seq as u32).wrapping_mul(FRAME_SAMPLES as u32)).to_be_bytes());
    rtp.extend_from_slice(&effective_ssrc.to_be_bytes());
    rtp.extend_from_slice(&[0xbe, 0xde, 0x00, 0x01, 0x51]);
    rtp.extend_from_slice(&seq.to_be_bytes());
    rtp.push(0x00);
    rtp.extend_from_slice(payload);
    rtp
}

/// Build compound RTCP SR (`PT=200`) + SDES (`PT=202`, empty CNAME) matching `ZaloCall` (`0x05`).
fn build_audio_rtcp_sr_sdes(
    ssrc: u32,
    now_ms: u64,
    rtp_ts: u32,
    packets: u32,
    octets: u32,
) -> Vec<u8> {
    let effective_ssrc = if ssrc != 0 { ssrc } else { 1 };
    let secs = (now_ms / 1000) + 2_208_988_800; // Unix epoch -> NTP epoch (1900)
    let frac = ((now_ms % 1000) << 32) / 1000;
    let mut rtcp = Vec::with_capacity(40);
    // SR (PT=200, length=6 32-bit words = 28 bytes)
    rtcp.extend_from_slice(&[0x80, 200, 0x00, 0x06]);
    rtcp.extend_from_slice(&effective_ssrc.to_be_bytes());
    rtcp.extend_from_slice(&(secs as u32).to_be_bytes());
    rtcp.extend_from_slice(&(frac as u32).to_be_bytes());
    rtcp.extend_from_slice(&rtp_ts.to_be_bytes());
    rtcp.extend_from_slice(&packets.to_be_bytes());
    rtcp.extend_from_slice(&octets.to_be_bytes());
    // SDES (PT=202, SC=1, length=2 32-bit words = 12 bytes, CNAME len=0)
    rtcp.extend_from_slice(&[0x81, 202, 0x00, 0x02]);
    rtcp.extend_from_slice(&effective_ssrc.to_be_bytes());
    rtcp.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    rtcp
}

fn decode_and_queue_audio(
    payload: &[u8],
    srtp_rx: Option<&mut SrtpContext>,
    neteq: &Arc<StdMutex<NetEq<Box<dyn OpusDecoderBackend>>>>,
    srtp_drop_count: &AtomicU64,
) -> bool {
    let plain_rtp = if let Some(ctx) = srtp_rx {
        match ctx.unprotect_rtp(payload) {
            Ok(p) => p,
            Err(e) => {
                // ADV-3: drop bad SRTP HMAC and track redacted drop count.
                let count = srtp_drop_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count == 1 || count.is_power_of_two() {
                    crate::logger::call_debug(&format!(
                        "[DEBUG-call] srtp_rx_drop total={count} payload_len={} err={e}",
                        payload.len()
                    ));
                    crate::logger::log_error(
                        "call::srtp",
                        &format!("ADV-3: dropped unauthenticated SRTP packet (total={count})"),
                    );
                }
                return false;
            }
        }
    } else {
        payload.to_vec()
    };
    if let Ok((seq, opus_payload)) = extract_rtp_payload(&plain_rtp) {
        if opus_payload.is_empty() {
            return false;
        }
        let mut g = neteq.lock().unwrap_or_else(|e| e.into_inner());
        g.push(seq, opus_payload.to_vec());
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
                // Best-effort UDP send to peer P2P address.
                let _ = socket.send_to(&pkt.encode(), addr).await;
            }
            P2pOutbound::ToRelay(pkt) => {
                if let Some(relay) = relay_addr {
                    // Best-effort UDP send to relay server.
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

#[cfg(all(feature = "call", target_os = "linux"))]
fn scale_pcm_pct(samples: &mut [i16], pct: u8) {
    if pct == 100 {
        return;
    }
    let factor = f32::from(pct) / 100.0;
    for s in samples.iter_mut() {
        let scaled = (f32::from(*s) * factor).round() as i32;
        *s = scaled.clamp(-32_768, 32_767) as i16;
    }
}

#[cfg(all(feature = "call", target_os = "linux"))]
fn pcm_level_pct(samples: &[i16]) -> u8 {
    let db = crate::services::call_media::AudioLeveler::compute_level_db(samples);
    let norm = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    (norm * 100.0).round() as u8
}

#[allow(clippy::too_many_arguments)]
fn spawn_audio_threads(
    call_id: u32,
    muted: Arc<AtomicBool>,
    held: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    reopen_mic: Arc<AtomicBool>,
    reopen_spk: Arc<AtomicBool>,
    pending_devices: Arc<StdMutex<(Option<String>, Option<String>)>>,
    mic_volume_pct: Arc<AtomicU8>,
    speaker_volume_pct: Arc<AtomicU8>,
    auto_mic_agc: Arc<AtomicBool>,
    auto_speaker_agc: Arc<AtomicBool>,
    live_mic_level: Arc<AtomicU8>,
    live_spk_level: Arc<AtomicU8>,
    live_mic_gain_pct: Arc<AtomicU8>,
    live_spk_gain_pct: Arc<AtomicU8>,
    tx_up: mpsc::Sender<[i16; FRAME_SAMPLES]>,
    neteq: Arc<StdMutex<NetEq<Box<dyn OpusDecoderBackend>>>>,
) -> Vec<std::thread::JoinHandle<()>> {
    #[cfg(all(feature = "call", target_os = "linux"))]
    {
        use crate::services::call_media::{
            wav_dump::{self, WavDumpMsg},
            Apm, AudioLeveler, LevelerConfig, PopOutcome, PulseCapture, PulsePlayback, CHANNELS,
            SAMPLE_RATE_HZ,
        };

        let dump_dir = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
        let (cap_wav_tx, pb_wav_tx, wav_handle) = match wav_dump::start_from_env(call_id, &dump_dir)
        {
            Some((c, p, h)) => (Some(c), Some(p), Some(h)),
            None => (None, None, None),
        };

        let apm: Option<Arc<StdMutex<Apm>>> =
            match Apm::new(SAMPLE_RATE_HZ as i32, CHANNELS as i32, true, true, true) {
                Ok(a) => {
                    crate::logger::call_debug("[DEBUG-call] webrtc_apm enabled (AEC+NS+HPF+AGC)");
                    Some(Arc::new(StdMutex::new(a)))
                }
                Err(e) => {
                    crate::logger::call_debug(&format!("[DEBUG-call] webrtc_apm unavailable: {e}"));
                    None
                }
            };

        let cap_latency_ms = Arc::new(AtomicI32::new(20));
        let pb_latency_ms = Arc::new(AtomicI32::new(60));
        let far_active = Arc::new(AtomicBool::new(false));

        let (init_mic, init_spk) = pending_devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        // Thread 1: Capture thread paced by PulseAudio record clock.
        let cap_stop = Arc::clone(&stop);
        let cap_held = Arc::clone(&held);
        let cap_pending = Arc::clone(&pending_devices);
        let cap_apm = apm.as_ref().map(Arc::clone);
        let cap_lat_out = Arc::clone(&cap_latency_ms);
        let cap_far_active = Arc::clone(&far_active);
        let cap_handle = std::thread::spawn(move || {
            let open_cap = |mic: Option<&str>| -> Option<PulseCapture> {
                match PulseCapture::open(normalize_device_id(mic)) {
                    Ok(c) => {
                        crate::logger::call_debug(&format!(
                            "[DEBUG-call] capture_open ok mic={:?}",
                            normalize_device_id(mic)
                        ));
                        Some(c)
                    }
                    Err(e) => {
                        crate::logger::call_debug(&format!(
                            "[DEBUG-call] capture_open FAILED mic={:?} err={e}",
                            normalize_device_id(mic)
                        ));
                        crate::logger::log_error(
                            "call::audio",
                            &format!("PulseCapture open failed: {e}"),
                        );
                        None
                    }
                }
            };

            let mut capture = open_cap(init_mic.as_deref());
            let mut mic_leveler =
                AudioLeveler::new(LevelerConfig::mic(), Some(Arc::clone(&cap_far_active)));
            let mut cap_buf = [0i16; FRAME_SAMPLES];
            let silence = [0i16; FRAME_SAMPLES];
            let mut last_stats_at = std::time::Instant::now();
            let mut cap_ok: u64 = 0;
            let mut cap_err: u64 = 0;
            let mut cap_peak: i32 = 0;
            let mut cap_max_ms: u128 = 0;
            let mut tx_up_drop: u64 = 0;

            while !cap_stop.load(Ordering::Relaxed) {
                if reopen_mic.swap(false, Ordering::AcqRel) {
                    let mic = cap_pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                        .clone();
                    drop(capture);
                    capture = open_cap(mic.as_deref());
                }

                let t0 = std::time::Instant::now();
                let read_ok = match capture.as_mut() {
                    Some(cap) => match cap.read_frame(&mut cap_buf) {
                        Ok(()) => true,
                        Err(e) => {
                            cap_err += 1;
                            if cap_err == 1 {
                                crate::logger::call_debug(&format!(
                                    "[DEBUG-call] audio_read_err err={e}"
                                ));
                            }
                            std::thread::sleep(Duration::from_millis(20));
                            false
                        }
                    },
                    None => {
                        std::thread::sleep(Duration::from_millis(20));
                        cap_buf = silence;
                        true
                    }
                };

                if read_ok && !cap_held.load(Ordering::Relaxed) {
                    cap_ok += 1;
                    cap_max_ms = cap_max_ms.max(t0.elapsed().as_millis());
                    let raw_buf = cap_buf;
                    if !muted.load(Ordering::Relaxed) {
                        if let Some(ref a) = cap_apm {
                            a.lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .process_capture(&mut cap_buf);
                        }
                        if auto_mic_agc.load(Ordering::Relaxed) {
                            mic_leveler.process(&mut cap_buf);
                            live_mic_gain_pct.store(mic_leveler.gain_pct(), Ordering::Relaxed);
                        } else {
                            let pct = mic_volume_pct.load(Ordering::Relaxed);
                            scale_pcm_pct(&mut cap_buf, pct);
                            live_mic_gain_pct.store(pct, Ordering::Relaxed);
                        }
                        live_mic_level.store(pcm_level_pct(&cap_buf), Ordering::Relaxed);
                        for &s in &cap_buf {
                            cap_peak = cap_peak.max((s as i32).abs());
                        }
                        if let Some(ref tx) = cap_wav_tx {
                            let _ = tx.try_send(WavDumpMsg::Capture {
                                raw: raw_buf,
                                proc: cap_buf,
                            });
                        }
                        if tx_up.try_send(cap_buf).is_err() {
                            tx_up_drop += 1;
                        }
                    } else {
                        live_mic_level.store(0, Ordering::Relaxed);
                        if let Some(ref tx) = cap_wav_tx {
                            let _ = tx.try_send(WavDumpMsg::Capture {
                                raw: raw_buf,
                                proc: silence,
                            });
                        }
                        if tx_up.try_send(silence).is_err() {
                            tx_up_drop += 1;
                        }
                    }
                }

                if last_stats_at.elapsed() >= Duration::from_secs(1) {
                    if let Some(us) = capture.as_ref().and_then(PulseCapture::latency_us) {
                        let ms = i32::try_from((us / 1000).min(200)).unwrap_or(20);
                        cap_lat_out.store(ms, Ordering::Relaxed);
                    }
                    crate::logger::call_debug(&format!(
                        "[DEBUG-call] capture_stats cap_ok={cap_ok} cap_err={cap_err} cap_peak={cap_peak} cap_max_ms={cap_max_ms} tx_up_drop={tx_up_drop}"
                    ));
                    last_stats_at = std::time::Instant::now();
                    cap_ok = 0;
                    cap_err = 0;
                    cap_peak = 0;
                    cap_max_ms = 0;
                    tx_up_drop = 0;
                }
            }
        });

        // Thread 2: Playout thread paced by PulseAudio playback clock (drives NetEq at exact 20 ms).
        let pb_pending = Arc::clone(&pending_devices);
        let pb_handle = std::thread::spawn(move || {
            let open_pb = |spk: Option<&str>| -> Option<PulsePlayback> {
                match PulsePlayback::open(normalize_device_id(spk)) {
                    Ok(p) => {
                        crate::logger::call_debug(&format!(
                            "[DEBUG-call] playback_open ok spk={:?}",
                            normalize_device_id(spk)
                        ));
                        Some(p)
                    }
                    Err(e) => {
                        crate::logger::call_debug(&format!(
                            "[DEBUG-call] playback_open FAILED spk={:?} err={e}",
                            normalize_device_id(spk)
                        ));
                        crate::logger::log_error(
                            "call::audio",
                            &format!("PulsePlayback open failed: {e}"),
                        );
                        None
                    }
                }
            };

            let mut playback = open_pb(init_spk.as_deref());
            let mut spk_leveler = AudioLeveler::new(LevelerConfig::speaker(), None);
            let mut last_far_speech = std::time::Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(std::time::Instant::now);
            let mut play_buf = [0i16; FRAME_SAMPLES];
            let silence = [0i16; FRAME_SAMPLES];
            let mut last_stats_at = std::time::Instant::now();
            let mut play_ok: u64 = 0;
            let mut play_plc: u64 = 0;
            let mut play_buf_wait: u64 = 0;
            let mut play_err: u64 = 0;
            let mut play_peak: i32 = 0;
            let mut play_max_ms: u128 = 0;

            while !stop.load(Ordering::Relaxed) {
                if reopen_spk.swap(false, Ordering::AcqRel) {
                    let spk = pb_pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .1
                        .clone();
                    drop(playback);
                    playback = open_pb(spk.as_deref());
                }

                let frame = if held.load(Ordering::Relaxed) {
                    far_active.store(false, Ordering::Relaxed);
                    live_spk_level.store(0, Ordering::Relaxed);
                    silence
                } else {
                    let outcome = {
                        let mut g = neteq.lock().unwrap_or_else(|e| e.into_inner());
                        g.pop_frame(&mut play_buf)
                    };
                    match outcome {
                        Ok(
                            PopOutcome::Decoded { .. }
                            | PopOutcome::Carry
                            | PopOutcome::FecRecovered,
                        ) => {
                            play_ok += 1;
                            if auto_speaker_agc.load(Ordering::Relaxed) {
                                spk_leveler.process(&mut play_buf);
                                live_spk_gain_pct.store(spk_leveler.gain_pct(), Ordering::Relaxed);
                            } else {
                                let pct = speaker_volume_pct.load(Ordering::Relaxed);
                                scale_pcm_pct(&mut play_buf, pct);
                                live_spk_gain_pct.store(pct, Ordering::Relaxed);
                            }
                            live_spk_level.store(pcm_level_pct(&play_buf), Ordering::Relaxed);
                            let sum_sq: i64 =
                                play_buf.iter().map(|&v| i64::from(v) * i64::from(v)).sum();
                            if sum_sq / FRAME_SAMPLES as i64 > 100_000 {
                                last_far_speech = std::time::Instant::now();
                                far_active.store(true, Ordering::Relaxed);
                            } else if last_far_speech.elapsed() >= Duration::from_millis(300) {
                                far_active.store(false, Ordering::Relaxed);
                            }
                            for &s in &play_buf {
                                play_peak = play_peak.max((s as i32).abs());
                            }
                            play_buf
                        }
                        Ok(PopOutcome::Plc) => {
                            play_plc += 1;
                            if auto_speaker_agc.load(Ordering::Relaxed) {
                                spk_leveler.process(&mut play_buf);
                                live_spk_gain_pct.store(spk_leveler.gain_pct(), Ordering::Relaxed);
                            } else {
                                let pct = speaker_volume_pct.load(Ordering::Relaxed);
                                scale_pcm_pct(&mut play_buf, pct);
                                live_spk_gain_pct.store(pct, Ordering::Relaxed);
                            }
                            live_spk_level.store(pcm_level_pct(&play_buf), Ordering::Relaxed);
                            play_buf
                        }
                        Ok(PopOutcome::Buffering) => {
                            play_buf_wait += 1;
                            live_spk_level.store(0, Ordering::Relaxed);
                            if last_far_speech.elapsed() >= Duration::from_millis(300) {
                                far_active.store(false, Ordering::Relaxed);
                            }
                            silence
                        }
                        Err(e) => {
                            play_err += 1;
                            live_spk_level.store(0, Ordering::Relaxed);
                            if play_err == 1 {
                                crate::logger::call_debug(&format!(
                                    "[DEBUG-call] neteq_pop_err err={e}"
                                ));
                            }
                            silence
                        }
                    }
                };

                if let Some(ref tx) = pb_wav_tx {
                    let _ = tx.try_send(WavDumpMsg::Render(frame));
                }
                if let Some(ref a) = apm {
                    a.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .process_render(&frame);
                }

                let t1 = std::time::Instant::now();
                match playback.as_mut() {
                    Some(pb) => {
                        if let Err(e) = pb.write_frame(&frame) {
                            play_err += 1;
                            if play_err == 1 {
                                crate::logger::call_debug(&format!(
                                    "[DEBUG-call] audio_write_err err={e}"
                                ));
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        } else {
                            play_max_ms = play_max_ms.max(t1.elapsed().as_millis());
                        }
                    }
                    None => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }

                if last_stats_at.elapsed() >= Duration::from_secs(1) {
                    if let Some(us) = playback.as_ref().and_then(PulsePlayback::latency_us) {
                        let ms = i32::try_from((us / 1000).min(200)).unwrap_or(60);
                        pb_latency_ms.store(ms, Ordering::Relaxed);
                    }
                    let total_delay = (cap_latency_ms.load(Ordering::Relaxed)
                        + pb_latency_ms.load(Ordering::Relaxed))
                    .clamp(20, 250);
                    if let Some(ref a) = apm {
                        a.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .set_delay_ms(total_delay);
                    }
                    crate::logger::call_debug(&format!(
                        "[DEBUG-call] playout_stats play_ok={play_ok} play_plc={play_plc} play_wait={play_buf_wait} play_err={play_err} play_peak={play_peak} play_max_ms={play_max_ms} apm_delay_ms={total_delay}"
                    ));
                    last_stats_at = std::time::Instant::now();
                    play_ok = 0;
                    play_plc = 0;
                    play_buf_wait = 0;
                    play_err = 0;
                    play_peak = 0;
                    play_max_ms = 0;
                }
            }
        });

        let mut handles = vec![cap_handle, pb_handle];
        if let Some(h) = wav_handle {
            handles.push(h);
        }
        handles
    }
    #[cfg(not(all(feature = "call", target_os = "linux")))]
    {
        let _ = (
            call_id,
            muted,
            held,
            stop,
            reopen_mic,
            reopen_spk,
            pending_devices,
            mic_volume_pct,
            speaker_volume_pct,
            auto_mic_agc,
            auto_speaker_agc,
            live_mic_level,
            live_spk_level,
            live_mic_gain_pct,
            live_spk_gain_pct,
            tx_up,
            neteq,
        );
        Vec::new()
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
        let rtp = build_minimal_rtp(1, 1, &[0xf8, 0xff, 0xfe]);
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

    #[tokio::test]
    async fn outgoing_call_transitions_ringing_connecting_active_with_init_response_ok() {
        let relay_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_sock.local_addr().unwrap();

        let relay_task = tokio::spawn(async move {
            let mut buf = [0u8; 512];
            for _ in 0..4 {
                if let Ok(Ok((n, from))) =
                    tokio::time::timeout(Duration::from_secs(2), relay_sock.recv_from(&mut buf))
                        .await
                {
                    if let Ok(ZRTPPacket::Control { cmd, seq, uid, .. }) =
                        ZRTPPacket::decode(&buf[..n])
                    {
                        if cmd == ZrtpCmd::Echo as u16 {
                            let reply = ZRTPPacket::Control {
                                is_reply: true,
                                magic: 1,
                                field02: 0,
                                seq,
                                uid,
                                token: 0,
                                cmd,
                                sub_cmd: 0,
                                body: ControlBody::Raw(vec![0, 0, 0, 0]),
                            };
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                        } else if cmd == ZrtpCmd::InitCallCaller as u16 {
                            let reply = ZRTPPacket::Control {
                                is_reply: true,
                                magic: 1,
                                field02: 0,
                                seq,
                                uid,
                                token: 0x1234_5678,
                                cmd,
                                sub_cmd: 0,
                                body: ControlBody::Raw(vec![0, 0, 0, 0]),
                            };
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                            break;
                        }
                    }
                }
            }
        });

        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_secs(60),
            Duration::from_secs(5),
            Duration::from_millis(500),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        let start_effects = {
            let mut g = engine.call_manager.state.lock().unwrap();
            g.relay_servers = vec![RelayServerAddr {
                ip: relay_addr.ip().to_string(),
                port: relay_addr.port(),
                server_type: 0,
                rtcp_addr: None,
                rtp_ipv6: None,
                rtcp_ipv6: None,
            }];
            g.session
                .apply(CallEvent::StartOutgoing {
                    call_id: 7001,
                    sess_id: "sess-7001".to_string(),
                    peer_id: "peer-out-1".to_string(),
                    srtp_mode: 1,
                })
                .unwrap()
        };
        engine.execute_call_effects(&s_snap, start_effects).await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::OutgoingRinging);
        assert!(engine.call_snapshot().started_at_ms.is_none());

        // Peer 402 (accepted) -> Connecting -> Echo + InitZRTP -> InitResponse(res=0) -> Active.
        let ans_sig = CallSignalWireData::from_value(&json!({
            "cmd": 402,
            "callId": 7001,
            "sessId": "sess-7001",
            "callerId": "self-1",
            "calleeId": "peer-out-1",
            "status": 0,
            "isVideo": 0
        }));
        engine.handle_voip_signal(&s_snap, ans_sig).await;

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if engine.call_snapshot().phase == CallPhase::Active {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("outgoing call should reach Active");

        let snap = engine.call_snapshot();
        assert_eq!(snap.phase, CallPhase::Active);
        assert!(snap.started_at_ms.is_some());

        engine.end_call().await.unwrap();
        let _ = relay_task.await;
    }

    #[tokio::test]
    async fn outgoing_call_ends_when_init_response_fails() {
        let relay_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_sock.local_addr().unwrap();

        let relay_task = tokio::spawn(async move {
            let mut buf = [0u8; 512];
            for _ in 0..4 {
                if let Ok(Ok((n, from))) =
                    tokio::time::timeout(Duration::from_secs(2), relay_sock.recv_from(&mut buf))
                        .await
                {
                    if let Ok(ZRTPPacket::Control { cmd, seq, uid, .. }) =
                        ZRTPPacket::decode(&buf[..n])
                    {
                        if cmd == ZrtpCmd::Echo as u16 {
                            let reply = ZRTPPacket::Control {
                                is_reply: true,
                                magic: 1,
                                field02: 0,
                                seq,
                                uid,
                                token: 0,
                                cmd,
                                sub_cmd: 0,
                                body: ControlBody::Raw(vec![0, 0, 0, 0]),
                            };
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                        } else if cmd == ZrtpCmd::InitCallCaller as u16 {
                            // Return non-zero error code (res = 1).
                            let reply = ZRTPPacket::Control {
                                is_reply: true,
                                magic: 1,
                                field02: 0,
                                seq,
                                uid,
                                token: 0,
                                cmd,
                                sub_cmd: 0,
                                body: ControlBody::Raw(vec![1, 0, 0, 0]),
                            };
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                            break;
                        }
                    }
                }
            }
        });

        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_secs(60),
            Duration::from_secs(5),
            Duration::from_millis(200),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        let start_effects = {
            let mut g = engine.call_manager.state.lock().unwrap();
            g.relay_servers = vec![RelayServerAddr {
                ip: relay_addr.ip().to_string(),
                port: relay_addr.port(),
                server_type: 0,
                rtcp_addr: None,
                rtp_ipv6: None,
                rtcp_ipv6: None,
            }];
            g.session
                .apply(CallEvent::StartOutgoing {
                    call_id: 7002,
                    sess_id: "sess-7002".to_string(),
                    peer_id: "peer-out-2".to_string(),
                    srtp_mode: 1,
                })
                .unwrap()
        };
        engine.execute_call_effects(&s_snap, start_effects).await;

        let ans_sig = CallSignalWireData::from_value(&json!({
            "cmd": 402,
            "callId": 7002,
            "sessId": "sess-7002",
            "callerId": "self-1",
            "calleeId": "peer-out-2",
            "status": 0,
            "isVideo": 0
        }));
        engine.handle_voip_signal(&s_snap, ans_sig).await;

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if engine.call_snapshot().phase == CallPhase::Idle {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("call should transition to Idle when InitResponse fails");

        {
            let captured = events.lock().unwrap();
            assert!(!captured.iter().any(|ev| matches!(
                ev,
                CoreEvent::CallStateChanged {
                    phase: CallPhase::Active,
                    ..
                }
            )));
        }
        let _ = relay_task.await;
    }

    #[tokio::test]
    async fn forged_srtp_increments_drop_counter_and_does_not_reset_watchdog() {
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
            Duration::from_millis(120),
            Duration::from_millis(50),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        let sig = CallSignalWireData::from_value(&json!({
            "cmd": 401,
            "callId": 7003,
            "sessId": "sess-7003",
            "callerId": "peer-7003",
            "calleeId": "self-1",
            "isVideo": 0,
            "srtpKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "extendData": {
                "srtpMode": 1,
                "maxFT": 60,
                "extraInfo": { "p2pAddr": format!("{}|{}", peer_addr.ip(), peer_addr.port()) }
            }
        }));
        engine.handle_voip_signal(&s_snap, sig).await;
        engine.answer_call(true).await.unwrap();

        // Receive P2P.BIND so we learn engine_addr.
        let mut buf = [0u8; 256];
        let (_n, engine_addr) =
            tokio::time::timeout(Duration::from_secs(2), peer_sock.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();

        // Send forged unauthenticated P2P.AUDIO (kind=6) every 30ms for 210ms (> 120ms watchdog).
        for i in 0..7u16 {
            let forged = ZRTPPacket::P2p {
                magic: 0x00,
                sender_role: 0, // caller (callee own_role is 1)
                call_id: 7003,
                kind: crate::protocol::call::P2pKind::Audio,
                body: crate::protocol::call::zrtp::P2pBody::Media(vec![
                    0x80, 111, 0, i as u8, 0, 0, 0, 0, 0, 0, 0, 1, 0xaa, 0xbb,
                ]),
            };
            let _ = peer_sock.send_to(&forged.encode(), engine_addr).await;
            tokio::time::sleep(Duration::from_millis(30)).await;
        }

        // Watchdog must still fire because forged SRTP packets must NOT refresh last_rx_at.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if engine.call_snapshot().phase == CallPhase::Idle {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("watchdog must fire despite forged SRTP packets");

        assert!(engine.call_manager.srtp_drop_count() >= 1);
    }

    #[tokio::test]
    async fn init_call_uses_32bit_media_uids_and_ignores_late_echo_replies() {
        let relay_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_sock.local_addr().unwrap();
        let observed_uids = Arc::new(StdMutex::new(None::<(u32, u32)>));
        let observed_uids_clone = Arc::clone(&observed_uids);

        let relay_task = tokio::spawn(async move {
            let mut buf = [0u8; 512];
            for _ in 0..4 {
                if let Ok(Ok((n, from))) =
                    tokio::time::timeout(Duration::from_secs(2), relay_sock.recv_from(&mut buf))
                        .await
                {
                    if let Ok(ZRTPPacket::Control {
                        cmd,
                        seq,
                        uid,
                        body,
                        ..
                    }) = ZRTPPacket::decode(&buf[..n])
                    {
                        if cmd == ZrtpCmd::Echo as u16 {
                            // First Echo reply triggers select_relay_with_echo, second Echo reply arrives
                            // while start_media_loop is waiting for InitCallCaller response.
                            let reply = ZRTPPacket::Control {
                                is_reply: true,
                                magic: 1,
                                field02: 0,
                                seq,
                                uid,
                                token: 0,
                                cmd,
                                sub_cmd: 0,
                                body: ControlBody::Raw(vec![0xff, 0xff, 0xff, 0xff]),
                            };
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                        } else if cmd == ZrtpCmd::InitCallCaller as u16 {
                            if let ControlBody::InitCall { peer_uid, .. } = body {
                                *observed_uids_clone.lock().unwrap() = Some((uid, peer_uid));
                            }
                            // Valid InitResponse: res=0, callId=7010, token=0xCAFE_BABE, publicAddr="203.0.113.9|49000"
                            let mut resp_body = Vec::new();
                            resp_body.extend_from_slice(&0i32.to_le_bytes());
                            resp_body.extend_from_slice(&7010u32.to_le_bytes());
                            resp_body.extend_from_slice(&0xCAFE_BABEu32.to_le_bytes());
                            let pub_addr = b"203.0.113.9|49000";
                            resp_body.extend_from_slice(&(pub_addr.len() as u16).to_le_bytes());
                            resp_body.extend_from_slice(pub_addr);
                            let reply = ZRTPPacket::Control {
                                is_reply: true,
                                magic: 1,
                                field02: 0,
                                seq,
                                uid,
                                token: 0xCAFE_BABE,
                                cmd,
                                sub_cmd: 2,
                                body: ControlBody::Raw(resp_body),
                            };
                            let _ = relay_sock.send_to(&reply.encode(), from).await;
                            break;
                        }
                    }
                }
            }
        });

        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_secs(60),
            Duration::from_secs(5),
            Duration::from_millis(500),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        let start_effects = {
            let mut g = engine.call_manager.state.lock().unwrap();
            g.local_media_uid = Some(314_159_265);
            g.peer_media_uid = Some(271_828_182);
            g.relay_servers = vec![RelayServerAddr {
                ip: relay_addr.ip().to_string(),
                port: relay_addr.port(),
                server_type: 1,
                rtcp_addr: Some(format!("{}:{}", relay_addr.ip(), relay_addr.port())),
                rtp_ipv6: None,
                rtcp_ipv6: None,
            }];
            g.session
                .apply(CallEvent::StartOutgoing {
                    call_id: 7010,
                    sess_id: "0123456789abcdef0123456789ABCDxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
                        .to_string(),
                    peer_id: "8491234567890123456".to_string(),
                    srtp_mode: 1,
                })
                .unwrap()
        };
        engine.execute_call_effects(&s_snap, start_effects).await;

        let _ = relay_task.await;
        tokio::time::sleep(Duration::from_millis(40)).await;

        assert_eq!(
            *observed_uids.lock().unwrap(),
            Some((314_159_265, 271_828_182))
        );
        {
            let g = engine.call_manager.state.lock().unwrap();
            assert_eq!(g.public_p2p_addr.as_deref(), Some("203.0.113.9|49000"));
            assert!(g.selected_relay.is_some());
        }
        engine.end_call().await.unwrap();
    }

    #[tokio::test]
    async fn incoming_401_then_402_answer_closes_silently_and_suppresses_late_401_replay() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_secs(60),
            Duration::from_secs(5),
            Duration::from_millis(500),
        );
        let s_snap = engine.session_arc().lock().await.clone();

        engine
            .handle_voip_signal(&s_snap, test_signal(401, 8001, "peer-multi-1"))
            .await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::IncomingRinging);

        // Phone answered the incoming call -> 402 arrives on desktop.
        engine
            .handle_voip_signal(&s_snap, test_signal(402, 8001, "peer-multi-1"))
            .await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);

        // Multi-device handling must not fabricate a local CallHistoryRecorded entry.
        {
            let ev_guard = events.lock().unwrap();
            assert!(!ev_guard
                .iter()
                .any(|e| matches!(e, CoreEvent::CallHistoryRecorded { .. })));
            assert!(ev_guard.iter().any(|e| matches!(
                e,
                CoreEvent::CallStateChanged {
                    phase: CallPhase::Idle,
                    end_reason: Some(reason),
                    ..
                } if reason == "handled_elsewhere"
            )));
        }

        // Late/replayed 401 for the same call_id must not cause ghost ringing.
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 8001, "peer-multi-1"))
            .await;
        assert_eq!(engine.call_snapshot().phase, CallPhase::Idle);
    }

    #[tokio::test]
    async fn conf_mute_audio_updates_peer_muted_and_settings_sync_devices() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::new(CollectSink(Arc::clone(&events)));
        let session = Session::new(false, false, false).unwrap();
        let engine = CoreEngine::with_call_timeouts(
            session,
            ":memory:",
            sink,
            Duration::from_secs(60),
            Duration::from_secs(5),
            Duration::from_millis(500),
        );
        let mut settings = crate::core::settings::AppSettings::default();
        settings.selected_mic_device = "mic-1".to_string();
        settings.selected_speaker_device = "spk-2".to_string();
        settings.auto_mic_agc = false;
        settings.mic_gain_percent = 75;
        engine.apply_call_audio_settings(&settings);
        assert_eq!(
            engine.call_manager.selected_devices(),
            (Some("mic-1".to_string()), Some("spk-2".to_string()))
        );
        assert!(!engine.call_manager.auto_mic_agc.load(Ordering::Relaxed));
        assert_eq!(
            engine.call_manager.mic_volume_pct.load(Ordering::Relaxed),
            75
        );

        let s_snap = engine.session_arc().lock().await.clone();
        engine
            .handle_voip_signal(&s_snap, test_signal(401, 8100, "peer-mute-1"))
            .await;
        let conf_mute = CallSignalWireData::from_value(&serde_json::json!({
            "cmd": 604,
            "act": "mute_audio",
            "callId": 8100,
            "params": { "type": 1 }
        }));
        engine.handle_voip_signal(&s_snap, conf_mute).await;
        assert!(engine.call_snapshot().peer_muted);

        let conf_unmute = CallSignalWireData::from_value(&serde_json::json!({
            "cmd": 604,
            "act": "mute_audio",
            "callId": 8100,
            "params": { "type": 0 }
        }));
        engine.handle_voip_signal(&s_snap, conf_unmute).await;
        assert!(!engine.call_snapshot().peer_muted);
    }
}
