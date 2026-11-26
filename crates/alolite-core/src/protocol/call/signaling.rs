//! ZRTC 1-1 VoIP HTTP signaling payloads (`11300..11311`, `12006`) and `extendData` negotiation.
//!
//! Wire contracts documented in `re-notes/06-call-media.md`, `re-notes/07-voip-log.md`,
//! `re-notes/08-video-call.md`, and `re-notes/14-http-rest-endpoint-catalog.md`.

use std::fmt;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::protocol::registry::voice_call;
use crate::protocol::session::Session;
use crate::protocol::wire::{self, ErrorEnvelopeWireData};

/// Numeric signal `cmd` codes exchanged over VoIP push/pull frames (`601`/`604`)
/// and mapped 1:1 to `/api/voicecall/*` HTTP endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum CallSignalCode {
    RequestCall = 401,
    Answer = 402,
    Cancel = 405,
    LogEndCall = 406,
    RingRing = 407,
    AnswerAck = 408,
    EndCall = 409,
    HoldReq = 411,
    HoldResp = 412,
    UnholdReq = 413,
    UnholdResp = 414,
    Request = 416,
    Conf = 12006,
}

impl CallSignalCode {
    /// Convert a raw numeric VoIP signal `cmd` into [`CallSignalCode`].
    pub const fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            401 => Self::RequestCall,
            402 => Self::Answer,
            405 => Self::Cancel,
            406 => Self::LogEndCall,
            407 => Self::RingRing,
            408 => Self::AnswerAck,
            409 => Self::EndCall,
            411 => Self::HoldReq,
            412 => Self::HoldResp,
            413 => Self::UnholdReq,
            414 => Self::UnholdResp,
            416 => Self::Request,
            12006 => Self::Conf,
            _ => return None,
        })
    }

    /// Return the numeric signal code (`401..=416`).
    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

/// Audio/video codec descriptor inside `extendData`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodecEntry {
    pub name: String,
    pub payload: u8,
    pub frm_ptime: u32,
    pub dynamic_fptime: u32,
}

impl Default for CodecEntry {
    fn default() -> Self {
        Self {
            name: "opus".to_string(),
            payload: 111,
            frm_ptime: 20,
            dynamic_fptime: 1,
        }
    }
}

/// `extendData` JSON payload carried inside `11300`, `11301`, and `11303`.
///
/// # Invariants
/// - `INV-8`: `srtp_mode` defaults to `1` (`AES-128-CM + HMAC-SHA1-80`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtendData {
    pub call_type: u8,
    pub new_zrtc: u8,
    pub packet_mode: u8,
    pub gcc_mode: u8,
    pub gcc_audio: u8,
    pub gcc_early_call: u8,
    pub gcc_svlr: u8,
    pub srtp_mode: u8,
    pub srtcp: u8,
    #[serde(rename = "sP2P")]
    pub s_p2p: u8,
    #[serde(rename = "maxFT")]
    pub max_ft: u16,
    pub platform: u8,
    #[serde(rename = "fecTP")]
    pub fec_tp: u8,
    pub support_hevc_decode: u8,
    pub tp_type: u8,
    pub sp_tcp: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<Vec<CodecEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_addr: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select_2side: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_info: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_result: Option<Value>,
}

impl Default for ExtendData {
    fn default() -> Self {
        Self {
            call_type: 0,
            new_zrtc: 1,
            packet_mode: 2,
            gcc_mode: 1,
            gcc_audio: 1,
            gcc_early_call: 0,
            gcc_svlr: 0,
            // INV-8: always negotiate SRTP by default.
            srtp_mode: 1,
            srtcp: 0,
            s_p2p: 1,
            max_ft: 60,
            platform: 1,
            fec_tp: 1,
            support_hevc_decode: 0,
            tp_type: 0,
            sp_tcp: 0,
            audio_codec: Some(vec![CodecEntry::default()]),
            server_addr: None,
            select_2side: None,
            extra_info: None,
            server_result: None,
        }
    }
}

impl ExtendData {
    /// Serialize `extendData` as a compact JSON string for form parameters.
    ///
    /// # Errors
    /// Returns an error if JSON serialization fails.
    pub fn to_json_string(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Parse `extendData` from either a JSON string or a JSON object [`Value`].
    ///
    /// # Errors
    /// Returns an error if the input is neither an object nor a valid JSON string.
    pub fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::String(s) => Ok(serde_json::from_str(s)?),
            Value::Object(_) => Ok(serde_json::from_value(v.clone())?),
            _ => bail!("invalid extendData value type"),
        }
    }
}

/// Relay server endpoint parsed from `401` / `416` signaling.
///
/// Redacts the IP address in [`fmt::Debug`] per `INV-8`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayServerAddr {
    pub ip: String,
    pub port: u16,
    #[serde(default)]
    pub server_type: u8,
}

impl fmt::Debug for RelayServerAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayServerAddr")
            .field("ip", &"<redacted-ip>")
            .field("port", &self.port)
            .field("server_type", &self.server_type)
            .finish()
    }
}

/// Raw SRTP master key + salt container (`INV-8`).
///
/// Automatically zeroized on drop and redacted in [`fmt::Debug`].
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SrtpKeyMaterial {
    bytes: Vec<u8>,
}

impl SrtpKeyMaterial {
    /// Wrap raw SRTP key material bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the raw key material bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for SrtpKeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SrtpKeyMaterial([REDACTED])")
    }
}

/// Typed Tier-1 wire struct for incoming VoIP signaling events (`401..416`)
/// and HTTP `/api/voicecall/*` `data` payloads.
///
/// # Wire Response Example
/// ```json
/// {
///   "cmd": 401,
///   "callId": 464564007,
///   "sessId": "sess-0001",
///   "callerId": "100001",
///   "calleeId": "100002",
///   "isVideo": 0,
///   "extendData": "{\"srtpMode\":1,\"maxFT\":60}"
/// }
/// ```
///
/// # Core Transformation
/// Extracts normalized IDs (`call_id`, `sess_id`, `peer_id`, `signal_code`, `extend_data`)
/// while redacting `rtp_ip` and `srtp_key` in [`fmt::Debug`] (`INV-8`).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CallSignalWireData {
    pub cmd: Option<Value>,
    #[serde(rename = "callId", alias = "call_id")]
    pub call_id: Option<Value>,
    #[serde(rename = "sessId", alias = "sess_id")]
    pub sess_id: Option<Value>,
    #[serde(rename = "callerId", alias = "uid", alias = "fromUid")]
    pub caller_id: Option<Value>,
    #[serde(rename = "calleeId", alias = "toUid")]
    pub callee_id: Option<Value>,
    pub status: Option<Value>,
    #[serde(rename = "isVideo", alias = "callType")]
    pub is_video: Option<Value>,
    #[serde(rename = "extendData")]
    pub extend_data: Option<Value>,
    #[serde(rename = "rtpIP", alias = "rtpIp", alias = "servers")]
    pub rtp_ip: Option<Value>,
    #[serde(rename = "srtpKey", alias = "key", alias = "srtp")]
    pub srtp_key: Option<Value>,
    pub codec: Option<Value>,
    #[serde(rename = "isOnCam")]
    pub is_on_cam: Option<Value>,
    #[serde(rename = "isOnMic")]
    pub is_on_mic: Option<Value>,
}

impl fmt::Debug for CallSignalWireData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallSignalWireData")
            .field("cmd", &self.cmd)
            .field("call_id", &self.call_id)
            .field("sess_id", &self.sess_id)
            .field("caller_id", &self.caller_id)
            .field("callee_id", &self.callee_id)
            .field("status", &self.status)
            .field("is_video", &self.is_video)
            .field("rtp_ip", &self.rtp_ip.as_ref().map(|_| "<redacted>"))
            .field("srtp_key", &self.srtp_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl CallSignalWireData {
    /// Construct from a raw JSON [`Value`], unwrapping `data` envelopes when present.
    pub fn from_value(v: &Value) -> Self {
        let inner = wire::unwrap_data(v);
        serde_json::from_value(inner.clone()).unwrap_or_default()
    }

    /// Return the parsed [`CallSignalCode`], if `cmd` matches `401..=416`.
    pub fn signal_code(&self) -> Option<CallSignalCode> {
        let code = wire::val_u64(self.cmd.as_ref())?;
        u16::try_from(code).ok().and_then(CallSignalCode::from_code)
    }

    /// Return the numeric `callId`.
    pub fn call_id_u32(&self) -> Option<u32> {
        wire::val_u64(self.call_id.as_ref()).and_then(|v| u32::try_from(v).ok())
    }

    /// Return `sessId` as a non-empty string.
    pub fn sess_id_str(&self) -> Option<String> {
        wire::field_str(self.sess_id.as_ref())
    }

    /// Return `callerId` as a non-empty string.
    pub fn caller_id_str(&self) -> Option<String> {
        wire::field_str(self.caller_id.as_ref())
    }

    /// Return `calleeId` as a non-empty string.
    pub fn callee_id_str(&self) -> Option<String> {
        wire::field_str(self.callee_id.as_ref())
    }

    /// Return `status` as `i64`.
    pub fn status_i64(&self) -> Option<i64> {
        wire::val_i64(self.status.as_ref())
    }

    /// Return `true` when `isVideo` / `callType` indicates a video call.
    pub fn is_video_call(&self) -> bool {
        wire::val_i64(self.is_video.as_ref()).unwrap_or(0) != 0
    }

    /// Parse `extendData` if present, falling back to `ExtendData::default()` (`srtpMode: 1`).
    pub fn parsed_extend_data(&self) -> ExtendData {
        self.extend_data
            .as_ref()
            .and_then(|v| ExtendData::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Parse relay server endpoints (`rtpIP` / `servers`) from signaling payloads.
    pub fn relay_servers(&self) -> Vec<RelayServerAddr> {
        fn parse_host_port(raw: &str, server_type: u8) -> Option<RelayServerAddr> {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some((ip, port_str)) = trimmed.rsplit_once(':') {
                if let Ok(port) = port_str.parse::<u16>() {
                    if !ip.is_empty() && port > 0 {
                        return Some(RelayServerAddr {
                            ip: ip.to_string(),
                            port,
                            server_type,
                        });
                    }
                }
            }
            None
        }

        let Some(val) = self.rtp_ip.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match val {
            Value::String(s) => {
                for part in s.split(',') {
                    if let Some(addr) = parse_host_port(part, 1) {
                        out.push(addr);
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    match item {
                        Value::String(s) => {
                            if let Some(addr) = parse_host_port(s, 1) {
                                out.push(addr);
                            }
                        }
                        Value::Object(_) => {
                            if let Ok(addr) =
                                serde_json::from_value::<RelayServerAddr>(item.clone())
                            {
                                if !addr.ip.is_empty() && addr.port > 0 {
                                    out.push(addr);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Value::Object(_) => {
                if let Ok(addr) = serde_json::from_value::<RelayServerAddr>(val.clone()) {
                    if !addr.ip.is_empty() && addr.port > 0 {
                        out.push(addr);
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// Parse SRTP master key + salt (`30` bytes) from `srtpKey` / `key` / `srtp` (`INV-8`).
    pub fn srtp_key_material(&self) -> Option<SrtpKeyMaterial> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let raw = wire::field_str(self.srtp_key.as_ref())?;
        if let Ok(decoded) = STANDARD.decode(raw.as_bytes()) {
            if decoded.len()
                >= crate::protocol::call::SRTP_MASTER_KEY_LEN
                    + crate::protocol::call::SRTP_MASTER_SALT_LEN
            {
                return Some(SrtpKeyMaterial::from_bytes(decoded));
            }
        }
        let bytes = raw.into_bytes();
        if bytes.len()
            >= crate::protocol::call::SRTP_MASTER_KEY_LEN
                + crate::protocol::call::SRTP_MASTER_SALT_LEN
        {
            Some(SrtpKeyMaterial::from_bytes(bytes))
        } else {
            None
        }
    }

    /// Extract peer P2P candidate [`std::net::SocketAddr`]s from `extendData` (`extraInfo` / `serverAddr`) if present.
    pub fn p2p_candidates(&self) -> Vec<std::net::SocketAddr> {
        let mut out = Vec::new();
        let ext = self.parsed_extend_data();
        for source in [ext.extra_info.as_ref(), ext.server_addr.as_ref()]
            .into_iter()
            .flatten()
        {
            collect_p2p_addrs_from_value(source, &mut out);
        }
        out
    }
}

fn collect_p2p_addrs_from_value(val: &Value, out: &mut Vec<std::net::SocketAddr>) {
    match val {
        Value::String(s) => {
            for part in s.split(',') {
                if let Some(addr) = crate::protocol::call::p2p::parse_p2p_addr(part) {
                    if !out.contains(&addr) {
                        out.push(addr);
                    }
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_p2p_addrs_from_value(item, out);
            }
        }
        Value::Object(map) => {
            if let (Some(ip), Some(port)) = (
                map.get("ip").and_then(Value::as_str),
                map.get("port").and_then(|p| wire::val_u64(Some(p))),
            ) {
                if let Ok(port_u16) = u16::try_from(port) {
                    if let Ok(addr) = format!("{ip}:{port_u16}").parse::<std::net::SocketAddr>() {
                        if !out.contains(&addr) {
                            out.push(addr);
                        }
                    }
                }
            }
            for key in ["p2pAddr", "p2pCandidates", "candidates"] {
                if let Some(v) = map.get(key) {
                    collect_p2p_addrs_from_value(v, out);
                }
            }
        }
        _ => {}
    }
}

/// Build `11300` `/api/voicecall/requestcall` parameters.
pub fn build_request_call_params(
    callee_id: &str,
    call_type: u8,
    extend_data: &ExtendData,
) -> Result<Value> {
    Ok(wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callType", Value::from(call_type)),
        ("extendData", Value::String(extend_data.to_json_string()?)),
    ]))
}

/// Build `11301` `/api/voicecall/request` parameters.
pub fn build_request_params(
    callee_id: &str,
    call_id: u32,
    sess_id: &str,
    extend_data: &ExtendData,
) -> Result<Value> {
    Ok(wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("extendData", Value::String(extend_data.to_json_string()?)),
    ]))
}

/// Build `11302` `/api/voicecall/ringring` parameters.
pub fn build_ring_ring_params(caller_id: &str, call_id: u32, sess_id: &str, status: i32) -> Value {
    wire::obj(&[
        ("callerId", Value::String(caller_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("status", Value::from(status)),
    ])
}

/// Build `11303` `/api/voicecall/answer` parameters (`status = 0` accept, `!= 0` reject).
pub fn build_answer_params(
    caller_id: &str,
    call_id: u32,
    sess_id: &str,
    status: i32,
    extend_data: &ExtendData,
) -> Result<Value> {
    Ok(wire::obj(&[
        ("callerId", Value::String(caller_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("status", Value::from(status)),
        ("extendData", Value::String(extend_data.to_json_string()?)),
    ]))
}

/// Build `11304` `/api/voicecall/answerack` parameters.
pub fn build_answer_ack_params(callee_id: &str, call_id: u32, sess_id: &str) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
    ])
}

/// Build `11305` `/api/voicecall/cancel` parameters.
pub fn build_cancel_params(callee_id: &str, call_id: u32, sess_id: &str) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
    ])
}

/// Build `11306` `/api/voicecall/endcall` parameters.
pub fn build_end_call_params(peer_id: &str, call_id: u32, sess_id: &str, reason: i32) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("reason", Value::from(reason)),
    ])
}

/// Build `11307` `/api/voicecall/holdreq` parameters.
pub fn build_hold_req_params(peer_id: &str, call_id: u32, sess_id: &str) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
    ])
}

/// Build `11308` `/api/voicecall/holdresp` parameters.
pub fn build_hold_resp_params(peer_id: &str, call_id: u32, sess_id: &str, status: i32) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("status", Value::from(status)),
    ])
}

/// Build `11309` `/api/voicecall/unholdreq` parameters.
pub fn build_unhold_req_params(peer_id: &str, call_id: u32, sess_id: &str) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
    ])
}

/// Build `11310` `/api/voicecall/unholdresp` parameters.
pub fn build_unhold_resp_params(peer_id: &str, call_id: u32, sess_id: &str, status: i32) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("status", Value::from(status)),
    ])
}

/// Build `11311` `/api/voicecall/logendcall` parameters.
pub fn build_log_end_call_params(
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
    duration_secs: u64,
    status: i32,
) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("sessId", Value::String(sess_id.to_string())),
        ("duration", Value::from(duration_secs)),
        ("status", Value::from(status)),
    ])
}

/// Build `12006` `/api/voicecall/conf` parameters (`sendOnOffCamMic`).
pub fn build_conf_params(
    callee_id: &str,
    call_id: u32,
    is_on_cam: bool,
    is_on_mic: bool,
    data: Option<Value>,
) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callId", Value::from(call_id)),
        ("isOnCam", Value::from(u8::from(is_on_cam))),
        ("isOnMic", Value::from(u8::from(is_on_mic))),
        (
            "data",
            data.unwrap_or_else(|| Value::Object(Default::default())),
        ),
    ])
}

async fn post_voicecall(
    session: &Session,
    spec: crate::protocol::registry::CommandSpec,
    params: Value,
) -> Result<CallSignalWireData> {
    let _ = wire::require_login(session)?;
    let resp = session.api_post(spec, Some(&params)).await?;
    let env = ErrorEnvelopeWireData::parse(&resp);
    if env.code() != 0 {
        bail!(
            "voicecall {} failed: error_code={} ({})",
            spec.id,
            env.code(),
            env.message()
        );
    }
    Ok(CallSignalWireData::from_value(&resp))
}

/// Send `11300` `/api/voicecall/requestcall` to initiate a 1-1 call.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn request_call(
    session: &Session,
    callee_id: &str,
    call_type: u8,
    extend_data: &ExtendData,
) -> Result<CallSignalWireData> {
    let params = build_request_call_params(callee_id, call_type, extend_data)?;
    post_voicecall(session, voice_call::REQUEST_CALL, params).await
}

/// Send `11301` `/api/voicecall/request` after ZRTP initialization.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn send_request(
    session: &Session,
    callee_id: &str,
    call_id: u32,
    sess_id: &str,
    extend_data: &ExtendData,
) -> Result<CallSignalWireData> {
    let params = build_request_params(callee_id, call_id, sess_id, extend_data)?;
    post_voicecall(session, voice_call::REQUEST, params).await
}

/// Send `11302` `/api/voicecall/ringring`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn send_ring_ring(
    session: &Session,
    caller_id: &str,
    call_id: u32,
    sess_id: &str,
    status: i32,
) -> Result<CallSignalWireData> {
    let params = build_ring_ring_params(caller_id, call_id, sess_id, status);
    post_voicecall(session, voice_call::RING_RING, params).await
}

/// Send `11303` `/api/voicecall/answer` (`status = 0` accept, non-zero reject).
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn answer_call(
    session: &Session,
    caller_id: &str,
    call_id: u32,
    sess_id: &str,
    status: i32,
    extend_data: &ExtendData,
) -> Result<CallSignalWireData> {
    let params = build_answer_params(caller_id, call_id, sess_id, status, extend_data)?;
    post_voicecall(session, voice_call::ANSWER, params).await
}

/// Send `11304` `/api/voicecall/answerack`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn send_answer_ack(
    session: &Session,
    callee_id: &str,
    call_id: u32,
    sess_id: &str,
) -> Result<CallSignalWireData> {
    let params = build_answer_ack_params(callee_id, call_id, sess_id);
    post_voicecall(session, voice_call::ANSWER_ACK, params).await
}

/// Send `11305` `/api/voicecall/cancel`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn cancel_call(
    session: &Session,
    callee_id: &str,
    call_id: u32,
    sess_id: &str,
) -> Result<CallSignalWireData> {
    let params = build_cancel_params(callee_id, call_id, sess_id);
    post_voicecall(session, voice_call::CANCEL, params).await
}

/// Send `11306` `/api/voicecall/endcall`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn end_call(
    session: &Session,
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
    reason: i32,
) -> Result<CallSignalWireData> {
    let params = build_end_call_params(peer_id, call_id, sess_id, reason);
    post_voicecall(session, voice_call::END_CALL, params).await
}

/// Send `11307` `/api/voicecall/holdreq`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn hold_request(
    session: &Session,
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
) -> Result<CallSignalWireData> {
    let params = build_hold_req_params(peer_id, call_id, sess_id);
    post_voicecall(session, voice_call::HOLD_REQ, params).await
}

/// Send `11308` `/api/voicecall/holdresp`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn hold_response(
    session: &Session,
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
    status: i32,
) -> Result<CallSignalWireData> {
    let params = build_hold_resp_params(peer_id, call_id, sess_id, status);
    post_voicecall(session, voice_call::HOLD_RESP, params).await
}

/// Send `11309` `/api/voicecall/unholdreq`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn unhold_request(
    session: &Session,
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
) -> Result<CallSignalWireData> {
    let params = build_unhold_req_params(peer_id, call_id, sess_id);
    post_voicecall(session, voice_call::UNHOLD_REQ, params).await
}

/// Send `11310` `/api/voicecall/unholdresp`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn unhold_response(
    session: &Session,
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
    status: i32,
) -> Result<CallSignalWireData> {
    let params = build_unhold_resp_params(peer_id, call_id, sess_id, status);
    post_voicecall(session, voice_call::UNHOLD_RESP, params).await
}

/// Send `11311` `/api/voicecall/logendcall`.
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn log_end_call(
    session: &Session,
    peer_id: &str,
    call_id: u32,
    sess_id: &str,
    duration_secs: u64,
    status: i32,
) -> Result<CallSignalWireData> {
    let params = build_log_end_call_params(peer_id, call_id, sess_id, duration_secs, status);
    post_voicecall(session, voice_call::LOG_END_CALL, params).await
}

/// Send `12006` `/api/voicecall/conf` (`isOnCam`, `isOnMic`, `data`).
///
/// # Errors
/// Returns an error if not logged in, HTTP/encryption fails, or `error_code != 0`.
pub async fn send_conf(
    session: &Session,
    callee_id: &str,
    call_id: u32,
    is_on_cam: bool,
    is_on_mic: bool,
    data: Option<Value>,
) -> Result<CallSignalWireData> {
    let params = build_conf_params(callee_id, call_id, is_on_cam, is_on_mic, data);
    post_voicecall(session, voice_call::CONF, params).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::registry::lookup_command;

    #[test]
    fn extend_data_defaults_to_srtp_mode_1_and_roundtrips() {
        let ext = ExtendData::default();
        // (INV-8) SRTP must be negotiated by default.
        assert_eq!(ext.srtp_mode, 1);
        assert_eq!(ext.max_ft, 60);
        assert_eq!(ext.packet_mode, 2);
        assert_eq!(ext.support_hevc_decode, 0);

        let encoded = ext.to_json_string().unwrap();
        let decoded = ExtendData::from_value(&Value::String(encoded)).unwrap();
        assert_eq!(decoded, ext);
    }

    #[test]
    fn voicecall_registry_commands_11300_to_11311_and_12006_resolve() {
        for id in 11300..=11311 {
            assert!(
                lookup_command(id).is_some(),
                "missing voicecall command {id}"
            );
        }
        assert!(lookup_command(12006).is_some());
        assert_eq!(lookup_command(11300), Some(voice_call::REQUEST_CALL));
        assert_eq!(lookup_command(12006), Some(voice_call::CONF));
    }

    #[test]
    fn signaling_payload_builders_produce_expected_wire_fields() {
        let ext = ExtendData::default();
        let req = build_request_call_params("user-2", 0, &ext).unwrap();
        assert_eq!(req["calleeId"], "user-2");
        assert_eq!(req["callType"], 0);
        let parsed_ext = ExtendData::from_value(&req["extendData"]).unwrap();
        assert_eq!(parsed_ext.srtp_mode, 1); // (INV-8)

        let ans = build_answer_params("user-1", 464564007, "sess-1", 0, &ext).unwrap();
        assert_eq!(ans["callerId"], "user-1");
        assert_eq!(ans["callId"], 464564007);
        assert_eq!(ans["status"], 0);

        let conf = build_conf_params("user-2", 464564007, false, true, None);
        assert_eq!(conf["calleeId"], "user-2");
        assert_eq!(conf["callId"], 464564007);
        assert_eq!(conf["isOnCam"], 0);
        assert_eq!(conf["isOnMic"], 1);

        let log_end = build_log_end_call_params("user-2", 464564007, "sess-1", 42, 0);
        assert_eq!(log_end["duration"], 42);
    }

    #[test]
    fn call_signal_wire_data_parses_401_invite_and_redacts_secrets_in_debug() {
        let raw = serde_json::json!({
            "error_code": 0,
            "data": {
                "cmd": 401,
                "callId": 464564007,
                "sessId": "sess-0001",
                "callerId": "100001",
                "calleeId": "100002",
                "isVideo": 0,
                "rtpIP": "192.0.2.10:4200",
                "srtpKey": "super-secret-master-key",
                "extendData": "{\"srtpMode\":1,\"maxFT\":60}"
            }
        });
        let parsed = CallSignalWireData::from_value(&raw);
        assert_eq!(parsed.signal_code(), Some(CallSignalCode::RequestCall));
        assert_eq!(parsed.call_id_u32(), Some(464564007));
        assert_eq!(parsed.sess_id_str().as_deref(), Some("sess-0001"));
        assert_eq!(parsed.caller_id_str().as_deref(), Some("100001"));
        assert!(!parsed.is_video_call());
        assert_eq!(parsed.parsed_extend_data().srtp_mode, 1);

        // (INV-8) Raw SRTP keys and peer IPs must never appear in Debug output.
        let dbg = format!("{parsed:?}");
        assert!(!dbg.contains("super-secret-master-key"));
        assert!(!dbg.contains("192.0.2.10"));

        let relay = RelayServerAddr {
            ip: "198.51.100.42".to_string(),
            port: 4200,
            server_type: 1,
        };
        assert!(!format!("{relay:?}").contains("198.51.100.42"));

        let key = SrtpKeyMaterial::from_bytes(vec![0xAA; 30]);
        assert!(!format!("{key:?}").contains("AA"));
    }

    #[test]
    fn p2p_candidates_extracts_ip_pipe_port_and_object_candidates() {
        let raw = serde_json::json!({
            "cmd": 401,
            "callId": 123,
            "extendData": {
                "srtpMode": 1,
                "extraInfo": {
                    "p2pAddr": "198.51.100.1|50000,198.51.100.2:50001",
                    "p2pCandidates": [
                        { "ip": "198.51.100.3", "port": 50002 }
                    ]
                }
            }
        });
        let parsed = CallSignalWireData::from_value(&raw);
        let cands = parsed.p2p_candidates();
        assert_eq!(cands.len(), 3);
        assert_eq!(cands[0], "198.51.100.1:50000".parse().unwrap());
        assert_eq!(cands[1], "198.51.100.2:50001".parse().unwrap());
        assert_eq!(cands[2], "198.51.100.3:50002".parse().unwrap());
    }
}
