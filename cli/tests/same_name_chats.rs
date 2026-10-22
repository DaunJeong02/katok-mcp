//! `Archive::same_name_chat_count`, which `send --chat` reads before it trusts an open window.

use katok::{archive::Archive, types::RawMessage};

fn message(chat: &str, name: &str, id: &str, at: &str) -> RawMessage {
    RawMessage {
        account_hash: "a".repeat(64),
        chat_id: chat.into(),
        chat_name: name.into(),
        chat_type: "group".into(),
        message_id: format!("{chat}-{id}"),
        sender_id: "500".into(),
        sender_nickname: "Alpha".into(),
        timestamp: at.parse().unwrap(),
        text: format!("synthetic {id}"),
        message_type: "text".into(),
        reply_to_message_id: None,
        mentioned_user_ids: Vec::new(),
        mentions_all: false,
    }
}

#[test]
fn counts_distinct_chats_per_name_including_the_target() {
    let temp = tempfile::tempdir().expect("temp dir");
    let archive = Archive::open(&temp.path().join("archive.sqlite3")).expect("archive");
    archive
        .sync_messages(&[
            message("100", "Synthetic Club", "1", "2026-01-01T09:00:00Z"),
            message("100", "Synthetic Club", "2", "2026-01-01T09:01:00Z"),
            message("200", "Synthetic Club", "1", "2026-01-01T10:00:00Z"),
            message("300", "Synthetic Solo", "1", "2026-01-01T11:00:00Z"),
        ])
        .expect("sync");

    assert_eq!(archive.same_name_chat_count("Synthetic Club").unwrap(), 2);
    assert_eq!(archive.same_name_chat_count("Synthetic Solo").unwrap(), 1);
    assert_eq!(archive.same_name_chat_count("Synthetic Absent").unwrap(), 0);

    // The same rows chat_identity ranks: chat 200 is the more recent of the two.
    assert_eq!(
        archive.chat_identity("100").unwrap(),
        Some(("Synthetic Club".to_string(), 1))
    );
    assert_eq!(
        archive.chat_identity("200").unwrap(),
        Some(("Synthetic Club".to_string(), 0))
    );
}
