//! `search keyword`: `--order` and literal matching of LIKE metacharacters.
//!
//! Each matching message sits in its own synthetic chat, so every hit is its own chunk and the
//! assertions never depend on how messages are grouped into chunks.

use assert_cmd::Command;
use serde_json::Value;

struct Archive {
    temp: tempfile::TempDir,
}

impl Archive {
    /// One message per `(chat_id, hour, text)`, all on 2026-01-01 UTC.
    fn with(rows: &[(&str, u32, &str)]) -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let lines: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, (chat, hour, text))| {
                serde_json::json!({
                    "account_hash": "acct-synthetic", "chat_id": chat,
                    "chat_name": format!("Room {chat}"), "chat_type": "group",
                    "message_id": format!("m{i}"), "sender_id": "u1",
                    "sender_nickname": "Synthetic",
                    "timestamp": format!("2026-01-01T{hour:02}:00:00Z"),
                    "text": text, "message_type": "text", "reply_to_message_id": null,
                })
                .to_string()
            })
            .collect();
        let fixture = temp.path().join("rows.jsonl");
        std::fs::write(&fixture, lines.join("\n")).expect("fixture");
        let archive = Self { temp };
        archive
            .command(&["sync", "--source", "fixture", fixture.to_str().unwrap()])
            .assert()
            .success();
        archive
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::cargo_bin("katok").expect("katok binary");
        command
            .env("HOME", self.temp.path())
            .env_remove("KATOK_ARCHIVE_KEY")
            .arg("--data-dir")
            .arg(self.temp.path().join("data"))
            .args(args);
        command
    }

    /// The `chat_id` of each keyword hit, in output order.
    fn hit_chats(&self, args: &[&str]) -> Vec<String> {
        let mut full = vec!["search", "keyword"];
        full.extend_from_slice(args);
        full.push("--json");
        let out = self.command(&full).assert().success().get_output().clone();
        let hits: Value = serde_json::from_slice(&out.stdout).expect("json hits");
        hits.as_array()
            .expect("hit array")
            .iter()
            .map(|hit| hit["chat_id"].as_str().expect("chat_id").to_string())
            .collect()
    }
}

#[test]
fn newest_order_takes_the_limit_from_the_recent_end() {
    let archive = Archive::with(&[
        ("chat-a", 9, "synthetic needle one"),
        ("chat-b", 10, "synthetic needle two"),
        ("chat-c", 11, "synthetic needle three"),
    ]);

    assert_eq!(
        archive.hit_chats(&["needle", "--limit", "2"]),
        ["chat-a", "chat-b"],
        "the default stays oldest first"
    );
    assert_eq!(
        archive.hit_chats(&["needle", "--limit", "2", "--order", "oldest"]),
        ["chat-a", "chat-b"]
    );
    assert_eq!(
        archive.hit_chats(&["needle", "--limit", "2", "--order", "newest"]),
        ["chat-c", "chat-b"],
        "newest first, and the limit keeps the two most recent"
    );
}

#[test]
fn newest_order_still_respects_the_chat_scope() {
    let archive = Archive::with(&[
        ("chat-a", 9, "synthetic needle one"),
        ("chat-b", 10, "synthetic needle two"),
    ]);
    assert_eq!(
        archive.hit_chats(&["needle", "--order", "newest", "--chat", "chat-a"]),
        ["chat-a"]
    );
}

#[test]
fn an_unknown_order_is_a_usage_error() {
    let archive = Archive::with(&[("chat-a", 9, "synthetic needle")]);
    archive
        .command(&["search", "keyword", "needle", "--order", "latest", "--json"])
        .assert()
        .code(2);
}

#[test]
fn percent_and_underscore_in_the_query_match_literally() {
    let archive = Archive::with(&[
        ("chat-a", 9, "synthetic 100% done"),
        ("chat-b", 10, "synthetic 1000 items"),
        ("chat-c", 11, "synthetic a_b key"),
        ("chat-d", 12, "synthetic axb key"),
        ("chat-e", 13, r"synthetic C:\tmp path"),
        ("chat-f", 14, "synthetic C:tmp path"),
    ]);

    assert_eq!(archive.hit_chats(&["100%"]), ["chat-a"]);
    assert_eq!(archive.hit_chats(&["a_b"]), ["chat-c"]);
    assert_eq!(archive.hit_chats(&[r"C:\tmp"]), ["chat-e"]);
    assert_eq!(
        archive.hit_chats(&["100"]),
        ["chat-a", "chat-b"],
        "a plain substring still matches every row containing it"
    );
}
