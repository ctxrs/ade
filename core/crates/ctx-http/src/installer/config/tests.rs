use super::*;
use std::sync::{Mutex, OnceLock};
use tempfile::tempdir;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
                unsafe { std::env::set_var(self.key, value) };
            }
            None => {
                // SAFETY: Guarded by ENV_LOCK so tests mutate process env serially.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }
}

#[test]
fn bundled_only_mode_errors_when_bundled_command_missing() {
    let _guard = env_lock().lock().expect("lock env");
    let temp = tempdir().expect("tempdir");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &temp.path().to_string_lossy());
    let _strict = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY", "1");
    let _providers = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY_PROVIDERS", "qwen");

    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "qwen".to_string(),
        AgentServerCommand {
            command: "/tmp/non-bundled-qwen".to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: Some(ManagedInstallMetadata {
                package: Some("qwen-managed".to_string()),
                version: Some("1.0.0".to_string()),
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            }),
        },
    );

    let err = resolve_runtime_provider_command(&cfg, "qwen").expect_err("should fail");
    assert!(err
        .to_string()
        .contains("runtime_command_missing_bundled: provider=qwen"));
}

#[test]
fn bundled_only_provider_scope_defaults_to_all_when_empty() {
    let _guard = env_lock().lock().expect("lock env");
    let _bundle_dir = EnvVarGuard::unset("CTX_BUNDLE_DIR");
    let _strict = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY", "1");
    let _providers = EnvVarGuard::set("CTX_E2E_BUNDLED_ONLY_PROVIDERS", " , ");
    assert!(bundled_only_mode_applies_to_provider("codex"));
    assert!(bundled_only_mode_applies_to_provider("acp-crp-bridge"));
}

#[test]
fn bundled_only_mode_can_be_disabled() {
    let _guard = env_lock().lock().expect("lock env");
    let _bundle_dir = EnvVarGuard::unset("CTX_BUNDLE_DIR");
    let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
    let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");
    assert!(!bundled_only_mode_applies_to_provider("codex"));
}

#[test]
fn bundle_dir_alone_does_not_force_bundled_only_mode() {
    let _guard = env_lock().lock().expect("lock env");
    let temp = tempdir().expect("tempdir");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &temp.path().to_string_lossy());
    let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
    let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");
    assert!(!bundled_only_mode_applies_to_provider("codex"));
}

#[cfg(unix)]
#[test]
fn resolve_runtime_provider_command_preserves_raw_bundle_symlink_paths() {
    use std::os::unix::fs::symlink;

    let _guard = env_lock().lock().expect("lock env");
    let temp = tempdir().expect("tempdir");
    let bundle_target = temp.path().join("bundle-target");
    let bundle_link = temp.path().join("bundle-link");
    let target_command = bundle_target.join("providers/codex/macos/aarch64/codex-crp");
    let raw_command = bundle_link.join("providers/codex/macos/aarch64/codex-crp");
    std::fs::create_dir_all(target_command.parent().expect("parent")).expect("mkdir");
    std::fs::write(&target_command, b"ok").expect("write command");
    symlink(&bundle_target, &bundle_link).expect("symlink bundle");

    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_link.to_string_lossy());
    let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
    let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");

    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex".to_string(),
        AgentServerCommand {
            command: raw_command.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: Some(ManagedInstallMetadata {
                package: Some("@openai/codex".to_string()),
                version: Some("1.0.0".to_string()),
                target: Some(InstallTarget::Container),
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            }),
        },
    );

    let resolved =
        resolve_runtime_provider_command_for_target(&cfg, "codex", Some(InstallTarget::Container))
            .expect("resolve runtime command")
            .expect("runtime command");
    assert_eq!(resolved.command_abs_path, raw_command.to_string_lossy());
    assert_ne!(
        std::fs::canonicalize(&raw_command)
            .expect("canonicalize raw command")
            .to_string_lossy(),
        resolved.command_abs_path
    );
}

#[test]
fn migration_moves_legacy_managed_provider_entries_into_target_buckets() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex".to_string(),
        AgentServerCommand {
            command: "/tmp/codex-host".to_string(),
            args: Vec::new(),
            dependencies: vec!["runtime-node-host".to_string()],
            managed: Some(ManagedInstallMetadata {
                package: Some("@openai/codex".to_string()),
                version: Some("0.2.54".to_string()),
                target: None,
                install_dir_rel: Some("providers/agent-servers/codex/0.2.54".to_string()),
                bin_dir_rel: Some("providers/agent-servers/codex/0.2.54/bin".to_string()),
                last_success_at: None,
                last_error: None,
            }),
        },
    );

    assert!(migrate_agent_server_config(&mut cfg));
    assert!(!cfg.providers.contains_key("codex"));
    assert!(!cfg.managed_installs.contains_key("codex"));
    assert_eq!(
        cfg.managed_install_targets
            .get("codex")
            .and_then(|targets| targets.get("host"))
            .and_then(|meta| meta.target),
        Some(InstallTarget::Host)
    );
    assert_eq!(
        cfg.managed_provider_targets
            .get("codex")
            .and_then(|targets| targets.get("host"))
            .map(|command| command.command.as_str()),
        Some("/tmp/codex-host")
    );
}

#[test]
fn migration_preserves_runtime_dependency_entries_and_infers_target_from_id() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_installs.insert(
        "runtime-node-container".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some("24.14.0".to_string()),
            target: None,
            install_dir_rel: Some("providers/runtimes/node/container".to_string()),
            bin_dir_rel: Some("providers/runtimes/node/container/bin".to_string()),
            last_success_at: None,
            last_error: None,
        },
    );

    assert!(migrate_agent_server_config(&mut cfg));
    assert_eq!(
        cfg.managed_installs
            .get("runtime-node-container")
            .and_then(|meta| meta.target),
        Some(InstallTarget::Container)
    );
    assert!(!cfg
        .managed_install_targets
        .contains_key("runtime-node-container"));
}

#[test]
fn resolve_runtime_provider_command_for_target_prefers_target_bucket() {
    let temp = tempdir().expect("tempdir");
    let host = temp.path().join("codex-host");
    let container = temp.path().join("codex-container");
    std::fs::write(&host, b"host").expect("write host runtime");
    std::fs::write(&container, b"container").expect("write container runtime");

    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([
            (
                "host".to_string(),
                AgentServerCommand {
                    command: host.to_string_lossy().to_string(),
                    args: vec!["--host".to_string()],
                    dependencies: vec!["runtime-node-host".to_string()],
                    managed: Some(ManagedInstallMetadata {
                        package: Some("@openai/codex".to_string()),
                        version: Some("1.0.0".to_string()),
                        target: Some(InstallTarget::Host),
                        install_dir_rel: None,
                        bin_dir_rel: None,
                        last_success_at: None,
                        last_error: None,
                    }),
                },
            ),
            (
                "container".to_string(),
                AgentServerCommand {
                    command: container.to_string_lossy().to_string(),
                    args: vec!["--container".to_string()],
                    dependencies: vec!["runtime-node-container".to_string()],
                    managed: Some(ManagedInstallMetadata {
                        package: Some("@openai/codex".to_string()),
                        version: Some("1.0.0".to_string()),
                        target: Some(InstallTarget::Container),
                        install_dir_rel: None,
                        bin_dir_rel: None,
                        last_success_at: None,
                        last_error: None,
                    }),
                },
            ),
        ]),
    );

    let host_resolved =
        resolve_runtime_provider_command_for_target(&cfg, "codex", Some(InstallTarget::Host))
            .expect("resolve host")
            .expect("host runtime");
    assert_eq!(
        host_resolved.command_abs_path,
        std::fs::canonicalize(&host)
            .expect("canonicalize host runtime")
            .to_string_lossy()
    );
    assert_eq!(host_resolved.args, vec!["--host".to_string()]);

    let container_resolved =
        resolve_runtime_provider_command_for_target(&cfg, "codex", Some(InstallTarget::Container))
            .expect("resolve container")
            .expect("container runtime");
    assert_eq!(
        container_resolved.command_abs_path,
        std::fs::canonicalize(&container)
            .expect("canonicalize container runtime")
            .to_string_lossy()
    );
    assert_eq!(container_resolved.args, vec!["--container".to_string()]);
}

#[test]
fn resolve_runtime_provider_command_for_target_does_not_use_bundled_seed_for_container() {
    let _guard = env_lock().lock().expect("lock env");
    let temp = tempdir().expect("tempdir");
    let bundle_dir = temp.path().join("bundle");
    let bundle_bin = bundle_dir.join("bin");
    let bundle_cmd = bundle_bin.join("acp-crp-bridge");
    std::fs::create_dir_all(&bundle_bin).expect("mkdir bundle bin");
    std::fs::write(&bundle_cmd, b"bridge").expect("write bundle bridge");
    std::fs::write(
        bundle_dir.join("manifest.json"),
        format!(
            r#"{{
  "version": 1,
  "providers": [
    {{
      "id": "acp-crp-bridge",
      "protocol": "crp",
      "version": "0.1.0",
      "os": "{}",
      "arch": "{}",
      "command": "acp-crp-bridge",
      "args": [],
      "sha256": "deadbeef"
    }}
  ]
}}"#,
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    )
    .expect("write bundle manifest");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());
    let _bundle_manifest = EnvVarGuard::unset("CTX_BUNDLE_MANIFEST");
    let _strict = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY");
    let _providers = EnvVarGuard::unset("CTX_E2E_BUNDLED_ONLY_PROVIDERS");

    let resolved = resolve_runtime_provider_command_for_target(
        &AgentServerConfigFile::default(),
        "acp-crp-bridge",
        Some(InstallTarget::Container),
    )
    .expect("resolve container target");

    assert!(
        resolved.is_none(),
        "container target must not reuse host bundled provider commands"
    );
}

#[test]
fn resolve_provider_login_command_reads_prepared_absolute_path() {
    let temp = tempdir().expect("tempdir");
    let login_cmd = temp.path().join("cursor-agent");
    std::fs::write(&login_cmd, b"cursor").expect("write login command");

    let mut cfg = AgentServerConfigFile::default();
    cfg.provider_login_commands.insert(
        "cursor".to_string(),
        AgentServerCommand {
            command: login_cmd.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        },
    );

    let resolved = resolve_provider_login_command(&cfg, "cursor")
        .expect("resolve login command")
        .expect("login command");
    assert_eq!(
        resolved,
        std::fs::canonicalize(&login_cmd).expect("canonicalize login command")
    );
}

#[test]
fn resolve_provider_login_command_rejects_relative_paths() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.provider_login_commands.insert(
        "cursor".to_string(),
        AgentServerCommand {
            command: "cursor-agent".to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        },
    );

    let err = resolve_provider_login_command(&cfg, "cursor")
        .expect_err("relative login command should fail");
    assert!(err
        .to_string()
        .contains("runtime_command_not_absolute: provider=cursor source=login_command"));
}

#[test]
fn migration_drops_legacy_bundled_provider_commands() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
            "codex".to_string(),
            AgentServerCommand {
                command:
                    "/Applications/ctx.app/Contents/Resources/bundles/providers/codex/macos/aarch64/codex"
                        .to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(ManagedInstallMetadata {
                    package: Some("@openai/codex".to_string()),
                    version: Some("0.2.54".to_string()),
                    target: None,
                    install_dir_rel: Some("bundles/providers/codex/macos/aarch64".to_string()),
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                }),
            },
        );
    cfg.managed_installs.insert(
        "codex".to_string(),
        ManagedInstallMetadata {
            package: Some("@openai/codex".to_string()),
            version: Some("0.2.54".to_string()),
            target: None,
            install_dir_rel: Some("bundles/providers/codex/macos/aarch64".to_string()),
            bin_dir_rel: None,
            last_success_at: None,
            last_error: None,
        },
    );

    assert!(migrate_agent_server_config(&mut cfg));
    assert!(!cfg.providers.contains_key("codex"));
    assert!(!cfg.managed_installs.contains_key("codex"));
}
