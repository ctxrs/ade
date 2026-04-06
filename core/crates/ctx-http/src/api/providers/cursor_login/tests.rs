use super::*;

struct TestEnvVar {
    key: &'static str,
    prev: Option<String>,
}

impl TestEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }
}

impl Drop for TestEnvVar {
    fn drop(&mut self) {
        unsafe {
            if let Some(prev) = self.prev.as_deref() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn write_mock_cursor_command(dir: &StdPath, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write mock cursor command");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)
            .expect("mock cursor metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("set mock cursor permissions");
    }
    path
}

#[cfg(unix)]
fn unix_mode(path: &StdPath) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn resolve_cursor_login_runtime_requires_managed_or_configured_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let _host_cursor = write_mock_cursor_command(temp.path(), "cursor-agent");
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let combined_path = if existing_path.is_empty() {
        temp.path().to_string_lossy().to_string()
    } else {
        format!("{}:{}", temp.path().to_string_lossy(), existing_path)
    };
    let _path_guard = TestEnvVar::set("PATH", &combined_path);

    let err = resolve_cursor_login_runtime_from_config(temp.path())
        .await
        .expect_err("host PATH cursor-agent should not be accepted");
    assert!(err
        .to_string()
        .contains("runtime_command_missing: provider=cursor-login"));
    assert!(err
        .to_string()
        .contains("host PATH lookup is not supported"));

    let cfg = installer::load_agent_server_config(temp.path())
        .await
        .expect("load config");
    assert!(
        !cfg.provider_login_commands.contains_key("cursor"),
        "host discovery must not persist a cursor login command"
    );
}

#[tokio::test]
async fn resolve_cursor_login_runtime_accepts_configured_login_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_path = write_mock_cursor_command(temp.path(), "cursor-agent");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(temp.path())
        .await
        .expect("load config");
    cfg.provider_login_commands.insert(
        "cursor".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["--ignored".to_string()],
            dependencies: vec!["dep".to_string()],
            managed: None,
        },
    );
    installer::save_agent_server_config(temp.path(), &cfg)
        .await
        .expect("save config");

    let resolved = resolve_cursor_login_runtime_from_config(temp.path())
        .await
        .expect("resolve configured login command");
    let expected = std::fs::canonicalize(&runtime_path).unwrap_or(runtime_path);
    assert_eq!(
        resolved.command_abs_path,
        expected.to_string_lossy().to_string()
    );
    assert_eq!(resolved.args, vec!["--ignored".to_string()]);
    assert_eq!(resolved.dependencies, vec!["dep".to_string()]);
    assert_eq!(
        resolved.source,
        installer::ProviderRuntimeCommandSource::UserOverride
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cursor_login_session_dirs_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let login_home = cursor_login_home(temp.path(), "login-id");
    let workdir = login_home.join("workspace");

    std::fs::create_dir_all(&workdir).expect("create workdir");
    std::fs::set_permissions(&login_home, std::fs::Permissions::from_mode(0o755))
        .expect("set login_home perms");
    std::fs::set_permissions(&workdir, std::fs::Permissions::from_mode(0o755))
        .expect("set workdir perms");

    ensure_private_dir(&login_home)
        .await
        .expect("secure login_home");
    ensure_private_dir(&workdir).await.expect("secure workdir");

    assert_eq!(unix_mode(&login_home), 0o700);
    assert_eq!(unix_mode(&workdir), 0o700);
}

#[cfg(unix)]
#[tokio::test]
async fn cursor_login_capture_file_is_repermissioned_to_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let login_home = cursor_login_home(temp.path(), "login-id");
    let hook_path = login_home.join("capture-hook.cjs");
    let capture_path = login_home.join("captured_tokens.jsonl");

    std::fs::create_dir_all(&login_home).expect("create login_home");
    std::fs::set_permissions(&login_home, std::fs::Permissions::from_mode(0o755))
        .expect("set login_home perms");
    std::fs::write(&capture_path, b"stale").expect("write capture file");
    std::fs::set_permissions(&capture_path, std::fs::Permissions::from_mode(0o644))
        .expect("set capture perms");

    write_cursor_capture_hook(&hook_path)
        .await
        .expect("write capture hook");
    initialize_cursor_capture_file(&capture_path)
        .await
        .expect("initialize capture file");

    assert_eq!(unix_mode(&login_home), 0o700);
    assert_eq!(unix_mode(&hook_path), 0o600);
    assert_eq!(unix_mode(&capture_path), 0o600);
    assert_eq!(
        std::fs::read(&capture_path).expect("read capture file"),
        b"",
        "capture file should be reset before the hook appends tokens"
    );
}

#[tokio::test]
async fn resolve_cursor_login_runtime_accepts_configured_runtime_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_path = write_mock_cursor_command(temp.path(), "cursor-agent");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(temp.path())
        .await
        .expect("load config");
    cfg.providers.insert(
        "cursor".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["cli.js".to_string()],
            dependencies: Vec::new(),
            managed: None,
        },
    );
    installer::save_agent_server_config(temp.path(), &cfg)
        .await
        .expect("save config");

    let resolved = resolve_cursor_login_runtime_from_config(temp.path())
        .await
        .expect("resolve configured runtime command");
    let expected = std::fs::canonicalize(&runtime_path).unwrap_or(runtime_path);
    assert_eq!(
        resolved.command_abs_path,
        expected.to_string_lossy().to_string()
    );
    assert_eq!(resolved.args, vec!["cli.js".to_string()]);
}
