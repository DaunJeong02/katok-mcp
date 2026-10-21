//! A synthetic KakaoTalk install shared by the CLI tests: a SQLCipher database with the
//! KakaoTalk schema built at test time under a temporary HOME, and a katok auth cache naming its
//! synthetic account. No real KakaoTalk, keychain, or system state is touched.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use katok::kakao::{auth, derive};
use rusqlite::{params, Connection};
use serde_json::Value;

pub const TEST_UUID: &str = "00000000-1111-2222-3333-444444444444";
pub const TEST_USER_ID: i64 = 1_000_000_001;
pub const GROUP_A: i64 = 100;
pub const GROUP_B: i64 = 200;
pub const PEER: i64 = 500;

pub struct Install {
    _temp: tempfile::TempDir,
    pub home: PathBuf,
    pub data: PathBuf,
    pub db_path: PathBuf,
    key: String,
}

pub fn install() -> Install {
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
    pub fn source(&self) -> Connection {
        open_source(&self.db_path, &self.key)
    }

    pub fn insert(&self, chat: i64, log: i64, author: i64, attachment: Option<&str>) {
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

    pub fn command(&self, args: &[&str]) -> assert_cmd::Command {
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

    pub fn json(&self, args: &[&str]) -> Value {
        let out = self.command(args).assert().success().get_output().clone();
        serde_json::from_slice(&out.stdout).expect("json stdout")
    }

    pub fn archive(&self) -> Connection {
        Connection::open(self.data.join("archive.sqlite3")).expect("open archive")
    }

    pub fn archived(&self, chat: i64) -> i64 {
        self.archive()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
                [chat.to_string()],
                |row| row.get(0),
            )
            .expect("count archived")
    }

    pub fn has_message(&self, chat: i64, log: i64) -> bool {
        self.archive()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE message_id = ?1)",
                [format!("{chat}-{log}")],
                |row| row.get(0),
            )
            .expect("look up message")
    }
}
