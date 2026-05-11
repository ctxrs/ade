use super::*;

#[tokio::test]
async fn resolve_claude_login_runtime_requires_managed_or_configured_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();

    let err = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect_err("missing managed/configured claude login command should fail");
    assert!(err
        .to_string()
        .contains("runtime_command_missing: provider=claude-cli"));
    assert!(err
        .to_string()
        .contains("host PATH lookup is not supported"));
}

#[tokio::test]
async fn resolve_claude_login_runtime_uses_configured_runtime_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();
    let runtime_path = data_root.join("claude-cli-mock.sh");
    std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(&data_root)
        .await
        .expect("load config for runtime resolution test");
    cfg.providers.insert(
        "claude-cli".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["--shim".to_string()],
            dependencies: vec!["dep-node".to_string()],
            managed: None,
        },
    );
    installer::save_agent_server_config(&data_root, &cfg)
        .await
        .expect("save config for runtime resolution test");

    let resolved = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect("resolve runtime from config");
    assert!(resolved.command_abs_path.contains("claude-cli-mock.sh"));
    assert_eq!(resolved.args, vec!["--shim".to_string()]);
    assert_eq!(resolved.dependencies, vec!["dep-node".to_string()]);
    assert_eq!(
        resolved.source,
        installer::ProviderRuntimeCommandSource::UserOverride
    );
}

#[tokio::test]
async fn resolve_claude_login_runtime_prefers_configured_runtime_command_when_login_command_missing(
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();
    let runtime_path = data_root.join("claude-cli-runtime-mock.sh");
    std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(&data_root)
        .await
        .expect("load config for runtime resolution test");
    cfg.providers.insert(
        "claude-cli".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["cli.js".to_string()],
            dependencies: Vec::new(),
            managed: None,
        },
    );
    installer::save_agent_server_config(&data_root, &cfg)
        .await
        .expect("save config for runtime resolution test");

    let resolved = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect("resolve runtime from provider command");
    assert!(resolved
        .command_abs_path
        .contains("claude-cli-runtime-mock.sh"));
    assert_eq!(resolved.args, vec!["cli.js".to_string()]);
}
