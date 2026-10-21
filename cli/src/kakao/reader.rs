//! Open KakaoTalk SQLCipher DBs read-only and map their schema to the katok
//! model. The open recipe uses `cipher_compatibility = 3`
//! then a passphrase `PRAGMA key`, then a `sqlite_master` probe.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

use super::auth::DatabaseKey;
use super::bplist;
use crate::types::{ChatReadState, RawMessage, SourceCursor};
use crate::{Error, Result};

/// Output of reading one or more KakaoTalk databases.
#[derive(Debug, Clone)]
pub struct ReaderOutput {
    pub chats: Vec<ChatRecord>,
    pub messages: Vec<RawMessage>,
    /// Where this read stopped in each database it read (the next incremental read starts
    /// there).
    pub cursors: Vec<SourceCursor>,
    /// Every database was read from a cursor, not in full.
    pub incremental: bool,
    /// Each room's read position, read in full on every read, incremental or not: reading a
    /// room in KakaoTalk moves it without adding a message.
    pub read_states: Vec<ChatReadState>,
}

/// More members than any KakaoTalk room holds; a watermark array past it is corrupt.
const WATERMARK_MAX_ENTRIES: usize = 16_384;

/// Rows before a cursor that an incremental read takes again. A valid cursor already covers
/// every insert (a new row always gets a rowid above the newest existing one), so this is only
/// margin; re-reading costs an unchanged upsert each.
const CURSOR_OVERLAP_ROWS: i64 = 64;

/// How many of the newest rows a read leaves as cursors for one database. The next read starts
/// after the newest of them that still holds, so KakaoTalk replacing or deleting the very newest
/// row (a status change, a deleted message) does not cost a full read.
const CURSOR_ANCHORS: usize = 8;

/// A chat room reduced to the fields katok needs.
#[derive(Debug, Clone)]
pub struct ChatRecord {
    pub chat_id: String,
    pub chat_name: String,
    pub chat_type: String,
}

#[derive(Debug, Clone)]
struct RoomMeta {
    name: Option<String>,
    chat_type: String,
    /// User-set custom room title (KakaoTalk "rename chat"), stored in
    /// `NTChatMeta` rather than `NTChatRoom.chatName`. `None` when never renamed.
    title: Option<String>,
    /// The peer's userId for a 1:1 direct chat (0 when not a direct chat).
    direct_member_id: i64,
    /// Display-member userIds for a group room (self excluded), used to
    /// reconstruct a name when neither `chatName` nor a title exists. Empty when
    /// unavailable.
    display_member_ids: Vec<i64>,
    /// An open chat's own name, from `NTOpenLink.linkName`.
    ///
    /// `NTChatRoom.chatName` is NULL for these, so without it the room falls
    /// through to a reconstructed member list and ends up stored under a name
    /// that appears nowhere in KakaoTalk.
    open_link_name: Option<String>,
}

/// Open `path` with `key` as a passphrase in cipher-compatibility mode 3,
/// read-only, and confirm the schema is readable. Returns Ok(true) on success,
/// Ok(false) if the key does not fit, Err only on unexpected I/O.
pub fn probe_database(path: &Path, key: &str) -> Result<bool> {
    match open_database(path, key) {
        Ok(conn) => {
            let ok = conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map(|count| count > 0)
                .unwrap_or(false);
            Ok(ok)
        }
        Err(_) => Ok(false),
    }
}

pub(crate) fn open_database(path: &Path, key: &str) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(Error::Sql)?;
    // The 256-hex key is ASCII hex, safe to interpolate inside single quotes.
    // The passphrase must be applied BEFORE `cipher_compatibility`; setting the
    // compatibility level first resets cipher state and yields "file is not a
    // database" against the bundled SQLCipher 4.5.x. `key` first then compat 3
    // matches both the bundled build and the system sqlcipher CLI.
    conn.execute_batch(&format!(
        "PRAGMA key = '{key}'; PRAGMA cipher_compatibility = 3;"
    ))
    .map_err(Error::Sql)?;
    // Force a read so an incorrect key fails here.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .map_err(Error::Sql)?;
    Ok(conn)
}

fn account_hash(user_id: i64) -> String {
    let digest = Sha256::digest(user_id.to_string().as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest.iter() {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// `NTChatRoom.type` of the account's own memo chat ("나와의 채팅").
const SELF_CHAT_ROOM_TYPE: i64 = 5;

/// Whether a room is the account's self chat: the memo room whose only member
/// is the account owner.
///
/// The room type is what identifies it. Member counts alone are not enough: a
/// group everyone else has left also has one active member and no display
/// members, and calling that a self chat would rename it after the owner and
/// make `send --chat` aim at the owner's memo window instead. The member
/// columns are still checked so an unexpected reuse of the type value by
/// KakaoTalk does not silently relabel a real conversation.
fn is_self_room(room_type: i64, direct_member: i64, active_members: i64) -> bool {
    room_type == SELF_CHAT_ROOM_TYPE && direct_member == 0 && active_members <= 1
}

fn classify_room(room_type: i64, direct_member: i64, active_members: i64) -> String {
    if is_self_room(room_type, direct_member, active_members) {
        "self".to_string()
    } else if room_type == 0 || room_type == 2 || direct_member > 0 || active_members == 2 {
        "direct".to_string()
    } else {
        "group".to_string()
    }
}

fn load_rooms(conn: &Connection) -> Result<HashMap<i64, RoomMeta>> {
    let mut stmt = conn
        .prepare(
            "SELECT chatId, type, chatName, activeMembersCount, directChatMemberUserId
             FROM NTChatRoom",
        )
        .map_err(Error::Sql)?;
    let mut rooms = HashMap::new();
    let rows = stmt
        .query_map([], |row| {
            let chat_id: i64 = row.get(0)?;
            let room_type: i64 = row.get(1)?;
            let name: Option<String> = row.get(2)?;
            let active_members: i64 = row.get(3)?;
            let direct_member: i64 = row.get(4)?;
            Ok((
                chat_id,
                RoomMeta {
                    name: name.filter(|value| !value.is_empty()),
                    chat_type: classify_room(room_type, direct_member, active_members),
                    title: None,
                    direct_member_id: direct_member,
                    display_member_ids: Vec::new(),
                    open_link_name: None,
                },
            ))
        })
        .map_err(Error::Sql)?;
    // Skip (not abort on) any single row that fails to deserialize; never log
    // row content.
    for row in rows.flatten() {
        let (chat_id, meta) = row;
        rooms.entry(chat_id).or_insert(meta);
    }
    enrich_titles(conn, &mut rooms);
    enrich_open_link_names(conn, &mut rooms);
    enrich_display_members(conn, &mut rooms);
    Ok(rooms)
}

/// Best-effort: fill `title` from the user-set room title KakaoTalk stores in
/// `NTChatMeta` (a `type = 3` row whose `content` is the current name), keeping
/// the highest `revision` when a room was renamed more than once. An older
/// schema without the table — or any read error — simply leaves titles unset.
fn enrich_titles(conn: &Connection, rooms: &mut HashMap<i64, RoomMeta>) {
    // Order ascending and overwrite as we go, so the last row seen per room is the
    // highest revision — with `content` breaking exact-revision ties — making the
    // chosen title deterministic regardless of SQLite's row order.
    let Ok(mut stmt) = conn.prepare(
        "SELECT chatId, content FROM NTChatMeta
         WHERE type = 3 AND content IS NOT NULL AND content <> ''
         ORDER BY revision ASC, content ASC",
    ) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    }) else {
        return;
    };
    for (chat_id, content) in rows.flatten() {
        if let Some(meta) = rooms.get_mut(&chat_id) {
            meta.title = Some(content);
        }
    }
}

/// Best-effort: fill `open_link_name` by joining `NTChatRoom.linkId` to
/// `NTOpenLink.linkName`.
///
/// An open chat has no `chatName`, so without this it falls through to a
/// reconstructed member list — a name that appears nowhere in the app, cannot be
/// searched for, and cannot be passed to `katok send --room`. An install missing
/// either column, or any read error, simply leaves the old behaviour rather than
/// aborting the room load.
fn enrich_open_link_names(conn: &Connection, rooms: &mut HashMap<i64, RoomMeta>) {
    let Ok(mut stmt) = conn.prepare(
        "SELECT r.chatId, l.linkName
         FROM NTChatRoom r
         JOIN NTOpenLink l ON l.linkId = r.linkId
         WHERE r.linkId <> 0 AND l.linkName IS NOT NULL AND l.linkName <> ''",
    ) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        let chat_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok((chat_id, name))
    }) else {
        return;
    };
    for (chat_id, name) in rows.flatten() {
        if let Some(meta) = rooms.get_mut(&chat_id) {
            meta.open_link_name = Some(name);
        }
    }
}

/// Best-effort: fill `display_member_ids` from `NTChatRoom.displayMemberIds`, a
/// binary-plist array of the room's display-member userIds. An older schema that
/// lacks the column — or any read/parse error — simply leaves group rooms
/// without a reconstructed name; it never aborts the room load.
fn enrich_display_members(conn: &Connection, rooms: &mut HashMap<i64, RoomMeta>) {
    let Ok(mut stmt) = conn.prepare("SELECT chatId, displayMemberIds FROM NTChatRoom") else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        let chat_id: i64 = row.get(0)?;
        let blob: Option<Vec<u8>> = row.get(1)?;
        Ok((chat_id, blob))
    }) else {
        return;
    };
    for (chat_id, blob) in rows.flatten() {
        if let Some(members) = blob.as_deref().and_then(bplist::int_array) {
            if let Some(meta) = rooms.get_mut(&chat_id) {
                meta.display_member_ids = members;
            }
        }
    }
}

/// Best-effort read of every room's [`ChatReadState`], limited to `only_chats` when given. A
/// column this install lacks reads as NULL rather than failing the query, and any read error
/// leaves the rooms without a read position; it never aborts the read.
fn load_read_states(
    conn: &Connection,
    user_id: i64,
    only_chats: Option<&[i64]>,
) -> Vec<ChatReadState> {
    let Ok(mut columns) = conn.prepare("SELECT name FROM pragma_table_info('NTChatRoom')") else {
        return Vec::new();
    };
    let present: Vec<String> = match columns.query_map([], |row| row.get(0)) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => return Vec::new(),
    };
    let column = |name: &'static str| {
        if present.iter().any(|have| have == name) {
            name
        } else {
            "NULL"
        }
    };
    let sql = format!(
        "SELECT chatId, {}, {}, {}, {} FROM NTChatRoom",
        column("lastSeenLogId"),
        column("countOfNewMessage"),
        column("watermarkKeys"),
        column("watermarkValues"),
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<i64>>(1).ok().flatten(),
            row.get::<_, Option<i64>>(2).ok().flatten(),
            row.get::<_, Option<Vec<u8>>>(3).ok().flatten(),
            row.get::<_, Option<Vec<u8>>>(4).ok().flatten(),
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter(|(chat_id, ..)| only_chats.is_none_or(|ids| ids.contains(chat_id)))
        .map(|(chat_id, last_seen, unread, keys, values)| ChatReadState {
            chat_id: chat_id.to_string(),
            last_seen_log_id: last_seen,
            new_message_count: unread,
            my_watermark_log_id: watermark_of(user_id, keys.as_deref(), values.as_deref()),
        })
        .collect()
}

/// `user_id`'s value in a room's watermark: the entry of `values` at the position `user_id`
/// holds in `keys`, both binary-plist integer arrays.
fn watermark_of(user_id: i64, keys: Option<&[u8]>, values: Option<&[u8]>) -> Option<i64> {
    let keys = bplist::int_array_up_to(keys?, WATERMARK_MAX_ENTRIES)?;
    let values = bplist::int_array_up_to(values?, WATERMARK_MAX_ENTRIES)?;
    let position = keys.iter().position(|key| *key == user_id)?;
    values.get(position).copied()
}

/// Resolve a human-readable chat name, falling back in order: an explicit
/// `chatName` → the user-set custom title → the direct peer's nickname (1:1
/// chats) → the account owner's own nickname (the self chat only) → joined
/// display-member nicknames (group chats) → the `chat-<id>` placeholder. KakaoTalk leaves `chatName` empty for auto-titled rooms and
/// composes the shown name from a title meta or from members at render time;
/// this mirrors that from the same DB, adding no logs.
fn resolve_chat_name(
    meta: Option<&RoomMeta>,
    chat_id: i64,
    self_user_id: i64,
    users: &HashMap<i64, String>,
) -> String {
    // Cap the reconstructed group name so a long member list stays readable and
    // a corrupt one cannot bloat the name cloned onto every message.
    const MAX_DISPLAY_MEMBERS: usize = 8;
    if let Some(meta) = meta {
        if let Some(name) = &meta.name {
            return name.clone();
        }
        if let Some(title) = &meta.title {
            return title.clone();
        }
        // An open chat carries its name on the link, not on the room. Ahead of
        // the member fallback because that fallback invents a name nobody in
        // KakaoTalk would recognise.
        if let Some(link_name) = &meta.open_link_name {
            return link_name.clone();
        }
        // Direct chat: the peer's nickname. Never the account owner itself
        // (an anomalous room must fall through, not take our name).
        if meta.direct_member_id != 0 && meta.direct_member_id != self_user_id {
            if let Some(name) = users.get(&meta.direct_member_id) {
                return name.clone();
            }
        }
        // The self chat is the one room that *does* take the owner's name:
        // KakaoTalk titles its window with the owner's nickname, and `send
        // --chat` finds a window by the archived name. Only a room positively
        // classified as the self chat gets here.
        if meta.chat_type == "self" {
            if let Some(name) = users.get(&self_user_id) {
                return name.clone();
            }
        }
        let joined: Vec<&str> = meta
            .display_member_ids
            .iter()
            .filter(|id| **id != self_user_id)
            .filter_map(|id| users.get(id).map(String::as_str))
            .take(MAX_DISPLAY_MEMBERS)
            .collect();
        if !joined.is_empty() {
            return joined.join(", ");
        }
    }
    format!("chat-{chat_id}")
}

fn load_users(conn: &Connection) -> Result<HashMap<i64, String>> {
    let mut stmt = conn
        .prepare(
            "SELECT userId, friendNickName, displayName, nickName
             FROM NTUser",
        )
        .map_err(Error::Sql)?;
    let mut users: HashMap<i64, String> = HashMap::new();
    let rows = stmt
        .query_map([], |row| {
            let user_id: i64 = row.get(0)?;
            let friend: Option<String> = row.get(1)?;
            let display: Option<String> = row.get(2)?;
            let nick: Option<String> = row.get(3)?;
            Ok((user_id, friend, display, nick))
        })
        .map_err(Error::Sql)?;
    // Skip (not abort on) any single row that fails to deserialize; never log
    // row content.
    for row in rows.flatten() {
        let (user_id, friend, display, nick) = row;
        let name = friend
            .filter(|value| !value.is_empty())
            .or(display.filter(|value| !value.is_empty()))
            .or(nick.filter(|value| !value.is_empty()));
        if let Some(name) = name {
            users.entry(user_id).or_insert(name);
        }
    }
    Ok(users)
}

/// Best-effort: parse an attachment/supplement JSON blob for a referenced
/// parent `logId` in the same chat. Only the JSON structure is inspected, never
/// message bodies.
///
/// Callers pass the `attachment` column first. A KakaoTalk reply (`type` 26)
/// stores `src_logId` at the top level of `attachment`, so a reader that
/// consults only `supplement` resolves no replies.
///
/// Both columns carry metadata for non-reply reasons too, so this stays
/// deliberately conservative to avoid synthesizing false reply edges:
///   * only reply-specific keys are accepted (the bare generic `logId` is NOT,
///     since media supplements legitimately embed it for non-reply reasons);
///   * those keys are looked up directly on the top-level object and, by
///     preference, inside a reply/src-shaped container (`reply`/`src`/`origin`/
///     `parent`), rather than anywhere at any depth.
fn reply_parent_log_id(supplement: Option<&str>) -> Option<i64> {
    let raw = supplement?;
    if raw.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let serde_json::Value::Object(root) = &value else {
        return None;
    };

    // Reply-specific keys only; the generic "logId" is intentionally excluded.
    const REPLY_KEYS: [&str; 4] = ["src_logId", "srcLogId", "parentLogId", "src_log_id"];
    // Containers a reply reference is shaped under.
    const CONTAINER_KEYS: [&str; 4] = ["reply", "src", "origin", "parent"];

    fn lookup_reply_id(map: &serde_json::Map<String, serde_json::Value>) -> Option<i64> {
        for key in REPLY_KEYS {
            if let Some(found) = map.get(key) {
                if let Some(id) = coerce_log_id(found) {
                    return Some(id);
                }
            }
        }
        None
    }

    // Prefer a reply/src-shaped container, then fall back to the top level.
    for container in CONTAINER_KEYS {
        if let Some(serde_json::Value::Object(inner)) = root.get(container) {
            if let Some(id) = lookup_reply_id(inner) {
                return Some(id);
            }
        }
    }
    lookup_reply_id(root)
}

/// Who a message mentions: the `user_id`s of the top-level `mentions` array of its
/// `attachment` JSON, and whether it carries `all_mention`. A reply's `src_mentions`, and any
/// `mentions` nested under the quoted message, belong to the message being quoted, so neither
/// is read. Nor is the body: an "@" in the text proves nothing.
fn mentions(attachment: Option<&str>) -> (Vec<String>, bool) {
    let Some(serde_json::Value::Object(root)) = attachment
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| serde_json::from_str(raw).ok())
    else {
        return (Vec::new(), false);
    };
    let mut ids: Vec<String> = Vec::new();
    if let Some(serde_json::Value::Array(entries)) = root.get("mentions") {
        for id in entries
            .iter()
            .filter_map(|entry| entry.get("user_id").and_then(coerce_log_id))
        {
            let id = id.to_string();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let all = matches!(
        root.get("all_mention"),
        Some(serde_json::Value::Object(_) | serde_json::Value::Bool(true))
    );
    (ids, all)
}

fn coerce_log_id(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(num) => num.as_i64().filter(|id| *id > 0),
        serde_json::Value::String(text) => text.trim().parse::<i64>().ok().filter(|id| *id > 0),
        _ => None,
    }
}

fn unreadable_database_warning() -> &'static str {
    "katok: skipping unreadable KakaoTalk db"
}

/// Read a single opened database into chats + messages. `users` is built once
/// across all DBs (see `read_databases`) so an author has one stable nickname —
/// the chunk grouping key — regardless of which DB a message came from.
fn read_one(
    conn: &Connection,
    user_id: i64,
    account: &str,
    users: &HashMap<i64, String>,
    after_rowid: Option<i64>,
    only_chats: Option<&[i64]>,
) -> Result<(HashMap<String, ChatRecord>, Vec<RawMessage>)> {
    let rooms = load_rooms(conn)?;
    // Resolve each room's display name once (peer/member joins need `users`),
    // then reuse per message rather than recomputing a join per row.
    let chat_names: HashMap<i64, String> = rooms
        .iter()
        .map(|(chat_id, meta)| {
            (
                *chat_id,
                resolve_chat_name(Some(meta), *chat_id, user_id, users),
            )
        })
        .collect();

    // `rowid > -1` is every row: one statement either way, and the rowid range is the table's
    // own key, so an incremental read touches only the new rows. A chat filter goes into the
    // statement too, so other rooms' bodies are never read into memory.
    let chat_filter = only_chats
        .map(|ids| format!(" AND chatId IN ({})", vec!["?"; ids.len()].join(",")))
        .unwrap_or_default();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT chatId, logId, authorId, type, message, sentAt, supplement, attachment
             FROM NTChatMessage
             WHERE rowid > ?{chat_filter} AND message IS NOT NULL AND message <> ''"
        ))
        .map_err(Error::Sql)?;
    let mut bound = vec![after_rowid.unwrap_or(-1)];
    bound.extend(only_chats.unwrap_or_default());

    let mut messages = Vec::new();
    let mut chats: HashMap<String, ChatRecord> = HashMap::new();

    let rows = stmt
        .query_map(rusqlite::params_from_iter(bound), |row| {
            let chat_id: i64 = row.get(0)?;
            let log_id: i64 = row.get(1)?;
            let author_id: i64 = row.get(2)?;
            let msg_type: i64 = row.get(3)?;
            let text: String = row.get(4)?;
            // sentAt is epoch seconds; tolerate a float column by reading f64.
            let sent_at: f64 = row.get::<_, f64>(5).unwrap_or(0.0);
            let supplement: Option<String> = row.get(6)?;
            let attachment: Option<String> = row.get(7)?;
            Ok((
                chat_id, log_id, author_id, msg_type, text, sent_at, supplement, attachment,
            ))
        })
        .map_err(Error::Sql)?;

    // Skip (and count) any single row that fails to deserialize — a non-UTF8
    // body or an unexpected column affinity in one corrupt row must not abort an
    // otherwise-healthy read. Row content is never logged.
    let mut skipped_rows: usize = 0;
    for row in rows {
        let (chat_id, log_id, author_id, msg_type, text, sent_at, supplement, attachment) =
            match row {
                Ok(values) => values,
                Err(_) => {
                    skipped_rows += 1;
                    continue;
                }
            };

        let room = rooms.get(&chat_id);
        let chat_type = room
            .map(|meta| meta.chat_type.clone())
            .unwrap_or_else(|| "direct".to_string());
        let chat_name = chat_names
            .get(&chat_id)
            .cloned()
            .unwrap_or_else(|| format!("chat-{chat_id}"));

        let sender_nickname = if author_id == user_id {
            users
                .get(&author_id)
                .cloned()
                .unwrap_or_else(|| "나".to_string())
        } else {
            users
                .get(&author_id)
                .cloned()
                .unwrap_or_else(|| format!("user-{author_id}"))
        };

        let timestamp = Utc
            .timestamp_opt(sent_at as i64, 0)
            .single()
            .unwrap_or_else(|| {
                Utc.timestamp_opt(0, 0)
                    .single()
                    .expect("epoch zero is valid")
            });

        let message_type = if msg_type == 1 {
            "text".to_string()
        } else {
            format!("type_{msg_type}")
        };

        // `attachment` first: KakaoTalk replies carry `src_logId` there, so
        // checking only `supplement` misses them.
        let reply_to_message_id = reply_parent_log_id(attachment.as_deref())
            .or_else(|| reply_parent_log_id(supplement.as_deref()))
            .filter(|parent| *parent != log_id)
            .map(|parent| format!("{chat_id}-{parent}"));
        let (mentioned_user_ids, mentions_all) = mentions(attachment.as_deref());

        let chat_id_str = chat_id.to_string();
        chats
            .entry(chat_id_str.clone())
            .or_insert_with(|| ChatRecord {
                chat_id: chat_id_str.clone(),
                chat_name: chat_name.clone(),
                chat_type: chat_type.clone(),
            });

        messages.push(RawMessage {
            account_hash: account.to_string(),
            chat_id: chat_id_str,
            chat_name,
            chat_type,
            message_id: format!("{chat_id}-{log_id}"),
            sender_id: author_id.to_string(),
            sender_nickname,
            timestamp,
            text,
            message_type,
            reply_to_message_id,
            mentioned_user_ids,
            mentions_all,
        });
    }

    if skipped_rows > 0 {
        eprintln!("katok: skipped {skipped_rows} unreadable KakaoTalk message row(s)");
    }

    Ok((chats, messages))
}

/// Read all openable databases, union + dedup messages by `message_id`
/// (`{chatId}-{logId}`), and sort chronologically per chat.
pub fn read_databases(
    database_files: &[PathBuf],
    user_id: i64,
    uuid: &str,
) -> Result<ReaderOutput> {
    read_databases_with_key(
        database_files,
        user_id,
        &DatabaseKey::derive(user_id, uuid),
        &[],
    )
}

/// Stable id of a database file for [`SourceCursor::db`]: a hash of the file name, which is
/// itself derived from the account and is never stored or printed.
pub fn database_id(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let digest = Sha256::digest(name.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// The newest [`CURSOR_ANCHORS`] `NTChatMessage` rows, newest first: where a read of this
/// snapshot stops.
fn newest_rows(conn: &Connection, db: &str) -> Result<Vec<SourceCursor>> {
    let mut stmt = conn
        .prepare("SELECT rowid, chatId, logId FROM NTChatMessage ORDER BY rowid DESC LIMIT ?1")
        .map_err(Error::Sql)?;
    let rows = stmt
        .query_map([CURSOR_ANCHORS as i64], |row| {
            Ok(SourceCursor {
                db: db.to_string(),
                rowid: row.get(0)?,
                chat_id: row.get(1)?,
                log_id: row.get(2)?,
            })
        })
        .map_err(Error::Sql)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Error::Sql)
}

/// Where an incremental read of `db` starts: after the newest of its cursors that still holds,
/// less the overlap. `None` (read everything) when none holds or it has none.
fn read_start(conn: &Connection, db: &str, cursors: &[SourceCursor]) -> Option<i64> {
    let mut mine: Vec<&SourceCursor> = cursors.iter().filter(|c| c.db == db).collect();
    mine.sort_by(|a, b| b.rowid.cmp(&a.rowid));
    mine.into_iter()
        .find(|c| cursor_holds(conn, c))
        .map(|c| (c.rowid - CURSOR_OVERLAP_ROWS).max(0))
}

/// Whether `cursor` still marks the same message: the row at its rowid must be the one it
/// recorded. A renumbered table (`VACUUM`), a deleted or replaced row, or a different database
/// all fail this; when no cursor of a database holds, it is read in full.
fn cursor_holds(conn: &Connection, cursor: &SourceCursor) -> bool {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT chatId, logId FROM NTChatMessage WHERE rowid = ?1",
        [cursor.rowid],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .optional()
    .ok()
    .flatten()
        == Some((cursor.chat_id, cursor.log_id))
}

/// [`read_databases`] with the key auth already derived, reading each database only past its
/// cursor in `cursors` when that cursor still holds ([`cursor_holds`]); a database without one
/// is read in full. Empty `cursors` is a full read. Either way the output carries the cursors
/// where this read stopped.
///
/// An incremental read sees new and replaced rows only. A row KakaoTalk changes in place keeps
/// its rowid and is picked up by the next full read.
pub fn read_databases_with_key(
    database_files: &[PathBuf],
    user_id: i64,
    key: &DatabaseKey,
    cursors: &[SourceCursor],
) -> Result<ReaderOutput> {
    read_databases_for_chats(database_files, user_id, key, cursors, None)
}

/// [`read_databases_with_key`] that reads only the messages of `only_chats` when given. The
/// cursors it returns are still the newest rows of the whole table, whichever chat they belong
/// to, so they mark where this snapshot ends rather than where a filtered chat ends.
pub fn read_databases_for_chats(
    database_files: &[PathBuf],
    user_id: i64,
    key: &DatabaseKey,
    cursors: &[SourceCursor],
    only_chats: Option<&[i64]>,
) -> Result<ReaderOutput> {
    let account = account_hash(user_id);

    // Open each DB once. A DB that fails to open is skipped with a one-line
    // warning (filename only, never content), mirroring the per-row degrade.
    let mut openable: Vec<(&PathBuf, Connection)> = Vec::new();
    let mut skipped = false;
    for path in database_files {
        match open_database(path, key.as_str()) {
            Ok(conn) => openable.push((path, conn)),
            Err(_) => {
                eprintln!("{}", unreadable_database_warning());
                skipped = true;
            }
        }
    }

    // Build the users (nickname) map ONCE across all openable DBs so an author
    // resolves to one stable `sender_nickname` — the chunk grouping key —
    // regardless of which DB a message came from. A load_users failure on one DB
    // is tolerated (that DB simply contributes no names).
    let mut users: HashMap<i64, String> = HashMap::new();
    for (_, conn) in &openable {
        if let Ok(db_users) = load_users(conn) {
            for (user, name) in db_users {
                users.entry(user).or_insert(name);
            }
        }
    }

    let mut chats: HashMap<String, ChatRecord> = HashMap::new();
    let mut messages_by_id: HashMap<String, RawMessage> = HashMap::new();
    let mut read_states: HashMap<String, ChatReadState> = HashMap::new();
    let mut stopped: Vec<SourceCursor> = Vec::new();
    let mut incremental = !openable.is_empty() && !skipped;

    for (path, conn) in &openable {
        let db = database_id(path);
        // One read transaction per database, so the cursor and the rows come from the same
        // snapshot: a message KakaoTalk writes meanwhile is either in both or in neither.
        let snapshot = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(_) => {
                eprintln!("{}", unreadable_database_warning());
                incremental = false;
                continue;
            }
        };
        let newest = newest_rows(conn, &db).unwrap_or_default();
        let after = read_start(conn, &db, cursors);
        incremental &= after.is_some();
        // A read_one failure on one DB (e.g. a schema mismatch in a secondary
        // store) must not abort messages already gathered from a healthy DB:
        // log a one-line warning (filename only) and skip this DB.
        let read = read_one(conn, user_id, &account, &users, after, only_chats);
        let db_read_states = load_read_states(conn, user_id, only_chats);
        drop(snapshot);
        let (db_chats, db_messages) = match read {
            Ok(result) => result,
            Err(_) => {
                eprintln!("{}", unreadable_database_warning());
                incremental = false;
                continue;
            }
        };
        stopped.extend(newest);
        for (chat_id, record) in db_chats {
            chats.entry(chat_id).or_insert(record);
        }
        for state in db_read_states {
            read_states.entry(state.chat_id.clone()).or_insert(state);
        }
        for message in db_messages {
            messages_by_id
                .entry(message.message_id.clone())
                .or_insert(message);
        }
    }

    let mut messages: Vec<RawMessage> = messages_by_id.into_values().collect();
    messages.sort_by(|a, b| {
        a.chat_id
            .cmp(&b.chat_id)
            .then_with(|| a.timestamp.cmp(&b.timestamp))
            .then_with(|| a.message_id.cmp(&b.message_id))
    });

    let mut chat_list: Vec<ChatRecord> = chats.into_values().collect();
    chat_list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
    let mut read_states: Vec<ChatReadState> = read_states.into_values().collect();
    read_states.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));

    Ok(ReaderOutput {
        chats: chat_list,
        messages,
        cursors: stopped,
        incremental,
        read_states,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_direct_and_group_rooms() {
        assert_eq!(classify_room(0, 0, 2), "direct");
        assert_eq!(classify_room(2, 0, 0), "direct");
        assert_eq!(classify_room(1, 999, 5), "direct"); // directChatMemberUserId > 0
        assert_eq!(classify_room(1, 0, 5), "group");
        assert_eq!(classify_room(4, 0, 100), "group");
    }

    #[test]
    fn classifies_the_memo_room_as_self() {
        assert_eq!(classify_room(SELF_CHAT_ROOM_TYPE, 0, 1), "self");
        assert_eq!(classify_room(SELF_CHAT_ROOM_TYPE, 0, 0), "self");
    }

    #[test]
    fn a_group_everyone_else_left_is_not_the_self_chat() {
        // Same member shape as the self chat, ordinary group type.
        assert_eq!(classify_room(1, 0, 1), "group");
        // The self-chat type with a peer or several members is not trusted.
        assert!(!is_self_room(SELF_CHAT_ROOM_TYPE, 999, 1));
        assert!(!is_self_room(SELF_CHAT_ROOM_TYPE, 0, 3));
    }

    fn room(chat_type: &str) -> RoomMeta {
        RoomMeta {
            name: None,
            chat_type: chat_type.to_string(),
            title: None,
            direct_member_id: 0,
            display_member_ids: Vec::new(),
            open_link_name: None,
        }
    }

    #[test]
    fn the_self_chat_is_named_after_the_owner() {
        let users = HashMap::from([(7_i64, "Owner Nick".to_string())]);
        assert_eq!(
            resolve_chat_name(Some(&room("self")), 42, 7, &users),
            "Owner Nick"
        );
        // Any other room with no members never takes the owner's name.
        assert_eq!(
            resolve_chat_name(Some(&room("group")), 42, 7, &users),
            "chat-42"
        );
        // An owner with no NTUser row keeps the placeholder rather than guessing.
        assert_eq!(
            resolve_chat_name(Some(&room("self")), 42, 8, &users),
            "chat-42"
        );
    }

    #[test]
    fn account_hash_is_64_hex() {
        let hash = account_hash(1_000_000_001);
        assert_eq!(hash.len(), 64);
        assert!(hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }

    #[test]
    fn parses_reply_parent_from_supplement() {
        let supplement = r#"{"reply":{"src_logId":12345,"text":"ignored"}}"#;
        assert_eq!(reply_parent_log_id(Some(supplement)), Some(12345));
    }

    #[test]
    fn parses_reply_parent_from_top_level_reply_key() {
        // Reply-specific key at the top level (no container) is still accepted.
        assert_eq!(
            reply_parent_log_id(Some(r#"{"src_logId":"777"}"#)),
            Some(777)
        );
        assert_eq!(
            reply_parent_log_id(Some(r#"{"parentLogId":888}"#)),
            Some(888)
        );
    }

    #[test]
    fn returns_none_for_non_reply_supplement() {
        assert_eq!(reply_parent_log_id(Some(r#"{"foo":"bar"}"#)), None);
        assert_eq!(reply_parent_log_id(Some("")), None);
        assert_eq!(reply_parent_log_id(None), None);
        assert_eq!(reply_parent_log_id(Some("not json")), None);
    }

    #[test]
    fn bare_logid_does_not_synthesize_reply_edge() {
        // A media/link-preview supplement that embeds a generic "logId" (or a
        // nested non-reply object) must NOT be read as a reply reference.
        assert_eq!(reply_parent_log_id(Some(r#"{"logId":4242}"#)), None);
        assert_eq!(
            reply_parent_log_id(Some(r#"{"media":{"logId":4242,"url":"x"}}"#)),
            None
        );
        // A reply key buried under an unrelated container is also not promoted.
        assert_eq!(
            reply_parent_log_id(Some(r#"{"attachment":{"src_logId":4242}}"#)),
            None
        );
    }

    #[test]
    fn reads_mentions_from_the_top_level_of_the_attachment() {
        let mentioned = r#"{"mentions":[{"user_id":500,"at":[1],"len":5},
            {"user_id":"600","at":[9],"len":3},{"user_id":500,"at":[20],"len":5}]}"#;
        assert_eq!(
            mentions(Some(mentioned)),
            (vec!["500".to_string(), "600".to_string()], false),
            "each id once, in order, numbers and strings alike"
        );
        assert_eq!(
            mentions(Some(r#"{"mentions":[{"user_id":9007199254740993}]}"#)).0,
            ["9007199254740993"],
            "ids past 2^53 keep every digit"
        );
        assert_eq!(
            mentions(Some(r#"{"all_mention":{"at":0}}"#)),
            (Vec::new(), true)
        );
    }

    #[test]
    fn a_quoted_message_s_mentions_are_not_this_message_s() {
        let reply = r#"{"src_logId":20,"src_mentions":[{"user_id":500}],
            "src":{"mentions":[{"user_id":600}]}}"#;
        assert_eq!(mentions(Some(reply)), (Vec::new(), false));
        for raw in ["", "not json", "[1,2]", r#"{"mentions":[{"at":[1]}]}"#] {
            assert_eq!(mentions(Some(raw)), (Vec::new(), false), "{raw}");
        }
        assert_eq!(mentions(None), (Vec::new(), false));
    }

    #[test]
    fn unreadable_database_warning_does_not_disclose_filename() {
        let filename =
            "de345a8eb68ff0db3c1f8b94817936a00471d335162afc05cdfc758f638a33d427ea7742d4d420";
        let warning = unreadable_database_warning();

        assert!(!warning.contains(filename));
        assert!(!warning.contains(".db"));
        assert!(warning.contains("skipping unreadable KakaoTalk db"));
    }
}
