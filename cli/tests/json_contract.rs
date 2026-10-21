//! The `--json` key sets an app embedding katok as a sidecar relies on, pinned as golden files in
//! `tests/fixtures/json-keys/`: every key path, nested ones included, with values left out.
//!
//! A key the golden lists but the output lacks fails: removing or renaming a field breaks those
//! callers, so `UPDATE_GOLDEN` refuses it too, and a deliberate removal means editing the golden
//! by hand. A key the output adds also fails until the golden is regenerated with
//! `UPDATE_GOLDEN=1 cargo test --test json_contract`, so every addition is reviewed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/json-keys")
}

fn key_paths(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                out.insert(path.clone());
                key_paths(child, &path, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                key_paths(item, &format!("{prefix}[]"), out);
            }
        }
        _ => {}
    }
}

fn check_golden(name: &str, command: &str, output: &Value) {
    let mut actual = BTreeSet::new();
    key_paths(output, "", &mut actual);
    let path = golden_dir().join(format!("{name}.keys"));
    let golden: BTreeSet<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    let missing: Vec<&String> = golden.difference(&actual).collect();
    let added: Vec<&String> = actual.difference(&golden).collect();
    assert!(
        missing.is_empty(),
        "`{command}` no longer prints {missing:?}, which {} lists. Removing or renaming a \
         `--json` field breaks apps that read it; add a new field instead.",
        path.display()
    );
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let mut body = format!("# katok {command}\n");
        for key in &actual {
            body.push_str(key);
            body.push('\n');
        }
        std::fs::create_dir_all(golden_dir()).expect("golden dir");
        std::fs::write(&path, body).expect("write golden");
        return;
    }
    assert!(
        added.is_empty(),
        "`{command}` prints keys {} does not list: {added:?}. If they are meant to be part of \
         the output, regenerate the golden with `UPDATE_GOLDEN=1 cargo test --test \
         json_contract` and commit it.",
        path.display()
    );
}

/// A synthetic archive with two rooms, a reply, a mention and a semantic index, under a
/// temporary HOME that has no KakaoTalk container.
struct Archive {
    temp: tempfile::TempDir,
    fixture: PathBuf,
}

impl Archive {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let line = |chat: &str, id: &str, minute: u32, reply: Option<&str>, mention: &[&str]| {
            serde_json::json!({
                "account_hash": "acct-synthetic", "chat_id": chat,
                "chat_name": format!("Room {chat}"), "chat_type": "group", "message_id": id,
                "sender_id": "u1", "sender_nickname": "Synthetic",
                "timestamp": format!("2026-01-01T09:{minute:02}:00Z"),
                "text": format!("synthetic text {id}"), "message_type": "text",
                "reply_to_message_id": reply, "mentioned_user_ids": mention,
            })
            .to_string()
        };
        let lines = [
            line("chat-a", "a1", 0, None, &[]),
            line("chat-a", "a2", 1, Some("a1"), &["u2"]),
            line("chat-b", "b1", 0, None, &[]),
        ];
        let fixture = temp.path().join("rooms.jsonl");
        std::fs::write(&fixture, lines.join("\n")).expect("fixture");
        let archive = Self { temp, fixture };
        archive.json(&["sync", "--source", "fixture", archive.fixture(), "--json"]);
        archive.json(&["index", "--json"]);
        archive
    }

    fn fixture(&self) -> &str {
        self.fixture.to_str().expect("utf8 path")
    }

    fn command(&self, args: &[&str]) -> assert_cmd::Command {
        let data = self.temp.path().join("data");
        let mut command = assert_cmd::Command::cargo_bin("katok").expect("katok binary");
        command
            .env("HOME", self.temp.path())
            .env("KATOK_EMBEDDER", "local-test")
            .env_remove("KATOK_ARCHIVE_KEY")
            .env_remove("KATOK_KAKAO_USER_ID")
            .arg("--data-dir")
            .arg(data)
            .args(args);
        command
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.command(args).assert().success().get_output().clone();
        serde_json::from_slice(&out.stdout).expect("json stdout")
    }
}

#[test]
fn rooms_json_keys_match_the_golden() {
    let archive = Archive::new();
    check_golden("rooms", "rooms --json", &archive.json(&["rooms", "--json"]));
}

#[test]
fn messages_json_keys_match_the_golden() {
    let archive = Archive::new();
    let output = archive.json(&["messages", "--chat", "chat-a", "--json"]);
    check_golden("messages", "messages --chat <id> --json", &output);
}

#[test]
fn doctor_json_keys_match_the_golden() {
    let archive = Archive::new();
    check_golden(
        "doctor",
        "doctor --json",
        &archive.json(&["doctor", "--json"]),
    );
    // HOME is a temp dir with no KakaoTalk container, so the probe opens nothing real.
    let probed = archive.json(&["doctor", "--macos-probe", "--json"]);
    check_golden("doctor-macos-probe", "doctor --macos-probe --json", &probed);
}

#[test]
fn sync_json_keys_match_the_golden() {
    let archive = Archive::new();
    let fixture = archive.fixture().to_string();
    let plain = archive.json(&["sync", "--source", "fixture", &fixture, "--json"]);
    check_golden("sync", "sync --json", &plain);
    let scoped = archive.json(&[
        "sync",
        "--source",
        "fixture",
        &fixture,
        "--chat",
        "chat-a",
        "--incremental",
        "--json",
    ]);
    check_golden(
        "sync-chat-incremental",
        "sync --chat <id> --incremental --json",
        &scoped,
    );
}

#[test]
fn forget_preview_json_keys_match_the_golden() {
    let archive = Archive::new();
    let output = archive.json(&["forget", "--chat", "chat-a", "--json"]);
    assert_eq!(output["deleted"], false, "a preview, nothing deleted");
    check_golden("forget-preview", "forget --chat <id> --json", &output);
}

#[test]
fn the_error_envelope_keys_match_the_golden() {
    let archive = Archive::new();
    let missing = archive.temp.path().join("no-such-fixture.jsonl");
    let out = archive
        .command(&[
            "sync",
            "--source",
            "fixture",
            missing.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .clone();
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("json error envelope");
    check_golden("error", "<any command> --json, on failure", &envelope);
}

#[test]
fn key_paths_cover_nesting_and_array_elements() {
    let value = serde_json::json!([{ "a": { "b": 1 }, "c": [{ "d": null }], "e": [] }]);
    let mut paths = BTreeSet::new();
    key_paths(&value, "", &mut paths);
    let expected: BTreeSet<String> = ["[].a", "[].a.b", "[].c", "[].c[].d", "[].e"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(paths, expected);
}
