//! KATOK_ARCHIVE_KEY: the archive and vector store are SQLCipher-encrypted, a plaintext archive
//! is migrated in place, and the key must be present to read an encrypted archive.

use katok::archive::{encrypt_in_place, is_plaintext_sqlite, Archive};
use std::path::Path;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

fn with_key<T>(key: Option<&str>, work: impl FnOnce() -> T) -> T {
    // Tests in this file run serially (see the lock below) because the key lives in the env.
    match key {
        Some(key) => std::env::set_var(katok::paths::ARCHIVE_KEY_ENV, key),
        None => std::env::remove_var(katok::paths::ARCHIVE_KEY_ENV),
    }
    let result = work();
    std::env::remove_var(katok::paths::ARCHIVE_KEY_ENV);
    result
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn header_is_plaintext(path: &Path) -> bool {
    is_plaintext_sqlite(path).unwrap()
}

#[test]
fn keyed_archive_is_not_plaintext_and_needs_the_key() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.sqlite3");
    with_key(Some(KEY), || {
        let archive = Archive::open(&path).unwrap();
        archive
            .connection()
            .execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (1);")
            .unwrap();
    });
    assert!(
        !header_is_plaintext(&path),
        "encrypted file must not carry the SQLite header"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let without = with_key(None, || Archive::open(&path).map(|_| ()));
    assert!(
        matches!(without, Err(katok::Error::ArchiveKeyRequired(_))),
        "{without:?}"
    );
    let wrong = with_key(Some(OTHER), || Archive::open(&path).map(|_| ()));
    assert!(wrong.is_err());
    with_key(Some(KEY), || {
        let archive = Archive::open(&path).unwrap();
        let n: i64 = archive
            .connection()
            .query_row("SELECT count(*) FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    });
}

#[test]
fn plaintext_archive_is_migrated_in_place_when_a_key_appears() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.sqlite3");
    with_key(None, || {
        let archive = Archive::open(&path).unwrap();
        archive
            .connection()
            .execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (7);")
            .unwrap();
    });
    assert!(header_is_plaintext(&path));
    std::fs::write(dir.path().join("archive.sqlite3-wal"), b"stale").unwrap();
    with_key(Some(KEY), || {
        let archive = Archive::open(&path).unwrap();
        let n: i64 = archive
            .connection()
            .query_row("SELECT x FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 7);
    });
    assert!(!header_is_plaintext(&path));
    assert!(!dir.path().join("archive.sqlite3-wal").exists());
    assert!(!dir.path().join("archive.sqlite3.encrypting").exists());
}

#[test]
fn a_non_hex_key_like_the_demo_harness_uses_works_as_a_passphrase() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.sqlite3");
    let demo = "demo0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";
    with_key(Some(demo), || {
        let archive = Archive::open(&path).unwrap();
        archive
            .connection()
            .execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (3);")
            .unwrap();
    });
    assert!(!header_is_plaintext(&path));
    with_key(Some(demo), || {
        let archive = Archive::open(&path).unwrap();
        let n: i64 = archive
            .connection()
            .query_row("SELECT x FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
    });
    assert!(with_key(Some(KEY), || Archive::open(&path).map(|_| ())).is_err());
}

#[test]
fn concurrent_openers_convert_a_plaintext_archive_once() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.sqlite3");
    with_key(None, || {
        let archive = Archive::open(&path).unwrap();
        archive
            .connection()
            .execute_batch("CREATE TABLE probe(x); INSERT INTO probe VALUES (9);")
            .unwrap();
    });
    std::env::set_var(katok::paths::ARCHIVE_KEY_ENV, KEY);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || {
                let archive = Archive::open(&path).unwrap();
                let n: i64 = archive
                    .connection()
                    .query_row("SELECT x FROM probe", [], |r| r.get(0))
                    .unwrap();
                n
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), 9);
    }
    std::env::remove_var(katok::paths::ARCHIVE_KEY_ENV);
    assert!(!header_is_plaintext(&path));
    assert!(!dir.path().join("archive.sqlite3.encrypting").exists());
}

#[test]
fn malformed_key_is_rejected_before_touching_the_file() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.sqlite3");
    let result = with_key(Some("short"), || Archive::open(&path).map(|_| ()));
    assert!(matches!(result, Err(katok::Error::InvalidArchiveKey)));
    assert!(!path.exists());
}

#[test]
fn encrypt_in_place_is_idempotent_on_an_already_encrypted_file() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.sqlite3");
    with_key(Some(KEY), || {
        Archive::open(&path).unwrap();
    });
    // Not plaintext, so open() never calls encrypt_in_place again; calling it directly on an
    // encrypted file fails cleanly instead of corrupting the archive.
    assert!(encrypt_in_place(&path, KEY).is_err());
    with_key(Some(KEY), || {
        Archive::open(&path).unwrap();
    });
}
