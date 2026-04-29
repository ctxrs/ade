use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

pub(super) async fn apply_provider_launch_overrides(
    provider_id: &str,
    workdir: &Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<()> {
    apply_provider_mcp_command_overrides(provider_id, provider_env);

    if provider_id == "openhands" {
        apply_openhands_launch_overrides(workdir, provider_env).await?;
        return Ok(());
    }

    strip_unused_daemon_auth_from_provider_env(provider_env);
    Ok(())
}

pub(super) fn apply_provider_mcp_command_overrides(
    provider_id: &str,
    provider_env: &mut HashMap<String, String>,
) {
    if matches!(provider_id, "fake" | "broken" | "opencode" | "kimi") {
        // These providers currently behave truthfully without the daemon MCP runtime.
        provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    }
}

fn strip_unused_daemon_auth_from_provider_env(provider_env: &mut HashMap<String, String>) {
    let mcp_disabled = provider_env
        .get("CTX_MCP_DISABLED")
        .and_then(|value| ctx_core::boolish::parse_boolish(value))
        .unwrap_or(false);
    if !mcp_disabled {
        return;
    }
    provider_env.remove("CTX_AUTH_TOKEN");
    provider_env.remove("CTX_MCP_TOKEN");
}

async fn apply_openhands_launch_overrides(
    workdir: &Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<()> {
    let Some(data_root) = provider_env
        .get("CTX_DATA_ROOT")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };
    let Some(session_id) = provider_env
        .get("CTX_SESSION_ID")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };

    let alias = ensure_openhands_workdir_alias(Path::new(data_root), session_id, workdir).await?;
    provider_env.insert(
        "OPENHANDS_WORK_DIR".to_string(),
        alias.to_string_lossy().to_string(),
    );
    Ok(())
}

async fn ensure_openhands_workdir_alias(
    data_root: &Path,
    session_id: &str,
    workdir: &Path,
) -> Result<PathBuf> {
    let alias_root = data_root
        .join("providers")
        .join("openhands")
        .join("workdir-aliases");
    tokio::fs::create_dir_all(&alias_root).await?;
    let alias = alias_root.join(session_id);
    reset_existing_alias(&alias, workdir).await?;
    match create_dir_symlink(workdir, &alias).await {
        Ok(()) => {}
        // Windows directory symlinks often require Developer Mode or elevated privileges.
        // Falling back to the real workdir keeps OpenHands usable when the short alias cannot
        // be created, even if the path is longer than ideal.
        Err(err) if should_fallback_to_direct_openhands_workdir(&err) => {
            return Ok(workdir.to_path_buf());
        }
        Err(err) => return Err(err),
    }
    Ok(alias)
}

fn should_fallback_to_direct_openhands_workdir(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::raw_os_error)
            == Some(1314)
    })
}

async fn reset_existing_alias(alias: &Path, target: &Path) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(alias).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("stat {}", alias.display())),
    };

    if metadata.file_type().is_symlink() {
        let existing_target = tokio::fs::read_link(alias)
            .await
            .with_context(|| format!("read link {}", alias.display()))?;
        if existing_target == target {
            return Ok(());
        }
        tokio::fs::remove_file(alias)
            .await
            .with_context(|| format!("remove stale symlink {}", alias.display()))?;
        return Ok(());
    }

    if metadata.is_dir() {
        tokio::fs::remove_dir_all(alias)
            .await
            .with_context(|| format!("remove stale directory {}", alias.display()))?;
    } else {
        tokio::fs::remove_file(alias)
            .await
            .with_context(|| format!("remove stale file {}", alias.display()))?;
    }
    Ok(())
}

async fn create_dir_symlink(source: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        let source_for_link = source.clone();
        let target_for_link = target.clone();
        tokio::task::spawn_blocking(move || symlink(source_for_link, target_for_link))
            .await
            .map_err(|err| anyhow!("joining symlink task: {err}"))?
            .with_context(|| format!("symlink {} -> {}", target.display(), source.display()))?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        let source_for_link = source.clone();
        let target_for_link = target.clone();
        tokio::task::spawn_blocking(move || symlink_dir(source_for_link, target_for_link))
            .await
            .map_err(|err| anyhow!("joining symlink task: {err}"))?
            .with_context(|| format!("symlink {} -> {}", target.display(), source.display()))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "directory symlinks are not supported on this platform"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::provider_ids::CODEX_PROVIDER_ID;

    #[tokio::test]
    async fn opencode_launch_overrides_disable_mcp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::from([
            ("CTX_AUTH_TOKEN".to_string(), "daemon-secret".to_string()),
            ("CTX_MCP_TOKEN".to_string(), "mcp-secret".to_string()),
        ]);

        apply_provider_launch_overrides("opencode", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
        assert!(!env.contains_key("CTX_AUTH_TOKEN"));
        assert!(!env.contains_key("CTX_MCP_TOKEN"));
        assert!(!env.contains_key("ACP_CWD"));
    }

    #[tokio::test]
    async fn kimi_launch_overrides_disable_mcp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::from([
            ("CTX_AUTH_TOKEN".to_string(), "daemon-secret".to_string()),
            ("CTX_MCP_TOKEN".to_string(), "mcp-secret".to_string()),
        ]);

        apply_provider_launch_overrides("kimi", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
        assert!(!env.contains_key("CTX_AUTH_TOKEN"));
        assert!(!env.contains_key("CTX_MCP_TOKEN"));
        assert!(!env.contains_key("ACP_CWD"));
    }

    #[test]
    fn fake_provider_disables_mcp_before_command_resolution() {
        let mut env = HashMap::new();

        apply_provider_mcp_command_overrides("fake", &mut env);

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
    }

    #[test]
    fn broken_test_provider_disables_mcp_before_command_resolution() {
        let mut env = HashMap::new();

        apply_provider_mcp_command_overrides("broken", &mut env);

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
    }

    #[tokio::test]
    async fn unrelated_launch_overrides_leave_env_unchanged() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::from([("CTX_MCP_DISABLED".to_string(), "0".to_string())]);

        apply_provider_launch_overrides(CODEX_PROVIDER_ID, temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("0"));
        assert!(!env.contains_key("ACP_CWD"));
    }

    #[tokio::test]
    async fn predisabled_mcp_strips_unused_daemon_tokens() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::from([
            ("CTX_MCP_DISABLED".to_string(), "1".to_string()),
            ("CTX_AUTH_TOKEN".to_string(), "daemon-secret".to_string()),
            ("CTX_MCP_TOKEN".to_string(), "mcp-secret".to_string()),
        ]);

        apply_provider_launch_overrides("codex", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
        assert!(!env.contains_key("CTX_AUTH_TOKEN"));
        assert!(!env.contains_key("CTX_MCP_TOKEN"));
    }

    #[tokio::test]
    async fn openhands_launch_overrides_set_short_workdir_alias() {
        let data_root = tempfile::tempdir().expect("data_root");
        let workdir_parent = tempfile::tempdir().expect("workdir_parent");
        let workdir = workdir_parent.path().join("nested").join("worktree");
        tokio::fs::create_dir_all(&workdir)
            .await
            .expect("create workdir");
        let mut env = HashMap::from([
            (
                "CTX_DATA_ROOT".to_string(),
                data_root.path().to_string_lossy().to_string(),
            ),
            ("CTX_SESSION_ID".to_string(), "session-123".to_string()),
        ]);

        apply_provider_launch_overrides("openhands", &workdir, &mut env)
            .await
            .expect("apply overrides");

        let alias = env
            .get("OPENHANDS_WORK_DIR")
            .map(PathBuf::from)
            .expect("OPENHANDS_WORK_DIR");
        assert_eq!(
            alias,
            data_root
                .path()
                .join("providers")
                .join("openhands")
                .join("workdir-aliases")
                .join("session-123")
        );
        let link_target = tokio::fs::read_link(&alias).await.expect("read alias");
        assert_eq!(link_target, workdir);
        assert!(!env.contains_key("CTX_MCP_DISABLED"));
    }

    #[test]
    fn openhands_workdir_fallback_detects_windows_symlink_privilege_errors() {
        let privilege_err = Err::<(), _>(std::io::Error::from_raw_os_error(1314))
            .with_context(|| "symlink failed")
            .expect_err("expected privilege error");
        let unrelated_err = Err::<(), _>(std::io::Error::from_raw_os_error(5))
            .with_context(|| "symlink failed")
            .expect_err("expected unrelated error");

        assert!(should_fallback_to_direct_openhands_workdir(&privilege_err));
        assert!(!should_fallback_to_direct_openhands_workdir(&unrelated_err));
    }
}
