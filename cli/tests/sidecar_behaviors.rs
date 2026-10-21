//! Flags and subcommands an app embedding katok as a sidecar relies on, driven through the CLI
//! against a synthetic KakaoTalk install: a SQLCipher database built at test time under a
//! temporary HOME, and a katok auth cache that names its synthetic account. No real KakaoTalk,
//! keychain, or system state is touched.

use std::path::{Path, PathBuf};

use katok::kakao::{auth, derive};
use rusqlite::{params, Connection};
use serde_json::Value;

const TEST_UUID: &str = "00000000-1111-2222-3333-444444444444";
const TEST_USER_ID: i64 = 1_000_000_001;
const GROUP_A: i64 = 100;
const GROUP_B: i64 = 200;
const PEER: i64 = 500;

struct Install {
    _temp: tempfile::TempDir,
    home: PathBuf,
    data: PathBuf,
    db_path: PathBuf,
    key: String,
}

fn install() -> Install {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let data = temp.path().join("data");
    let container = auth::container_dir(&home);
    std::fs::create_dir_all(&container).expect("container dir");
    std::fs::create_dir_all(data.join("kakao")).expect("auth cache dir");
    std::fs::write(
        data.join("kakao").join("auth.json"),
        serde_json::json!({ "user_id": TEST_USER_ID, "uuid": TEST_UUID }).to_string(),
    )
    .expect("auth cache");
    let db_path = container.join(derive::database_name(TEST_USER_ID, TEST_UUID));
    let key = derive::secure_key(TEST_USER_ID, TEST_UUID);
    let conn = open_source(&db_path, &key);
    conn.execute_batch(
        "CREATE TABLE NTChatRoom (
            chatId INTEGER NOT NULL DEFAULT 0, linkId INTEGER NOT NULL DEFAULT 0,
            type INTEGER NOT NULL DEFAULT 0, chatName TEXT,
            activeMembersCount INTEGER NOT NULL DEFAULT 0,
            directChatMemberUserId INTEGER NOT NULL DEFAULT 0, displayMemberIds BLOB,
            PRIMARY KEY (chatId, linkId));
         CREATE TABLE NTUser (
            userId INTEGER NOT NULL DEFAULT 0, linkId INTEGER NOT NULL DEFAULT 0,
            friendNickName TEXT, nickName TEXT, displayName TEXT,
            PRIMARY KEY (userId, linkId));
         CREATE TABLE NTChatMessage (
            chatId INTEGER NOT NULL DEFAULT 0, logId INTEGER NOT NULL DEFAULT 0,
            msgId INTEGER NOT NULL DEFAULT 0, authorId INTEGER NOT NULL DEFAULT 0,
            type INTEGER NOT NULL DEFAULT -1, supplement TEXT, attachment TEXT, message TEXT,
            sentAt INTEGER DEFAULT 0,
            PRIMARY KEY (chatId, logId, msgId));
         INSERT INTO NTChatRoom(chatId, linkId, type, chatName, activeMembersCount)
             VALUES (100, 0, 1, 'Synthetic Group A', 3), (200, 0, 1, 'Synthetic Group B', 3);
         INSERT INTO NTUser(userId, linkId, friendNickName) VALUES (500, 0, 'Alpha');",
    )
    .expect("schema");
    let install = Install {
        _temp: temp,
        home,
        data,
        db_path,
        key,
    };
    for log in 1..=3 {
        install.insert(GROUP_A, log, PEER, None);
        install.insert(GROUP_B, 1_000 + log, PEER, None);
    }
    install
}

fn open_source(path: &Path, key: &str) -> Connection {
    let conn = Connection::open(path).expect("open source db");
    conn.execute_batch(&format!(
        "PRAGMA key = '{key}'; PRAGMA cipher_compatibility = 3;"
    ))
    .expect("source key");
    conn
}

impl Install {
    fn source(&self) -> Connection {
        open_source(&self.db_path, &self.key)
    }

    fn insert(&self, chat: i64, log: i64, author: i64, attachment: Option<&str>) {
        self.source()
            .execute(
                "INSERT INTO NTChatMessage(chatId, logId, msgId, authorId, type, attachment,
                                           message, sentAt)
                 VALUES (?1, ?2, ?2, ?3, 1, ?4, ?5, ?6)",
                params![
                    chat,
                    log,
                    author,
                    attachment,
                    format!("synthetic message {log}"),
                    1_700_000_000 + log
                ],
            )
            .expect("insert message");
    }

    fn command(&self, args: &[&str]) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("katok").expect("katok binary");
        command
            .env("HOME", &self.home)
            .env_remove("KATOK_ARCHIVE_KEY")
            .env_remove("KATOK_KAKAO_USER_ID")
            .arg("--data-dir")
            .arg(&self.data)
            .args(args);
        command
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.command(args).assert().success().get_output().clone();
        serde_json::from_slice(&out.stdout).expect("json stdout")
    }

    fn archive(&self) -> Connection {
        Connection::open(self.data.join("archive.sqlite3")).expect("open archive")
    }

    fn archived(&self, chat: i64) -> i64 {
        self.archive()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
                [chat.to_string()],
                |row| row.get(0),
            )
            .expect("count archived")
    }

    fn has_message(&self, chat: i64, log: i64) -> bool {
        self.archive()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE message_id = ?1)",
                [format!("{chat}-{log}")],
                |row| row.get(0),
            )
            .expect("look up message")
    }
}

#[test]
fn a_filtered_sync_archives_only_the_named_chats() {
    let install = install();
    install.json(&["sync", "--source", "macos", "--chat", "100", "--json"]);
    assert_eq!(install.archived(GROUP_A), 3);
    assert_eq!(
        install.archived(GROUP_B),
        0,
        "an unnamed room is not archived"
    );
    let chats: i64 = install
        .archive()
        .query_row("SELECT COUNT(*) FROM chats", [], |row| row.get(0))
        .unwrap();
    assert_eq!(chats, 1, "nor listed");
}

#[test]
fn a_filtered_sync_leaves_the_unfiltered_cursor_where_it_was() {
    let install = install();
    install.json(&["sync", "--source", "macos", "--json"]);
    // Room B's new row first, then more rows in room A than the reader's re-read margin, so a
    // cursor moved to the newest row would start reading after room B's row.
    install.insert(GROUP_B, 2_000, PEER, None);
    for log in 10..90 {
        install.insert(GROUP_A, log, PEER, None);
    }
    install.json(&[
        "sync",
        "--source",
        "macos",
        "--chat",
        "100",
        "--incremental",
        "--json",
    ]);
    assert!(!install.has_message(GROUP_B, 2_000));

    let report = install.json(&["sync", "--source", "macos", "--incremental", "--json"]);
    assert_eq!(report["incremental"], Value::Bool(true));
    assert!(
        install.has_message(GROUP_B, 2_000),
        "the unfiltered incremental sync still reads the room the filtered one skipped"
    );
}

#[test]
fn each_set_of_chats_keeps_its_own_cursor() {
    let install = install();
    let filtered = |chats: &[&str]| {
        let mut args = vec!["sync", "--source", "macos", "--incremental", "--json"];
        for chat in chats {
            args.extend(["--chat", chat]);
        }
        install.json(&args)
    };
    assert_eq!(filtered(&["100"])["incremental"], Value::Bool(false));
    install.insert(GROUP_A, 50, PEER, None);
    let again = filtered(&["100"]);
    assert_eq!(again["incremental"], Value::Bool(true));
    assert_eq!(again["inserted_messages"], 1);

    assert_eq!(
        filtered(&["100", "200"])["incremental"],
        Value::Bool(false),
        "a new set of chats reads them in full"
    );
    assert_eq!(install.archived(GROUP_B), 3);
    assert_eq!(
        filtered(&["200", "100", "100"])["incremental"],
        Value::Bool(true),
        "the set is the same whatever the order or repetition"
    );
}

#[test]
fn a_chat_filter_is_refused_where_it_cannot_hold() {
    let install = install();
    for args in [
        &[
            "sync",
            "--source",
            "macos",
            "--chat",
            "100",
            "--prune-preview",
        ][..],
        &[
            "sync",
            "--source",
            "macos",
            "--chat",
            "100",
            "--prune-deleted",
        ],
        &["sync", "--source", "macos", "--chat", " "],
        &["sync", "--source", "macos", "--chat", "not-a-number"],
    ] {
        install.command(args).assert().failure();
    }
    assert!(!install.data.join("archive.sqlite3").exists() || install.archived(GROUP_A) == 0);
}

#[test]
fn a_fixture_source_filters_too() {
    let install = install();
    let fixture = format!(
        "{}/tests/fixtures/kakao/replies.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let other = install.json(&[
        "sync",
        "--source",
        "fixture",
        &fixture,
        "--chat",
        "chat-other",
        "--json",
    ]);
    assert_eq!(other["total_messages"], 0);
    let named = install.json(&[
        "sync",
        "--source",
        "fixture",
        &fixture,
        "--chat",
        "chat-group-1",
        "--json",
    ]);
    assert_eq!(named["total_messages"], 3);
}

/// Two synthetic rooms written as a fixture archive: a reply chain in each, and one reply that
/// crosses from the kept room into the forgotten one.
fn fixture_archive() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("temp dir");
    let line = |chat: &str, id: &str, minute: u32, reply: Option<&str>| {
        serde_json::json!({
            "account_hash": "acct-synthetic", "chat_id": chat, "chat_name": format!("Room {chat}"),
            "chat_type": "group", "message_id": id, "sender_id": "u1",
            "sender_nickname": "Synthetic", "timestamp": format!("2026-01-01T09:{minute:02}:00Z"),
            "text": format!("synthetic text {id}"), "message_type": "text",
            "reply_to_message_id": reply,
        })
        .to_string()
    };
    let lines = [
        line("chat-forget", "f1", 0, None),
        line("chat-forget", "f2", 1, Some("f1")),
        line("chat-forget", "f3", 30, Some("f1")),
        line("chat-keep", "k1", 0, None),
        line("chat-keep", "k2", 1, Some("k1")),
        line("chat-keep", "k3", 40, Some("f3")),
    ];
    let path = temp.path().join("rooms.jsonl");
    std::fs::write(&path, lines.join("\n")).expect("fixture");
    (temp, path)
}

fn katok_in(data: &Path, args: &[&str]) -> assert_cmd::Command {
    let mut command = assert_cmd::Command::cargo_bin("katok").expect("katok binary");
    command
        .env("HOME", data)
        .env("KATOK_EMBEDDER", "local-test")
        .env_remove("KATOK_ARCHIVE_KEY")
        .arg("--data-dir")
        .arg(data)
        .args(args);
    command
}

fn json_in(data: &Path, args: &[&str]) -> Value {
    let out = katok_in(data, args).assert().success().get_output().clone();
    serde_json::from_slice(&out.stdout).expect("json stdout")
}

/// The ids a chat's rows are keyed by, read before anything is deleted, so what is left
/// afterwards is counted by key rather than through rows that may themselves be gone.
struct ChatKeys {
    chat: String,
    messages: Vec<String>,
    chunks: Vec<String>,
    chunk_rowids: Vec<String>,
    parents: Vec<String>,
}

fn keys_of(data: &Path, chat: &str) -> ChatKeys {
    let conn = Connection::open(data.join("archive.sqlite3")).expect("archive");
    let column = |sql: &str| -> Vec<String> {
        let mut stmt = conn.prepare(sql).expect("prepare");
        stmt.query_map([chat], |row| row.get::<_, rusqlite::types::Value>(0))
            .expect("query")
            .map(|value| match value.expect("row") {
                rusqlite::types::Value::Integer(n) => n.to_string(),
                rusqlite::types::Value::Text(text) => text,
                other => panic!("unexpected key {other:?}"),
            })
            .collect()
    };
    ChatKeys {
        chat: chat.to_string(),
        messages: column("SELECT message_id FROM messages WHERE chat_id = ?1"),
        chunks: column("SELECT chunk_id FROM chunks WHERE chat_id = ?1"),
        chunk_rowids: column("SELECT rowid FROM chunks WHERE chat_id = ?1"),
        parents: column("SELECT parent_id FROM parent_chunks WHERE chat_id = ?1"),
    }
}

/// Rows per archive table that `keys` address.
fn rows_by_key(data: &Path, keys: &ChatKeys) -> std::collections::BTreeMap<&'static str, i64> {
    let conn = Connection::open(data.join("archive.sqlite3")).expect("archive");
    let list = |values: &[String]| -> String {
        values
            .iter()
            .map(|value| format!("'{}'", value.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };
    let (m, c, r, p) = (
        list(&keys.messages),
        list(&keys.chunks),
        keys.chunk_rowids.join(","),
        list(&keys.parents),
    );
    let chat = list(std::slice::from_ref(&keys.chat));
    [
        ("messages", format!("messages WHERE message_id IN ({m})")),
        ("chats", format!("chats WHERE chat_id IN ({chat})")),
        ("chunks", format!("chunks WHERE chunk_id IN ({c})")),
        ("chunks_fts", format!("chunks_fts WHERE rowid IN ({r})")),
        (
            "chunk_messages",
            format!("chunk_messages WHERE chunk_id IN ({c}) OR message_id IN ({m})"),
        ),
        (
            "chunk_parent_refs",
            format!("chunk_parent_refs WHERE child_chunk_id IN ({c}) OR parent_chunk_id IN ({c})"),
        ),
        (
            "parent_chunks",
            format!("parent_chunks WHERE parent_id IN ({p})"),
        ),
        (
            "parent_chunk_children",
            format!("parent_chunk_children WHERE parent_id IN ({p}) OR chunk_id IN ({c})"),
        ),
        (
            "reply_edges",
            format!("reply_edges WHERE child_message_id IN ({m})"),
        ),
    ]
    .into_iter()
    .map(|(table, from)| {
        let count = conn
            .query_row(&format!("SELECT COUNT(*) FROM {from}"), [], |row| {
                row.get(0)
            })
            .unwrap_or_else(|error| panic!("{table}: {error}"));
        (table, count)
    })
    .collect()
}

#[test]
fn forget_counts_first_then_removes_one_chat_everywhere() {
    let (fixture_dir, fixture) = fixture_archive();
    let data = fixture_dir.path().join("data");
    let fixture = fixture.to_str().unwrap();
    json_in(&data, &["sync", "--source", "fixture", fixture, "--json"]);
    json_in(&data, &["index", "--json"]);
    for chat in ["chat-forget", "chat-keep"] {
        json_in(&data, &["transcript", "--chat", chat, "--json"]);
        let media = data.join("media").join(chat).join("album");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(media.join("a.jpg"), b"synthetic").unwrap();
        std::fs::write(media.join("../b.jpg"), b"synthetic").unwrap();
    }
    let forget_keys = keys_of(&data, "chat-forget");
    let keep_keys = keys_of(&data, "chat-keep");
    let before = rows_by_key(&data, &forget_keys);
    let kept_before = rows_by_key(&data, &keep_keys);
    for (table, count) in &before {
        assert!(*count > 0, "the fixture must give {table} rows to delete");
    }

    let preview = json_in(&data, &["forget", "--chat", "chat-forget", "--json"]);
    assert_eq!(preview["deleted"], false);
    let reported = &preview["chats"][0];
    for (table, count) in &before {
        assert_eq!(reported[*table], *count, "{table} count in the preview");
    }
    assert_eq!(reported["media_files"], 2);
    assert_eq!(reported["transcripts"], 1);
    assert_eq!(preview["semantic_rebuild_required"], true);
    assert_eq!(
        rows_by_key(&data, &forget_keys),
        before,
        "a preview deletes nothing"
    );
    assert!(data.join("media").join("chat-forget").exists());
    assert_eq!(
        json_in(&data, &["doctor", "--json"])["semantic_rebuild_required"],
        false
    );

    let done = json_in(
        &data,
        &["forget", "--chat", "chat-forget", "--yes", "--json"],
    );
    assert_eq!(done["deleted"], true);
    assert_eq!(done["next_step"], "katok index --full --json");
    for (table, count) in rows_by_key(&data, &forget_keys) {
        assert_eq!(count, 0, "{table} still holds the forgotten chat");
    }
    let kept_after = rows_by_key(&data, &keep_keys);
    let mut kept_expected = kept_before.clone();
    // The one reference from the kept room into the forgotten one goes; the rest stays.
    kept_expected.insert("chunk_parent_refs", kept_before["chunk_parent_refs"] - 1);
    assert_eq!(kept_after, kept_expected);
    let edge: (Option<String>, Option<String>) = Connection::open(data.join("archive.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT parent_chunk_id, unresolved_reason FROM reply_edges
             WHERE child_message_id = 'k3'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the kept room's reply edge stays");
    assert_eq!(edge, (None, Some("parent_not_in_archive".to_string())));

    assert!(!data.join("media").join("chat-forget").exists());
    assert!(data.join("media").join("chat-keep").join("b.jpg").exists());
    let transcripts: Vec<String> = std::fs::read_dir(data.join("transcripts"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(transcripts.len(), 1);
    assert!(transcripts[0].starts_with("chat-keep-"));

    let rooms = json_in(&data, &["rooms", "--json"]);
    assert_eq!(rooms.as_array().unwrap().len(), 1);
    assert_eq!(rooms[0]["chat_id"], "chat-keep");
    let hits = json_in(&data, &["search", "bm25", "f1", "--json"]);
    assert_eq!(hits, serde_json::json!([]));

    assert_eq!(
        json_in(&data, &["doctor", "--json"])["semantic_rebuild_required"],
        true
    );
    json_in(&data, &["index", "--full", "--json"]);
    assert_eq!(
        json_in(&data, &["doctor", "--json"])["semantic_rebuild_required"],
        false
    );
}

#[test]
fn forget_refuses_a_chat_id_that_names_a_path() {
    let (dir, _) = fixture_archive();
    let data = dir.path().join("data");
    std::fs::create_dir_all(data.join("media")).unwrap();
    std::fs::write(dir.path().join("outside.txt"), b"synthetic").unwrap();
    for chat in ["..", "../..", "a/b"] {
        katok_in(&data, &["forget", "--chat", chat, "--yes", "--json"])
            .assert()
            .failure();
    }
    assert!(dir.path().join("outside.txt").exists());
    assert!(data.join("media").exists());
}

#[test]
fn messages_report_who_each_message_mentions() {
    let install = install();
    let me = TEST_USER_ID.to_string();
    install.insert(
        GROUP_A,
        10,
        PEER,
        Some(&format!(
            r#"{{"mentions":[{{"user_id":{TEST_USER_ID},"at":[1],"len":4}},{{"user_id":600,"at":[7],"len":4}}]}}"#
        )),
    );
    install.insert(GROUP_A, 11, PEER, Some(r#"{"all_mention":{"at":0}}"#));
    install.insert(
        GROUP_A,
        12,
        PEER,
        Some(&format!(
            r#"{{"src_logId":10,"src_mentions":[{{"user_id":{TEST_USER_ID}}}]}}"#
        )),
    );
    install.json(&["sync", "--source", "macos", "--json"]);
    let listed = install.json(&["messages", "--chat", "100", "--json"]);
    let by_id = |id: &str| -> Value {
        listed["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["message_id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} missing"))
    };
    assert_eq!(
        by_id("100-10")["mentioned_user_ids"],
        serde_json::json!([me, "600"])
    );
    assert_eq!(by_id("100-10")["mentions_all"], false);
    assert_eq!(by_id("100-11")["mentioned_user_ids"], serde_json::json!([]));
    assert_eq!(by_id("100-11")["mentions_all"], true);
    assert_eq!(
        by_id("100-12")["mentioned_user_ids"],
        serde_json::json!([]),
        "a reply quoting a mention does not mention anyone itself"
    );
    assert_eq!(by_id("100-1")["mentioned_user_ids"], serde_json::json!([]));
    assert_eq!(by_id("100-1")["mentions_all"], false);
}

#[test]
fn an_archive_from_before_mentions_upgrades_without_rewriting_its_rows() {
    let (dir, fixture) = fixture_archive();
    let data = dir.path().join("data");
    let fixture = fixture.to_str().unwrap();
    json_in(&data, &["sync", "--source", "fixture", fixture, "--json"]);
    let archive = Connection::open(data.join("archive.sqlite3")).unwrap();
    archive
        .execute_batch(
            "ALTER TABLE messages DROP COLUMN mentioned_user_ids;
             ALTER TABLE messages DROP COLUMN mentions_all;",
        )
        .expect("rewind the schema");
    drop(archive);

    let again = json_in(&data, &["sync", "--source", "fixture", fixture, "--json"]);
    assert_eq!(
        again["updated_messages"], 0,
        "no mention anywhere, nothing changed"
    );
    let listed = json_in(&data, &["messages", "--chat", "chat-keep", "--json"]);
    assert_eq!(
        listed["messages"][0]["mentioned_user_ids"],
        serde_json::json!([])
    );
    assert_eq!(listed["messages"][0]["mentions_all"], false);

    let mut lines: Vec<Value> = std::fs::read_to_string(fixture)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    lines[3]["mentioned_user_ids"] = serde_json::json!(["600"]);
    let mentioned = dir.path().join("mentioned.jsonl");
    let body: Vec<String> = lines.iter().map(Value::to_string).collect();
    std::fs::write(&mentioned, body.join("\n")).unwrap();
    let report = json_in(
        &data,
        &[
            "sync",
            "--source",
            "fixture",
            mentioned.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        report["updated_messages"], 1,
        "only the row that gained a mention"
    );
    let listed = json_in(&data, &["messages", "--chat", "chat-keep", "--json"]);
    assert_eq!(
        listed["messages"][0]["mentioned_user_ids"],
        serde_json::json!(["600"])
    );
}

/// A `bplist00` whose root is an array of 8-byte integers, the shape KakaoTalk stores
/// `watermarkKeys`/`watermarkValues` in.
fn bplist_ints(values: &[i64]) -> Vec<u8> {
    let count = values.len();
    let mut out = b"bplist00".to_vec();
    let mut offsets = vec![out.len()];
    if count < 15 {
        out.push(0xa0 | count as u8);
    } else {
        out.push(0xaf);
        out.push(0x11);
        out.extend((count as u16).to_be_bytes());
    }
    for index in 1..=count {
        out.extend((index as u16).to_be_bytes());
    }
    for value in values {
        offsets.push(out.len());
        out.push(0x13);
        out.extend(value.to_be_bytes());
    }
    let table = out.len();
    for offset in &offsets {
        out.extend((*offset as u32).to_be_bytes());
    }
    out.extend([0u8; 6]);
    out.extend([4, 2]);
    out.extend((count as u64 + 1).to_be_bytes());
    out.extend(0u64.to_be_bytes());
    out.extend((table as u64).to_be_bytes());
    out
}

/// Past 2^53, where a JSON number in JavaScript stops being exact.
const BIG_LOG_ID: i64 = 9_007_199_254_740_993;

impl Install {
    fn add_read_state_columns(&self) {
        self.source()
            .execute_batch(
                "ALTER TABLE NTChatRoom ADD COLUMN lastSeenLogId INTEGER;
                 ALTER TABLE NTChatRoom ADD COLUMN countOfNewMessage INTEGER;
                 ALTER TABLE NTChatRoom ADD COLUMN watermarkKeys BLOB;
                 ALTER TABLE NTChatRoom ADD COLUMN watermarkValues BLOB;",
            )
            .expect("read state columns");
    }

    fn set_read_state(&self, chat: i64, last_seen: Option<i64>, unread: i64, marks: &[(i64, i64)]) {
        let keys: Vec<i64> = marks.iter().map(|(user, _)| *user).collect();
        let values: Vec<i64> = marks.iter().map(|(_, log)| *log).collect();
        self.source()
            .execute(
                "UPDATE NTChatRoom SET lastSeenLogId = ?2, countOfNewMessage = ?3,
                     watermarkKeys = ?4, watermarkValues = ?5
                 WHERE chatId = ?1",
                params![
                    chat,
                    last_seen,
                    unread,
                    bplist_ints(&keys),
                    bplist_ints(&values)
                ],
            )
            .expect("read state");
    }

    fn room(&self, chat: i64) -> Value {
        let rooms = self.json(&["rooms", "--json"]);
        let id = chat.to_string();
        rooms
            .as_array()
            .unwrap()
            .iter()
            .find(|room| room["chat_id"] == id.as_str())
            .cloned()
            .unwrap_or_else(|| panic!("room {chat} missing"))
    }
}

#[test]
fn rooms_report_each_read_position_kakaotalk_keeps() {
    let install = install();
    install.add_read_state_columns();
    install.set_read_state(
        GROUP_A,
        Some(BIG_LOG_ID),
        2,
        &[(PEER, 5), (TEST_USER_ID, BIG_LOG_ID + 2)],
    );
    install.set_read_state(GROUP_B, None, 0, &[(PEER, 7)]);
    install.json(&["sync", "--source", "macos", "--json"]);

    let a = install.room(GROUP_A);
    assert_eq!(
        a["last_seen_log_id"],
        BIG_LOG_ID.to_string(),
        "exact, as a string"
    );
    assert_eq!(a["new_message_count"], 2);
    assert_eq!(a["my_watermark_log_id"], (BIG_LOG_ID + 2).to_string());
    let b = install.room(GROUP_B);
    assert_eq!(b["last_seen_log_id"], Value::Null);
    assert_eq!(b["new_message_count"], 0);
    assert_eq!(
        b["my_watermark_log_id"],
        Value::Null,
        "no watermark entry of mine"
    );

    // Reading the room in KakaoTalk moves the position without adding a message.
    install.set_read_state(GROUP_A, Some(3), 0, &[(TEST_USER_ID, 3)]);
    let quiet = install.json(&["sync", "--source", "macos", "--incremental", "--json"]);
    assert_eq!(quiet["inserted_messages"], 0);
    let a = install.room(GROUP_A);
    assert_eq!(a["last_seen_log_id"], "3");
    assert_eq!(a["new_message_count"], 0);
    assert_eq!(a["my_watermark_log_id"], "3");
}

#[test]
fn a_watermark_is_found_in_a_room_of_hundreds() {
    let install = install();
    install.add_read_state_columns();
    let mut marks: Vec<(i64, i64)> = (0..400).map(|n| (10_000 + n, n)).collect();
    marks.push((TEST_USER_ID, 77));
    install.set_read_state(GROUP_A, Some(1), 0, &marks);
    install.json(&["sync", "--source", "macos", "--chat", "100", "--json"]);
    assert_eq!(install.room(GROUP_A)["my_watermark_log_id"], "77");
}

#[test]
fn read_positions_are_null_where_kakaotalk_has_none_and_skip_unarchived_rooms() {
    let install = install();
    install.json(&["sync", "--source", "macos", "--chat", "100", "--json"]);
    let a = install.room(GROUP_A);
    for field in [
        "last_seen_log_id",
        "new_message_count",
        "my_watermark_log_id",
    ] {
        assert_eq!(a[field], Value::Null, "{field} without the source column");
    }

    // An install with some of the columns still reports the ones it has.
    install
        .source()
        .execute_batch(
            "ALTER TABLE NTChatRoom ADD COLUMN lastSeenLogId INTEGER;
             ALTER TABLE NTChatRoom ADD COLUMN countOfNewMessage INTEGER;
             UPDATE NTChatRoom SET lastSeenLogId = 2, countOfNewMessage = 1;",
        )
        .expect("partial read state columns");
    install.json(&["sync", "--source", "macos", "--chat", "100", "--json"]);
    let a = install.room(GROUP_A);
    assert_eq!(a["last_seen_log_id"], "2");
    assert_eq!(a["new_message_count"], 1);
    assert_eq!(a["my_watermark_log_id"], Value::Null);

    // A room KakaoTalk lists with a read position but no message stays out of the archive.
    install
        .source()
        .execute(
            "INSERT INTO NTChatRoom(chatId, linkId, type, chatName, activeMembersCount,
                                    lastSeenLogId, countOfNewMessage)
             VALUES (300, 0, 1, 'Synthetic Empty Room', 3, 4, 0)",
            [],
        )
        .expect("empty room");
    install.json(&["sync", "--source", "macos", "--json"]);
    let rooms = install.json(&["rooms", "--json"]);
    let ids: Vec<&str> = rooms
        .as_array()
        .unwrap()
        .iter()
        .map(|room| room["chat_id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.contains(&"300"),
        "a read position never adds a room: {ids:?}"
    );
    assert_eq!(ids.len(), 2);
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// Restores a path's mode when dropped, so a failed assertion still leaves a temp dir that can
/// be removed.
struct ModeGuard<'a>(&'a Path, u32);

impl Drop for ModeGuard<'_> {
    fn drop(&mut self) {
        set_mode(self.0, self.1);
    }
}

#[test]
fn a_plain_doctor_never_opens_the_container() {
    let install = install();
    let container = auth::container_dir(&install.home);
    set_mode(&container, 0o000);
    let _restore = ModeGuard(&container, 0o755);
    let doctor = install.json(&["doctor", "--json"]);
    assert_eq!(
        doctor["source_access"],
        serde_json::json!({ "status": "not_checked", "failed_at": null, "errno": null })
    );
    let app = doctor["kakao_app"]["status"].as_str().unwrap();
    assert!(["running", "not_running"].contains(&app), "{app}");
}

#[test]
fn doctor_probe_reports_source_access_with_the_errno_it_met() {
    let install = install();
    let probe = |install: &Install| install.json(&["doctor", "--macos-probe", "--json"]);
    assert_eq!(
        probe(&install)["source_access"],
        serde_json::json!({ "status": "ok", "failed_at": null, "errno": null })
    );

    {
        set_mode(&install.db_path, 0o000);
        let _restore = ModeGuard(&install.db_path, 0o644);
        assert_eq!(
            probe(&install)["source_access"],
            serde_json::json!({ "status": "denied", "failed_at": "open", "errno": libc::EACCES })
        );
    }

    let container = auth::container_dir(&install.home);
    {
        set_mode(&container, 0o000);
        let _restore = ModeGuard(&container, 0o755);
        assert_eq!(
            probe(&install)["source_access"],
            serde_json::json!({ "status": "denied", "failed_at": "opendir", "errno": libc::EACCES })
        );
    }

    std::fs::remove_file(&install.db_path).unwrap();
    assert_eq!(
        probe(&install)["source_access"],
        serde_json::json!({ "status": "missing", "failed_at": "open", "errno": null })
    );
    std::fs::remove_dir_all(&container).unwrap();
    assert_eq!(
        probe(&install)["source_access"],
        serde_json::json!({ "status": "missing", "failed_at": "opendir", "errno": libc::ENOENT })
    );
}

#[test]
fn a_running_process_is_found_by_its_exact_name() {
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    let running = katok::kakao::process_running("sleep");
    child.kill().ok();
    child.wait().ok();
    assert_eq!(running, Some(true));
    assert_eq!(
        katok::kakao::process_running("katok-test-no-such-process"),
        Some(false)
    );
}
