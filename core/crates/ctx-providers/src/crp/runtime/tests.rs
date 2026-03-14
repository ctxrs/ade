use super::*;
use std::fs;

#[test]
fn container_exec_outer_process_env_skips_provider_home_and_xdg_keys() {
    assert!(should_skip_outer_process_env_key("HOME", true));
    assert!(should_skip_outer_process_env_key("TMPDIR", true));
    assert!(should_skip_outer_process_env_key("XDG_CONFIG_HOME", true));
    assert!(should_skip_outer_process_env_key("XDG_STATE_HOME", true));
    assert!(!should_skip_outer_process_env_key("OPENAI_API_KEY", true));
    assert!(!should_skip_outer_process_env_key("HOME", false));
}

#[test]
fn rewrite_bundled_path_for_linux_rewrites_provider_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let host = tmp
        .path()
        .join("bundles/providers/acp-crp-bridge/macos/aarch64/acp-crp-bridge");
    let linux = tmp
        .path()
        .join("bundles/providers/acp-crp-bridge/linux/aarch64/acp-crp-bridge");
    fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
    fs::write(&linux, b"ok").expect("write");

    let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
        .expect("rewrite should succeed");
    assert_eq!(rewritten, linux.to_string_lossy());
}

#[test]
fn rewrite_bundled_path_for_linux_rewrites_e2e_bundle_provider_paths() {
    let tmp = tempfile::Builder::new()
        .prefix("ctx-e2e-bundles-runtime-probe-")
        .tempdir()
        .expect("tempdir");
    let host = tmp.path().join("providers/codex/macos/aarch64/codex-crp");
    let linux = tmp.path().join("providers/codex/linux/aarch64/codex-crp");
    fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
    fs::write(&linux, b"ok").expect("write linux");
    fs::write(tmp.path().join("manifest.json"), "{}").expect("write manifest");

    let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
        .expect("rewrite should succeed");
    assert_eq!(rewritten, linux.to_string_lossy());
}

#[test]
fn rewrite_bundled_path_for_linux_rewrites_runtime_flavor_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let host = tmp
        .path()
        .join("bundles/runtimes/node/macos/aarch64/node-v24.12.0-darwin-arm64/bin/node");
    let linux = tmp
        .path()
        .join("bundles/runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64/bin/node");
    fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
    fs::write(&linux, b"ok").expect("write");
    let manifest_path = tmp.path().join("bundles/manifest.json");
    fs::write(
        &manifest_path,
        serde_json::json!({
            "version": 1,
            "providers": [],
            "runtimes": [
                {
                    "id": "node",
                    "os": "linux",
                    "arch": "aarch64",
                    "root": "runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64",
                    "bin": "bin/node"
                }
            ],
            "images": [],
            "daemons": []
        })
        .to_string(),
    )
    .expect("write manifest");

    let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
        .expect("rewrite should succeed");
    assert_eq!(rewritten, linux.to_string_lossy());
}

#[test]
fn rewrite_bundled_path_for_linux_rewrites_runtime_flavor_directory_for_e2e_bundle_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let host = tmp
        .path()
        .join("runtimes/node/macos/aarch64/node-v24.12.0-darwin-arm64/bin/node");
    let linux = tmp
        .path()
        .join("runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64/bin/node");
    fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
    fs::write(&linux, b"ok").expect("write");
    let manifest_path = tmp.path().join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::json!({
            "version": 1,
            "providers": [],
            "runtimes": [
                {
                    "id": "node",
                    "os": "linux",
                    "arch": "aarch64",
                    "root": "runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64",
                    "bin": "bin/node"
                }
            ],
            "images": [],
            "daemons": []
        })
        .to_string(),
    )
    .expect("write manifest");

    let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
        .expect("rewrite should succeed");
    assert_eq!(rewritten, linux.to_string_lossy());
}

#[test]
fn rewrite_container_args_for_linux_rewrites_nested_acp_command_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let host_provider = tmp
        .path()
        .join("bundles/providers/pi/macos/aarch64/pi-acp.js");
    let linux_provider = tmp
        .path()
        .join("bundles/providers/pi/linux/aarch64/pi-acp.js");
    let host_node = tmp
        .path()
        .join("bundles/runtimes/node/macos/aarch64/node-v1/bin/node");
    let linux_node = tmp
        .path()
        .join("bundles/runtimes/node/linux/aarch64/node-v1/bin/node");
    fs::create_dir_all(linux_provider.parent().expect("parent")).expect("mkdir");
    fs::create_dir_all(host_node.parent().expect("parent")).expect("mkdir host node");
    fs::create_dir_all(linux_node.parent().expect("parent")).expect("mkdir");
    fs::write(&linux_provider, b"ok").expect("write");
    fs::write(&host_node, b"ok").expect("write host node");
    fs::write(&linux_node, b"ok").expect("write");

    let raw_acp = format!("{} --foo", host_provider.to_string_lossy());
    let args = vec!["--acp-command".to_string(), raw_acp];
    let mut env = HashMap::new();
    env.insert(
        "PATH".to_string(),
        host_node
            .parent()
            .expect("node dir")
            .to_string_lossy()
            .to_string(),
    );
    let rewritten = rewrite_container_args_for_linux(&args, &env).expect("rewrite args");
    assert_eq!(rewritten.len(), 2);
    let parsed = shlex::split(&rewritten[1]).expect("parse rewritten command");
    assert_eq!(
        parsed,
        vec![
            linux_node.to_string_lossy().to_string(),
            linux_provider.to_string_lossy().to_string(),
            "--foo".to_string(),
        ]
    );
}

#[test]
fn rewrite_container_args_for_linux_preserves_quoted_paths_with_spaces() {
    let tmp = tempfile::Builder::new()
        .prefix("ctx bundles with spaces ")
        .tempdir()
        .expect("tempdir");
    let host_provider = tmp
        .path()
        .join("bundles/providers/pi/macos/aarch64/pi-acp.js");
    let linux_provider = tmp
        .path()
        .join("bundles/providers/pi/linux/aarch64/pi-acp.js");
    let host_node = tmp
        .path()
        .join("bundles/runtimes/node/macos/aarch64/node-v1/bin/node");
    let linux_node = tmp
        .path()
        .join("bundles/runtimes/node/linux/aarch64/node-v1/bin/node");
    fs::create_dir_all(linux_provider.parent().expect("parent")).expect("mkdir");
    fs::create_dir_all(host_node.parent().expect("parent")).expect("mkdir host node");
    fs::create_dir_all(linux_node.parent().expect("parent")).expect("mkdir");
    fs::write(&linux_provider, b"ok").expect("write");
    fs::write(&host_node, b"ok").expect("write host node");
    fs::write(&linux_node, b"ok").expect("write");

    let raw_acp = shlex::try_join(
        [
            host_provider.to_string_lossy().to_string(),
            "--flag".to_string(),
        ]
        .iter()
        .map(String::as_str),
    )
    .expect("quote acp command");
    let args = vec!["--acp-command".to_string(), raw_acp];
    let mut env = HashMap::new();
    env.insert(
        "PATH".to_string(),
        host_node
            .parent()
            .expect("node dir")
            .to_string_lossy()
            .to_string(),
    );
    let rewritten = rewrite_container_args_for_linux(&args, &env).expect("rewrite args");
    assert_eq!(rewritten.len(), 2);
    let parsed = shlex::split(&rewritten[1]).expect("parse rewritten command");
    assert_eq!(
        parsed,
        vec![
            linux_node.to_string_lossy().to_string(),
            linux_provider.to_string_lossy().to_string(),
            "--flag".to_string(),
        ]
    );
}

#[test]
fn rewrite_container_args_for_linux_keeps_explicit_node_binary_for_acp_command() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let host_provider = tmp
        .path()
        .join("bundles/providers/pi/macos/aarch64/pi-acp.js");
    let linux_provider = tmp
        .path()
        .join("bundles/providers/pi/linux/aarch64/pi-acp.js");
    let host_node = tmp
        .path()
        .join("bundles/runtimes/node/macos/aarch64/node-v1/bin/node");
    let linux_node = tmp
        .path()
        .join("bundles/runtimes/node/linux/aarch64/node-v1/bin/node");
    fs::create_dir_all(linux_provider.parent().expect("parent")).expect("mkdir");
    fs::create_dir_all(linux_node.parent().expect("parent")).expect("mkdir");
    fs::write(&linux_provider, b"ok").expect("write provider");
    fs::write(&linux_node, b"ok").expect("write node");

    let raw_acp = shlex::try_join(
        [
            host_node.to_string_lossy().to_string(),
            host_provider.to_string_lossy().to_string(),
            "--flag".to_string(),
        ]
        .iter()
        .map(String::as_str),
    )
    .expect("quote acp command");
    let args = vec!["--acp-command".to_string(), raw_acp];
    let mut env = HashMap::new();
    env.insert(
        "PATH".to_string(),
        linux_node
            .parent()
            .expect("node dir")
            .to_string_lossy()
            .to_string(),
    );

    let rewritten = rewrite_container_args_for_linux(&args, &env).expect("rewrite args");
    let parsed = shlex::split(&rewritten[1]).expect("parse rewritten command");
    assert_eq!(
        parsed,
        vec![
            linux_node.to_string_lossy().to_string(),
            linux_provider.to_string_lossy().to_string(),
            "--flag".to_string(),
        ]
    );
}

#[test]
fn rewrite_container_command_for_linux_uses_explicit_node_binary_for_js_entrypoints() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let node_dir = tmp.path().join("runtimes/node/linux/aarch64/node-v1/bin");
    fs::create_dir_all(&node_dir).expect("mkdir node dir");
    let node_bin = node_dir.join("node");
    fs::write(&node_bin, b"ok").expect("write node");
    let script = tmp.path().join("providers/pi/linux/aarch64/pi-acp.js");
    fs::create_dir_all(script.parent().expect("parent")).expect("mkdir script parent");
    fs::write(&script, b"#!/usr/bin/env node\n").expect("write script");

    let mut env = HashMap::new();
    env.insert("PATH".to_string(), node_dir.to_string_lossy().to_string());
    let args = vec!["--flag".to_string()];

    let (command, rewritten_args) =
        rewrite_container_command_for_linux(script.to_string_lossy().as_ref(), &args, &env)
            .expect("rewrite command");

    assert_eq!(command, node_bin.to_string_lossy());
    assert_eq!(
        rewritten_args,
        vec![script.to_string_lossy().to_string(), "--flag".to_string()]
    );
}

#[test]
fn rewrite_bundled_path_for_linux_errors_when_linux_target_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let host = tmp
        .path()
        .join("bundles/providers/cursor/macos/aarch64/cursor-agent-acp.js");
    fs::create_dir_all(host.parent().expect("parent")).expect("mkdir");
    fs::write(&host, b"ok").expect("write");

    let err = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
        .expect_err("expected missing linux target error");
    let msg = err.to_string();
    assert!(msg.contains("missing linux bundled path"));
}

#[test]
fn rewrite_bundled_path_for_linux_ignores_managed_install_provider_paths() {
    let path =
        "/tmp/providers/agent-servers/cursor-agent-acp/node_modules/@scope/pkg/dist/bin/app.js";
    let rewritten = rewrite_bundled_path_for_linux(path).expect("rewrite should succeed");
    assert_eq!(rewritten, path);
}

#[test]
fn rewrite_bundled_path_for_linux_ignores_managed_install_runtime_paths() {
    let path = "/tmp/runtimes/node/v24.12.0/bin/node";
    let rewritten = rewrite_bundled_path_for_linux(path).expect("rewrite should succeed");
    assert_eq!(rewritten, path);
}

#[test]
fn rewrite_container_args_for_linux_rejects_invalid_shell_command() {
    let args = vec!["--acp-command".to_string(), "\"unterminated".to_string()];
    let err =
        rewrite_container_args_for_linux(&args, &HashMap::new()).expect_err("expected parse error");
    assert!(err
        .to_string()
        .contains("invalid shell command in --acp-command"));
}
