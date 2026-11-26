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
        // Observed in ZaloCall ZCallInfo::_processRequestZrtpJob (0x1000cb558):
        // active audio codec negotiated in 416/402 is opus/16000/1 on PT 112.
        Self {
            name: "opus/16000/1".to_string(),
            payload: 112,
            frm_ptime: 20,
            dynamic_fptime: 0,
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
#[derive(Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayServerAddr {
    pub ip: String,
    pub port: u16,
    #[serde(default)]
    pub server_type: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtcp_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtp_ipv6: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtcp_ipv6: Option<String>,
}

impl RelayServerAddr {
    /// Format the IPv4 RTP `ip:port` endpoint string.
    pub fn rtp_host_port(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }

    /// Format the IPv4 RTCP `ip:port` endpoint string, falling back to [`Self::rtp_host_port`].
    pub fn rtcp_host_port(&self) -> String {
        self.rtcp_addr
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.rtp_host_port())
    }
}

impl fmt::Debug for RelayServerAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayServerAddr")
            .field("ip", &"<redacted-ip>")
            .field("port", &self.port)
            .field("server_type", &self.server_type)
            .field(
                "rtcp_addr",
                &self.rtcp_addr.as_ref().map(|_| "<redacted-rtcp>"),
            )
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
    pub act: Option<Value>,
    #[serde(rename = "callId", alias = "call_id", alias = "id")]
    pub call_id: Option<Value>,
    #[serde(rename = "sessId", alias = "sess_id", alias = "session")]
    pub sess_id: Option<Value>,
    #[serde(rename = "callerId", alias = "uid", alias = "fromUid")]
    pub caller_id: Option<Value>,
    #[serde(rename = "calleeId", alias = "toUid")]
    pub callee_id: Option<Value>,
    #[serde(rename = "uidN")]
    pub uid_n: Option<Value>,
    #[serde(rename = "fromId", alias = "uidFrom")]
    pub from_media_id: Option<Value>,
    #[serde(rename = "toId", alias = "uidTo")]
    pub to_media_id: Option<Value>,
    pub status: Option<Value>,
    #[serde(rename = "isVideo", alias = "callType")]
    pub is_video: Option<Value>,
    #[serde(rename = "extendData")]
    pub extend_data: Option<Value>,
    #[serde(
        rename = "rtpIP",
        alias = "rtpIp",
        alias = "rtpAddress",
        alias = "rtpSerIp"
    )]
    pub rtp_ip: Option<Value>,
    #[serde(rename = "rtcpIP", alias = "rtcpIp", alias = "rtcpAddress")]
    pub rtcp_ip: Option<Value>,
    pub servers: Option<Value>,
    #[serde(rename = "zrtc_config")]
    pub zrtc_config: Option<Value>,
    #[serde(rename = "srtpKey", alias = "key", alias = "srtp")]
    pub srtp_key: Option<Value>,
    pub codec: Option<Value>,
    #[serde(rename = "Dname")]
    pub display_name: Option<Value>,
    pub avatar: Option<Value>,
    #[serde(rename = "isOnCam")]
    pub is_on_cam: Option<Value>,
    #[serde(rename = "isOnMic")]
    pub is_on_mic: Option<Value>,
    #[serde(rename = "type")]
    pub conf_type: Option<Value>,
}

impl fmt::Debug for CallSignalWireData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallSignalWireData")
            .field("cmd", &self.cmd)
            .field("act", &self.act)
            .field("call_id", &self.call_id)
            .field("sess_id", &self.sess_id)
            .field("caller_id", &self.caller_id)
            .field("callee_id", &self.callee_id)
            .field("status", &self.status)
            .field("is_video", &self.is_video)
            .field("rtp_ip", &self.rtp_ip.as_ref().map(|_| "<redacted>"))
            .field("servers", &self.servers.as_ref().map(|_| "<redacted>"))
            .field("srtp_key", &self.srtp_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn parse_json_object_field(v: Option<&Value>) -> Option<serde_json::Map<String, Value>> {
    match v? {
        Value::Object(map) => Some(map.clone()),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            match serde_json::from_str::<Value>(trimmed).ok()? {
                Value::Object(map) => Some(map),
                _ => None,
            }
        }
        _ => None,
    }
}

fn canonical_signal_key(key: &str) -> &str {
    match key {
        "call_id" | "id" => "callId",
        "sess_id" | "session" => "sessId",
        "uid" | "fromUid" => "callerId",
        "toUid" => "calleeId",
        "uidFrom" => "fromId",
        "uidTo" => "toId",
        "callType" => "isVideo",
        "rtpIp" | "rtpAddress" | "rtpSerIp" => "rtpIP",
        "rtcpIp" | "rtcpAddress" => "rtcpIP",
        "key" | "srtp" => "srtpKey",
        other => other,
    }
}

fn insert_canonical_fields(
    target: &mut serde_json::Map<String, Value>,
    source: &serde_json::Map<String, Value>,
    overwrite: bool,
) {
    for (k, val) in source {
        if k == "data" || k == "params" {
            continue;
        }
        let canon = canonical_signal_key(k).to_string();
        if overwrite || !target.contains_key(&canon) {
            target.insert(canon, val.clone());
        }
    }
}

impl CallSignalWireData {
    /// Construct from a raw JSON [`Value`], unwrapping `data` and nested `params` envelopes
    /// (including JSON-encoded strings from WebSocket `601`/`604` VoIP controls) when present.
    pub fn from_value(v: &Value) -> Self {
        let mut merged = serde_json::Map::new();
        if let Some(outer_map) = parse_json_object_field(Some(v)) {
            insert_canonical_fields(&mut merged, &outer_map, true);
            if let Some(data_map) = parse_json_object_field(outer_map.get("data")) {
                insert_canonical_fields(&mut merged, &data_map, true);
                if let Some(params_map) = parse_json_object_field(data_map.get("params")) {
                    insert_canonical_fields(&mut merged, &params_map, false);
                }
            }
            if let Some(params_map) = parse_json_object_field(outer_map.get("params")) {
                insert_canonical_fields(&mut merged, &params_map, false);
            }
        }
        if merged.is_empty() {
            let inner = wire::unwrap_data(v);
            if let Some(inner_map) = parse_json_object_field(Some(inner)) {
                insert_canonical_fields(&mut merged, &inner_map, true);
            }
        }
        serde_json::from_value(Value::Object(merged)).unwrap_or_default()
    }

    /// Return the parsed [`CallSignalCode`] from numeric `cmd` (`401..=416`) or WebSocket `act` string.
    pub fn signal_code(&self) -> Option<CallSignalCode> {
        if let Some(code) = wire::val_u64(self.cmd.as_ref())
            .and_then(|c| u16::try_from(c).ok())
            .and_then(CallSignalCode::from_code)
        {
            return Some(code);
        }
        let act = wire::field_str(self.act.as_ref())?;
        Some(match act.as_str() {
            "request" => CallSignalCode::RequestCall,
            "ring_ring" => CallSignalCode::RingRing,
            "answer" => CallSignalCode::Answer,
            "answer_ack" => CallSignalCode::AnswerAck,
            "cancel" | "reject" | "busy" => CallSignalCode::Cancel,
            "end_call" => CallSignalCode::EndCall,
            "hold_req" | "hold" => CallSignalCode::HoldReq,
            "hold_resp" => CallSignalCode::HoldResp,
            "unhold_req" | "unhold" => CallSignalCode::UnholdReq,
            "unhold_resp" => CallSignalCode::UnholdResp,
            "onoff_camera" | "mute_audio" | "share_screen" => CallSignalCode::Conf,
            _ => return None,
        })
    }

    /// Extract remote peer mic mute state (`Some(true)` = muted, `Some(false)` = unmuted)
    /// from incoming `mute_audio` (`params.type`/`status`) or `12006` (`isOnMic`) frames.
    pub fn peer_muted_update(&self) -> Option<bool> {
        if self.signal_code() != Some(CallSignalCode::Conf) {
            return None;
        }
        let act = wire::field_str(self.act.as_ref());
        if let Some(ref a) = act {
            if a != "mute_audio" {
                return None;
            }
            let raw = wire::val_u64(self.conf_type.as_ref())
                .or_else(|| wire::val_u64(self.status.as_ref()))?;
            return Some(raw == 1);
        }
        let raw = wire::val_u64(self.is_on_mic.as_ref())?;
        (raw == 0 || raw == 1).then_some(raw == 1)
    }

    /// Return the numeric `callId`.
    pub fn call_id_u32(&self) -> Option<u32> {
        wire::val_u64(self.call_id.as_ref()).and_then(|v| u32::try_from(v).ok())
    }

    /// Return `sessId` as a non-empty string.
    pub fn sess_id_str(&self) -> Option<String> {
        wire::field_str(self.sess_id.as_ref())
    }

    /// Return the caller's Zalo account ID (`uidN` / `callerId`, falling back to `fromId`).
    pub fn caller_id_str(&self) -> Option<String> {
        wire::field_str(self.uid_n.as_ref())
            .or_else(|| wire::field_str(self.caller_id.as_ref()))
            .or_else(|| wire::field_str(self.from_media_id.as_ref()))
    }

    /// Return the callee's Zalo account ID (`calleeId`, falling back to `toId`).
    pub fn callee_id_str(&self) -> Option<String> {
        wire::field_str(self.callee_id.as_ref())
            .or_else(|| wire::field_str(self.to_media_id.as_ref()))
    }

    /// Return the 32-bit VoIP media UID (`fromId` in `11300`, `uidFrom` in `act:"request"`).
    ///
    /// Verified in `ZaloCall`:
    /// - `ZMessageDeserializer::_parse401Response` (`0x10001d692`): `fromId` $\rightarrow$ `resp+0x10`
    /// - `ZMessageDeserializer::_parseActRequest` (`0x10001c88c`): `uidFrom` $\rightarrow$ `info+0x14`
    pub fn from_media_uid_u32(&self) -> Option<u32> {
        wire::val_u64(self.from_media_id.as_ref())
            .or_else(|| wire::val_u64(self.caller_id.as_ref()))
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v != 0)
    }

    /// Return the 32-bit VoIP media UID (`toId` in `11300`, `uidTo` in `act:"request"`).
    ///
    /// Verified in `ZaloCall`:
    /// - `ZMessageDeserializer::_parse401Response` (`0x10001d6b0`): `toId` $\rightarrow$ `resp+0x14`
    /// - `ZMessageDeserializer::_parseActRequest` (`0x10001c89f`): `uidTo` $\rightarrow$ `info+0x18`
    pub fn to_media_uid_u32(&self) -> Option<u32> {
        wire::val_u64(self.to_media_id.as_ref())
            .or_else(|| wire::val_u64(self.callee_id.as_ref()))
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v != 0)
    }

    /// Return optional peer display name (`Dname` inside incoming `params`).
    pub fn peer_display_name(&self) -> Option<String> {
        wire::field_str(self.display_name.as_ref())
    }

    /// Return optional peer avatar URL (`avatar` inside incoming `params`).
    pub fn peer_avatar_url(&self) -> Option<String> {
        wire::field_str(self.avatar.as_ref())
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

    /// Parse relay server endpoints (`servers` / `rtpIP`) from signaling payloads.
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
                            rtcp_addr: None,
                            rtp_ipv6: None,
                            rtcp_ipv6: None,
                        });
                    }
                }
            }
            None
        }

        fn parse_relay_object(obj: &serde_json::Map<String, Value>) -> Option<RelayServerAddr> {
            let hp = obj
                .get("rtpaddr")
                .or_else(|| obj.get("rtp"))
                .or_else(|| obj.get("addr"))
                .and_then(Value::as_str)?;
            let mut addr = parse_host_port(hp, 1)?;
            addr.rtcp_addr = obj
                .get("rtcpaddr")
                .or_else(|| obj.get("rtcp"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            addr.rtp_ipv6 = obj
                .get("rtpIPv6")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            addr.rtcp_ipv6 = obj
                .get("rtcpIPv6")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Some(addr)
        }

        fn collect_from_val(val: &Value, out: &mut Vec<RelayServerAddr>) {
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
                            Value::Object(obj) => {
                                if let Some(addr) = parse_relay_object(obj) {
                                    out.push(addr);
                                    continue;
                                }
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
                Value::Object(obj) => {
                    if let Some(addr) = parse_relay_object(obj) {
                        out.push(addr);
                        return;
                    }
                    if let Ok(addr) = serde_json::from_value::<RelayServerAddr>(val.clone()) {
                        if !addr.ip.is_empty() && addr.port > 0 {
                            out.push(addr);
                        }
                    }
                }
                _ => {}
            }
        }

        let mut out = Vec::new();
        if let Some(servers) = self.servers.as_ref() {
            collect_from_val(servers, &mut out);
        }
        if let Some(rtp_ip) = self.rtp_ip.as_ref() {
            let rtcp_fallback = wire::field_str(self.rtcp_ip.as_ref());
            let prev_len = out.len();
            collect_from_val(rtp_ip, &mut out);
            if let Some(rtcp) = rtcp_fallback {
                for added in &mut out[prev_len..] {
                    if added.rtcp_addr.is_none() {
                        added.rtcp_addr = Some(rtcp.clone());
                    }
                }
            }
        }
        out
    }

    /// Parse SRTP master key + salt (`30` bytes) from `srtpKey` / `key` / `srtp`, falling back to
    /// the first 30 ASCII bytes of `sessId` per `CallController::parseCallConfig` (`0x100171c50`) (`INV-8`).
    pub fn srtp_key_material(&self) -> Option<SrtpKeyMaterial> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let required_len = crate::protocol::call::SRTP_MASTER_KEY_LEN
            + crate::protocol::call::SRTP_MASTER_SALT_LEN;

        if let Some(raw) = wire::field_str(self.srtp_key.as_ref()) {
            if let Ok(decoded) = STANDARD.decode(raw.as_bytes()) {
                if decoded.len() >= required_len {
                    return Some(SrtpKeyMaterial::from_bytes(decoded));
                }
            }
            let bytes = raw.into_bytes();
            if bytes.len() >= required_len {
                return Some(SrtpKeyMaterial::from_bytes(bytes));
            }
        }

        // In 1-1 ZRTC calls (`11300` / `416`), `srtpKey` is omitted and the 30-byte master key + salt
        // is embedded as the first 30 ASCII bytes of the 154-byte `sessId`.
        if let Some(sess_id) = self.sess_id_str() {
            if sess_id.len() >= required_len {
                return Some(SrtpKeyMaterial::from_bytes(
                    sess_id.as_bytes()[..required_len].to_vec(),
                ));
            }
        }
        None
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

/// Generate a client-side non-zero `callId` in `100_000_000..1_900_000_000` for `11300`.
fn next_client_call_id() -> u32 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(100_000_000);
    ((ms % 1_800_000_000) + 100_000_000) as u32
}

/// Build `11300` `/api/voicecall/requestcall` parameters.
pub fn build_request_call_params(
    callee_id: &str,
    call_type: u8,
    _extend_data: &ExtendData,
) -> Result<Value> {
    let type_request = if call_type == 0 { 1 } else { call_type };
    Ok(wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callId", Value::from(next_client_call_id())),
        ("codec", Value::String("[]".to_string())),
        ("typeRequest", Value::from(type_request)),
    ]))
}

/// Build `11301` `/api/voicecall/request` parameters (`ZMessageSerializer::sendZRtpRequestCall` @ `0x100026020`).
pub fn build_request_params(
    callee_id: &str,
    call_id: u32,
    sess_id: &str,
    extend_data: &ExtendData,
) -> Result<Value> {
    let first_relay = extend_data
        .server_addr
        .as_ref()
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("rtp").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let first_rtcp = extend_data
        .server_addr
        .as_ref()
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("rtcp").and_then(Value::as_str))
        .unwrap_or(first_relay.as_str())
        .to_string();
    let active_codecs = extend_data
        .audio_codec
        .clone()
        .unwrap_or_else(|| vec![CodecEntry::default()]);
    let codec_json = serde_json::to_string(&active_codecs)?;
    Ok(wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("rtcpAddress", Value::String(first_rtcp)),
        ("rtpAddress", Value::String(first_relay)),
        ("codec", Value::String(codec_json)),
        ("session", Value::String(sess_id.to_string())),
        ("callId", Value::from(call_id)),
        ("extendData", Value::String(extend_data.to_json_string()?)),
        ("subCommand", Value::from(3)),
    ]))
}

/// Build `11302` `/api/voicecall/ringring` parameters.
pub fn build_ring_ring_params(caller_id: &str, call_id: u32, _sess_id: &str, status: i32) -> Value {
    wire::obj(&[
        ("callerId", Value::String(caller_id.to_string())),
        ("callId", Value::from(call_id)),
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
    let (codec_json, first_relay, first_rtcp, ext_json) = if status == 0 {
        let rtp = extend_data
            .server_addr
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("rtp").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let rtcp = extend_data
            .server_addr
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("rtcp").and_then(Value::as_str))
            .unwrap_or(rtp.as_str())
            .to_string();
        let codecs = extend_data
            .audio_codec
            .clone()
            .unwrap_or_else(|| vec![CodecEntry::default()]);
        (
            serde_json::to_string(&codecs)?,
            rtp,
            rtcp,
            extend_data.to_json_string()?,
        )
    } else {
        (String::new(), String::new(), String::new(), String::new())
    };
    Ok(wire::obj(&[
        ("callerId", Value::String(caller_id.to_string())),
        ("callId", Value::from(call_id)),
        ("status", Value::from(status)),
        ("codec", Value::String(codec_json)),
        ("rtcpAddress", Value::String(first_rtcp)),
        ("rtpAddress", Value::String(first_relay)),
        ("session_id", Value::String(sess_id.to_string())),
        ("extendData", Value::String(ext_json)),
        ("subCommand", Value::from(3)),
    ]))
}

/// Build `11304` `/api/voicecall/answerack` parameters.
pub fn build_answer_ack_params(callee_id: &str, call_id: u32, _sess_id: &str) -> Value {
    wire::obj(&[
        ("calleeId", Value::String(callee_id.to_string())),
        ("callId", Value::from(call_id)),
        ("status", Value::from(0)),
    ])
}

/// Build `11305` `/api/voicecall/cancel` parameters.
pub fn build_cancel_params(callee_id: &str, call_id: u32, _sess_id: &str) -> Value {
    wire::obj(&[
        ("callerId", Value::String(callee_id.to_string())),
        ("callId", Value::from(call_id)),
        ("callType", Value::from(1)),
        ("status", Value::from(0)),
    ])
}

/// Build `11306` `/api/voicecall/endcall` parameters.
pub fn build_end_call_params(peer_id: &str, call_id: u32, _sess_id: &str, _reason: i32) -> Value {
    wire::obj(&[
        ("uidTo", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("status", Value::from(3)),
    ])
}

/// Build `11307` `/api/voicecall/holdreq` parameters.
pub fn build_hold_req_params(peer_id: &str, call_id: u32, _sess_id: &str) -> Value {
    wire::obj(&[
        ("toId", Value::String(peer_id.to_string())),
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
    ])
}

/// Build `11308` `/api/voicecall/holdresp` parameters.
pub fn build_hold_resp_params(peer_id: &str, call_id: u32, _sess_id: &str, status: i32) -> Value {
    wire::obj(&[
        ("toId", Value::String(peer_id.to_string())),
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("status", Value::from(status)),
    ])
}

/// Build `11309` `/api/voicecall/unholdreq` parameters.
pub fn build_unhold_req_params(peer_id: &str, call_id: u32, _sess_id: &str) -> Value {
    wire::obj(&[
        ("toId", Value::String(peer_id.to_string())),
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
    ])
}

/// Build `11310` `/api/voicecall/unholdresp` parameters.
pub fn build_unhold_resp_params(peer_id: &str, call_id: u32, _sess_id: &str, status: i32) -> Value {
    wire::obj(&[
        ("toId", Value::String(peer_id.to_string())),
        ("calleeId", Value::String(peer_id.to_string())),
        ("callId", Value::from(call_id)),
        ("status", Value::from(status)),
    ])
}

/// Build `11311` `/api/voicecall/logendcall` parameters.
pub fn build_log_end_call_params(
    peer_id: &str,
    call_id: u32,
    _sess_id: &str,
    duration_secs: u64,
    status: i32,
) -> Value {
    wire::obj(&[
        ("call_id", Value::from(call_id)),
        ("partner_id", Value::String(peer_id.to_string())),
        ("status", Value::from(status)),
        ("logdata", Value::String(String::new())),
        ("duration", Value::from(duration_secs)),
        ("role", Value::from(0)),
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

fn debug_json_shape(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(_) => "num".to_string(),
        Value::String(s) => format!("str(len={})", s.len()),
        Value::Array(a) => format!(
            "arr(len={},first={})",
            a.len(),
            a.first().map(debug_json_shape).unwrap_or_default()
        ),
        Value::Object(m) => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, val)| format!("{k}:{}", debug_json_shape(val)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

async fn get_voicecall(
    session: &Session,
    spec: crate::protocol::registry::CommandSpec,
    mut params: Value,
    with_imei: bool,
) -> Result<CallSignalWireData> {
    let _ = wire::require_login(session)?;
    if with_imei {
        if let Some(obj) = params.as_object_mut() {
            obj.entry("imei")
                .or_insert_with(|| Value::String(session.imei.clone()));
        }
    }
    let resp = session.api_get(spec, Some(&params)).await?;
    crate::logger::call_debug(&format!(
        "[DEBUG-call] voicecall_resp cmd={} shape={}",
        spec.id,
        debug_json_shape(&resp)
    ));
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
    let client_call_id = params.get("callId").and_then(Value::as_u64);
    let mut wire = get_voicecall(session, voice_call::REQUEST_CALL, params, true).await?;
    if wire.call_id_u32().is_none() {
        if let Some(cid) = client_call_id {
            wire.call_id = Some(Value::from(cid));
        }
    }
    Ok(wire)
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
    get_voicecall(session, voice_call::REQUEST, params, true).await
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
    get_voicecall(session, voice_call::RING_RING, params, true).await
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
    get_voicecall(session, voice_call::ANSWER, params, false).await
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
    get_voicecall(session, voice_call::ANSWER_ACK, params, true).await
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
    get_voicecall(session, voice_call::CANCEL, params, true).await
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
    get_voicecall(session, voice_call::END_CALL, params, true).await
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
    get_voicecall(session, voice_call::HOLD_REQ, params, true).await
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
    get_voicecall(session, voice_call::HOLD_RESP, params, true).await
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
    get_voicecall(session, voice_call::UNHOLD_REQ, params, true).await
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
    get_voicecall(session, voice_call::UNHOLD_RESP, params, true).await
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
    get_voicecall(session, voice_call::LOG_END_CALL, params, true).await
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
    get_voicecall(session, voice_call::CONF, params, false).await
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
        assert!(req["callId"].as_u64().unwrap_or(0) >= 100_000_000);
        assert_eq!(req["typeRequest"], 1);
        assert_eq!(req["codec"], "[]");

        let req_init = build_request_params("user-2", 464564007, "sess-1", &ext).unwrap();
        assert_eq!(req_init["callId"], 464564007);
        assert_eq!(req_init["subCommand"], 3);
        let parsed_ext = ExtendData::from_value(&req_init["extendData"]).unwrap();
        assert_eq!(parsed_ext.srtp_mode, 1); // (INV-8)

        let ans = build_answer_params("user-1", 464564007, "sess-1", 0, &ext).unwrap();
        assert_eq!(ans["callerId"], "user-1");
        assert_eq!(ans["callId"], 464564007);
        assert_eq!(ans["status"], 0);
        assert_eq!(ans["subCommand"], 3);

        let conf = build_conf_params("user-2", 464564007, false, true, None);
        assert_eq!(conf["calleeId"], "user-2");
        assert_eq!(conf["callId"], 464564007);
        assert_eq!(conf["isOnCam"], 0);
        assert_eq!(conf["isOnMic"], 1);

        let log_end = build_log_end_call_params("user-2", 464564007, "sess-1", 42, 0);
        assert_eq!(log_end["call_id"], 464564007);
        assert_eq!(log_end["partner_id"], "user-2");
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
            rtcp_addr: Some("198.51.100.42:4201".to_string()),
            rtp_ipv6: None,
            rtcp_ipv6: None,
        };
        assert!(!format!("{relay:?}").contains("198.51.100.42"));

        let key = SrtpKeyMaterial::from_bytes(vec![0xAA; 30]);
        assert!(!format!("{key:?}").contains("AA"));
    }

    #[test]
    fn call_signal_wire_data_extracts_32bit_media_uids_and_sess_id_srtp_fallback() {
        let sess_154 = "0123456789abcdef0123456789ABCD".to_string() + &"x".repeat(124);
        let resp_11300 = serde_json::json!({
            "error_code": 0,
            "data": {
                "id": 1188977383,
                "fromId": 314159265,
                "toId": 271828182,
                "sessId": sess_154,
                "rtpIP": "198.51.100.10:4200",
                "rtcpIP": "198.51.100.10:4201",
                "servers": [
                    {
                        "rtpaddr": "198.51.100.11:4200",
                        "rtcpaddr": "198.51.100.11:4201",
                        "rtpIPv6": "[2001:db8::1]:4200",
                        "rtcpIPv6": "[2001:db8::1]:4201",
                        "spTcp": 0,
                        "bonus": 0
                    }
                ]
            }
        });
        let parsed = CallSignalWireData::from_value(&resp_11300);
        assert_eq!(parsed.call_id_u32(), Some(1188977383));
        assert_eq!(parsed.from_media_uid_u32(), Some(314159265));
        assert_eq!(parsed.to_media_uid_u32(), Some(271828182));

        // (INV-8) When `srtpKey` is absent in 11300, first 30 ASCII bytes of `sessId` provide SRTP key+salt.
        let srtp = parsed
            .srtp_key_material()
            .expect("should fallback to 30-byte sessId prefix");
        assert_eq!(srtp.as_bytes(), b"0123456789abcdef0123456789ABCD");

        let relays = parsed.relay_servers();
        assert_eq!(relays.len(), 2);
        assert_eq!(relays[0].rtp_host_port(), "198.51.100.11:4200");
        assert_eq!(relays[0].rtcp_host_port(), "198.51.100.11:4201");
        assert_eq!(relays[1].rtp_host_port(), "198.51.100.10:4200");
        assert_eq!(relays[1].rtcp_host_port(), "198.51.100.10:4201");
    }

    #[test]
    fn call_signal_wire_data_parses_ws_act_request_envelope_with_nested_params() {
        let ws_control = serde_json::json!({
            "fileType": "voip",
            "act": "request",
            "data": {
                "callId": 1188977383,
                "uidFrom": 314159265,
                "uidTo": 271828182,
                "uidN": "8491234567890123456",
                "session": "0123456789abcdef0123456789ABCDxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                "rtpAddress": "198.51.100.11:4200",
                "rtcpAddress": "198.51.100.11:4201",
                "params": "{\"id\":1188977383,\"Dname\":\"Caller Name\",\"avatar\":\"https://example.com/a.jpg\",\"extendData\":\"{\\\"srtpMode\\\":1,\\\"maxFT\\\":60}\"}"
            }
        });
        let parsed = CallSignalWireData::from_value(&ws_control);
        assert_eq!(parsed.signal_code(), Some(CallSignalCode::RequestCall));
        assert_eq!(parsed.call_id_u32(), Some(1188977383));
        assert_eq!(
            parsed.caller_id_str().as_deref(),
            Some("8491234567890123456")
        );
        assert_eq!(parsed.from_media_uid_u32(), Some(314159265));
        assert_eq!(parsed.to_media_uid_u32(), Some(271828182));
        assert_eq!(parsed.peer_display_name().as_deref(), Some("Caller Name"));
        assert!(parsed.srtp_key_material().is_some());
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
