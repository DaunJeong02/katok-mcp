//! `katok sync --incremental`: the reader reads each KakaoTalk database only past the cursor the
//! previous sync stored (its newest local rowid), and falls back to a full read whenever that
//! cursor no longer marks the same message. Synthetic SQLCipher databases only; auth comes from
//! injected overrides, so no real KakaoTalk or system state is touched.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use katok::archive::Archive;
use katok::kakao::{auth, derive, AuthOptions};
use katok::types::SourceCursor;
use rusqlite::{params, Connection};

const TEST_UUID: &str = "00000000-1111-2222-3333-444444444444";
const TEST_USER_ID: i64 = 1_000_000_001;
/// More rows than the reader's re-read margin, so an incremental read is visibly smaller.
const FIRST_ROWS: i64 = 150;

struct Fixture {
    _temp: tempfile::TempDir,
    db_path: PathBuf,
    key: String,
    options: AuthOptions,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let container = auth::container_dir(&home);
    std::fs::create_dir_all(&container).expect("container dir");
    let db_path = container.join(derive::database_name(TEST_USER_ID, TEST_UUID));
    let key = derive::secure_key(TEST_USER_ID, TEST_UUID);
    let conn = open(&db_path, &key);
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
         CREATE TABLE NTChatMeta (
            chatId INTEGER NOT NULL DEFAULT 0, type INTEGER NOT NULL DEFAULT 0,
            revision INTEGER NOT NULL DEFAULT 0, content TEXT,
            PRIMARY KEY (chatId, type, revision));
         INSERT INTO NTChatRoom(chatId, linkId, type, chatName, activeMembersCount,
                                directChatMemberUserId)
             VALUES (100, 0, 1, 'Synthetic Group', 3, 0);
         INSERT INTO NTUser(userId, linkId, friendNickName, nickName, displayName)
             VALUES (500, 0, 'Alpha', NULL, NULL);",
    )
    .expect("schema");
    for log in 1..=FIRST_ROWS {
        insert(&conn, log, 1_700_000_000 + log);
    }
    Fixture {
        _temp: temp,
        db_path,
        key,
        options: AuthOptions {
            home,
            data_dir,
            user_id_override: Some(TEST_USER_ID),
            uuid_override: Some(TEST_UUID.to_string()),
            max_user_id: 0,
        },
    }
}

fn open(path: &Path, key: &str) -> Connection {
    let conn = Connection::open(path).expect("open db");
    conn.execute_batch(&format!(
        "PRAGMA key = '{key}'; PRAGMA cipher_compatibility = 3;"
    ))
    .expect("key");
    conn
}

fn insert(conn: &Connection, log: i64, sent_at: i64) {
    conn.execute(
        "INSERT INTO NTChatMessage(chatId, logId, msgId, authorId, type, message, sentAt)
         VALUES (100, ?1, ?1, 500, 1, ?2, ?3)",
        params![log, format!("synthetic message {log}"), sent_at],
    )
    .expect("insert message");
}

fn ids(messages: &[katok::types::RawMessage]) -> HashSet<String> {
    messages.iter().map(|m| m.message_id.clone()).collect()
}

fn read(f: &Fixture, cursors: &[SourceCursor]) -> katok::kakao::ReaderOutput {
    katok::kakao::read_kakao_since(&f.options, cursors).expect("read")
}

#[test]
fn a_full_read_reports_where_it_stopped() {
    let f = fixture();
    let full = read(&f, &[]);
    assert!(!full.incremental, "no cursor is a full read");
    assert_eq!(full.messages.len(), FIRST_ROWS as usize);
    assert_eq!(
        full.cursors.len(),
        8,
        "the newest rows, so one replaced row costs nothing"
    );
    let cursor = &full.cursors[0];
    assert_eq!((cursor.chat_id, cursor.log_id), (100, FIRST_ROWS));
    assert_eq!(cursor.rowid, FIRST_ROWS, "newest local row first");
    assert!(full.cursors.windows(2).all(|w| w[0].rowid > w[1].rowid));
    assert_eq!(cursor.db.len(), 16, "a short hash, never the file name");
    let name = f.db_path.file_name().unwrap().to_string_lossy();
    assert!(!name.contains(&cursor.db));
}

#[test]
fn an_incremental_read_returns_what_was_added_since() {
    let f = fixture();
    let full = read(&f, &[]);
    let conn = open(&f.db_path, &f.key);
    insert(&conn, FIRST_ROWS + 1, 1_800_000_000);
    insert(&conn, FIRST_ROWS + 2, 1_800_000_001);
    // Received late: sent long before the last sync, written to the database only now.
    insert(&conn, 10_000, 1_600_000_000);
    drop(conn);

    let next = read(&f, &full.cursors);
    assert!(next.incremental);
    let got = ids(&next.messages);
    for log in [FIRST_ROWS + 1, FIRST_ROWS + 2, 10_000] {
        assert!(got.contains(&format!("100-{log}")), "missing {log}");
    }
    assert!(
        next.messages.len() < FIRST_ROWS as usize,
        "read {} rows, expected only the tail",
        next.messages.len()
    );
    assert!(!got.contains("100-1"), "the oldest rows are not read again");
    assert_eq!(
        next.cursors[0].log_id, 10_000,
        "stops at the newest local row"
    );
    assert_eq!(next.cursors[0].rowid, FIRST_ROWS + 3);

    // Nothing new: still incremental, and nothing past the cursor but the margin.
    let quiet = read(&f, &next.cursors);
    assert!(quiet.incremental);
    assert_eq!(quiet.cursors, next.cursors);
}

#[test]
fn a_replaced_newest_row_still_reads_incrementally() {
    let f = fixture();
    let full = read(&f, &[]);
    let conn = open(&f.db_path, &f.key);
    // The newest row is gone and SQLite hands its rowid to the next message: that cursor no
    // longer holds, and the one below it does.
    conn.execute("DELETE FROM NTChatMessage WHERE logId = ?1", [FIRST_ROWS])
        .expect("delete");
    insert(&conn, FIRST_ROWS + 1, 1_800_000_000);
    let reused: i64 = conn
        .query_row(
            "SELECT rowid FROM NTChatMessage WHERE logId = ?1",
            [FIRST_ROWS + 1],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reused, full.cursors[0].rowid,
        "precondition: the rowid was reused"
    );
    drop(conn);
    let next = read(&f, &full.cursors);
    assert!(next.incremental);
    assert!(next.messages.len() < FIRST_ROWS as usize);
    assert!(ids(&next.messages).contains(&format!("100-{}", FIRST_ROWS + 1)));
}

#[test]
fn when_no_cursor_marks_its_message_everything_is_read() {
    let f = fixture();
    let full = read(&f, &[]);

    // A cursor naming another message at its rowid (a renumbered table) is stale.
    let mut wrong = full.cursors[0].clone();
    wrong.log_id += 1;
    let renumbered = read(&f, &[wrong]);
    assert!(!renumbered.incremental);
    assert_eq!(renumbered.messages.len(), FIRST_ROWS as usize);

    // Every row a cursor named is gone: nothing to compare, so a full read too.
    let conn = open(&f.db_path, &f.key);
    for c in &full.cursors {
        conn.execute("DELETE FROM NTChatMessage WHERE rowid = ?1", [c.rowid])
            .expect("delete");
    }
    drop(conn);
    let gone = read(&f, &full.cursors);
    assert!(!gone.incremental);
    assert_eq!(
        gone.messages.len(),
        FIRST_ROWS as usize - full.cursors.len()
    );
}

#[test]
fn a_cursor_of_another_database_is_ignored() {
    let f = fixture();
    let mut other = read(&f, &[]).cursors[0].clone();
    other.db = "0000000000000000".into();
    let out = read(&f, &[other]);
    assert!(!out.incremental);
    assert_eq!(out.messages.len(), FIRST_ROWS as usize);
}

#[test]
fn the_archive_keeps_source_cursors_per_source() {
    let temp = tempfile::tempdir().expect("temp dir");
    let archive = Archive::open(&temp.path().join("archive.sqlite3")).expect("archive");
    assert!(archive.source_cursors("macos").unwrap().is_empty());
    let cursor = SourceCursor {
        db: "00112233aabbccdd".into(),
        rowid: 88,
        chat_id: 100,
        log_id: 7,
    };
    archive
        .record_source_cursors("macos", std::slice::from_ref(&cursor))
        .unwrap();
    assert_eq!(
        archive.source_cursors("macos").unwrap(),
        vec![cursor.clone()]
    );
    assert!(archive.source_cursors("fixture").unwrap().is_empty());

    let other = SourceCursor {
        db: "ffeeddccbbaa9988".into(),
        ..cursor.clone()
    };
    archive
        .record_source_cursors("macos", std::slice::from_ref(&other))
        .unwrap();
    let moved = [
        SourceCursor {
            rowid: 91,
            log_id: 9,
            ..cursor.clone()
        },
        SourceCursor {
            rowid: 90,
            log_id: 8,
            ..cursor
        },
    ];
    archive.record_source_cursors("macos", &moved).unwrap();
    let mut stored = archive.source_cursors("macos").unwrap();
    stored.sort_by_key(|c| (c.db.clone(), -c.rowid));
    // A database's new cursors replace its old ones; another database keeps its own.
    assert_eq!(stored, [moved[0].clone(), moved[1].clone(), other]);

    // A value this build cannot read is dropped, so that database is read in full.
    archive
        .connection()
        .execute(
            "INSERT INTO sync_cursors(source_id, cursor_value)
             VALUES ('macos-rowid:ffffffffffffffff', 'not-a-cursor')",
            [],
        )
        .unwrap();
    assert_eq!(archive.source_cursors("macos").unwrap().len(), 3);
}

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/kakao/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn sync_json(data_dir: &Path, extra: &[&str]) -> serde_json::Value {
    let fixture = fixture_path("replies.jsonl");
    let mut args = vec![
        "--data-dir",
        data_dir.to_str().expect("utf8 path"),
        "sync",
        "--source",
        "fixture",
        &fixture,
        "--json",
    ];
    args.extend_from_slice(extra);
    let out = assert_cmd::Command::cargo_bin("katok")
        .expect("katok binary")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("sync json")
}

#[test]
fn the_report_says_incremental_only_when_asked() {
    let dir = tempfile::tempdir().expect("temp dir");
    let full = sync_json(dir.path(), &[]);
    assert!(
        full.get("incremental").is_none(),
        "default JSON shape is unchanged: {full}"
    );
    // A fixture has no cursors, so asking for incremental reads everything and says so.
    let asked = sync_json(dir.path(), &["--incremental"]);
    assert_eq!(asked["incremental"], serde_json::json!(false));
    assert_eq!(asked["inserted_messages"], serde_json::json!(0));
    assert_eq!(asked["total_messages"], full["total_messages"]);
}

#[test]
fn incremental_cannot_be_combined_with_a_prune() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = fixture_path("replies.jsonl");
    for prune in ["--prune-preview", "--prune-deleted"] {
        assert_cmd::Command::cargo_bin("katok")
            .expect("katok binary")
            .args([
                "--data-dir",
                dir.path().to_str().unwrap(),
                "sync",
                "--source",
                "fixture",
                &fixture,
                "--incremental",
                prune,
            ])
            .assert()
            .failure();
    }
}

#[test]
fn the_newest_time_seen_never_moves_back() {
    let temp = tempfile::tempdir().expect("temp dir");
    let archive = Archive::open(&temp.path().join("archive.sqlite3")).expect("archive");
    let message = |id: &str, at: &str| katok::types::RawMessage {
        account_hash: "a".repeat(64),
        chat_id: "100".into(),
        chat_name: "Synthetic Group".into(),
        chat_type: "group".into(),
        message_id: format!("100-{id}"),
        sender_id: "500".into(),
        sender_nickname: "Alpha".into(),
        timestamp: at.parse().unwrap(),
        text: format!("synthetic {id}"),
        message_type: "text".into(),
        reply_to_message_id: None,
    };
    let cursor = |archive: &Archive| -> String {
        archive
            .connection()
            .query_row(
                "SELECT cursor_value FROM sync_cursors WHERE source_id = ?1",
                [&"a".repeat(64)],
                |r| r.get(0),
            )
            .unwrap()
    };
    archive
        .sync_messages(&[message("2", "2026-01-02T00:00:00Z")])
        .unwrap();
    let newest = cursor(&archive);
    // An incremental read can carry only a late, older message.
    archive
        .sync_messages(&[message("1", "2026-01-01T00:00:00Z")])
        .unwrap();
    assert_eq!(cursor(&archive), newest);
    archive
        .sync_messages(&[message("3", "2026-01-03T00:00:00Z")])
        .unwrap();
    assert!(cursor(&archive) > newest);
}
