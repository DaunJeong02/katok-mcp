use crate::kakao::{AuthOptions, ReaderOutput};
use crate::{
    types::{ChatRoomState, RawMessage, SourceCursor},
    Result,
};
use std::path::{Path, PathBuf};
use std::process::Command;

pub trait SourceAdapter {
    fn chats(&self) -> Result<Vec<ChatSummary>>;
    fn messages(&self) -> Result<Vec<RawMessage>>;

    /// Messages past `cursors` (where the previous sync of this source stopped), with the
    /// cursors where this read stopped. Empty `cursors` reads everything. A source that has no
    /// cursors reads everything and returns none, which leaves every sync a full one.
    fn messages_since(&self, _cursors: &[SourceCursor]) -> Result<SourceRead> {
        Ok(SourceRead {
            messages: self.messages()?,
            cursors: Vec::new(),
            incremental: false,
            room_states: Vec::new(),
        })
    }

    /// [`Self::messages_since`] for `chat_ids` only. This default reads everything and drops the
    /// other chats afterwards; a source that can filter while reading overrides it.
    fn messages_since_for_chats(
        &self,
        cursors: &[SourceCursor],
        chat_ids: &[String],
    ) -> Result<SourceRead> {
        let mut read = self.messages_since(cursors)?;
        read.messages
            .retain(|message| chat_ids.contains(&message.chat_id));
        read.room_states
            .retain(|state| chat_ids.contains(&state.chat_id));
        Ok(read)
    }
}

/// One read of a source for `sync`.
#[derive(Debug, Clone)]
pub struct SourceRead {
    pub messages: Vec<RawMessage>,
    pub cursors: Vec<SourceCursor>,
    /// Every database was read past a cursor that still held, not in full.
    pub incremental: bool,
    /// Each room's state (read position, members). Empty for a source that has none.
    pub room_states: Vec<ChatRoomState>,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ChatSummary {
    pub chat_id: String,
    pub chat_name: String,
    pub chat_type: String,
}

pub struct FixtureAdapter {
    path: std::path::PathBuf,
}

impl FixtureAdapter {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl SourceAdapter for FixtureAdapter {
    fn chats(&self) -> Result<Vec<ChatSummary>> {
        let mut chats = self
            .messages()?
            .into_iter()
            .map(|message| ChatSummary {
                chat_id: message.chat_id,
                chat_name: message.chat_name,
                chat_type: message.chat_type,
            })
            .collect::<Vec<_>>();
        chats.sort_by(|left, right| left.chat_id.cmp(&right.chat_id));
        chats.dedup_by(|left, right| left.chat_id == right.chat_id);
        Ok(chats)
    }

    fn messages(&self) -> Result<Vec<RawMessage>> {
        crate::fixture::read_fixture(&self.path)
    }
}

/// Reads the current macOS KakaoTalk encrypted database natively (no Python,
/// no `kakaocli`). One read is shared between `chats()` and `messages()`: the
/// first call decrypts + scans and memoizes the result, so a caller that
/// invokes both on the same instance pays the decrypt once.
pub struct MacosAdapter {
    options: AuthOptions,
    cached: std::cell::OnceCell<ReaderOutput>,
}

impl MacosAdapter {
    /// Build an adapter for the given `home` and katok `data_dir`.
    pub fn new(home: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            options: AuthOptions::new(home, data_dir),
            cached: std::cell::OnceCell::new(),
        }
    }

    /// Read the databases once and memoize the output. Subsequent calls clone
    /// the cached `ReaderOutput` instead of re-resolving auth and re-decrypting.
    /// Errors are not cached, so a transient failure can be retried.
    fn read(&self) -> Result<ReaderOutput> {
        if let Some(output) = self.cached.get() {
            return Ok(output.clone());
        }
        let output = crate::kakao::read_kakao_with_options(&self.options)?;
        // Ignore a lost race: `get_or_init` is not fallible-friendly, so set and
        // re-read; on the (single-threaded) common path this stores our value.
        let _ = self.cached.set(output.clone());
        Ok(output)
    }
}

impl SourceAdapter for MacosAdapter {
    fn chats(&self) -> Result<Vec<ChatSummary>> {
        let output = self.read()?;
        Ok(output
            .chats
            .into_iter()
            .map(|chat| ChatSummary {
                chat_id: chat.chat_id,
                chat_name: chat.chat_name,
                chat_type: chat.chat_type,
            })
            .collect())
    }

    fn messages(&self) -> Result<Vec<RawMessage>> {
        Ok(self.read()?.messages)
    }

    /// Not memoized: each call reads the databases from `cursors` on.
    fn messages_since(&self, cursors: &[SourceCursor]) -> Result<SourceRead> {
        let output = crate::kakao::read_kakao_since(&self.options, cursors)?;
        Ok(SourceRead {
            messages: output.messages,
            cursors: output.cursors,
            incremental: output.incremental,
            room_states: output.room_states,
        })
    }

    /// Filters in the query itself, so other chats' messages are never read.
    fn messages_since_for_chats(
        &self,
        cursors: &[SourceCursor],
        chat_ids: &[String],
    ) -> Result<SourceRead> {
        let ids = chat_ids
            .iter()
            .map(|id| id.trim().parse::<i64>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| {
                crate::Error::Kakao("--chat expects numeric KakaoTalk chat ids".to_string())
            })?;
        let output = crate::kakao::read_kakao_since_for_chats(&self.options, cursors, &ids)?;
        Ok(SourceRead {
            messages: output.messages,
            cursors: output.cursors,
            incremental: output.incremental,
            room_states: output.room_states,
        })
    }
}

pub struct KakaocliAdapter;

impl SourceAdapter for KakaocliAdapter {
    fn chats(&self) -> Result<Vec<ChatSummary>> {
        let output = run_kakaocli("chats")?;
        parse_kakaocli_json("chats", &output)
    }

    fn messages(&self) -> Result<Vec<RawMessage>> {
        let output = run_kakaocli("messages")?;
        parse_kakaocli_json("messages", &output)
    }
}

fn parse_kakaocli_json<T>(command: &str, bytes: &[u8]) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|err| {
        crate::Error::Kakaocli(format!("kakaocli {command} returned invalid JSON: {err}"))
    })
}

fn run_kakaocli(command: &str) -> Result<Vec<u8>> {
    let output = Command::new("kakaocli")
        .arg(command)
        .arg("--json")
        .output()
        .map_err(|err| {
            crate::Error::Kakaocli(format!(
                "kakaocli not found on PATH; install kakaocli or ensure it is executable ({err})"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            "no stderr output (run `kakaocli auth` to check database access)"
        } else {
            detail
        };
        return Err(crate::Error::Kakaocli(format!(
            "kakaocli {command} failed ({}): {detail}",
            output.status
        )));
    }
    Ok(output.stdout)
}
