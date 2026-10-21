use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct RawMessage {
    pub account_hash: String,
    pub chat_id: String,
    pub chat_name: String,
    pub chat_type: String,
    pub message_id: String,
    pub sender_id: String,
    pub sender_nickname: String,
    pub timestamp: DateTime<Utc>,
    pub text: String,
    pub message_type: String,
    pub reply_to_message_id: Option<String>,
    /// Decimal user ids the message mentions, from KakaoTalk's `attachment` metadata. Strings
    /// because the ids exceed what a JSON number holds exactly in JavaScript.
    #[serde(default)]
    pub mentioned_user_ids: Vec<String>,
    /// The message mentions everyone in the room.
    #[serde(default)]
    pub mentions_all: bool,
}

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub inserted_messages: usize,
    pub updated_messages: usize,
    pub total_messages: usize,
    pub chunks: usize,
    /// How many chats had their chunks recomputed. A quiet sync touches very few.
    pub rebuilt_chats: usize,
    /// Wall-clock milliseconds per stage, so "sync is slow" can be answered with a number
    /// instead of a guess about which stage is at fault.
    pub timings_ms: SyncTimings,
    /// Which chats changed, and where the earliest change landed in each.
    ///
    /// Always computed so the caller can scope the chunk rebuild to the tail after the last
    /// stable window boundary. Serialized into JSON only when `include_touched` is true
    /// (`katok sync --json --touched`); otherwise omitted so default output stays byte-identical
    /// for existing consumers.
    pub touched_chats: Vec<TouchedChat>,
    /// Gate for JSON emission of `touched_chats`. Default false.
    pub include_touched: bool,
    /// `katok sync --incremental` only: whether every source database was read from its cursor
    /// (`false` when one had none or a stale one and was read in full). Omitted otherwise, so the
    /// default JSON shape is unchanged.
    pub incremental: Option<bool>,
}

/// Where the last sync stopped in one source database: one of the newest `NTChatMessage` rows it
/// saw (a sync leaves several per database), by rowid plus that row's `(chatId, logId)`, so a
/// table renumbered underneath it (a `VACUUM`) or a row that was replaced is noticed. The next
/// sync starts after the newest one that still holds, or reads the database in full.
///
/// The rowid is KakaoTalk's local insertion order, not the send time, so a message the Mac only
/// receives later (after sleep, or offline) still lands after the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCursor {
    /// Stable id of the database file: a hash of its file name, never the name itself.
    pub db: String,
    pub rowid: i64,
    pub chat_id: i64,
    pub log_id: i64,
}

impl Serialize for SyncReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // Field order matches the historical derive order so flag-off output stays byte-identical.
        let field_count =
            6 + usize::from(self.include_touched) + usize::from(self.incremental.is_some());
        let mut state = serializer.serialize_struct("SyncReport", field_count)?;
        state.serialize_field("inserted_messages", &self.inserted_messages)?;
        state.serialize_field("updated_messages", &self.updated_messages)?;
        state.serialize_field("total_messages", &self.total_messages)?;
        state.serialize_field("chunks", &self.chunks)?;
        state.serialize_field("rebuilt_chats", &self.rebuilt_chats)?;
        state.serialize_field("timings_ms", &self.timings_ms)?;
        if self.include_touched {
            state.serialize_field("touched_chats", &self.touched_chats)?;
        }
        if let Some(incremental) = self.incremental {
            state.serialize_field("incremental", &incremental)?;
        }
        state.end()
    }
}

/// A chat that changed this sync, with the earliest changed message's send-order key.
///
/// The key `(timestamp, message_id)` is what scopes the rebuild: only chunks and parent windows
/// from the last window boundary before this key onward can have moved. See
/// `docs/incremental-chunking-tail-scope.md`.
#[derive(Debug, Clone, Serialize)]
pub struct TouchedChat {
    pub chat_id: String,
    /// RFC3339 timestamp of the earliest inserted-or-updated message in this chat.
    pub earliest_changed_timestamp: String,
    /// Message id of that same earliest-changed message. Currently unread: the
    /// gap-derived cut guarantees a >=300s separation at `P`, so `started_at >= P`
    /// needs no tiebreak. Kept so the report states the full change key.
    pub earliest_changed_message_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncTimings {
    /// Decrypting and scanning the KakaoTalk database.
    pub read_source: u128,
    /// Upserting chats and messages into the archive.
    pub upsert_messages: u128,
    /// Rebuilding chunks and parent windows.
    pub rebuild_chunks: u128,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(include_touched: bool) -> SyncReport {
        SyncReport {
            inserted_messages: 3,
            updated_messages: 0,
            total_messages: 3,
            chunks: 2,
            rebuilt_chats: 1,
            timings_ms: SyncTimings {
                read_source: 1,
                upsert_messages: 1,
                rebuild_chunks: 1,
            },
            touched_chats: vec![TouchedChat {
                chat_id: "chat-group-1".into(),
                earliest_changed_timestamp: "2026-01-01T09:00:00+00:00".into(),
                earliest_changed_message_id: "m1".into(),
            }],
            include_touched,
            incremental: None,
        }
    }

    #[test]
    fn sync_report_json_omits_touched_chats_unless_flagged() {
        let off = serde_json::to_string_pretty(&sample_report(false)).expect("serialize off");
        let on = serde_json::to_string_pretty(&sample_report(true)).expect("serialize on");

        assert!(
            !off.contains("touched_chats"),
            "default serialization must omit touched_chats:\n{off}"
        );
        assert!(
            on.contains("\"touched_chats\""),
            "flagged serialization must include touched_chats:\n{on}"
        );

        // Flag-on without the opt-in field equals flag-off exactly (same report, same timings).
        let mut on_value: serde_json::Value = serde_json::from_str(&on).expect("parse on");
        on_value
            .as_object_mut()
            .expect("object")
            .remove("touched_chats");
        let off_value: serde_json::Value = serde_json::from_str(&off).expect("parse off");
        assert_eq!(
            off_value, on_value,
            "flag-off output must be the flag-on payload minus touched_chats"
        );

        // Historical field set (order is fixed by Serialize; Value map iteration is sorted).
        let keys: std::collections::BTreeSet<&str> = off_value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "chunks",
                "inserted_messages",
                "rebuilt_chats",
                "timings_ms",
                "total_messages",
                "updated_messages",
            ]
            .into_iter()
            .collect()
        );
        // Pretty-print field order must stay historical so flag-off is byte-stable for consumers.
        assert!(
            off.starts_with("{\n  \"inserted_messages\":"),
            "flag-off pretty JSON must open with inserted_messages:\n{off}"
        );
    }

    #[test]
    fn sync_report_json_emits_empty_touched_chats_when_flagged_and_quiet() {
        let mut quiet = sample_report(true);
        quiet.touched_chats.clear();
        quiet.rebuilt_chats = 0;
        quiet.inserted_messages = 0;
        let json = serde_json::to_value(&quiet).expect("serialize quiet");
        assert_eq!(json["touched_chats"], serde_json::json!([]));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkListItem {
    pub chunk_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub sender_nickname: String,
    pub started_at: String,
    pub ended_at: String,
    pub message_count: usize,
    pub parent_chunk_ids: Vec<String>,
    pub window_parent_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chunk {
    pub chunk_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub sender_nickname: String,
    pub started_at: String,
    pub ended_at: String,
    pub text: String,
    pub message_count: usize,
    pub message_ids: Vec<String>,
    pub parent_chunk_ids: Vec<String>,
    pub window_parent_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParentChunk {
    pub parent_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub started_at: String,
    pub ended_at: String,
    pub text: String,
    pub message_count: usize,
    pub child_chunk_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkContext {
    pub chunk: Chunk,
    pub previous: Option<ChunkSummary>,
    pub next: Option<ChunkSummary>,
    pub parent_windows: Vec<ParentChunk>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkSummary {
    pub chunk_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub sender_nickname: String,
    pub started_at: String,
    pub ended_at: String,
    pub message_count: usize,
    pub parent_chunk_ids: Vec<String>,
    pub window_parent_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub ranker: &'static str,
    pub unit: &'static str,
    pub rank: usize,
    pub chunk_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub sender_nickname: String,
    pub started_at: String,
    pub ended_at: String,
    pub snippet: String,
    pub score: f64,
    pub parent_chunk_ids: Vec<String>,
    pub child_chunk_ids: Vec<String>,
}

/// One archived message with the sender identity, as `katok messages` reports it.
///
/// Added on the talk2build branch. `is_mine` compares `sha256(sender_id)` with the archive's
/// `account_hash`, which is how the macOS reader derives the account hash (see
/// `kakao::reader::account_hash`), so it needs no auth cache read.
#[derive(Debug, Clone, Serialize)]
pub struct MessageRow {
    pub message_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub chat_type: String,
    pub sender_id: String,
    pub sender_nickname: String,
    pub is_mine: bool,
    pub timestamp: String,
    pub text: String,
    pub message_type: String,
    pub reply_to_message_id: Option<String>,
    pub is_feed: bool,
    /// See [`RawMessage::mentioned_user_ids`]. Empty for a message archived before mentions
    /// were read, until a full sync reads it again.
    pub mentioned_user_ids: Vec<String>,
    pub mentions_all: bool,
}

/// One chat as the archive knows it, with tail metadata for cursoring and room lists.
#[derive(Debug, Clone, Serialize)]
pub struct RoomOverview {
    pub chat_id: String,
    pub chat_name: String,
    pub chat_type: String,
    pub message_count: usize,
    pub last_timestamp: Option<String>,
    pub last_message_id: Option<String>,
}
