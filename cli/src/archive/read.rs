use super::{Archive, StoredMessage};
use crate::{
    types::{Chunk, ChunkSummary, MessageRow, RoomOverview},
    Error, Result,
};
use rusqlite::{params, OptionalExtension};

/// The backward walk over one chat's chunks that [`Archive::tail_rebuild_start`] drives.
///
/// Bound as `(?1 = chat_id, ?2 = the earliest changed timestamp)`. Lives here rather than inline
/// so the query-plan test asserts against the text that actually runs. `idx_chunks_chat_started`
/// serves all three parts of it — the equality, the range, and the descending order — so the walk
/// visits the chat's newest chunks and stops, instead of sorting the whole table to find them.
pub const TAIL_REBUILD_START_QUERY: &str = "SELECT started_at, ended_at FROM chunks
     WHERE chat_id = ?1 AND started_at < ?2
     ORDER BY started_at DESC";

/// One chat's messages in send order, that [`Archive::raw_messages_for_chat_since`] runs.
///
/// Bound as `(?1 = chat_id, ?2 = an RFC3339 floor or NULL for the whole chat)`. Lives here rather
/// than inline so the query-plan test asserts against the text that actually runs.
///
/// The floor is written as `timestamp >= COALESCE(?2, '')` rather than `?2 IS NULL OR
/// timestamp >= ?2`: the two select the same rows — every stored `timestamp` is text, so a NULL
/// floor collapsed to `''` still admits all of them — but only the `COALESCE` form is sargable, so
/// SQLite can carry the floor into the index instead of filtering it row by row. With
/// `idx_messages_chat_timestamp(chat_id, timestamp, message_id)` the plan is a single
/// `SEARCH ... (chat_id=? AND timestamp>?)` that also serves the `ORDER BY`, so a scoped tail read
/// touches only the tail and never sorts or scans the whole archive.
pub const RAW_MESSAGES_FOR_CHAT_SINCE_QUERY: &str =
    "SELECT account_hash, chat_id, chat_name, chat_type, message_id,
            sender_nickname, timestamp, text, message_type
     FROM messages
     WHERE chat_id = ?1 AND timestamp >= COALESCE(?2, '')
     ORDER BY timestamp, message_id";

impl Archive {
    pub fn chunks_for_chat(&self, chat_id: &str) -> Result<Vec<ChunkSummary>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT chunk_id, chat_id, chat_name, sender_nickname, started_at,
                    ended_at, message_count
             FROM chunks
             WHERE chat_id = ?1
             ORDER BY started_at, chunk_id",
            )
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map(params![chat_id], |row| {
                Ok(ChunkSummary {
                    chunk_id: row.get(0)?,
                    chat_id: row.get(1)?,
                    chat_name: row.get(2)?,
                    sender_nickname: row.get(3)?,
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    message_count: row.get::<_, i64>(6)? as usize,
                    parent_chunk_ids: Vec::new(),
                    window_parent_ids: Vec::new(),
                })
            })
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        rows.into_iter()
            .map(|mut summary| {
                summary.parent_chunk_ids = self.parent_chunks(&summary.chunk_id)?;
                summary.window_parent_ids = self.window_parent_ids(&summary.chunk_id)?;
                Ok(summary)
            })
            .collect()
    }

    pub fn get_chunk(&self, chunk_id: &str) -> Result<Option<Chunk>> {
        let row = self
            .conn
            .query_row(
                "SELECT chunk_id, chat_id, chat_name, sender_nickname, started_at,
                    ended_at, text, message_count
             FROM chunks WHERE chunk_id = ?1",
                params![chunk_id],
                |row| {
                    Ok(Chunk {
                        chunk_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        chat_name: row.get(2)?,
                        sender_nickname: row.get(3)?,
                        started_at: row.get(4)?,
                        ended_at: row.get(5)?,
                        text: row.get(6)?,
                        message_count: row.get::<_, i64>(7)? as usize,
                        message_ids: Vec::new(),
                        parent_chunk_ids: Vec::new(),
                        window_parent_ids: Vec::new(),
                    })
                },
            )
            .optional()
            .map_err(Error::Sql)?;

        match row {
            Some(mut chunk) => {
                chunk.message_ids = self.chunk_messages(chunk_id)?;
                chunk.parent_chunk_ids = self.parent_chunks(chunk_id)?;
                chunk.window_parent_ids = self.window_parent_ids(chunk_id)?;
                Ok(Some(chunk))
            }
            None => Ok(None),
        }
    }

    pub fn all_chunks(&self) -> Result<Vec<Chunk>> {
        let mut stmt = self
            .conn
            .prepare("SELECT chunk_id FROM chunks ORDER BY started_at, chunk_id")
            .map_err(Error::Sql)?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        ids.into_iter()
            .map(|id| {
                self.get_chunk(&id)
                    .and_then(|chunk| chunk.ok_or(Error::MissingChunk(id)))
            })
            .collect()
    }

    /// A chat's display name and the timestamp of its last message.
    ///
    /// `send` addresses rooms by the name KakaoTalk shows, and those names are
    /// not unique — so the last-message time comes along as the only thing on
    /// the chat list that tells two same-named rooms apart.
    /// Returns the display name and, when other chats share it, how many of them
    /// are more recently active.
    ///
    /// That count is the room's position among the same-named rows of the chat
    /// list, which orders most-recent-first — and position survives what an
    /// exact timestamp does not. The archive only knows what the last sync saw,
    /// so in a room being typed in right now its idea of "last message" is stale
    /// within a minute, while the ordering only changes if a *different*
    /// same-named room overtakes it.
    pub fn chat_identity(&self, chat_id: &str) -> Result<Option<(String, usize)>> {
        let Some(name): Option<String> = self
            .conn
            .query_row(
                "SELECT chat_name FROM messages WHERE chat_id = ?1 LIMIT 1",
                [chat_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sql)?
        else {
            return Ok(None);
        };
        let rank: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM (
                     SELECT chat_id, MAX(timestamp) AS last FROM messages
                     WHERE chat_name = ?1 GROUP BY chat_id
                 ) peers
                 WHERE peers.chat_id <> ?2
                   AND peers.last > (SELECT MAX(timestamp) FROM messages WHERE chat_id = ?2)",
                params![name, chat_id],
                |row| row.get(0),
            )
            .map_err(Error::Sql)?;
        Ok(Some((name, rank as usize)))
    }

    pub fn raw_messages(&self) -> Result<Vec<StoredMessage>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT account_hash, chat_id, chat_name, chat_type, message_id,
                    sender_nickname, timestamp, text, message_type
             FROM messages ORDER BY chat_id, timestamp, message_id",
            )
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map([], StoredMessage::from_row)
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }

    /// Same rows and same order as [`Archive::raw_messages`], restricted to `chat_ids`.
    ///
    /// The ordering must stay identical: chunk boundaries are decided by walking messages in
    /// this order, so a different order would produce different chunks for the same data.
    pub fn raw_messages_for_chats(&self, chat_ids: &[String]) -> Result<Vec<StoredMessage>> {
        if chat_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", chat_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT account_hash, chat_id, chat_name, chat_type, message_id,
                    sender_nickname, timestamp, text, message_type
             FROM messages WHERE chat_id IN ({placeholders})
             ORDER BY chat_id, timestamp, message_id"
            ))
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(chat_ids),
                StoredMessage::from_row,
            )
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }

    /// Messages of one chat in send order, optionally from `since` (RFC3339) onward.
    ///
    /// Same rows and same order as [`Archive::raw_messages`] for that chat. `None` reads the whole
    /// chat; a floor reads only the tail the scoped rebuild recomputes.
    pub fn raw_messages_for_chat_since(
        &self,
        chat_id: &str,
        since: Option<&str>,
    ) -> Result<Vec<StoredMessage>> {
        let mut stmt = self
            .conn
            .prepare(RAW_MESSAGES_FOR_CHAT_SINCE_QUERY)
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map(params![chat_id, since], StoredMessage::from_row)
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }

    /// The `started_at` a scoped rebuild of `chat_id` may start from, given the earliest changed
    /// message's timestamp, or `None` when the whole chat must be rebuilt.
    ///
    /// Walks the chat's existing chunks backward from the change and returns the start of the
    /// newest chunk whose gap to its predecessor exceeds `DEFAULT_PARENT_WINDOW_SECONDS`. That
    /// chunk begins a parent window that a time gap fixes independently of any character
    /// accumulation, so it is a stable boundary: everything before it is provably identical to a
    /// full rebuild (docs/incremental-chunking-tail-scope.md). If no such interior boundary exists
    /// the chat has no gap to cut at and is rebuilt whole.
    ///
    /// Reading from a time-gap boundary rather than the theoretically-minimal last window means a
    /// character-split window is recomputed rather than reused, which stays correct and keeps the
    /// scope independent of room size (bounded by the current conversation burst).
    pub fn tail_rebuild_start(
        &self,
        chat_id: &str,
        earliest_changed_timestamp: &str,
    ) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare(TAIL_REBUILD_START_QUERY)
            .map_err(Error::Sql)?;
        let mut rows = stmt
            .query(params![chat_id, earliest_changed_timestamp])
            .map_err(Error::Sql)?;

        // Iterating newest-first, `newer_started_at` holds the chunk just above the current row;
        // the current (older) row's `ended_at` and that start bound the gap between them.
        let mut newer_started_at: Option<String> = None;
        while let Some(row) = rows.next().map_err(Error::Sql)? {
            let started_at: String = row.get(0).map_err(Error::Sql)?;
            let ended_at: String = row.get(1).map_err(Error::Sql)?;
            if let Some(newer) = &newer_started_at {
                let older_end =
                    chrono::DateTime::parse_from_rfc3339(&ended_at).map_err(Error::Time)?;
                let newer_start =
                    chrono::DateTime::parse_from_rfc3339(newer).map_err(Error::Time)?;
                let gap = newer_start.signed_duration_since(older_end).num_seconds();
                if gap > crate::chunking::DEFAULT_PARENT_WINDOW_SECONDS {
                    return Ok(Some(newer.clone()));
                }
            }
            newer_started_at = Some(started_at);
        }
        Ok(None)
    }

    /// Messages of one chat in send order, optionally from `since` onward.
    ///
    /// Timestamps are stored as `DateTime<Utc>::to_rfc3339()` text, always with a `+00:00`
    /// offset, and the filter compares them as strings. That orders by instant only against a
    /// bound in the same form: a `Z` bound sorts after the stored row at its own instant, and a
    /// `+09:00` bound is nine hours off. `since` must therefore already be in the stored form;
    /// the CLI converts `--since` to it before calling.
    pub fn messages_for_transcript(
        &self,
        chat_id: &str,
        since: Option<&str>,
    ) -> Result<Vec<StoredMessage>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT account_hash, chat_id, chat_name, chat_type, message_id,
                    sender_nickname, timestamp, text, message_type
             FROM messages
             WHERE chat_id = ?1 AND (?2 IS NULL OR timestamp >= ?2)
             ORDER BY timestamp, message_id",
            )
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map(params![chat_id, since], StoredMessage::from_row)
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }

    /// Messages of one chat inside `[since, until]`, newest `limit` rows, returned oldest first.
    ///
    /// `since` and `until` are compared as strings, so they must be in the stored
    /// `DateTime<Utc>::to_rfc3339()` form, as for [`Archive::messages_for_transcript`].
    ///
    /// Added on the talk2build branch for the mirror viewer and the poll cursor. `me_override`
    /// forces the user id that counts as the account owner for `is_mine` and `mentions_me`
    /// (synthetic fixtures do not carry a sha256 account hash).
    pub fn messages_for_chat_range(
        &self,
        chat_id: &str,
        since: Option<&str>,
        until: Option<&str>,
        limit: usize,
        me_override: Option<&str>,
    ) -> Result<Vec<MessageRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT account_hash, chat_id, chat_name, chat_type, message_id, sender_id,
                        sender_nickname, timestamp, text, message_type, reply_to_message_id,
                        mentioned_user_ids, mentions_all
                 FROM messages
                 WHERE chat_id = ?1
                   AND timestamp >= COALESCE(?2, '')
                   AND (?3 IS NULL OR timestamp <= ?3)
                 ORDER BY timestamp DESC, message_id DESC
                 LIMIT ?4",
            )
            .map_err(Error::Sql)?;
        let mut rows = stmt
            .query_map(params![chat_id, since, until, limit as i64], |row| {
                let account_hash: String = row.get(0)?;
                let sender_id: String = row.get(5)?;
                let message_type: String = row.get(9)?;
                let text: String = row.get(8)?;
                let is_mine = is_me(&sender_id, &account_hash, me_override);
                let is_feed = crate::transcript::is_feed(&message_type, &text);
                let mentioned_user_ids: Vec<String> = row
                    .get::<_, Option<String>>(11)?
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default();
                let mentions_me = mentions_me(&mentioned_user_ids, &account_hash, me_override);
                Ok(MessageRow {
                    message_id: row.get(4)?,
                    chat_id: row.get(1)?,
                    chat_name: row.get(2)?,
                    chat_type: row.get(3)?,
                    sender_id,
                    sender_nickname: row.get(6)?,
                    is_mine,
                    timestamp: row.get(7)?,
                    text,
                    message_type,
                    reply_to_message_id: row.get(10)?,
                    is_feed,
                    mentioned_user_ids,
                    mentions_all: row.get(12)?,
                    mentions_me,
                })
            })
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        rows.reverse();
        Ok(rows)
    }

    /// Every chat in the archive with its message count and newest message key.
    pub fn rooms_overview(&self) -> Result<Vec<RoomOverview>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.chat_id, c.chat_name, c.chat_type,
                        (SELECT COUNT(*) FROM messages m WHERE m.chat_id = c.chat_id),
                        (SELECT m.timestamp FROM messages m WHERE m.chat_id = c.chat_id
                           ORDER BY m.timestamp DESC, m.message_id DESC LIMIT 1),
                        (SELECT m.message_id FROM messages m WHERE m.chat_id = c.chat_id
                           ORDER BY m.timestamp DESC, m.message_id DESC LIMIT 1),
                        c.last_seen_log_id, c.new_message_count, c.my_watermark_log_id,
                        c.member_count, c.member_user_ids, c.is_open_chat
                 FROM chats c
                 ORDER BY 5 DESC NULLS LAST, c.chat_name",
            )
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RoomOverview {
                    chat_id: row.get(0)?,
                    chat_name: row.get(1)?,
                    chat_type: row.get(2)?,
                    message_count: row.get::<_, i64>(3)? as usize,
                    last_timestamp: row.get(4)?,
                    last_message_id: row.get(5)?,
                    last_seen_log_id: row.get::<_, Option<i64>>(6)?.map(|id| id.to_string()),
                    new_message_count: row.get(7)?,
                    my_watermark_log_id: row.get::<_, Option<i64>>(8)?.map(|id| id.to_string()),
                    member_count: row.get(9)?,
                    member_user_ids: row
                        .get::<_, Option<String>>(10)?
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                    is_open_chat: row.get(11)?,
                })
            })
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }

    fn chunk_messages(&self, chunk_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT message_id FROM chunk_messages WHERE chunk_id = ?1 ORDER BY ordinal")
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map(params![chunk_id], |row| row.get::<_, String>(0))
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }

    pub(super) fn parent_chunks(&self, chunk_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT parent_chunk_id FROM chunk_parent_refs
             WHERE child_chunk_id = ?1 ORDER BY parent_chunk_id",
            )
            .map_err(Error::Sql)?;
        let rows = stmt
            .query_map(params![chunk_id], |row| row.get::<_, String>(0))
            .map_err(Error::Sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Sql)?;
        Ok(rows)
    }
}

/// Whether `user_id` is the account owner: equal to `me_override` when one is given, otherwise
/// hashing to the archive's `account_hash`. `is_mine` and `mentions_me` both go through here so
/// the two cannot disagree on who "me" is.
fn is_me(user_id: &str, account_hash: &str, me_override: Option<&str>) -> bool {
    match me_override {
        Some(me) => me == user_id,
        None => sender_account_hash(user_id) == account_hash,
    }
}

/// Whether any of `mentioned_user_ids` is the account owner, by [`is_me`]. A room-wide mention
/// (`mentions_all`) is not counted: it is reported on its own.
fn mentions_me(
    mentioned_user_ids: &[String],
    account_hash: &str,
    me_override: Option<&str>,
) -> bool {
    mentioned_user_ids
        .iter()
        .any(|id| is_me(id, account_hash, me_override))
}

/// Mirrors `kakao::reader::account_hash`: lowercase hex sha256 of the decimal sender id.
fn sender_account_hash(sender_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(sender_id.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "1000000001";
    const OTHER: &str = "600";

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn mentions_me_when_my_id_hashes_to_the_account_hash() {
        let account_hash = sender_account_hash(ME);
        assert!(mentions_me(&ids(&[OTHER, ME]), &account_hash, None));
    }

    #[test]
    fn does_not_mention_me_when_only_others_are_mentioned() {
        let account_hash = sender_account_hash(ME);
        assert!(!mentions_me(&ids(&[OTHER]), &account_hash, None));
    }

    #[test]
    fn does_not_mention_me_without_any_mention() {
        let account_hash = sender_account_hash(ME);
        assert!(!mentions_me(&[], &account_hash, None));
    }

    #[test]
    fn me_override_decides_who_is_mentioned_instead_of_the_account_hash() {
        let account_hash = "acct-synthetic";
        assert!(mentions_me(&ids(&[OTHER, "u2"]), account_hash, Some("u2")));
        assert!(!mentions_me(&ids(&[OTHER]), account_hash, Some("u2")));
        let hashed = sender_account_hash(ME);
        assert!(
            !mentions_me(&ids(&[ME]), &hashed, Some("u2")),
            "with an override the account hash is not consulted"
        );
    }

    #[test]
    fn is_mine_and_mentions_me_share_one_rule() {
        let account_hash = sender_account_hash(ME);
        assert!(is_me(ME, &account_hash, None));
        assert!(!is_me(OTHER, &account_hash, None));
        assert!(is_me(OTHER, &account_hash, Some(OTHER)));
    }
}
