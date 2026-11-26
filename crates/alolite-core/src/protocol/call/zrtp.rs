//! ZRTC UDP packet wire codec (`0x01` Control, `0x02` ServerReply, `0x03..0x0f` Relay Media, `0x7f` P2P).
//!
//! Byte layouts and command tables documented in `re-notes/16-zrtc-network-handover-p2p-and-audio-engine.md`.

use anyhow::{bail, Result};

/// Outer ZRTP datagram message types (byte 0).
pub const MSG_CONTROL: u8 = 0x01;
pub const MSG_SERVER_REPLY: u8 = 0x02;
pub const MSG_AUDIO_RTP_UP: u8 = 0x03;
pub const MSG_AUDIO_RTP_DOWN: u8 = 0x04;
pub const MSG_AUDIO_RTCP_UP: u8 = 0x05;
pub const MSG_MEDIA_07_UP: u8 = 0x07;
pub const MSG_VIDEO_RTP_UP: u8 = 0x0d;
pub const MSG_VIDEO_DOWN: u8 = 0x0e;
pub const MSG_VIDEO_RTCP_UP: u8 = 0x0f;
pub const MSG_P2P_EXT: u8 = 0x7f;

/// Fixed header length for `0x01` (CONTROL) and `0x02` (SERVER_REPLY).
pub const CONTROL_HEADER_LEN: usize = 21;

/// Fixed header length for `0x7f` (P2P_EXT).
pub const P2P_HEADER_LEN: usize = 9;

/// Control command IDs (`cmd` at offset `0x12`, `u16 LE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ZrtpCmd {
    Ping = 2,
    EndCall = 3,
    Echo = 5,
    InitCallCaller = 11,
    InitCallCallee = 12,
    ChangeAddress = 14,
    ReqForward = 32,
    GroupMessage = 33,
    RoomUpdated = 50,
    InitZaviPing = 51,
}

impl ZrtpCmd {
    /// Convert a raw `u16` command ID into [`ZrtpCmd`].
    pub const fn from_u16(cmd: u16) -> Option<Self> {
        Some(match cmd {
            2 => Self::Ping,
            3 => Self::EndCall,
            5 => Self::Echo,
            11 => Self::InitCallCaller,
            12 => Self::InitCallCallee,
            14 => Self::ChangeAddress,
            32 => Self::ReqForward,
            33 => Self::GroupMessage,
            50 => Self::RoomUpdated,
            51 => Self::InitZaviPing,
            _ => return None,
        })
    }
}

/// `REQ_FORWARD` (`cmd = 32`) sub-commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ReqForwardSubCmd {
    CodecStepDown = 1,
    StopP2p = 2,
    RestartP2p = 3,
    PartnerCamOff = 4,
    PartnerCamOn = 5,
    PartnerCamPaused = 6,
    ChannelPrime = 8,
    LinkHeartbeat = 10,
    VideoQualityConfig = 12,
    VideoQualityRequest = 13,
}

impl ReqForwardSubCmd {
    /// Convert a raw `u8` sub-command into [`ReqForwardSubCmd`].
    pub const fn from_u8(sub: u8) -> Option<Self> {
        Some(match sub {
            1 => Self::CodecStepDown,
            2 => Self::StopP2p,
            3 => Self::RestartP2p,
            4 => Self::PartnerCamOff,
            5 => Self::PartnerCamOn,
            6 => Self::PartnerCamPaused,
            8 => Self::ChannelPrime,
            10 => Self::LinkHeartbeat,
            12 => Self::VideoQualityConfig,
            13 => Self::VideoQualityRequest,
            _ => return None,
        })
    }
}

/// Direct P2P (`msgType = 0x7f`) packet kinds (`kind` at offset `0x07`, `u16 LE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum P2pKind {
    Bind = 1,
    BindPeer = 2,
    BindAnswer = 3,
    Keepalive = 4,
    Signal = 5,
    Audio = 6,
    AudioRtcp = 7,
    Video = 8,
    VideoRtcp = 9,
}

impl P2pKind {
    /// Convert a raw `u16` kind into [`P2pKind`].
    pub const fn from_u16(kind: u16) -> Option<Self> {
        Some(match kind {
            1 => Self::Bind,
            2 => Self::BindPeer,
            3 => Self::BindAnswer,
            4 => Self::Keepalive,
            5 => Self::Signal,
            6 => Self::Audio,
            7 => Self::AudioRtcp,
            8 => Self::Video,
            9 => Self::VideoRtcp,
            _ => return None,
        })
    }
}

/// `P2P.SIGNAL` (`0x7f`, `kind = 5`) sub-commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum P2pSignalSubCmd {
    Start = 1,
    Stop = 2,
}

impl P2pSignalSubCmd {
    /// Convert a raw `u8` into [`P2pSignalSubCmd`].
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Start,
            2 => Self::Stop,
            _ => return None,
        })
    }
}

/// Decoded payload for `0x01` Control / `0x02` ServerReply packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlBody {
    /// `cmd = 2` (`PING`) or `cmd = 14` (`CHANGE_ADDRESS`): `u16str sessId`.
    SessId(String),
    /// `cmd = 3` (`END_CALL`): `u32 LE callId`.
    EndCall { call_id: u32 },
    /// `cmd = 5` (`ECHO`): `u16str timestamp`.
    Echo(String),
    /// `cmd = 11` / `12` (`INIT_CALL_CALLER` / `INIT_CALL_CALLEE`).
    InitCall {
        call_id: u32,
        peer_uid: u32,
        sess_id: String,
        extra: Option<String>,
    },
    /// `cmd = 32` (`REQ_FORWARD`) `subCmd 1..=6` or `cmd = 33` (`GROUP_MESSAGE`): empty body.
    Empty,
    /// `cmd = 32`, `subCmd = 8`: 8-byte prime payload.
    ForwardPrime([u8; 8]),
    /// `cmd = 32`, `subCmd = 10`: 6 x `u32 LE` link heartbeat.
    LinkHeartbeat([u32; 6]),
    /// `cmd = 50` (`ROOM_UPDATED`): active member UIDs.
    RoomUpdated(Vec<u32>),
    /// `cmd = 51` (`INIT_ZAVI_PING`).
    InitZaviPing {
        field0: u32,
        field1: u32,
        sess_id: String,
        extra: Option<String>,
    },
    /// Raw fallback for `SERVER_REPLY` (`0x02`) or other variable payloads.
    Raw(Vec<u8>),
}

/// Decoded payload for `0x7f` Direct P2P packets (`kind = 1..=9`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2pBody {
    /// `kind = 1` (`BIND`, 15 bytes total).
    Bind { seq: u16, cand_index: u32 },
    /// `kind = 2` (`BIND_PEER`): `seq + candIndex + addrLen + "ip|port" + 0xff`.
    BindPeer {
        seq: u16,
        cand_index: u32,
        addr: String,
    },
    /// `kind = 3` (`BIND_ANSWER`): `seq + candIndex + addrLen + "ip|port"`.
    BindAnswer {
        seq: u16,
        cand_index: u32,
        addr: String,
    },
    /// `kind = 4` (`KEEPALIVE`): `subCmd (1=ping, 2=pong) + u32 LE seq + payload`.
    Keepalive {
        sub_cmd: u8,
        seq: u32,
        payload: Vec<u8>,
    },
    /// `kind = 5` (`SIGNAL`, 14 bytes total): `subCmd (1=start, 2=stop) + u32 LE seq`.
    Signal { sub_cmd: u8, seq: u32 },
    /// `kind = 6` (`AUDIO`), `7` (`AUDIO_RTCP`), `8` (`VIDEO`), `9` (`VIDEO_RTCP`).
    Media(Vec<u8>),
}

/// Unified ZRTC UDP datagram representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZRTPPacket {
    /// `0x01` Control or `0x02` ServerReply with 21-byte header.
    Control {
        is_reply: bool,
        magic: u8,
        field02: u32,
        seq: u32,
        uid: u32,
        token: u32,
        cmd: u16,
        sub_cmd: u8,
        body: ControlBody,
    },
    /// `0x7f` Direct P2P extension packet (`kind = 1..=9`).
    P2p {
        magic: u8,
        sender_role: u8,
        call_id: u32,
        kind: P2pKind,
        body: P2pBody,
    },
    /// Uplink relay media datagram (`0x03`, `0x05`, `0x07`, `0x0d`, `0x0f`).
    RelayMediaUp {
        msg_type: u8,
        server_token: u32,
        payload: Vec<u8>,
    },
    /// Downlink relay media datagram (`0x04`, `0x0e`).
    RelayMediaDown { msg_type: u8, payload: Vec<u8> },
}

impl ZRTPPacket {
    /// Construct a `PING` (`cmd = 2`, `subCmd = 1`) control packet.
    pub fn ping(seq: u32, uid: u32, token: u32, sess_id: impl Into<String>) -> Self {
        Self::Control {
            is_reply: false,
            magic: 0x01,
            field02: 0,
            seq,
            uid,
            token,
            cmd: ZrtpCmd::Ping as u16,
            sub_cmd: 0x01,
            body: ControlBody::SessId(sess_id.into()),
        }
    }

    /// Construct an `END_CALL` (`cmd = 3`, `subCmd = 0`) control packet.
    pub fn end_call(seq: u32, uid: u32, token: u32, call_id: u32) -> Self {
        Self::Control {
            is_reply: false,
            magic: 0x7e,
            field02: 0,
            seq,
            uid,
            token,
            cmd: ZrtpCmd::EndCall as u16,
            sub_cmd: 0x00,
            body: ControlBody::EndCall { call_id },
        }
    }

    /// Construct an `INIT_CALL` (`cmd = 11` caller / `12` callee) control packet.
    pub fn init_call(
        is_caller: bool,
        seq: u32,
        uid: u32,
        call_id: u32,
        peer_uid: u32,
        sess_id: impl Into<String>,
    ) -> Self {
        Self::Control {
            is_reply: false,
            magic: 0x7e,
            field02: 0,
            seq,
            uid,
            token: 0,
            cmd: if is_caller {
                ZrtpCmd::InitCallCaller as u16
            } else {
                ZrtpCmd::InitCallCallee as u16
            },
            sub_cmd: 0x02,
            body: ControlBody::InitCall {
                call_id,
                peer_uid,
                sess_id: sess_id.into(),
                extra: None,
            },
        }
    }

    /// Construct a `CHANGE_ADDRESS` (`cmd = 14`, `subCmd = 1`) control packet.
    pub fn change_address(seq: u32, uid: u32, token: u32, sess_id: impl Into<String>) -> Self {
        Self::Control {
            is_reply: false,
            magic: 0x01,
            field02: 0,
            seq,
            uid,
            token,
            cmd: ZrtpCmd::ChangeAddress as u16,
            sub_cmd: 0x01,
            body: ControlBody::SessId(sess_id.into()),
        }
    }

    /// Construct a `REQ_FORWARD` (`cmd = 32`) bare 21-byte control packet (`subCmd 1..=6`).
    pub fn req_forward_empty(seq: u32, uid: u32, token: u32, sub_cmd: ReqForwardSubCmd) -> Self {
        Self::Control {
            is_reply: false,
            magic: 0x01,
            field02: 0,
            seq,
            uid,
            token,
            cmd: ZrtpCmd::ReqForward as u16,
            sub_cmd: sub_cmd as u8,
            body: ControlBody::Empty,
        }
    }

    /// Construct a `P2P.SIGNAL` (`0x7f`, `kind = 5`) packet.
    pub fn p2p_signal(sender_role: u8, call_id: u32, sub_cmd: P2pSignalSubCmd, seq: u32) -> Self {
        Self::P2p {
            magic: 0x00,
            sender_role,
            call_id,
            kind: P2pKind::Signal,
            body: P2pBody::Signal {
                sub_cmd: sub_cmd as u8,
                seq,
            },
        }
    }

    /// Serialize this packet into wire UDP bytes.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Control {
                is_reply,
                magic,
                field02,
                seq,
                uid,
                token,
                cmd,
                sub_cmd,
                body,
            } => {
                let mut out = Vec::with_capacity(CONTROL_HEADER_LEN + 32);
                out.push(if *is_reply {
                    MSG_SERVER_REPLY
                } else {
                    MSG_CONTROL
                });
                out.push(*magic);
                out.extend_from_slice(&field02.to_le_bytes());
                out.extend_from_slice(&seq.to_le_bytes());
                out.extend_from_slice(&uid.to_le_bytes());
                out.extend_from_slice(&token.to_le_bytes());
                out.extend_from_slice(&cmd.to_le_bytes());
                out.push(*sub_cmd);
                encode_control_body(body, &mut out);
                out
            }
            Self::P2p {
                magic,
                sender_role,
                call_id,
                kind,
                body,
            } => {
                let mut out = Vec::with_capacity(P2P_HEADER_LEN + 32);
                out.push(MSG_P2P_EXT);
                out.push(*magic);
                out.push(*sender_role);
                out.extend_from_slice(&call_id.to_le_bytes());
                out.extend_from_slice(&(*kind as u16).to_le_bytes());
                encode_p2p_body(body, &mut out);
                out
            }
            Self::RelayMediaUp {
                msg_type,
                server_token,
                payload,
            } => {
                let mut out = Vec::with_capacity(5 + payload.len());
                out.push(*msg_type);
                out.extend_from_slice(&server_token.to_le_bytes());
                out.extend_from_slice(payload);
                out
            }
            Self::RelayMediaDown { msg_type, payload } => {
                let mut out = Vec::with_capacity(1 + payload.len());
                out.push(*msg_type);
                out.extend_from_slice(payload);
                out
            }
        }
    }

    /// Deserialize wire UDP bytes into a [`ZRTPPacket`].
    ///
    /// # Errors
    /// Returns an error if the datagram is empty, truncated, or carries an unknown `msgType` / `kind`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let &msg_type = bytes
            .first()
            .ok_or_else(|| anyhow::anyhow!("empty ZRTP packet"))?;
        match msg_type {
            MSG_CONTROL | MSG_SERVER_REPLY => decode_control(msg_type == MSG_SERVER_REPLY, bytes),
            MSG_P2P_EXT => decode_p2p(bytes),
            MSG_AUDIO_RTP_UP | MSG_AUDIO_RTCP_UP | MSG_MEDIA_07_UP | MSG_VIDEO_RTP_UP
            | MSG_VIDEO_RTCP_UP => {
                if bytes.len() < 5 {
                    bail!(
                        "truncated uplink relay media packet ({} bytes)",
                        bytes.len()
                    );
                }
                let server_token = u32::from_le_bytes(bytes[1..5].try_into()?);
                Ok(Self::RelayMediaUp {
                    msg_type,
                    server_token,
                    payload: bytes[5..].to_vec(),
                })
            }
            MSG_AUDIO_RTP_DOWN | MSG_VIDEO_DOWN => Ok(Self::RelayMediaDown {
                msg_type,
                payload: bytes[1..].to_vec(),
            }),
            other => bail!("unknown ZRTP msgType: 0x{other:02x}"),
        }
    }
}

fn write_u16_str(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes[..usize::from(len)]);
}

fn read_u16_str(buf: &[u8], offset: &mut usize) -> Result<String> {
    if *offset + 2 > buf.len() {
        bail!("truncated u16 string length");
    }
    let len = usize::from(u16::from_le_bytes([buf[*offset], buf[*offset + 1]]));
    *offset += 2;
    if *offset + len > buf.len() {
        bail!("truncated u16 string body (need {len} bytes)");
    }
    let s = std::str::from_utf8(&buf[*offset..*offset + len])?.to_string();
    *offset += len;
    Ok(s)
}

fn read_u32_le(buf: &[u8], offset: &mut usize) -> Result<u32> {
    if *offset + 4 > buf.len() {
        bail!("truncated u32 LE");
    }
    let v = u32::from_le_bytes(buf[*offset..*offset + 4].try_into()?);
    *offset += 4;
    Ok(v)
}

fn encode_control_body(body: &ControlBody, out: &mut Vec<u8>) {
    match body {
        ControlBody::SessId(s) | ControlBody::Echo(s) => write_u16_str(s, out),
        ControlBody::EndCall { call_id } => out.extend_from_slice(&call_id.to_le_bytes()),
        ControlBody::InitCall {
            call_id,
            peer_uid,
            sess_id,
            extra,
        } => {
            out.extend_from_slice(&call_id.to_le_bytes());
            out.extend_from_slice(&peer_uid.to_le_bytes());
            write_u16_str(sess_id, out);
            if let Some(ex) = extra {
                write_u16_str(ex, out);
            }
        }
        ControlBody::Empty => {}
        ControlBody::ForwardPrime(bytes) => out.extend_from_slice(bytes),
        ControlBody::LinkHeartbeat(words) => {
            for w in words {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        ControlBody::RoomUpdated(uids) => {
            for uid in uids {
                out.extend_from_slice(&uid.to_le_bytes());
            }
        }
        ControlBody::InitZaviPing {
            field0,
            field1,
            sess_id,
            extra,
        } => {
            out.extend_from_slice(&field0.to_le_bytes());
            out.extend_from_slice(&field1.to_le_bytes());
            write_u16_str(sess_id, out);
            if let Some(ex) = extra {
                write_u16_str(ex, out);
            }
        }
        ControlBody::Raw(bytes) => out.extend_from_slice(bytes),
    }
}

fn decode_control(is_reply: bool, bytes: &[u8]) -> Result<ZRTPPacket> {
    if bytes.len() < CONTROL_HEADER_LEN {
        bail!("truncated ZRTP control header ({} bytes)", bytes.len());
    }
    let magic = bytes[1];
    let field02 = u32::from_le_bytes(bytes[2..6].try_into()?);
    let seq = u32::from_le_bytes(bytes[6..10].try_into()?);
    let uid = u32::from_le_bytes(bytes[10..14].try_into()?);
    let token = u32::from_le_bytes(bytes[14..18].try_into()?);
    let cmd = u16::from_le_bytes(bytes[18..20].try_into()?);
    let sub_cmd = bytes[20];
    let payload = &bytes[CONTROL_HEADER_LEN..];

    let body = if is_reply {
        match ZrtpCmd::from_u16(cmd) {
            Some(ZrtpCmd::RoomUpdated) if payload.len().is_multiple_of(4) => {
                ControlBody::RoomUpdated(decode_u32_le_list(payload)?)
            }
            _ => ControlBody::Raw(payload.to_vec()),
        }
    } else {
        decode_control_request_body(cmd, sub_cmd, payload)?
    };

    Ok(ZRTPPacket::Control {
        is_reply,
        magic,
        field02,
        seq,
        uid,
        token,
        cmd,
        sub_cmd,
        body,
    })
}

fn decode_u32_le_list(payload: &[u8]) -> Result<Vec<u32>> {
    if !payload.len().is_multiple_of(4) {
        bail!("ROOM_UPDATED payload must be multiple of 4 bytes");
    }
    Ok(payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect())
}

fn decode_control_request_body(cmd: u16, sub_cmd: u8, payload: &[u8]) -> Result<ControlBody> {
    let mut off = 0usize;
    match ZrtpCmd::from_u16(cmd) {
        Some(ZrtpCmd::Ping) | Some(ZrtpCmd::ChangeAddress) => {
            Ok(ControlBody::SessId(read_u16_str(payload, &mut off)?))
        }
        Some(ZrtpCmd::EndCall) => Ok(ControlBody::EndCall {
            call_id: read_u32_le(payload, &mut off)?,
        }),
        Some(ZrtpCmd::Echo) => Ok(ControlBody::Echo(read_u16_str(payload, &mut off)?)),
        Some(ZrtpCmd::InitCallCaller) | Some(ZrtpCmd::InitCallCallee) => {
            let call_id = read_u32_le(payload, &mut off)?;
            let peer_uid = read_u32_le(payload, &mut off)?;
            let sess_id = read_u16_str(payload, &mut off)?;
            let extra = if off < payload.len() {
                Some(read_u16_str(payload, &mut off)?)
            } else {
                None
            };
            Ok(ControlBody::InitCall {
                call_id,
                peer_uid,
                sess_id,
                extra,
            })
        }
        Some(ZrtpCmd::ReqForward) => match sub_cmd {
            1..=6 => {
                if !payload.is_empty() {
                    bail!("REQ_FORWARD subCmd {sub_cmd} expects empty payload");
                }
                Ok(ControlBody::Empty)
            }
            8 => {
                let arr: [u8; 8] = payload
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("REQ_FORWARD subCmd 8 expects 8 bytes"))?;
                Ok(ControlBody::ForwardPrime(arr))
            }
            10 => {
                if payload.len() != 24 {
                    bail!("REQ_FORWARD subCmd 10 expects 24 bytes");
                }
                let mut words = [0u32; 6];
                for (i, chunk) in payload.as_chunks::<4>().0.iter().enumerate() {
                    words[i] = u32::from_le_bytes(*chunk);
                }
                Ok(ControlBody::LinkHeartbeat(words))
            }
            _ => Ok(ControlBody::Raw(payload.to_vec())),
        },
        Some(ZrtpCmd::GroupMessage) => Ok(ControlBody::Empty),
        Some(ZrtpCmd::RoomUpdated) => Ok(ControlBody::RoomUpdated(decode_u32_le_list(payload)?)),
        Some(ZrtpCmd::InitZaviPing) => {
            let field0 = read_u32_le(payload, &mut off)?;
            let field1 = read_u32_le(payload, &mut off)?;
            let sess_id = read_u16_str(payload, &mut off)?;
            let extra = if off < payload.len() {
                Some(read_u16_str(payload, &mut off)?)
            } else {
                None
            };
            Ok(ControlBody::InitZaviPing {
                field0,
                field1,
                sess_id,
                extra,
            })
        }
        None => Ok(ControlBody::Raw(payload.to_vec())),
    }
}

fn encode_p2p_body(body: &P2pBody, out: &mut Vec<u8>) {
    match body {
        P2pBody::Bind { seq, cand_index } => {
            out.extend_from_slice(&seq.to_le_bytes());
            out.extend_from_slice(&cand_index.to_be_bytes());
        }
        P2pBody::BindPeer {
            seq,
            cand_index,
            addr,
        } => {
            out.extend_from_slice(&seq.to_le_bytes());
            out.extend_from_slice(&cand_index.to_be_bytes());
            let addr_bytes = addr.as_bytes();
            let len = u32::try_from(addr_bytes.len()).unwrap_or(u32::MAX);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&addr_bytes[..len as usize]);
            out.push(0xff);
        }
        P2pBody::BindAnswer {
            seq,
            cand_index,
            addr,
        } => {
            out.extend_from_slice(&seq.to_le_bytes());
            out.extend_from_slice(&cand_index.to_be_bytes());
            let addr_bytes = addr.as_bytes();
            let len = u32::try_from(addr_bytes.len()).unwrap_or(u32::MAX);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&addr_bytes[..len as usize]);
        }
        P2pBody::Keepalive {
            sub_cmd,
            seq,
            payload,
        } => {
            out.push(*sub_cmd);
            out.extend_from_slice(&seq.to_le_bytes());
            out.extend_from_slice(payload);
        }
        P2pBody::Signal { sub_cmd, seq } => {
            out.push(*sub_cmd);
            out.extend_from_slice(&seq.to_le_bytes());
        }
        P2pBody::Media(bytes) => out.extend_from_slice(bytes),
    }
}

fn decode_p2p(bytes: &[u8]) -> Result<ZRTPPacket> {
    if bytes.len() < P2P_HEADER_LEN {
        bail!("truncated P2P header ({} bytes)", bytes.len());
    }
    let magic = bytes[1];
    let sender_role = bytes[2];
    let call_id = u32::from_le_bytes(bytes[3..7].try_into()?);
    let kind_raw = u16::from_le_bytes(bytes[7..9].try_into()?);
    let kind = P2pKind::from_u16(kind_raw)
        .ok_or_else(|| anyhow::anyhow!("unknown P2P kind: {kind_raw}"))?;
    let payload = &bytes[P2P_HEADER_LEN..];

    let body = match kind {
        P2pKind::Bind => {
            if payload.len() < 6 {
                bail!("truncated P2P Bind payload");
            }
            let seq = u16::from_le_bytes(payload[0..2].try_into()?);
            let cand_index = u32::from_be_bytes(payload[2..6].try_into()?);
            P2pBody::Bind { seq, cand_index }
        }
        P2pKind::BindPeer => {
            if payload.len() < 10 {
                bail!("truncated P2P BindPeer header");
            }
            let seq = u16::from_le_bytes(payload[0..2].try_into()?);
            let cand_index = u32::from_be_bytes(payload[2..6].try_into()?);
            let addr_len = u32::from_le_bytes(payload[6..10].try_into()?) as usize;
            if payload.len() < 10 + addr_len + 1 {
                bail!("truncated P2P BindPeer addr");
            }
            let addr = std::str::from_utf8(&payload[10..10 + addr_len])?.to_string();
            P2pBody::BindPeer {
                seq,
                cand_index,
                addr,
            }
        }
        P2pKind::BindAnswer => {
            if payload.len() < 10 {
                bail!("truncated P2P BindAnswer header");
            }
            let seq = u16::from_le_bytes(payload[0..2].try_into()?);
            let cand_index = u32::from_be_bytes(payload[2..6].try_into()?);
            let addr_len = u32::from_le_bytes(payload[6..10].try_into()?) as usize;
            if payload.len() < 10 + addr_len {
                bail!("truncated P2P BindAnswer addr");
            }
            let addr = std::str::from_utf8(&payload[10..10 + addr_len])?.to_string();
            P2pBody::BindAnswer {
                seq,
                cand_index,
                addr,
            }
        }
        P2pKind::Keepalive => {
            if payload.len() < 5 {
                bail!("truncated P2P Keepalive payload");
            }
            let sub_cmd = payload[0];
            let seq = u32::from_le_bytes(payload[1..5].try_into()?);
            P2pBody::Keepalive {
                sub_cmd,
                seq,
                payload: payload[5..].to_vec(),
            }
        }
        P2pKind::Signal => {
            if payload.len() < 5 {
                bail!("truncated P2P Signal payload");
            }
            let sub_cmd = payload[0];
            let seq = u32::from_le_bytes(payload[1..5].try_into()?);
            P2pBody::Signal { sub_cmd, seq }
        }
        P2pKind::Audio | P2pKind::AudioRtcp | P2pKind::Video | P2pKind::VideoRtcp => {
            P2pBody::Media(payload.to_vec())
        }
    };

    Ok(ZRTPPacket::P2p {
        magic,
        sender_role,
        call_id,
        kind,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_packets_0x01_and_0x02_roundtrip() {
        let cases = vec![
            ZRTPPacket::ping(1, 100001, 999, "sess-abc"),
            ZRTPPacket::end_call(2, 100001, 999, 464564007),
            ZRTPPacket::Control {
                is_reply: false,
                magic: 0x7e,
                field02: 0,
                seq: 3,
                uid: 100001,
                token: 0,
                cmd: ZrtpCmd::Echo as u16,
                sub_cmd: 1,
                body: ControlBody::Echo("1732600000000".to_string()),
            },
            ZRTPPacket::init_call(true, 4, 100001, 464564007, 100002, "sess-abc"),
            ZRTPPacket::init_call(false, 5, 100002, 464564007, 100001, "sess-abc"),
            ZRTPPacket::change_address(6, 100001, 999, "sess-abc"),
            ZRTPPacket::req_forward_empty(7, 100001, 999, ReqForwardSubCmd::StopP2p),
            ZRTPPacket::Control {
                is_reply: false,
                magic: 0x01,
                field02: 0,
                seq: 8,
                uid: 100001,
                token: 999,
                cmd: ZrtpCmd::ReqForward as u16,
                sub_cmd: ReqForwardSubCmd::ChannelPrime as u8,
                body: ControlBody::ForwardPrime([4, 0, 0, 0, 0, 0, 0, 0]),
            },
            ZRTPPacket::Control {
                is_reply: false,
                magic: 0x01,
                field02: 0,
                seq: 9,
                uid: 100001,
                token: 999,
                cmd: ZrtpCmd::ReqForward as u16,
                sub_cmd: ReqForwardSubCmd::LinkHeartbeat as u8,
                body: ControlBody::LinkHeartbeat([20, 0, 1, 1, 50, 0]),
            },
            ZRTPPacket::Control {
                is_reply: false,
                magic: 0x01,
                field02: 0,
                seq: 10,
                uid: 100001,
                token: 999,
                cmd: ZrtpCmd::GroupMessage as u16,
                sub_cmd: 14,
                body: ControlBody::Empty,
            },
            ZRTPPacket::Control {
                is_reply: true,
                magic: 0x7e,
                field02: 0,
                seq: 11,
                uid: 100001,
                token: 999,
                cmd: ZrtpCmd::RoomUpdated as u16,
                sub_cmd: 0,
                body: ControlBody::RoomUpdated(vec![100001, 100002]),
            },
            ZRTPPacket::Control {
                is_reply: false,
                magic: 0x7e,
                field02: 0,
                seq: 12,
                uid: 100001,
                token: 0,
                cmd: ZrtpCmd::InitZaviPing as u16,
                sub_cmd: 1,
                body: ControlBody::InitZaviPing {
                    field0: 0,
                    field1: 0,
                    sess_id: "sess-zavi".to_string(),
                    extra: Some("0".to_string()),
                },
            },
            ZRTPPacket::Control {
                is_reply: true,
                magic: 0x7e,
                field02: 0,
                seq: 13,
                uid: 100001,
                token: 12345,
                cmd: ZrtpCmd::InitCallCaller as u16,
                sub_cmd: 0,
                body: ControlBody::Raw(vec![0, 1, 2, 3]),
            },
        ];

        for pkt in cases {
            let wire = pkt.encode();
            let decoded = ZRTPPacket::decode(&wire).expect("decode control");
            assert_eq!(decoded, pkt);
        }

        // Verify bare REQ_FORWARD subCmd=2 is exactly 21 bytes (`re-notes/16` §3).
        let bare = ZRTPPacket::req_forward_empty(1, 2, 3, ReqForwardSubCmd::StopP2p).encode();
        assert_eq!(bare.len(), 21);
    }

    #[test]
    fn p2p_packets_0x7f_kinds_1_to_9_roundtrip() {
        let cases = vec![
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 0,
                call_id: 464564007,
                kind: P2pKind::Bind,
                body: P2pBody::Bind {
                    seq: 1,
                    cand_index: 0x01020304,
                },
            },
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 1,
                call_id: 464564007,
                kind: P2pKind::BindPeer,
                body: P2pBody::BindPeer {
                    seq: 2,
                    cand_index: 1,
                    addr: "198.51.100.1|50000".to_string(),
                },
            },
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 0,
                call_id: 464564007,
                kind: P2pKind::BindAnswer,
                body: P2pBody::BindAnswer {
                    seq: 3,
                    cand_index: 1,
                    addr: "198.51.100.2|50001".to_string(),
                },
            },
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 0,
                call_id: 464564007,
                kind: P2pKind::Keepalive,
                body: P2pBody::Keepalive {
                    sub_cmd: 1,
                    seq: 10,
                    payload: vec![0xaa, 0xbb],
                },
            },
            ZRTPPacket::p2p_signal(0, 464564007, P2pSignalSubCmd::Stop, 2),
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 0,
                call_id: 464564007,
                kind: P2pKind::Audio,
                body: P2pBody::Media(vec![0x80, 0xef, 0x00, 0x01]),
            },
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 0,
                call_id: 464564007,
                kind: P2pKind::AudioRtcp,
                body: P2pBody::Media(vec![0x80, 0xc8, 0x00, 0x06]),
            },
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 1,
                call_id: 464564007,
                kind: P2pKind::Video,
                body: P2pBody::Media(vec![0x80, 0x61, 0x00, 0x02]),
            },
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 1,
                call_id: 464564007,
                kind: P2pKind::VideoRtcp,
                body: P2pBody::Media(vec![0x80, 0xc9, 0x00, 0x01]),
            },
        ];

        for pkt in cases {
            let wire = pkt.encode();
            let decoded = ZRTPPacket::decode(&wire).expect("decode p2p");
            assert_eq!(decoded, pkt);
        }

        // Verify exact lengths from `re-notes/16` §4: Bind=15 B, Signal=14 B.
        assert_eq!(
            ZRTPPacket::P2p {
                magic: 0,
                sender_role: 0,
                call_id: 1,
                kind: P2pKind::Bind,
                body: P2pBody::Bind {
                    seq: 0,
                    cand_index: 0
                },
            }
            .encode()
            .len(),
            15
        );
        assert_eq!(
            ZRTPPacket::p2p_signal(0, 1, P2pSignalSubCmd::Stop, 0)
                .encode()
                .len(),
            14
        );
    }

    #[test]
    fn relay_media_and_malformed_inputs_behave_as_specified() {
        let up = ZRTPPacket::RelayMediaUp {
            msg_type: MSG_AUDIO_RTP_UP,
            server_token: 0x11223344,
            payload: vec![1, 2, 3, 4],
        };
        assert_eq!(ZRTPPacket::decode(&up.encode()).unwrap(), up);

        let down = ZRTPPacket::RelayMediaDown {
            msg_type: MSG_AUDIO_RTP_DOWN,
            payload: vec![5, 6, 7],
        };
        assert_eq!(ZRTPPacket::decode(&down.encode()).unwrap(), down);

        assert!(ZRTPPacket::decode(&[]).is_err());
        assert!(ZRTPPacket::decode(&[0x01, 0x00]).is_err());
        assert!(ZRTPPacket::decode(&[0x7f, 0x00, 0x00]).is_err());
        assert!(ZRTPPacket::decode(&[0x99, 0x00]).is_err());
    }
}
