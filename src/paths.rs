use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Environment variable carrying the SQLCipher raw key (64 hex chars) for the archive and the
/// semantic vector store. Set by talk-2-build from its Keychain entry; never written to disk.
pub const ARCHIVE_KEY_ENV: &str = "KATOK_ARCHIVE_KEY";

/// The archive key from the environment, validated. `None` keeps the plaintext behaviour of
/// upstream katok for people who run the CLI by hand.
pub fn archive_key() -> Result<Option<String>> {
    match std::env::var(ARCHIVE_KEY_ENV) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => {
            let valid = value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
            if valid {
                Ok(Some(value.to_ascii_lowercase()))
            } else {
                Err(Error::InvalidArchiveKey)
            }
        }
        Err(_) => Ok(None),
    }
}

/// `PRAGMA key` statement for a raw hex key. Hex only, so no quoting issues.
pub fn key_pragma(key: &str) -> String {
    format!("PRAGMA key = \"x'{key}'\";")
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
