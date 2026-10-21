use crate::support::print_payload;
use anyhow::{Context, Result};
use katok::archive::{Archive, ChatFootprint};
use std::path::Path;

/// Left in the semantic index directory when a chat is forgotten: its vectors cannot be removed
/// one chat at a time, so they stay on disk until the next `katok index` builds a generation
/// from the archive as it is now.
pub(crate) const REBUILD_MARKER: &str = "REBUILD_REQUIRED";

#[derive(Debug, Default, serde::Serialize)]
struct ForgottenChat {
    chat_id: String,
    #[serde(flatten)]
    archive: ChatFootprint,
    media_files: usize,
    transcripts: usize,
}

pub(crate) fn run(
    chats: &[String],
    yes: bool,
    json: bool,
    archive_path: &Path,
    data_dir: &Path,
    semantic_dir: &Path,
) -> Result<()> {
    let chats = super::chat_filter(chats)?;
    if !chats.iter().all(|id| is_path_component(id)) {
        anyhow::bail!("--chat values must be chat ids, not paths");
    }
    let archive = if archive_path.is_file() {
        Some(Archive::open(archive_path).context("open archive")?)
    } else {
        None
    };

    let mut report = Vec::with_capacity(chats.len());
    for chat_id in &chats {
        let footprint = match &archive {
            Some(archive) => archive.chat_footprint(chat_id).context("count chat rows")?,
            None => ChatFootprint::default(),
        };
        report.push(ForgottenChat {
            chat_id: chat_id.clone(),
            archive: footprint,
            media_files: count_files(&media_dir(data_dir, chat_id)),
            transcripts: transcripts_of(data_dir, chat_id).len(),
        });
    }
    let index_holds_them = semantic_dir.is_dir()
        && report
            .iter()
            .any(|chat| chat.archive.chunks + chat.archive.parent_chunks > 0);

    if yes {
        if let Some(archive) = &archive {
            archive
                .in_transaction(|| {
                    for chat_id in &chats {
                        archive.forget_chat(chat_id)?;
                    }
                    Ok::<_, katok::Error>(())
                })
                .context("delete chat rows")?;
        }
        for chat_id in &chats {
            let media = media_dir(data_dir, chat_id);
            if media.exists() {
                std::fs::remove_dir_all(&media).context("remove saved media")?;
            }
            for transcript in transcripts_of(data_dir, chat_id) {
                std::fs::remove_file(transcript).context("remove transcript")?;
            }
        }
        if index_holds_them {
            write_rebuild_marker(semantic_dir).context("mark semantic index for rebuild")?;
        }
    }

    print_payload(
        json,
        &serde_json::json!({
            "deleted": yes,
            "chats": report,
            "semantic_rebuild_required": index_holds_them,
            "next_step": (yes && index_holds_them).then_some("katok index --full --json"),
        }),
    )
}

fn is_path_component(id: &str) -> bool {
    id != "." && id != ".." && !id.contains(['/', '\\', '\0'])
}

fn media_dir(data_dir: &Path, chat_id: &str) -> std::path::PathBuf {
    data_dir.join("media").join(chat_id)
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => count_files(&entry.path()),
            Ok(_) => 1,
            Err(_) => 0,
        })
        .sum()
}

/// Transcripts `katok transcript` wrote for `chat_id` into its default directory, recognised by
/// the header line it writes (`# <name> (chat <chat_id>)`) rather than by file name.
fn transcripts_of(data_dir: &Path, chat_id: &str) -> Vec<std::path::PathBuf> {
    use std::io::BufRead;
    let Ok(entries) = std::fs::read_dir(data_dir.join("transcripts")) else {
        return Vec::new();
    };
    let header_end = format!("(chat {chat_id})");
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .filter(|path| {
            std::fs::File::open(path)
                .ok()
                .and_then(|file| std::io::BufReader::new(file).lines().next()?.ok())
                .is_some_and(|first| first.starts_with("# ") && first.ends_with(&header_end))
        })
        .collect()
}

fn write_rebuild_marker(semantic_dir: &Path) -> std::io::Result<()> {
    let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let marker = serde_json::json!({ "reason": "forget", "at": stamp });
    std::fs::write(semantic_dir.join(REBUILD_MARKER), format!("{marker}\n"))
}

/// The marker's contents, or `None` when there is none. `katok index` reads it before it starts
/// and passes it to [`clear_rebuild_marker`] after it commits.
pub(crate) fn rebuild_marker(semantic_dir: &Path) -> Option<String> {
    std::fs::read_to_string(semantic_dir.join(REBUILD_MARKER)).ok()
}

/// Remove the marker if it is still the one `seen` before the index run started. A chat
/// forgotten while that run was embedding left a newer marker, and the generation just
/// committed may still hold its vectors.
pub(crate) fn clear_rebuild_marker(semantic_dir: &Path, seen: Option<&str>) {
    if seen.is_some() && rebuild_marker(semantic_dir).as_deref() == seen {
        let _ = std::fs::remove_file(semantic_dir.join(REBUILD_MARKER));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_clears_only_the_marker_it_saw_before_starting() {
        let dir = tempfile::tempdir().expect("temp dir");
        let seen = rebuild_marker(dir.path());
        write_rebuild_marker(dir.path()).expect("marker");
        clear_rebuild_marker(dir.path(), seen.as_deref());
        assert!(
            rebuild_marker(dir.path()).is_some(),
            "a chat forgotten after the run started keeps its marker"
        );

        let seen = rebuild_marker(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(2));
        write_rebuild_marker(dir.path()).expect("newer marker");
        clear_rebuild_marker(dir.path(), seen.as_deref());
        assert!(rebuild_marker(dir.path()).is_some(), "nor does a newer one");

        let seen = rebuild_marker(dir.path());
        clear_rebuild_marker(dir.path(), seen.as_deref());
        assert!(rebuild_marker(dir.path()).is_none());
    }

    #[test]
    fn chat_ids_that_name_paths_are_refused() {
        for id in ["100", "chat-group-1", "...x"] {
            assert!(is_path_component(id), "{id}");
        }
        for id in [".", "..", "a/b", "../x", "a\\b", "a\0b"] {
            assert!(!is_path_component(id), "{id:?}");
        }
    }
}
