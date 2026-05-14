use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::Value;

const CODEX_OAUTH_AUTHORITY_LOCK_FILE: &str = ".ctx-refresh-token.lock";

fn codex_home_from_env() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn codex_home_has_refresh_token(home: &Path) -> Result<bool> {
    let auth_path = home.join("auth.json");
    let payload = match std::fs::read_to_string(&auth_path) {
        Ok(payload) => payload,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading Codex auth at {}", auth_path.display()));
        }
    };
    let auth: Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid Codex auth JSON at {}", auth_path.display()))?;
    Ok(auth
        .get("tokens")
        .and_then(|value| value.as_object())
        .and_then(|tokens| tokens.get("refresh_token"))
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty()))
}

pub(super) fn acquire_codex_oauth_authority_lock() -> Result<Option<File>> {
    let Some(codex_home) = codex_home_from_env() else {
        return Ok(None);
    };
    if !codex_home_has_refresh_token(&codex_home)? {
        return Ok(None);
    }
    std::fs::create_dir_all(&codex_home)
        .with_context(|| format!("creating Codex home at {}", codex_home.display()))?;
    let lock_path = codex_home.join(CODEX_OAUTH_AUTHORITY_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening Codex OAuth authority lock {}", lock_path.display()))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(file)),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            anyhow::bail!(
                "Another Codex session is already using this signed-in account. ctx serializes Codex OAuth sessions to protect rotating refresh tokens; wait for the active session to finish or sign in with a separate Codex account."
            )
        }
        Err(err) => Err(err)
            .with_context(|| format!("locking Codex OAuth authority {}", lock_path.display())),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::acquire_codex_oauth_authority_lock;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.as_deref() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn oauth_authority_lock_is_taken_for_refresh_capable_home() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let tempdir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tempdir.path().join("auth.json"),
            r#"{"tokens":{"access_token":"access","refresh_token":"refresh"}}"#,
        )
        .expect("write auth");
        let _guard = EnvGuard::set("CODEX_HOME", tempdir.path().to_string_lossy().as_ref());

        let first = acquire_codex_oauth_authority_lock()
            .expect("first lock")
            .expect("oauth lock");
        assert!(tempdir.path().join(".ctx-refresh-token.lock").exists());
        drop(first);
    }

    #[test]
    fn oauth_authority_lock_skips_api_key_home() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let tempdir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tempdir.path().join("auth.json"),
            r#"{"OPENAI_API_KEY":"key"}"#,
        )
        .expect("write auth");
        let _guard = EnvGuard::set("CODEX_HOME", tempdir.path().to_string_lossy().as_ref());

        assert!(acquire_codex_oauth_authority_lock()
            .expect("api key homes do not need OAuth lock")
            .is_none());
    }
}
