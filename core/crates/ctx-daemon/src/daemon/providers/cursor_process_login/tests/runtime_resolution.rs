use super::fixtures::{write_mock_cursor_command, TestEnvVar};
use super::*;
use crate::test_support::TestDaemon;

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
        !cfg.provider_login_executables.contains_key("cursor"),
        "host discovery must not persist a cursor login executable"
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
    cfg.provider_login_executables.insert(
        "cursor".to_string(),
        installer::ProviderLoginExecutable {
            executable_path: runtime_path_str,
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
    assert!(resolved.args.is_empty());
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

#[tokio::test]
async fn cursor_login_start_rejects_missing_runtime_without_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");

    let err = daemon
        .handle()
        .providers()
        .start_cursor_login_for_route(CursorLoginStartRouteRequest::default())
        .await
        .expect_err("missing runtime should fail before session creation");

    assert_eq!(err.kind(), CursorLoginRouteErrorKind::BadRequest);
    assert!(err
        .message()
        .contains("runtime_command_missing: provider=cursor-login"));
    assert!(daemon.provider_login_session_caches_empty().await);
}

#[tokio::test]
async fn cursor_login_start_rejects_config_parse_error_without_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cfg_path = installer::agent_server_config_path(temp.path());
    tokio::fs::create_dir_all(cfg_path.parent().expect("config parent"))
        .await
        .expect("create config parent");
    tokio::fs::write(&cfg_path, b"{not-json")
        .await
        .expect("write malformed config");
    let daemon =
        TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
            .await
            .expect("test daemon");

    let err = daemon
        .handle()
        .providers()
        .start_cursor_login_for_route(CursorLoginStartRouteRequest::default())
        .await
        .expect_err("config parse failure should fail before session creation");

    assert_eq!(err.kind(), CursorLoginRouteErrorKind::Internal);
    assert!(err.message().contains("agent server config"));
    assert!(daemon.provider_login_session_caches_empty().await);
}
