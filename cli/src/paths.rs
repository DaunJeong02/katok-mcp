use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Environment variable carrying the SQLCipher raw key (64 hex chars) for the archive and the
/// semantic vector store. Set by talk-2-build from its Keychain entry; never written to disk.
pub const ARCHIVE_KEY_ENV: &str = "KATOK_ARCHIVE_KEY";

/// Minimum key length accepted; talk-2-build always passes 64 characters.
pub const MIN_KEY_LEN: usize = 16;

/// The archive key from the environment, validated. `None` keeps the plaintext behaviour of
/// upstream katok for people who run the CLI by hand. The value is handed to SQLCipher in the
/// same `x'…'` form talk-2-build uses for its own database: 64 hex characters act as a raw key,
/// anything else (the demo harness key) is treated by SQLCipher as a passphrase.
pub fn archive_key() -> Result<Option<String>> {
    match std::env::var(ARCHIVE_KEY_ENV) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => {
            let valid = value.len() >= MIN_KEY_LEN
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if valid {
                Ok(Some(value))
            } else {
                Err(Error::InvalidArchiveKey)
            }
        }
        Err(_) => Ok(None),
    }
}

/// The key literal for `PRAGMA key` and `ATTACH … KEY`. The character set is restricted by
/// `archive_key`, so no quoting issues.
pub fn key_literal(key: &str) -> String {
    format!("\"x'{key}'\"")
}

pub fn key_pragma(key: &str) -> String {
    format!("PRAGMA key = {};", key_literal(key))
}

/// Exclusive advisory lock on `<archive>.lock`, held while a plaintext archive is converted so
/// two katok processes started together (the app polls and lists rooms at once) do not both
/// rewrite the file.
pub struct MigrationLock {
    _file: std::fs::File,
}

impl MigrationLock {
    pub fn acquire(archive: &Path) -> Result<Self> {
        let mut lock_path = archive.as_os_str().to_owned();
        lock_path.push(".lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(Error::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: flock on a valid, open file descriptor owned by `file`.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                return Err(Error::Io(std::io::Error::last_os_error()));
            }
        }
        Ok(Self { _file: file })
    }
}

pub fn ensure_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).map_err(Error::Io)?.permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(Error::Io)?;
    }
    Ok(())
}

pub fn default_data_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or(Error::HomeDirUnavailable)?;
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("katok"))
}

pub fn ensure_private_dir(path: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(Error::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).map_err(Error::Io)?.permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).map_err(Error::Io)?;
    }
    Ok(())
}
