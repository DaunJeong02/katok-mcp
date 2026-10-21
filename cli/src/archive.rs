use crate::{Error, Result};
use rusqlite::Connection;
use std::path::Path;

mod forget;
mod model;
mod parent;
mod read;
mod schema;
mod write;

pub use forget::ChatFootprint;
pub use model::{ChunkDraft, DeletedMessage, ParentChunkDraft, StoredMessage};
pub use read::{RAW_MESSAGES_FOR_CHAT_SINCE_QUERY, TAIL_REBUILD_START_QUERY};
pub use write::{
    DELETE_CHAT_CHUNKS_STATEMENTS, DELETE_REPLY_EDGES_FOR_CHAT, INSERT_CHUNK_PARENT_REFS_FOR_CHAT,
    INSERT_REPLY_EDGES_FOR_CHAT, RESOLVE_REPLY_EDGES_FOR_CHAT, SCOPED_REF_REBUILD_STATEMENTS,
};

pub struct Archive {
    pub(super) conn: Connection,
}

const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// True when the file exists, is non-empty and carries the plaintext SQLite header. An
/// encrypted (SQLCipher) file starts with random salt bytes instead.
pub fn is_plaintext_sqlite(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Error::Io(error)),
    };
    let mut header = [0u8; 16];
    let read = file.read(&mut header).map_err(Error::Io)?;
    Ok(read == 16 && &header == SQLITE_HEADER)
}

/// Re-writes a plaintext archive as an encrypted one with `key`, atomically: export into a
/// sibling file, then rename over the original and drop stale WAL/SHM files. The old plaintext
/// bytes are unlinked, not scrubbed (SSD wear levelling makes scrubbing meaningless).
pub fn encrypt_in_place(path: &Path, key: &str) -> Result<()> {
    let target = path.with_extension("sqlite3.encrypting");
    let _ = std::fs::remove_file(&target);
    {
        let plain = Connection::open(path).map_err(Error::Sql)?;
        // Fold any WAL content into the main file before copying.
        plain
            .execute_batch("PRAGMA journal_mode = DELETE;")
            .map_err(Error::Sql)?;
        let target_sql = target.to_string_lossy().replace('\'', "''");
        plain
            .execute_batch(&format!(
                "ATTACH DATABASE '{target_sql}' AS encrypted KEY {};\n\
                 SELECT sqlcipher_export('encrypted');\n\
                 DETACH DATABASE encrypted;",
                crate::paths::key_literal(key)
            ))
            .map_err(Error::Sql)?;
    }
    crate::paths::ensure_private_file(&target)?;
    std::fs::rename(&target, path).map_err(Error::Io)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut stale = path.as_os_str().to_owned();
        stale.push(suffix);
        let _ = std::fs::remove_file(stale);
    }
    Ok(())
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::paths::ensure_private_dir(parent)?;
        }
        let key = crate::paths::archive_key()?;
        if let Some(key) = key.as_deref() {
            if is_plaintext_sqlite(path)? {
                let _lock = crate::paths::MigrationLock::acquire(path)?;
                // Another process may have finished the conversion while we waited.
                if is_plaintext_sqlite(path)? {
                    encrypt_in_place(path, key)?;
                }
            }
        }
        let conn = Connection::open(path).map_err(Error::Sql)?;
        if let Some(key) = key.as_deref() {
            conn.execute_batch(&crate::paths::key_pragma(key))
                .map_err(Error::Sql)?;
        }
        // A wrong or missing key surfaces here as "file is not a database".
        if let Err(error) = conn.execute_batch("SELECT count(*) FROM sqlite_master;") {
            if key.is_none() && !is_plaintext_sqlite(path)? {
                return Err(Error::ArchiveKeyRequired(path.to_path_buf()));
            }
            return Err(Error::Sql(error));
        }
        crate::paths::ensure_private_file(path)?;
        let archive = Self { conn };
        schema::migrate(&archive.conn)?;
        Ok(archive)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Run `work` inside a single SQLite transaction, rolling back if it fails.
    ///
    /// Without this every statement commits on its own, so a sync paid one journal fsync per
    /// row — and a sync that died midway left the archive partially written with nothing to roll
    /// back to.
    pub fn in_transaction<T, E>(
        &self,
        work: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|err| E::from(Error::Sql(err)))?;
        let value = work()?;
        tx.commit().map_err(|err| E::from(Error::Sql(err)))?;
        Ok(value)
    }
}
