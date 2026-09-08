//! talk2build branch additions: `messages`, `rooms`, and `search --chat`.

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/kakao/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn synced_data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    Command::cargo_bin("katok")
        .expect("katok binary")
        .args([
            "--data-dir",
            dir.path().to_str().expect("utf8 path"),
            "sync",
            "--source",
            "fixture",
            &fixture_path("replies.jsonl"),
            "--json",
        ])
        .assert()
        .success();
    dir
}

#[test]
fn rooms_lists_every_chat_with_tail_metadata() {
    let dir = synced_data_dir();
    let output = Command::cargo_bin("katok")
        .expect("katok binary")
        .args([
            "--data-dir",
            dir.path().to_str().expect("utf8 path"),
            "rooms",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rooms: serde_json::Value = serde_json::from_slice(&output).expect("rooms json");
    let list = rooms.as_array().expect("array");
    assert!(!list.is_empty());
    let group = list
        .iter()
        .find(|room| room["chat_id"] == "chat-group-1")
        .expect("fixture group room");
    assert_eq!(group["chat_name"], "Synthetic Team");
    assert!(group["message_count"].as_u64().expect("count") >= 3);
    assert!(group["last_timestamp"].is_string());
    assert!(group["last_message_id"].is_string());
}

#[test]
fn messages_returns_oldest_first_with_mine_flag_and_reply_edges() {
    let dir = synced_data_dir();
    let output = Command::cargo_bin("katok")
        .expect("katok binary")
        .args([
            "--data-dir",
            dir.path().to_str().expect("utf8 path"),
            "messages",
            "--chat",
            "chat-group-1",
            "--me",
            "u2",
            "--limit",
            "2",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: serde_json::Value = serde_json::from_slice(&output).expect("messages json");
    assert_eq!(payload["count"], 2);
    let messages = payload["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2, "limit takes the newest two");
    let first_ts = messages[0]["timestamp"].as_str().expect("ts");
    let second_ts = messages[1]["timestamp"].as_str().expect("ts");
    assert!(first_ts <= second_ts, "oldest first");
    let by_u2 = messages
        .iter()
        .find(|message| message["sender_id"] == "u2")
        .expect("u2 message in newest two");
    assert_eq!(by_u2["is_mine"], true);
    assert_eq!(by_u2["is_feed"], false);
    let by_other = messages.iter().find(|message| message["sender_id"] != "u2");
    if let Some(other) = by_other {
        assert_eq!(other["is_mine"], false);
    }
}

#[test]
fn messages_since_cursor_excludes_older_rows() {
    let dir = synced_data_dir();
    let output = Command::cargo_bin("katok")
        .expect("katok binary")
        .args([
            "--data-dir",
            dir.path().to_str().expect("utf8 path"),
            "messages",
            "--chat",
            "chat-group-1",
            "--since",
            "2026-01-01T09:02:00+00:00",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: serde_json::Value = serde_json::from_slice(&output).expect("messages json");
    for message in payload["messages"].as_array().expect("messages array") {
        assert_ne!(message["message_id"], "m1");
        assert_ne!(message["message_id"], "m2");
    }
    let reply = payload["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|message| message["message_id"] == "m3")
        .expect("m3 present");
    assert_eq!(reply["reply_to_message_id"], "m1");
}

#[test]
fn search_chat_scope_filters_other_rooms() {
    let dir = synced_data_dir();
    Command::cargo_bin("katok")
        .expect("katok binary")
        .args([
            "--data-dir",
            dir.path().to_str().expect("utf8 path"),
            "search",
            "bm25",
            "보고서",
            "--chat",
            "chat-does-not-exist",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("chunk_id").not());

    Command::cargo_bin("katok")
        .expect("katok binary")
        .args([
            "--data-dir",
            dir.path().to_str().expect("utf8 path"),
            "search",
            "keyword",
            "보고서",
            "--chat",
            "chat-group-1",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"chat_id\": \"chat-group-1\""));
}
