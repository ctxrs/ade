use super::fixtures::{write_mock_cursor_command, TestEnvVar};
use super::*;

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
