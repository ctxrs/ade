use std::collections::HashMap;

use tempfile::tempdir;

use super::*;

#[test]
fn normalizes_qwen_command_with_openai_auth_type() {
    let temp = tempdir().unwrap();
    let input = installer::AgentServerCommand {
        command: "/tmp/qwen".to_string(),
        args: vec!["--experimental-acp".to_string()],
        dependencies: Vec::new(),
        managed: None,
    };
    let normalized =
        normalize_acp_provider_command(temp.path(), "qwen", input).expect("normalized qwen");
    assert_eq!(
        normalized.args,
        vec![
            "--experimental-acp".to_string(),
            "--auth-type".to_string(),
            "openai".to_string(),
        ]
    );
}

#[test]
fn normalizes_goose_command_with_acp_and_developer_builtin() {
    let temp = tempdir().unwrap();
    let input = installer::AgentServerCommand {
        command: "/tmp/goose".to_string(),
        args: Vec::new(),
        dependencies: Vec::new(),
        managed: None,
    };
    let normalized =
        normalize_acp_provider_command(temp.path(), "goose", input).expect("normalized goose");
    assert_eq!(
        normalized.args,
        vec![
            "acp".to_string(),
            "--with-builtin".to_string(),
            "developer".to_string(),
        ]
    );
}

#[test]
fn preserves_existing_goose_developer_builtin() {
    let temp = tempdir().unwrap();
    let input = installer::AgentServerCommand {
        command: "/tmp/goose".to_string(),
        args: vec![
            "acp".to_string(),
            "--with-builtin".to_string(),
            "developer,computercontroller".to_string(),
        ],
        dependencies: Vec::new(),
        managed: None,
    };
    let normalized =
        normalize_acp_provider_command(temp.path(), "goose", input).expect("normalized goose");
    assert_eq!(
        normalized.args,
        vec![
            "acp".to_string(),
            "--with-builtin".to_string(),
            "developer,computercontroller".to_string(),
        ]
    );
}

#[test]
fn classifies_upstream_openhands_runtime_contract() {
    let cmd = installer::AgentServerCommand {
        command: "/tmp/openhands".to_string(),
        args: vec!["acp".to_string(), "--override-with-envs".to_string()],
        dependencies: Vec::new(),
        managed: None,
    };

    assert_eq!(
        openhands_runtime_contract_for_command(&cmd),
        OpenHandsRuntimeContract::UpstreamAcp
    );
}

#[test]
fn classifies_legacy_openhands_shim_runtime_contract() {
    let cmd = installer::AgentServerCommand {
        command: "/tmp/openhands-acp.js".to_string(),
        args: Vec::new(),
        dependencies: Vec::new(),
        managed: None,
    };

    assert_eq!(
        openhands_runtime_contract_for_command(&cmd),
        OpenHandsRuntimeContract::ShimAcp
    );
}

#[tokio::test]
async fn openhands_bridge_adapter_inspect_surfaces_runtime_contract_details() {
    let temp = tempdir().unwrap();
    let bridge_cmd = temp.path().join("acp-crp-bridge");
    std::fs::write(&bridge_cmd, b"bridge").unwrap();

    let adapter = acp_bridge_adapter(
        "openhands",
        &installer::AgentServerCommand {
            command: bridge_cmd.to_string_lossy().to_string(),
            args: vec!["--stdio".to_string()],
            dependencies: Vec::new(),
            managed: None,
        },
        installer::AgentServerCommand {
            command: "/tmp/openhands-acp.js".to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        },
    );

    let status = adapter.inspect().await.expect("inspect status");
    assert_eq!(
        status
            .details
            .get("openhands_runtime_contract")
            .map(String::as_str),
        Some("shim_acp")
    );
    assert_eq!(
        status
            .details
            .get("openhands_real_runtime")
            .map(String::as_str),
        Some("false")
    );
    assert_eq!(
        status
            .details
            .get("openhands_runtime_contract_note")
            .map(String::as_str),
        Some("runtime command still points at the legacy `openhands-acp` shim")
    );
}

fn create_gemini_runtime_layout(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let node_bin = root
        .join("bundle")
        .join("runtimes")
        .join("node")
        .join("bin")
        .join("node");
    let cli_entry = root
        .join("bundle")
        .join("providers")
        .join("gemini")
        .join("node_modules")
        .join("@google")
        .join("gemini-cli")
        .join("dist")
        .join("index.js");
    let core_entry = root
        .join("bundle")
        .join("providers")
        .join("gemini")
        .join("node_modules")
        .join("@google")
        .join("gemini-cli-core")
        .join("dist")
        .join("index.js");
    std::fs::create_dir_all(node_bin.parent().unwrap()).unwrap();
    std::fs::create_dir_all(cli_entry.parent().unwrap()).unwrap();
    std::fs::create_dir_all(core_entry.parent().unwrap()).unwrap();
    std::fs::write(&node_bin, b"node").unwrap();
    std::fs::write(&cli_entry, b"cli").unwrap();
    std::fs::write(&core_entry, b"core").unwrap();
    (node_bin, cli_entry, core_entry)
}

#[test]
fn wraps_explicit_gemini_node_entrypoint_for_acp() {
    let temp = tempdir().unwrap();
    let data_root = temp.path().join("data");
    let (node_bin, cli_entry, core_entry) = create_gemini_runtime_layout(temp.path());

    let input = installer::AgentServerCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec![
            cli_entry.to_string_lossy().to_string(),
            "--experimental-acp".to_string(),
        ],
        dependencies: Vec::new(),
        managed: None,
    };
    let wrapped =
        normalize_acp_provider_command(&data_root, "gemini", input).expect("wrapped gemini");

    assert_eq!(wrapped.command, node_bin.to_string_lossy().to_string());
    assert_eq!(
        wrapped.args.get(1).map(String::as_str),
        Some("--experimental-acp")
    );
    let wrapper_arg = wrapped.args.first().expect("wrapper arg");
    assert!(wrapper_arg.ends_with("gemini-acp-wrapper.mjs"));
    let wrapper_path = PathBuf::from(wrapper_arg);
    assert!(wrapper_path.exists());
    let wrapper_body = std::fs::read_to_string(wrapper_path).unwrap();
    assert!(wrapper_body.contains("GEMINI_CLI_NO_RELAUNCH"));
    assert!(wrapper_body.contains(cli_entry.to_string_lossy().as_ref()));
    assert!(wrapper_body.contains(core_entry.to_string_lossy().as_ref()));
}

#[test]
fn rejects_path_style_gemini_command() {
    let temp = tempdir().unwrap();
    let data_root = temp.path().join("data");
    let gemini_bin = temp.path().join("bundle").join("bin").join("gemini");
    std::fs::create_dir_all(gemini_bin.parent().unwrap()).unwrap();
    std::fs::write(&gemini_bin, b"gemini").unwrap();

    let input = installer::AgentServerCommand {
        command: gemini_bin.to_string_lossy().to_string(),
        args: vec!["--experimental-acp".to_string()],
        dependencies: Vec::new(),
        managed: None,
    };
    let err = normalize_acp_provider_command(&data_root, "gemini", input).unwrap_err();

    assert!(err
        .to_string()
        .contains("must use an explicit absolute node executable"));
}

#[test]
fn rejects_relative_gemini_entrypoint() {
    let temp = tempdir().unwrap();
    let data_root = temp.path().join("data");
    let (node_bin, _, _) = create_gemini_runtime_layout(temp.path());

    let input = installer::AgentServerCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec![
            "node_modules/@google/gemini-cli/dist/index.js".to_string(),
            "--experimental-acp".to_string(),
        ],
        dependencies: Vec::new(),
        managed: None,
    };
    let err = normalize_acp_provider_command(&data_root, "gemini", input).unwrap_err();

    assert!(err
        .to_string()
        .contains("Gemini ACP entrypoint must be an explicit absolute path"));
}

#[test]
fn rejects_gemini_runtime_when_core_package_is_missing() {
    let temp = tempdir().unwrap();
    let data_root = temp.path().join("data");
    let (node_bin, cli_entry, core_entry) = create_gemini_runtime_layout(temp.path());
    std::fs::remove_file(core_entry).unwrap();

    let input = installer::AgentServerCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec![
            cli_entry.to_string_lossy().to_string(),
            "--experimental-acp".to_string(),
        ],
        dependencies: Vec::new(),
        managed: None,
    };
    let err = normalize_acp_provider_command(&data_root, "gemini", input).unwrap_err();

    assert!(err
        .to_string()
        .contains("Gemini ACP companion package is missing"));
}

#[test]
fn runtime_probe_command_wraps_acp_provider_with_bridge() {
    let temp = tempdir().unwrap();
    let cursor_cmd = temp.path().join("cursor-agent");
    let bridge_cmd = temp.path().join("acp-crp-bridge");
    std::fs::write(&cursor_cmd, b"cursor").unwrap();
    std::fs::write(&bridge_cmd, b"bridge").unwrap();
    let cfg = installer::AgentServerConfigFile {
        providers: HashMap::from([
            (
                "cursor".to_string(),
                installer::AgentServerCommand {
                    command: cursor_cmd.to_string_lossy().to_string(),
                    args: vec!["--experimental-acp".to_string()],
                    dependencies: vec!["cursor-dep".to_string()],
                    managed: None,
                },
            ),
            (
                "acp-crp-bridge".to_string(),
                installer::AgentServerCommand {
                    command: bridge_cmd.to_string_lossy().to_string(),
                    args: vec!["--log-level".to_string(), "debug".to_string()],
                    dependencies: vec!["bridge-dep".to_string()],
                    managed: None,
                },
            ),
        ]),
        provider_login_commands: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
    };

    let resolved = runtime_probe_command_as_agent_command(temp.path(), &cfg, "cursor")
        .expect("probe command")
        .expect("runtime command");

    assert_eq!(
        PathBuf::from(&resolved.command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("acp-crp-bridge")
    );
    assert_eq!(resolved.args.len(), 4);
    assert_eq!(resolved.args[0], "--log-level");
    assert_eq!(resolved.args[1], "debug");
    assert_eq!(resolved.dependencies, vec!["bridge-dep", "cursor-dep"]);
}

#[test]
fn runtime_probe_command_rejects_path_style_gemini_runtime() {
    let temp = tempdir().unwrap();
    let gemini_bin = temp.path().join("bundle").join("bin").join("gemini");
    let bridge_cmd = temp.path().join("acp-crp-bridge");
    std::fs::create_dir_all(gemini_bin.parent().unwrap()).unwrap();
    std::fs::write(&gemini_bin, b"gemini").unwrap();
    std::fs::write(&bridge_cmd, b"bridge").unwrap();
    let cfg = installer::AgentServerConfigFile {
        providers: HashMap::from([
            (
                "gemini".to_string(),
                installer::AgentServerCommand {
                    command: gemini_bin.to_string_lossy().to_string(),
                    args: vec!["--experimental-acp".to_string()],
                    dependencies: Vec::new(),
                    managed: None,
                },
            ),
            (
                "acp-crp-bridge".to_string(),
                installer::AgentServerCommand {
                    command: bridge_cmd.to_string_lossy().to_string(),
                    args: vec!["--log-level".to_string(), "debug".to_string()],
                    dependencies: Vec::new(),
                    managed: None,
                },
            ),
        ]),
        provider_login_commands: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
    };

    let err = runtime_probe_command_as_agent_command(temp.path(), &cfg, "gemini").unwrap_err();
    assert!(err
        .to_string()
        .contains("must use an explicit absolute node executable"));
}
