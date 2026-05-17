use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

const CODEX_CONTINUITY_RUNTIME_LOCK_FILE: &str = ".ctx-continuity-runtime.lock";

pub(super) struct CodexRuntimeLocks {
    _continuity: File,
}

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

pub(super) fn acquire_codex_runtime_locks() -> Result<Option<CodexRuntimeLocks>> {
    let Some(codex_home) = codex_home_from_env() else {
        return Ok(None);
    };
    std::fs::create_dir_all(&codex_home)
        .with_context(|| format!("creating Codex home at {}", codex_home.display()))?;
    let continuity = acquire_codex_continuity_runtime_lock(&codex_home)?;
    if codex_home_has_refresh_token(&codex_home)? {
        anyhow::bail!(
            "Codex runtime home {} contains tokens.refresh_token. ctx session runtimes must use access-token-only OAuth auth so refresh authority stays in the account broker.",
            codex_home.display()
        );
    }
    Ok(Some(CodexRuntimeLocks {
        _continuity: continuity,
    }))
}

fn acquire_codex_continuity_runtime_lock(codex_home: &Path) -> Result<File> {
    let lock_path = codex_home.join(CODEX_CONTINUITY_RUNTIME_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| {
            format!(
                "opening Codex continuity runtime lock {}",
                lock_path.display()
            )
        })?;
    match fs2::FileExt::try_lock_shared(&file) {
        Ok(()) => Ok(file),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            anyhow::bail!(
                "Codex home {} is undergoing continuity migration. Retry after launch preparation finishes.",
                codex_home.display()
            )
        }
        Err(err) => Err(err).with_context(|| {
            format!(
                "locking Codex continuity runtime lock {}",
                lock_path.display()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::acquire_codex_runtime_locks;

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
    fn refresh_capable_home_is_rejected() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let tempdir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tempdir.path().join("auth.json"),
            r#"{"tokens":{"access_token":"access","refresh_token":"refresh"}}"#,
        )
        .expect("write auth");
        let _guard = EnvGuard::set("CODEX_HOME", tempdir.path().to_string_lossy().as_ref());

        let error = match acquire_codex_runtime_locks() {
            Ok(_) => panic!("refresh home should be rejected"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("access-token-only OAuth auth"),
            "unexpected error: {error:#}"
        );
        assert!(tempdir.path().join(".ctx-continuity-runtime.lock").exists());
        assert!(!tempdir.path().join(".ctx-refresh-token.lock").exists());
    }

    #[test]
    fn runtime_lock_is_taken_for_api_key_home_without_oauth_authority_lock() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let tempdir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tempdir.path().join("auth.json"),
            r#"{"OPENAI_API_KEY":"key"}"#,
        )
        .expect("write auth");
        let _guard = EnvGuard::set("CODEX_HOME", tempdir.path().to_string_lossy().as_ref());

        let first = acquire_codex_runtime_locks()
            .expect("api key home runtime lock")
            .expect("runtime locks");
        assert!(tempdir.path().join(".ctx-continuity-runtime.lock").exists());
        assert!(!tempdir.path().join(".ctx-refresh-token.lock").exists());
        drop(first);
    }

    #[test]
    fn runtime_lock_accepts_access_token_only_oauth_home() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let tempdir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tempdir.path().join("auth.json"),
            r#"{"tokens":{"access_token":"access","account_id":"acct"}}"#,
        )
        .expect("write auth");
        let _guard = EnvGuard::set("CODEX_HOME", tempdir.path().to_string_lossy().as_ref());

        let first = acquire_codex_runtime_locks()
            .expect("access-only home runtime lock")
            .expect("runtime locks");
        assert!(tempdir.path().join(".ctx-continuity-runtime.lock").exists());
        assert!(!tempdir.path().join(".ctx-refresh-token.lock").exists());
        drop(first);
    }
}
