use super::write::{DELETE_CHAT_CHUNKS_STATEMENTS, DELETE_REPLY_EDGES_FOR_CHAT};
use super::Archive;
use crate::{Error, Result};
use rusqlite::params;

/// What one chat holds in the archive, table by table: the rows [`Archive::forget_chat`]
/// removes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct ChatFootprint {
    pub messages: usize,
    pub chats: usize,
    pub chunks: usize,
    pub chunks_fts: usize,
    pub chunk_messages: usize,
    pub chunk_parent_refs: usize,
    pub parent_chunks: usize,
    pub parent_chunk_children: usize,
    pub reply_edges: usize,
}

const CHAT_CHUNKS: &str = "SELECT chunk_id FROM chunks WHERE chat_id = ?1";

/// A reply into this chat from another one loses its parent rather than its row; the child's
/// chat still owns the edge.
const UNRESOLVE_REPLY_EDGES_INTO_CHAT: &str = "UPDATE reply_edges
     SET parent_chunk_id = NULL, unresolved_reason = 'parent_not_in_archive'
     WHERE parent_message_id IN (SELECT message_id FROM messages WHERE chat_id = ?1)";

impl Archive {
    pub fn chat_footprint(&self, chat_id: &str) -> Result<ChatFootprint> {
        let count = |sql: &str| -> Result<usize> {
            self.conn
                .query_row(sql, params![chat_id], |row| row.get::<_, i64>(0))
                .map(|count| count as usize)
                .map_err(Error::Sql)
        };
        Ok(ChatFootprint {
            messages: count("SELECT COUNT(*) FROM messages WHERE chat_id = ?1")?,
            chats: count("SELECT COUNT(*) FROM chats WHERE chat_id = ?1")?,
            chunks: count("SELECT COUNT(*) FROM chunks WHERE chat_id = ?1")?,
            chunks_fts: count(
                "SELECT COUNT(*) FROM chunks_fts
                 WHERE rowid IN (SELECT rowid FROM chunks WHERE chat_id = ?1)",
            )?,
            chunk_messages: count(&format!(
                "SELECT COUNT(*) FROM chunk_messages WHERE chunk_id IN ({CHAT_CHUNKS})"
            ))?,
            chunk_parent_refs: count(&format!(
                "SELECT COUNT(*) FROM chunk_parent_refs
                 WHERE child_chunk_id IN ({CHAT_CHUNKS}) OR parent_chunk_id IN ({CHAT_CHUNKS})"
            ))?,
            parent_chunks: count("SELECT COUNT(*) FROM parent_chunks WHERE chat_id = ?1")?,
            parent_chunk_children: count(
                "SELECT COUNT(*) FROM parent_chunk_children
                 WHERE parent_id IN (SELECT parent_id FROM parent_chunks WHERE chat_id = ?1)",
            )?,
            reply_edges: count(
                "SELECT COUNT(*) FROM reply_edges
                 WHERE child_message_id IN (SELECT message_id FROM messages WHERE chat_id = ?1)",
            )?,
        })
    }

    /// Remove every row of `chat_id` from the archive and return what was there. Run it inside
    /// [`Archive::in_transaction`]: the steps depend on each other's order (edges and chunk rows
    /// are found through the messages and chunks deleted last).
    pub fn forget_chat(&self, chat_id: &str) -> Result<ChatFootprint> {
        let footprint = self.chat_footprint(chat_id)?;
        let execute = |sql: &str| -> Result<()> {
            self.conn
                .execute(sql, params![chat_id])
                .map(drop)
                .map_err(Error::Sql)
        };
        execute(DELETE_REPLY_EDGES_FOR_CHAT)?;
        execute(UNRESOLVE_REPLY_EDGES_INTO_CHAT)?;
        for sql in DELETE_CHAT_CHUNKS_STATEMENTS {
            self.conn
                .execute(sql, params![chat_id, Option::<&str>::None])
                .map_err(Error::Sql)?;
        }
        execute("DELETE FROM messages WHERE chat_id = ?1")?;
        execute("DELETE FROM chats WHERE chat_id = ?1")?;
        Ok(footprint)
    }
}
