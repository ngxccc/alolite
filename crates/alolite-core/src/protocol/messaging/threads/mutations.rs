//! Server mutations for conversation threads: pin/unpin, hide/unhide, and delete conversation history.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clock::{message_time, now_ms};
use crate::protocol::messaging::threads::ChatMessage;
use crate::protocol::session::Session;
use crate::protocol::wire::{
    field_str, obj, require_login, strip_group_prefix, unwrap_data, ActionAck, WireData,
};

/// `/api/pinconvers/list` (cmd `12120`) payload: the pinned conversation list.
///
/// # Wire Response Example
/// ```json
/// { "conversations": [ "1234567890", { "id": "g9876543210" } ] }
/// ```
///
/// # Core Transformation
/// Accepts both the wrapped `conversations` array and a bare array root, matching the shipped
/// client's two response shapes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PinnedConversationsWireData {
    #[serde(rename = "conversations")]
    pub conversations: Option<Vec<Value>>,
}

impl PinnedConversationsWireData {
    /// Pinned entries from `conversations`, falling back to a bare array `root`.
    pub fn entries(&self, root: &Value) -> Vec<Value> {
        self.conversations
            .clone()
            .or_else(|| root.as_array().cloned())
            .unwrap_or_default()
    }
}

/// One pinned conversation entry: a bare id string or an object carrying an id field.
///
/// # Wire Response Example
/// ```json
/// { "id": "g9876543210" }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PinnedConversationWireData {
    #[serde(rename = "id")]
    pub id: Option<Value>,
    #[serde(rename = "threadId")]
    pub thread_id_camel: Option<Value>,
    #[serde(rename = "thread_id")]
    pub thread_id_snake: Option<Value>,
}

impl PinnedConversationWireData {
    /// Thread id (`id` > `threadId` > `thread_id`).
    pub fn thread_id(&self) -> Option<String> {
        crate::protocol::wire::field_str(self.id.as_ref())
            .or_else(|| crate::protocol::wire::field_str(self.thread_id_camel.as_ref()))
            .or_else(|| crate::protocol::wire::field_str(self.thread_id_snake.as_ref()))
    }
}

/// `GET /api/pinconvers/list` (cmd `12120`) — fetch pinned thread IDs.
///
/// # Errors
/// Returns an error if unauthenticated or the HTTP call fails.
pub async fn get_pinned(s: &Session) -> Result<Vec<String>> {
    require_login(s)?;
    let url = format!(
        "{}/api/pinconvers/list?{}",
        s.domains.conversation,
        s.common_params()
    );
    let resp = s.get(&url).await?;
    let data = unwrap_data(&resp);
    let wire = PinnedConversationsWireData::from_value(data);
    Ok(wire
        .entries(data)
        .into_iter()
        .filter_map(|item| {
            item.as_str()
                .map(str::to_string)
                .or_else(|| PinnedConversationWireData::from_value(&item).thread_id())
        })
        .collect())
}

fn parse_pin_topic_item(item: &Value) -> Option<ChatMessage> {
    let params_val = match item.get("params") {
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw).unwrap_or(Value::Null),
        Some(v) if v.is_object() => v.clone(),
        _ => Value::Null,
    };
    let lookup = |keys: &[&str]| -> Option<String> {
        for k in keys {
            if let Some(s) = field_str(params_val.get(*k)).or_else(|| field_str(item.get(*k))) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    };

    let id = lookup(&["globalMsgId", "msgId", "msg_id", "id"]).unwrap_or_default();
    let client_message_id =
        lookup(&["cliMsgId", "client_msg_id", "clientMsgId"]).unwrap_or_default();
    let text = lookup(&["title", "content", "msg", "message", "description"])
        .unwrap_or_else(|| "[Tin nhắn ghim]".to_string());
    let sender = lookup(&["sender_name", "senderName", "dName", "creatorName"])
        .unwrap_or_else(|| "Tin nhắn ghim".to_string());
    let sender_id = lookup(&["sender_uid", "senderId", "creatorId", "fromUid"]);
    let ts = params_val
        .get("ts")
        .or_else(|| params_val.get("time"))
        .or_else(|| item.get("createTime"))
        .or_else(|| item.get("ts"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(0);
    let time = if ts > 0 {
        message_time(ts)
    } else {
        String::new()
    };

    if id.is_empty() && text == "[Tin nhắn ghim]" {
        return None;
    }

    Some(ChatMessage {
        id,
        client_message_id,
        sender,
        sender_id,
        text,
        time,
        ts,
        is_pinned: true,
        link_thumb: lookup(&["thumb"]),
        link_url: lookup(&["href"]),
        ..Default::default()
    })
}

/// `GET /api/board/pin/list` (cmd `12001`, group) or `GET /api/friendboard/list` (cmd `12065`, 1:1)
/// — fetch pinned message topics for a conversation.
///
/// # Errors
/// Returns an error if unauthenticated or the HTTP call fails.
pub async fn fetch_pinned_topics(
    s: &Session,
    thread_id: &str,
    is_group: bool,
) -> Result<Vec<ChatMessage>> {
    require_login(s)?;
    let (cmd_id, body) = if is_group {
        let clean_gid = strip_group_prefix(thread_id);
        (
            12001,
            obj(&[
                ("groupId", json!(clean_gid)),
                ("boardVersion", json!(0)),
                ("imei", json!(s.imei)),
            ]),
        )
    } else {
        (
            12065,
            obj(&[("conversationId", json!(thread_id)), ("version", json!(0))]),
        )
    };
    let resp = s.api_get_id(cmd_id, Some(&body)).await?;
    let data = unwrap_data(&resp);
    let arr = data
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| data.get("topics").and_then(Value::as_array))
        .or_else(|| data.get("data").and_then(Value::as_array))
        .or_else(|| data.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(arr.iter().filter_map(parse_pin_topic_item).collect())
}

/// Pin or unpin a conversation (`updatePinnedConversationsV2`, cmd `12122`).
///
/// # Errors
/// Returns an error if unauthenticated, encryption fails, or the HTTP call fails.
pub async fn update_pinned_conversation(
    s: &Session,
    thread_id: &str,
    is_group: bool,
    pin: bool,
) -> Result<ActionAck> {
    require_login(s)?;
    let clean = strip_group_prefix(thread_id).trim_start_matches('u').trim();
    let prefixed = if is_group {
        format!("g{clean}")
    } else {
        format!("u{clean}")
    };
    let body = json!({
        "actionType": if pin { 1 } else { 2 },
        "conversations": [prefixed],
        "tab": 0
    });
    let params = s.encoded_params(&body)?;
    let url = format!(
        "{}/api/pinconvers/updatev2?{}",
        s.domains.conversation,
        s.common_params()
    );
    let resp = s.post_form(&url, format!("params={params}")).await?;
    let dec = s.maybe_decrypt(resp, s.session_key_bytes()).await;
    Ok(ActionAck::from_wire(&dec))
}

/// Hide or unhide a conversation (`updateItemHiddenChat`, cmd `10049`).
///
/// # Errors
/// Returns an error if unauthenticated, encryption fails, or the HTTP call fails.
pub async fn set_hidden_conversation(
    s: &Session,
    thread_id: &str,
    is_group: bool,
    hide: bool,
) -> Result<ActionAck> {
    require_login(s)?;
    let clean = strip_group_prefix(thread_id).trim();
    let entry = json!([{
        "thread_id": clean,
        "is_group": if is_group { 1 } else { 0 }
    }]);
    let (add_str, del_str) = if hide {
        (entry.to_string(), "[]".to_string())
    } else {
        ("[]".to_string(), entry.to_string())
    };
    let body = json!({
        "del_threads": del_str,
        "add_threads": add_str,
        "imei": s.imei
    });
    let params = s.encoded_params(&body)?;
    let url = format!(
        "{}/api/hiddenconvers/add-remove?{}",
        s.domains.conversation,
        s.common_params()
    );
    let resp = s.post_form(&url, format!("params={params}")).await?;
    let dec = s.maybe_decrypt(resp, s.session_key_bytes()).await;
    Ok(ActionAck::from_wire(&dec))
}

/// Delete conversation history for me (`deleteOneOneConversationV2` / `deleteGroupConversationV2`, cmd `11778` / `11835`).
///
/// # Errors
/// Returns an error if unauthenticated, encryption fails, or the HTTP call fails.
pub async fn delete_conversation(
    s: &Session,
    thread_id: &str,
    is_group: bool,
    last_cli_msg_id: &str,
) -> Result<ActionAck> {
    require_login(s)?;
    let clean = strip_group_prefix(thread_id).trim();
    let cli_id = if last_cli_msg_id.is_empty() {
        now_ms().to_string()
    } else {
        last_cli_msg_id.to_string()
    };
    if is_group {
        let body = json!({
            "grid": clean,
            "cliMsgId": cli_id,
            "conver": {},
            "onlyMe": 1,
            "imei": s.imei
        });
        let params = s.encoded_params(&body)?;
        let url = format!(
            "{}/api/group/deleteconver?{}",
            s.domains.group,
            s.common_params()
        );
        let resp = s.post_form(&url, format!("params={params}")).await?;
        let dec = s.maybe_decrypt(resp, s.session_key_bytes()).await;
        Ok(ActionAck::from_wire(&dec))
    } else {
        let body = json!({
            "toid": clean,
            "cliMsgId": cli_id,
            "conver": {},
            "onlyMe": 1,
            "imei": s.imei
        });
        let params = s.encoded_params(&body)?;
        let url = format!(
            "{}/api/message/deleteconver?{}",
            s.domains.chat,
            s.common_params()
        );
        let resp = s.post_form(&url, format!("params={params}")).await?;
        let dec = s.maybe_decrypt(resp, s.session_key_bytes()).await;
        Ok(ActionAck::from_wire(&dec))
    }
}
