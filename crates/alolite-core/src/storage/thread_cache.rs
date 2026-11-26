//! Fast-first-paint JSON snapshot cache (`threads_cache.json` and `accounts/{uin}_threads.json`).
//!
//! Keeps local disk I/O inside `src/storage/` so `src/protocol/` remains pure protocol & wire logic.

use std::sync::Mutex;

use crate::protocol::messaging::threads::{deduplicate_threads, Thread};
use crate::storage::models::MessageStore;

// Serializes read-modify-write cycles on `threads_cache.json` / `accounts/{uin}_threads.json`
// across concurrent `spawn_blocking` tasks without holding locks across `.await`.
static CACHE_LOCK: Mutex<()> = Mutex::new(());

fn active_uin_from_session() -> Option<String> {
    if let Ok(text) = std::fs::read_to_string("session.json") {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(uin) = val
                .pointer("/login/uin")
                .and_then(serde_json::Value::as_str)
            {
                if !uin.is_empty() {
                    return Some(uin.to_string());
                }
            }
        }
    }
    None
}

/// Load locally cached conversations and message history scoped to the active account.
pub fn load_threads_cache() -> Vec<Thread> {
    let _guard = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_threads_cache_unlocked()
}

fn load_threads_cache_unlocked() -> Vec<Thread> {
    if let Some(uin) = active_uin_from_session() {
        let acc_path = format!("accounts/{uin}_threads.json");
        if let Ok(text) = std::fs::read_to_string(&acc_path) {
            if let Ok(mut threads) = serde_json::from_str::<Vec<Thread>>(&text) {
                deduplicate_threads(&mut threads, None);
                return threads;
            }
        }
        // When an active account UIN is known, never fall back to global `threads_cache.json`
        // to prevent cross-account unread badge or thread leakage.
        return Vec::new();
    }
    if std::path::Path::new("session.json").exists() {
        if let Ok(text) = std::fs::read_to_string("threads_cache.json") {
            if let Ok(mut threads) = serde_json::from_str::<Vec<Thread>>(&text) {
                deduplicate_threads(&mut threads, None);
                return threads;
            }
        }
    }
    Vec::new()
}

/// Update `unread` state for `thread_id` in the active account's fast-paint cache.
pub fn update_cached_thread_unread(
    thread_id: &str,
    last_msg_id: Option<&str>,
    marked_unread: Option<bool>,
) {
    if thread_id.is_empty() {
        return;
    }
    let _guard = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut threads = load_threads_cache_unlocked();
    if let Some(t) = threads.iter_mut().find(|t| t.id == thread_id) {
        if let Some(msg_id) = last_msg_id {
            if !msg_id.is_empty() {
                t.unread.advance_seen(msg_id);
            }
            t.unread.unread_ids.clear();
            t.unread.mention_ids.clear();
        }
        if let Some(mu) = marked_unread {
            t.unread.marked_unread = mu;
        }
        save_threads_cache_unlocked(&threads);
    }
}

/// Merge delta `updates` into the active account's fast-paint cache without wiping other threads.
pub fn merge_threads_into_cache(updates: &[Thread]) {
    if updates.is_empty() {
        return;
    }
    let _guard = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut threads = load_threads_cache_unlocked();
    for inc in updates {
        if inc.id.is_empty() || inc.id == "0" {
            continue;
        }
        if let Some(existing) = threads.iter_mut().find(|t| t.id == inc.id) {
            if !inc.name.is_empty() {
                existing.name = inc.name.clone();
            }
            if !inc.preview.is_empty() {
                existing.preview = inc.preview.clone();
            }
            if !inc.time.is_empty() {
                existing.time = inc.time.clone();
            }
            if inc.avatar.is_some() {
                existing.avatar = inc.avatar.clone();
            }
            if inc.presentation.member_count.is_some() {
                existing.presentation.member_count = inc.presentation.member_count;
            }
            if !inc.presentation.members.is_empty() {
                existing.presentation.members = inc.presentation.members.clone();
            }
            existing.is_group = existing.is_group || inc.is_group;
            crate::storage::threads::merge_unread_monotonic(&mut existing.unread, &inc.unread);
            for m in &inc.messages {
                if !existing
                    .messages
                    .iter()
                    .any(|em| !m.id.is_empty() && em.id == m.id)
                {
                    existing.messages.push(m.clone());
                }
            }
            existing.messages.sort_by_key(|m| m.ts);
        } else {
            threads.push(inc.clone());
        }
    }
    save_threads_cache_unlocked(&threads);
}

/// Persist a slim snapshot (`PRELOAD_MSGS` newest messages per thread) for instant UI startup.
pub fn save_threads_cache(threads: &[Thread]) {
    let _guard = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save_threads_cache_unlocked(threads);
}

fn save_threads_cache_unlocked(threads: &[Thread]) {
    let keep = MessageStore::PRELOAD_MSGS;
    let slim: Vec<Thread> = threads
        .iter()
        .map(|t| Thread {
            presentation: t.presentation.clone(),
            unread: t.unread.clone(),
            id: t.id.clone(),
            name: t.name.clone(),
            preview: t.preview.clone(),
            time: t.time.clone(),
            avatar: t.avatar.clone(),
            is_group: t.is_group,
            messages: t.messages[t.messages.len().saturating_sub(keep)..].to_vec(),
            pinned_messages: t.pinned_messages.clone(),
        })
        .collect();
    if let Ok(text) = serde_json::to_string(&slim) {
        let _ = std::fs::write("threads_cache.json", &text);
        if let Some(uin) = active_uin_from_session() {
            let _ = std::fs::create_dir_all("accounts");
            let _ = std::fs::write(format!("accounts/{uin}_threads.json"), text);
        }
    }
}
