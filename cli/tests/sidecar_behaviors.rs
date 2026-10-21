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
