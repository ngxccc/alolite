use std::collections::{BTreeSet, HashMap};

use crate::protocol::messaging::model::{CallOutcome, MessageKind};
use crate::protocol::messaging::polling::PollState;
use crate::protocol::messaging::threads::{
    deduplicate_threads, is_generic_thread_name, ChatMessage, Thread,
};
use crate::protocol::messaging::unread::{compare_ids, UnreadState};
use crate::protocol::wire::{opt_string, opt_u64, strip_group_prefix, WireData};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::models::{MessageStore, StoredMessage, StoredThread};

/// `messages.raw_json` as written by the store when a message is persisted.
///
/// # Wire Response Example
/// ```json
/// {
///   "stickerId": 12345,
///   "cateId": 1234,
///   "type": 7,
///   "reply_to_id": "1731215399000_9",
///   "reply_to_text": "Tin trước đó",
///   "is_pinned": true,
///   "link_url": "https://example.com"
/// }
/// ```
///
/// # Core Transformation
/// Rehydrates the sticker, reply, reaction, pin/star, and link columns of a cached
/// [`ChatMessage`]; the newest `raw_json` wins because the store rewrites it on merge.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CachedMessageWireData {
    #[serde(rename = "stickerId")]
    pub sticker_id_camel: Option<serde_json::Value>,
    #[serde(rename = "id")]
    pub id: Option<serde_json::Value>,
    #[serde(rename = "cateId")]
    pub cate_id: Option<serde_json::Value>,
    #[serde(rename = "catId")]
    pub cat_id: Option<serde_json::Value>,
    #[serde(rename = "type")]
    pub sticker_type: Option<serde_json::Value>,
    #[serde(rename = "reply_to_id")]
    pub reply_to_id: Option<serde_json::Value>,
    #[serde(rename = "reply_to_sender")]
    pub reply_to_sender: Option<serde_json::Value>,
    #[serde(rename = "reply_to_text")]
    pub reply_to_text: Option<serde_json::Value>,
    #[serde(rename = "reactions")]
    pub reactions: Option<Vec<serde_json::Value>>,
    #[serde(rename = "is_pinned")]
    pub is_pinned: Option<serde_json::Value>,
    #[serde(rename = "is_starred")]
    pub is_starred: Option<serde_json::Value>,
    #[serde(rename = "link_url")]
    pub link_url: Option<serde_json::Value>,
    #[serde(rename = "link_title")]
    pub link_title: Option<serde_json::Value>,
    #[serde(rename = "link_desc")]
    pub link_desc: Option<serde_json::Value>,
    #[serde(rename = "link_thumb")]
    pub link_thumb: Option<serde_json::Value>,
    #[serde(rename = "link_src")]
    pub link_src: Option<serde_json::Value>,
    #[serde(rename = "client_message_id")]
    pub client_message_id: Option<serde_json::Value>,
    #[serde(rename = "cliMsgId")]
    pub cli_msg_id: Option<serde_json::Value>,
    #[serde(rename = "call_outcome")]
    pub call_outcome: Option<serde_json::Value>,
    #[serde(rename = "call_duration_secs")]
    pub call_duration_secs: Option<serde_json::Value>,
}

impl CachedMessageWireData {
    /// Sticker id (`stickerId` > `id`).
    pub fn sticker_id(&self) -> Option<u64> {
        opt_u64(self.sticker_id_camel.as_ref()).or_else(|| opt_u64(self.id.as_ref()))
    }

    /// Sticker category id (`cateId` > `catId`).
    pub fn sticker_cate_id(&self) -> Option<u64> {
        opt_u64(self.cate_id.as_ref()).or_else(|| opt_u64(self.cat_id.as_ref()))
    }

    /// Sticker type as a `u32`.
    pub fn sticker_type(&self) -> Option<u32> {
        opt_u64(self.sticker_type.as_ref()).map(|n| n as u32)
    }

    /// Quoted message id.
    pub fn reply_to_id(&self) -> Option<String> {
        opt_string(self.reply_to_id.as_ref())
    }

    /// Quoted message sender.
    pub fn reply_to_sender(&self) -> Option<String> {
        opt_string(self.reply_to_sender.as_ref())
    }

    /// Quoted message text.
    pub fn reply_to_text(&self) -> Option<String> {
        opt_string(self.reply_to_text.as_ref())
    }

    /// Legacy reaction emoji list stored alongside `raw_json`.
    pub fn reactions(&self) -> Vec<String> {
        self.reactions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    /// Whether the thread/message was pinned.
    pub fn is_pinned(&self) -> bool {
        self.is_pinned
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    /// Whether the message was starred.
    pub fn is_starred(&self) -> bool {
        self.is_starred
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    /// Link card URL.
    pub fn link_url(&self) -> Option<String> {
        opt_string(self.link_url.as_ref())
    }

    /// Link card title.
    pub fn link_title(&self) -> Option<String> {
        opt_string(self.link_title.as_ref())
    }

    /// Link card description.
    pub fn link_desc(&self) -> Option<String> {
        opt_string(self.link_desc.as_ref())
    }

    /// Link card thumbnail.
    pub fn link_thumb(&self) -> Option<String> {
        opt_string(self.link_thumb.as_ref())
    }

    /// Link card source label.
    pub fn link_src(&self) -> Option<String> {
        opt_string(self.link_src.as_ref())
    }

    /// Client message id (`client_message_id` > `cliMsgId`).
    pub fn client_message_id(&self) -> Option<String> {
        opt_string(self.client_message_id.as_ref()).or_else(|| opt_string(self.cli_msg_id.as_ref()))
    }

    /// Persisted 1-1 call outcome.
    pub fn call_outcome(&self) -> Option<CallOutcome> {
        self.call_outcome
            .as_ref()
            .and_then(|v| serde_json::from_value::<CallOutcome>(v.clone()).ok())
    }

    /// Persisted 1-1 call duration in seconds.
    pub fn call_duration_secs(&self) -> Option<u64> {
        opt_u64(self.call_duration_secs.as_ref())
    }
}

pub(super) fn upsert_thread_on_conn(conn: &rusqlite::Connection, t: &StoredThread) -> Result<()> {
    // Guard against clobbering a real name already persisted in SQLite with `"Nhóm <gid>"` / `"Người dùng <uid>"`.
    let name_sql = if is_generic_thread_name(&t.name) {
        "CASE WHEN trim(threads.name) = '' THEN excluded.name ELSE threads.name END"
    } else {
        "excluded.name"
    };
    let sql = format!(
        "INSERT INTO threads (thread_id, name, is_group, last_ts)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(thread_id) DO UPDATE SET
             name = {name_sql},
             is_group = excluded.is_group,
             last_ts = MAX(COALESCE(last_ts, 0), excluded.last_ts)"
    );
    conn.execute(
        &sql,
        params![
            t.thread_id,
            t.name,
            if t.is_group { 1 } else { 0 },
            t.last_ts,
        ],
    )?;
    Ok(())
}

impl MessageStore {
    /// Delete an entire conversation and all its messages from local SQLite store (#24).
    pub fn delete_thread(&mut self, thread_id: &str) -> Result<()> {
        if thread_id.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "DELETE FROM messages WHERE thread_id = ?1",
            params![thread_id],
        )?;
        self.conn.execute(
            "DELETE FROM threads WHERE thread_id = ?1",
            params![thread_id],
        )?;
        Ok(())
    }

    /// Insert or update thread metadata without overwriting a real display name with a placeholder.
    pub fn upsert_thread(&mut self, t: &StoredThread) -> Result<()> {
        upsert_thread_on_conn(&self.conn, t)
    }

    /// Retrieve all persisted thread metadata ordered by `last_ts` descending.
    pub fn all_threads(&self) -> Result<Vec<StoredThread>> {
        let mut stmt = self.conn.prepare(
            "SELECT thread_id, name, is_group, last_ts
             FROM threads
             ORDER BY last_ts DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let is_group_num: i64 = row.get(2).unwrap_or(0);
            Ok(StoredThread {
                thread_id: row.get(0)?,
                name: row.get(1).unwrap_or_default(),
                is_group: is_group_num != 0,
                last_ts: row.get(3).unwrap_or(0),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Load persisted [`UnreadState`] for all threads in SQLite keyed by `thread_id`.
    ///
    /// # Errors
    /// Returns an error if the SQLite query fails.
    pub fn load_thread_unreads(&self) -> Result<HashMap<String, UnreadState>> {
        let mut stmt = self.conn.prepare(
            "SELECT thread_id, last_seen_id, unread_ids_json, mention_ids_json, is_marked_unread, is_muted, muted_until
             FROM threads",
        )?;
        let rows = stmt.query_map([], |row| {
            let thread_id: String = row.get(0)?;
            let last_seen_id: Option<String> = row.get(1)?;
            let unread_json: String = row.get(2).unwrap_or_else(|_| "[]".into());
            let mention_json: String = row.get(3).unwrap_or_else(|_| "[]".into());
            let marked_unread: i64 = row.get(4).unwrap_or(0);
            let muted: i64 = row.get(5).unwrap_or(0);
            let muted_until: Option<i64> = row.get(6)?;
            let unread_ids: BTreeSet<String> =
                serde_json::from_str(&unread_json).unwrap_or_default();
            let mention_ids: BTreeSet<String> =
                serde_json::from_str(&mention_json).unwrap_or_default();
            Ok((
                thread_id,
                UnreadState {
                    last_seen_id,
                    unread_ids,
                    mention_ids,
                    muted: muted != 0,
                    muted_until,
                    marked_unread: marked_unread != 0,
                    fresh: false,
                    settings_fresh: false,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for r in rows {
            let (tid, state) = r?;
            map.insert(tid, state);
        }
        Ok(map)
    }

    /// Optimistically advance `last_seen_id` and clear `is_marked_unread` for `thread_id` in SQLite.
    ///
    /// # Errors
    /// Returns an error if the SQLite transaction or query fails.
    ///
    /// # Invariants
    /// - INV-6: `last_seen_id` is monotonic and never regresses to an older numeric message ID.
    pub fn mark_thread_seen(&mut self, thread_id: &str, last_msg_id: &str) -> Result<()> {
        if thread_id.is_empty() {
            return Ok(());
        }
        let existing = self.load_thread_unread_single(thread_id)?;
        let mut state = existing.unwrap_or_default();
        if !last_msg_id.is_empty() {
            state.advance_seen(last_msg_id);
        }
        state.unread_ids.clear();
        state.mention_ids.clear();
        state.marked_unread = false;
        self.write_thread_unread(thread_id, &state)
    }

    /// Persist a manual unread mark (`is_marked_unread`) for `thread_id` in SQLite.
    ///
    /// # Errors
    /// Returns an error if the SQLite update fails.
    pub fn set_thread_marked_unread(&mut self, thread_id: &str, marked_unread: bool) -> Result<()> {
        if thread_id.is_empty() {
            return Ok(());
        }
        let mut state = self
            .load_thread_unread_single(thread_id)?
            .unwrap_or_default();
        state.marked_unread = marked_unread;
        self.write_thread_unread(thread_id, &state)
    }

    fn load_thread_unread_single(&self, thread_id: &str) -> Result<Option<UnreadState>> {
        let mut stmt = self.conn.prepare(
            "SELECT last_seen_id, unread_ids_json, mention_ids_json, is_marked_unread, is_muted, muted_until
             FROM threads WHERE thread_id = ?1",
        )?;
        stmt.query_row(params![thread_id], |row| {
            let last_seen_id: Option<String> = row.get(0)?;
            let unread_json: String = row.get(1).unwrap_or_else(|_| "[]".into());
            let mention_json: String = row.get(2).unwrap_or_else(|_| "[]".into());
            let marked_unread: i64 = row.get(3).unwrap_or(0);
            let muted: i64 = row.get(4).unwrap_or(0);
            let muted_until: Option<i64> = row.get(5)?;
            Ok(UnreadState {
                last_seen_id,
                unread_ids: serde_json::from_str(&unread_json).unwrap_or_default(),
                mention_ids: serde_json::from_str(&mention_json).unwrap_or_default(),
                muted: muted != 0,
                muted_until,
                marked_unread: marked_unread != 0,
                fresh: false,
                settings_fresh: false,
            })
        })
        .optional()
        .map_err(Into::into)
    }

    fn write_thread_unread(&self, thread_id: &str, state: &UnreadState) -> Result<()> {
        write_thread_unread_on_conn(&self.conn, thread_id, state)
    }

    /// Load persisted [`PollState`] from `sync_state`, returning `None` if absent.
    ///
    /// # Errors
    /// Returns an error if the SQLite query fails.
    pub fn load_poll_state(&self) -> Result<Option<PollState>> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = 'poll_state'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(
            raw.and_then(|s| match serde_json::from_str::<PollState>(&s) {
                Ok(ps) => Some(ps),
                Err(e) => {
                    crate::logger::log_error("storage::load_poll_state", &e.to_string());
                    None
                }
            }),
        )
    }

    /// Persist [`PollState`] into `sync_state`.
    ///
    /// # Errors
    /// Returns an error if serialization or SQLite write fails.
    pub fn save_poll_state(&self, state: &PollState) -> Result<()> {
        save_poll_state_on_conn(&self.conn, state)
    }

    /// Hydrate `threads` with persisted threads and message history from SQLite.
    /// Returns the total number of messages loaded into `threads`.
    pub fn hydrate_threads(
        &self,
        threads: &mut Vec<Thread>,
        limit_per_thread: usize,
    ) -> Result<usize> {
        deduplicate_threads(threads, None);

        let unreads = self.load_thread_unreads().unwrap_or_default();
        let stored_threads = self.all_threads().unwrap_or_default();
        for st in stored_threads {
            if st.thread_id.is_empty() || st.thread_id == "0" {
                continue;
            }
            let stored_unread = unreads.get(&st.thread_id).cloned().unwrap_or_default();
            if let Some(existing) = threads.iter_mut().find(|t| {
                t.id == st.thread_id
                    || (st.is_group
                        && t.is_group
                        && strip_group_prefix(&t.id) == strip_group_prefix(&st.thread_id))
            }) {
                if is_generic_thread_name(&existing.name)
                    && !st.name.is_empty()
                    && !is_generic_thread_name(&st.name)
                {
                    existing.name = st.name;
                }
                existing.is_group = existing.is_group || st.is_group;
                merge_unread_monotonic(&mut existing.unread, &stored_unread);
                existing.presentation.muted = existing.unread.is_muted();
            } else if !st.is_group
                && st.name.contains("Cloud của tôi")
                && threads
                    .iter()
                    .any(|t| !t.is_group && t.name.contains("Cloud của tôi"))
            {
                // Migrate stale session-scoped `send2me_id` ("Cloud của tôi") in SQLite to the active session's `id`.
                // Normal contacts/groups must NEVER be merged by display name alone (#63).
                if let Some(live_id) = threads
                    .iter()
                    .find(|t| !t.is_group && t.name.contains("Cloud của tôi"))
                    .map(|t| t.id.clone())
                {
                    let _ = self.conn.execute(
                        "UPDATE messages SET thread_id = ?1 WHERE thread_id = ?2",
                        params![live_id, st.thread_id],
                    );
                    let _ = self.conn.execute(
                        "DELETE FROM threads WHERE thread_id = ?1",
                        params![st.thread_id],
                    );
                }
            } else {
                let muted = stored_unread.is_muted();
                let mut t = Thread {
                    presentation: Default::default(),
                    unread: stored_unread,
                    id: st.thread_id.clone(),
                    name: if st.name.is_empty() {
                        format!("Người dùng {}", st.thread_id)
                    } else {
                        st.name
                    },
                    preview: String::new(),
                    time: String::new(),
                    avatar: None,
                    is_group: st.is_group,
                    messages: Vec::new(),
                    pinned_messages: Vec::new(),
                };
                t.presentation.muted = muted;
                threads.push(t);
            }
        }

        // Primary-key lookups for the messages actually in memory. This used to load every
        // (msg_id, thread_id) row in the database into a HashMap on each call. One transaction
        // around them: in autocommit every SELECT takes and drops the file lock, ~50 µs on
        // Windows, which made 62k lookups cost 3 s.
        let tx = self.conn.unchecked_transaction()?;
        let mut owner_of = self
            .conn
            .prepare("SELECT thread_id FROM messages WHERE msg_id = ?1")?;

        let mut loaded_count = 0;
        for t in threads.iter_mut() {
            let before = t.messages.len();
            let tid = t.id.clone();
            t.messages.retain(|m| {
                owner_of
                    .query_row(params![m.id], |r| r.get::<_, String>(0))
                    .map_or(true, |owner| owner == tid)
            });
            if t.messages.len() != before && t.messages.is_empty() {
                t.preview.clear();
                t.time.clear();
                let _ = self.conn.execute(
                    "UPDATE threads SET last_ts = 0 WHERE thread_id = ?1",
                    params![t.id],
                );
            }
            let saved_msgs = self.messages_for(&t.id, limit_per_thread)?;
            for sm in saved_msgs {
                let time = if sm.ts > 0 {
                    crate::clock::message_time(sm.ts)
                } else {
                    String::new()
                };
                let mut sticker_id = None;
                let mut sticker_cate_id = None;
                let mut sticker_type = None;
                let mut reply_to_id = None;
                let mut reply_to_sender = None;
                let mut reply_to_text = None;
                let mut reactions = Vec::new();
                let mut is_pinned = false;
                let mut is_starred = false;
                let mut link_url = None;
                let mut link_title = None;
                let mut link_desc = None;
                let mut link_thumb = None;
                let mut link_src = None;
                let mut call_outcome = None;
                let mut call_duration_secs = None;
                let mut client_message_id = String::new();
                if !sm.raw_json.is_empty() {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&sm.raw_json) {
                        let cached = CachedMessageWireData::from_value(&val);
                        sticker_id = cached.sticker_id();
                        sticker_cate_id = cached.sticker_cate_id();
                        sticker_type = cached.sticker_type();
                        reply_to_id = cached.reply_to_id();
                        reply_to_sender = cached.reply_to_sender();
                        reply_to_text = cached.reply_to_text();
                        let r_summaries =
                            crate::protocol::messaging::reactions::parse_reactions(&val);
                        if !r_summaries.is_empty() {
                            reactions =
                                crate::protocol::messaging::reactions::legacy_emojis(&r_summaries);
                        } else {
                            reactions = cached.reactions();
                        }
                        is_pinned = cached.is_pinned();
                        is_starred = cached.is_starred();
                        link_url = cached.link_url();
                        link_title = cached.link_title();
                        link_desc = cached.link_desc();
                        link_thumb = cached.link_thumb();
                        link_src = cached.link_src();
                        call_outcome = cached.call_outcome();
                        call_duration_secs = cached.call_duration_secs();
                        if let Some(cid) = cached.client_message_id() {
                            client_message_id = cid;
                        }
                    }
                }
                let sender_id_opt = if !sm.sender_id.is_empty() {
                    Some(sm.sender_id.clone())
                } else if sm.sender_name.chars().all(|c| c.is_ascii_digit())
                    && !sm.sender_name.is_empty()
                {
                    Some(sm.sender_name.clone())
                } else {
                    None
                };
                let cm = ChatMessage {
                    kind: MessageKind::from_wire_name(&sm.msg_type),
                    id: sm.msg_id,
                    sender: if !sm.sender_name.is_empty() {
                        sm.sender_name
                    } else {
                        sm.sender_id
                    },
                    sender_id: sender_id_opt,
                    text: sm.text,
                    time,
                    is_outgoing: sm.is_outgoing,
                    ts: sm.ts,
                    sticker_id,
                    sticker_cate_id,
                    sticker_type,
                    reply_to_id,
                    reply_to_sender,
                    reply_to_text,
                    reactions,
                    is_pinned,
                    is_starred,
                    link_url,
                    link_title,
                    link_desc,
                    link_thumb,
                    link_src,
                    call_outcome,
                    call_duration_secs,

                    client_message_id,
                    ..Default::default()
                };
                if let Some(existing) = t.messages.iter_mut().find(|existing| {
                    (!existing.id.is_empty() && existing.id == cm.id)
                        || (existing.text == cm.text
                            && existing.is_outgoing == cm.is_outgoing
                            && (existing.ts == cm.ts
                                || (existing.ts != 0
                                    && cm.ts != 0
                                    && (existing.ts - cm.ts).abs() < 5000)))
                }) {
                    if existing.sender_id.is_none() && cm.sender_id.is_some() {
                        existing.sender_id = cm.sender_id;
                    }
                    if existing.sender.chars().all(|c| c.is_ascii_digit())
                        && !cm.sender.is_empty()
                        && !cm.sender.chars().all(|c| c.is_ascii_digit())
                    {
                        existing.sender = cm.sender;
                    }
                    if existing.sticker_id.is_none() && cm.sticker_id.is_some() {
                        existing.sticker_id = cm.sticker_id;
                        existing.sticker_cate_id = cm.sticker_cate_id;
                        existing.sticker_type = cm.sticker_type;
                    }
                    if existing.reply_to_text.is_none() && cm.reply_to_text.is_some() {
                        existing.reply_to_id = cm.reply_to_id;
                        existing.reply_to_sender = cm.reply_to_sender;
                        existing.reply_to_text = cm.reply_to_text;
                    }
                    if existing.link_url.is_none() && cm.link_url.is_some() {
                        existing.link_url = cm.link_url;
                        existing.link_title = cm.link_title;
                        existing.link_desc = cm.link_desc;
                        existing.link_thumb = cm.link_thumb;
                        existing.link_src = cm.link_src;
                        if !cm.text.is_empty() {
                            existing.text = cm.text;
                        }
                    }
                    if existing.reactions.is_empty() && !cm.reactions.is_empty() {
                        existing.reactions = cm.reactions;
                    }
                    if existing.client_message_id.is_empty() && !cm.client_message_id.is_empty() {
                        existing.client_message_id = cm.client_message_id;
                    }
                    if existing.kind == MessageKind::Unknown && cm.kind != MessageKind::Unknown {
                        existing.kind = cm.kind;
                    }
                    if existing.call_outcome.is_none() && cm.call_outcome.is_some() {
                        existing.call_outcome = cm.call_outcome;
                    }
                    if existing.call_duration_secs.is_none() && cm.call_duration_secs.is_some() {
                        existing.call_duration_secs = cm.call_duration_secs;
                    }
                    existing.is_pinned = existing.is_pinned || cm.is_pinned;
                    existing.is_starred = existing.is_starred || cm.is_starred;
                } else {
                    t.messages.push(cm);
                    loaded_count += 1;
                }
            }
            if !t.messages.is_empty() {
                t.messages.sort_by_key(|m| m.ts);
                if let Some(newest) = t.messages.last() {
                    if !newest.text.is_empty() {
                        t.preview = newest.text.replace('\n', " ");
                        t.time = newest.time.clone();
                    }
                }
            }
        }
        drop(owner_of);
        tx.commit()?;
        deduplicate_threads(threads, None);
        Ok(loaded_count)
    }
}

fn write_thread_unread_on_conn(
    conn: &rusqlite::Connection,
    thread_id: &str,
    state: &UnreadState,
) -> Result<()> {
    let unread_json = serde_json::to_string(&state.unread_ids)?;
    let mention_json = serde_json::to_string(&state.mention_ids)?;
    conn.execute(
        "UPDATE threads
         SET last_seen_id = ?2,
             unread_ids_json = ?3,
             mention_ids_json = ?4,
             is_marked_unread = ?5,
             is_muted = ?6,
             muted_until = ?7
         WHERE thread_id = ?1",
        params![
            thread_id,
            state.last_seen_id,
            unread_json,
            mention_json,
            if state.marked_unread { 1 } else { 0 },
            if state.muted { 1 } else { 0 },
            state.muted_until,
        ],
    )?;
    Ok(())
}

fn save_poll_state_on_conn(conn: &rusqlite::Connection, state: &PollState) -> Result<()> {
    let json = serde_json::to_string(state)?;
    conn.execute(
        "INSERT INTO sync_state (key, value) VALUES ('poll_state', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![json],
    )?;
    Ok(())
}

/// Merge `incoming` unread state into `target` monotonically without regressing `last_seen_id`.
pub(crate) fn merge_unread_monotonic(target: &mut UnreadState, incoming: &UnreadState) {
    if let Some(point) = &incoming.last_seen_id {
        target.advance_seen(point);
    }
    for id in &incoming.unread_ids {
        let newer = target
            .last_seen_id
            .as_deref()
            .is_none_or(|point| compare_ids(id, point).is_some_and(|o| o.is_gt()));
        if newer {
            target.unread_ids.insert(id.clone());
        }
    }
    for id in &incoming.mention_ids {
        if target.unread_ids.contains(id) {
            target.mention_ids.insert(id.clone());
        }
    }
    if incoming.settings_fresh || (target.last_seen_id.is_none() && incoming.last_seen_id.is_some())
    {
        target.muted = incoming.muted;
        target.muted_until = incoming.muted_until;
        target.marked_unread = incoming.marked_unread;
    } else if incoming.marked_unread {
        target.marked_unread = true;
    }
}

/// Persist threads and their messages to SQLite store, returning the count of newly inserted messages.
pub fn persist_threads(store: &mut MessageStore, threads: &[Thread]) -> Result<usize> {
    persist_threads_with_poll_state(store, threads, None)
}

/// Persist threads, messages, [`UnreadState`], and optional [`PollState`] in one SQLite transaction.
///
/// # Errors
/// Returns an error if the transaction, thread/message upsert, or `PollState` write fails.
///
/// # Invariants
/// - INV-6: `last_seen_id` is monotonic and `PollState` commits atomically with ingested messages.
pub fn persist_threads_with_poll_state(
    store: &mut MessageStore,
    threads: &[Thread],
    poll_state: Option<&PollState>,
) -> Result<usize> {
    // One transaction for the whole snapshot: in autocommit every row was its own fsync.
    store.conn.execute_batch("BEGIN")?;
    let res = persist_threads_inner(store, threads).and_then(|count| {
        if let Some(ps) = poll_state {
            save_poll_state_on_conn(&store.conn, ps)?;
        }
        Ok(count)
    });
    store
        .conn
        .execute_batch(if res.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    res
}

fn persist_threads_inner(store: &mut MessageStore, threads: &[Thread]) -> Result<usize> {
    let mut new_count = 0;
    let existing_unreads = store.load_thread_unreads().unwrap_or_default();
    for t in threads {
        let last_ts = t
            .messages
            .iter()
            .filter(|m| m.text != "sendBubbleMessage")
            .map(|m| m.ts)
            .max()
            .unwrap_or(0);
        let st = StoredThread {
            thread_id: t.id.clone(),
            name: t.name.clone(),
            is_group: t.is_group,
            last_ts,
        };
        store.upsert_thread(&st)?;

        // Preserve existing SQLite UnreadState when a caller (e.g. archive ingest) passes Default::default().
        let is_bare_default = t.unread.last_seen_id.is_none()
            && t.unread.unread_ids.is_empty()
            && t.unread.mention_ids.is_empty()
            && !t.unread.marked_unread
            && !t.unread.muted
            && !t.unread.fresh
            && !t.unread.settings_fresh;
        if !is_bare_default {
            let mut merged = existing_unreads.get(&t.id).cloned().unwrap_or_default();
            merge_unread_monotonic(&mut merged, &t.unread);
            write_thread_unread_on_conn(&store.conn, &t.id, &merged)?;
        }

        for m in t.messages.iter().filter(|m| m.text != "sendBubbleMessage") {
            let mut sm = StoredMessage::from(m);
            sm.thread_id = t.id.clone();
            if sm.msg_id.is_empty() {
                sm.msg_id = format!("{}_{}", t.id, sm.ts);
            }
            if store.upsert_message(&sm)? {
                new_count += 1;
            }
        }
    }
    Ok(new_count)
}

/// Convenience helper: open `zalo_history.db` (or `path`) and hydrate `threads` with saved history.
pub fn load_history_into_threads(path: &str, threads: &mut Vec<Thread>, limit_per_thread: usize) {
    if let Ok(store) = MessageStore::open(path) {
        let _ = store.hydrate_threads(threads, limit_per_thread);
    }
}

/// Load persisted [`PollState`] for account `uin`, returning `None` if not yet saved.
pub fn load_poll_state_for_account(uin: &str) -> Option<PollState> {
    let db_path = super::models::db_path_for_account(uin);
    MessageStore::open(&db_path)
        .ok()
        .and_then(|store| store.load_poll_state().ok().flatten())
}

/// Hydrate `threads` from the SQLite history of account `uin` (`PRELOAD_MSGS` messages per thread).
///
/// Owns the account-to-database-path mapping so `protocol/` never needs to know about the store.
pub fn preload_account_history(uin: &str, threads: &mut Vec<Thread>) {
    let db_path = super::models::db_path_for_account(uin);
    load_history_into_threads(&db_path, threads, MessageStore::PRELOAD_MSGS);
}

/// Persist `threads` into the SQLite store of account `uin`, then refresh `threads_cache.json`.
///
/// Persistence is best-effort: an unopenable store or a failed transaction is skipped silently and
/// the cache file is still written, matching the former in-protocol behaviour of `poll_new_messages`.
pub fn persist_account_threads(uin: &str, threads: &[Thread]) {
    persist_account_threads_with_poll_state(uin, threads, None);
}

/// Persist `threads` and optional [`PollState`] atomically into SQLite for `uin`, then refresh cache.
pub fn persist_account_threads_with_poll_state(
    uin: &str,
    threads: &[Thread],
    poll_state: Option<&PollState>,
) {
    let db_path = super::models::db_path_for_account(uin);
    if let Ok(mut store) = MessageStore::open(&db_path) {
        let _ = persist_threads_with_poll_state(&mut store, threads, poll_state);
    }
    super::thread_cache::save_threads_cache(threads);
}
