use std::collections::HashMap;
use std::path::Path;

use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_matrix::{
    ProviderArchiveKind, ProviderArchiveTarget, ProviderInstall, ProviderMatrixEntry,
    ProviderMatrixEntryKind, ProviderRelease, ProviderReleaseStatus,
};
use ctx_providers::adapters::{ProviderHealth, ProviderStatus, ProviderUsability};
use sha2::{Digest, Sha256};

use crate::{
    provider_status_matrix::{apply_matrix_to_status, probe_command_version},
    AgentServerCommand, AgentServerConfigFile, ManagedInstallMetadata,
};

const CURRENT_CTX_VERSION: Option<&str> = Some("0.59.0-canary.deadbeefcafe");

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct ScopedEnvVar {
    key: &'static str,
    old: Option<String>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn release(
    version: &str,
    status: ProviderReleaseStatus,
    context_min: Option<&str>,
) -> ProviderRelease {
    ProviderRelease {
        version: version.to_string(),
        status,
        upstream_version: Some(version.to_string()),
        provenance: None,
        context_min: context_min.map(ToOwned::to_owned),
        context_max: None,
        notes: None,
    }
}

fn codex_archive_entry(
    version: &str,
    sha256: &str,
    releases: Vec<ProviderRelease>,
) -> ProviderMatrixEntry {
    ProviderMatrixEntry {
        id: "codex".to_string(),
        kind: ProviderMatrixEntryKind::Harness,
        display_name: Some("Codex".to_string()),
        tier: Some("tier1".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Archive {
            version: version.to_string(),
            args: Vec::new(),
            targets: HashMap::from([(
                "linux-x86_64".to_string(),
                ProviderArchiveTarget {
                    url: "https://example.invalid/codex.tar.gz".to_string(),
                    sha256: Some(sha256.to_string()),
                    size_bytes: None,
                    archive: ProviderArchiveKind::TarGz,
                    bin_path: "codex-crp".to_string(),
                },
            )]),
        }),
        provider_dependencies: Vec::new(),
        dependencies: Vec::new(),
        version_probe: None,
        releases,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn provider_version_probe_scrubs_daemon_auth_env() {
    let _env_guard = crate::test_support::process_env_test_lock().lock().await;
    let script = r#"
for key in CTX_AUTH_TOKEN CTX_MCP_TOKEN CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN; do
  eval "value=\${$key:-}"
  if [ -n "$value" ]; then
    echo "unexpected $key" >&2
    exit 91
  fi
done
echo "provider 1.2.3"
"#;

    let _guards: Vec<_> = ctx_core::env::DAEMON_AUTH_ENV_VARS
        .iter()
        .map(|key| ScopedEnvVar::set(key, "daemon-secret"))
        .collect();
    let version = probe_command_version("/bin/sh", &["-c".to_string(), script.to_string()]).await;

    assert_eq!(version.as_deref(), Some("1.2.3"));
}

fn codex_npm_entry(releases: Vec<ProviderRelease>) -> ProviderMatrixEntry {
    let managed_version = releases
        .last()
        .map(|release| release.version.clone())
        .unwrap_or_else(|| "1.0.0".to_string());
    ProviderMatrixEntry {
        id: "codex".to_string(),
        kind: ProviderMatrixEntryKind::Harness,
        display_name: Some("Codex".to_string()),
        tier: Some("tier1".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Npm {
            package: "@openai/codex".to_string(),
            version: managed_version,
            entrypoint: "node_modules/@openai/codex/bin.js".to_string(),
            args: Vec::new(),
            targets: std::collections::HashMap::new(),
        }),
        provider_dependencies: Vec::new(),
        dependencies: Vec::new(),
        version_probe: None,
        releases,
    }
}

fn managed_archive_cfg(
    command_path: &Path,
    version: &str,
    installed_sha256: &str,
) -> AgentServerConfigFile {
    let target = InstallTarget::LinuxX8664;
    let target_key = target.as_str().to_string();
    let meta = ManagedInstallMetadata {
        package: Some("https://example.invalid/codex.tar.gz".to_string()),
        version: Some(version.to_string()),
        artifact_fingerprint: Some(installed_sha256.to_string()),
        archive_sha256: Some(installed_sha256.to_string()),
        target: Some(target),
        install_dir_rel: Some(format!("providers/agent-servers/codex/{version}")),
        bin_dir_rel: None,
        last_success_at: None,
        last_error: None,
    };
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([(target_key.clone(), meta.clone())]),
    );
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([(
            target_key,
            AgentServerCommand {
                command: command_path.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(meta),
            },
        )]),
    );
    cfg
}

fn managed_hybrid_npm_container_cfg(
    version: &str,
    installed_sha256: &str,
) -> AgentServerConfigFile {
    let target = InstallTarget::Container;
    let target_key = target.as_str().to_string();
    let meta = ManagedInstallMetadata {
        package: Some("@openai/codex".to_string()),
        version: Some(version.to_string()),
        artifact_fingerprint: Some(installed_sha256.to_string()),
        archive_sha256: Some(installed_sha256.to_string()),
        target: Some(target),
        install_dir_rel: Some(format!("providers/agent-servers/codex/{version}")),
        bin_dir_rel: Some(format!("providers/agent-servers/codex/{version}/bin")),
        last_success_at: None,
        last_error: None,
    };
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([(target_key.clone(), meta.clone())]),
    );
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([(
            target_key,
            AgentServerCommand {
                command: "/tmp/codex-container".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(meta),
            },
        )]),
    );
    cfg
}

fn managed_npm_cfg(version: &str) -> AgentServerConfigFile {
    let target = InstallTarget::Host;
    let target_key = target.as_str().to_string();
    let meta = ManagedInstallMetadata {
        package: Some("@openai/codex".to_string()),
        version: Some(version.to_string()),
        artifact_fingerprint: Some(format!("npm:@openai/codex@{version}")),
        archive_sha256: None,
        target: Some(target),
        install_dir_rel: Some(format!("providers/agent-servers/codex/{version}")),
        bin_dir_rel: Some(format!("providers/agent-servers/codex/{version}/bin")),
        last_success_at: None,
        last_error: None,
    };
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([(target_key.clone(), meta.clone())]),
    );
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([(
            target_key,
            AgentServerCommand {
                command: "/tmp/codex".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(meta),
            },
        )]),
    );
    cfg
}

fn installed_status(provider_id: &str, install_target: InstallTarget) -> ProviderStatus {
    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: true,
        detected_path: Some(format!("/tmp/{provider_id}")),
        version: None,
        capabilities: None,
        health: ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([(
            "install_target".to_string(),
            install_target.as_str().to_string(),
        )]),
        usability: ProviderUsability::default(),
    }
}

#[tokio::test]
async fn provider_status_matrix_marks_supported_stale_runtime_updateable_without_blocking() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = temp.path().join("codex");
    std::fs::write(&runtime, b"old-runtime").expect("write runtime");

    let old_sha = sha256_hex(b"old-archive");
    let new_sha = sha256_hex(b"new-archive");
    let entry = codex_archive_entry(
        "1.0.1",
        &new_sha,
        vec![
            release("1.0.0", ProviderReleaseStatus::Supported, None),
            release("1.0.1", ProviderReleaseStatus::Supported, Some("0.59.0")),
        ],
    );
    let cfg = managed_archive_cfg(&runtime, "1.0.0", &old_sha);
    let mut status = installed_status("codex", InstallTarget::LinuxX8664);

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert_eq!(status.version.as_deref(), Some("1.0.0"));
    assert!(matches!(status.health, ProviderHealth::Ok));
    assert_eq!(
        status
            .details
            .get("matrix_recommended_version")
            .map(String::as_str),
        Some("1.0.1")
    );
    assert_eq!(
        status
            .details
            .get("matrix_update_available")
            .map(String::as_str),
        Some("true")
    );
}

#[tokio::test]
async fn provider_status_matrix_marks_missing_runtime_dependency_updateable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = temp.path().join("codex");
    std::fs::write(&runtime, b"matching-runtime").expect("write runtime");

    let sha = sha256_hex(b"matching-archive");
    let entry = codex_archive_entry(
        "1.0.1",
        &sha,
        vec![release("1.0.1", ProviderReleaseStatus::Supported, None)],
    );
    let mut cfg = managed_archive_cfg(&runtime, "1.0.1", &sha);
    cfg.managed_provider_targets
        .get_mut("codex")
        .and_then(|targets| targets.get_mut(InstallTarget::LinuxX8664.as_str()))
        .expect("codex linux target")
        .dependencies = vec!["runtime-node-host".to_string()];
    let mut status = installed_status("codex", InstallTarget::LinuxX8664);

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert!(matches!(status.health, ProviderHealth::Ok));
    assert_eq!(
        status
            .details
            .get("managed_dependency_update_available")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        status
            .details
            .get("matrix_update_available")
            .map(String::as_str),
        Some("true")
    );
}

#[tokio::test]
async fn provider_status_matrix_marks_stale_implicit_node_runtime_updateable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = temp.path().join("codex");
    std::fs::write(&runtime, b"matching-runtime").expect("write runtime");

    let sha = sha256_hex(b"matching-archive");
    let entry = codex_archive_entry(
        "1.0.1",
        &sha,
        vec![release("1.0.1", ProviderReleaseStatus::Supported, None)],
    );
    let mut cfg = managed_archive_cfg(&runtime, "1.0.1", &sha);
    let dependency_id = "runtime-node-linux-x86_64";
    cfg.managed_provider_targets
        .get_mut("codex")
        .and_then(|targets| targets.get_mut(InstallTarget::LinuxX8664.as_str()))
        .expect("codex linux target")
        .dependencies = vec![dependency_id.to_string()];
    cfg.managed_install_targets.insert(
        dependency_id.to_string(),
        HashMap::from([(
            InstallTarget::LinuxX8664.as_str().to_string(),
            ManagedInstallMetadata {
                package: Some("node-runtime".to_string()),
                version: Some(crate::NODE_VERSION.to_string()),
                artifact_fingerprint: Some(format!(
                    "runtime:node:{}:sha256:{}",
                    crate::NODE_VERSION,
                    "0".repeat(64)
                )),
                archive_sha256: Some("0".repeat(64)),
                target: Some(InstallTarget::LinuxX8664),
                install_dir_rel: Some("runtimes/node/stale".to_string()),
                bin_dir_rel: Some("runtimes/node/stale/bin".to_string()),
                last_success_at: None,
                last_error: None,
            },
        )]),
    );
    let mut status = installed_status("codex", InstallTarget::LinuxX8664);

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert!(matches!(status.health, ProviderHealth::Ok));
    assert_eq!(
        status
            .details
            .get("managed_dependency_update_available")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        status
            .details
            .get("matrix_update_available")
            .map(String::as_str),
        Some("true")
    );
}

#[tokio::test]
async fn provider_status_matrix_uses_dependency_id_target_for_implicit_node_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = temp.path().join("codex");
    std::fs::write(&runtime, b"matching-runtime").expect("write runtime");

    let sha = sha256_hex(b"matching-archive");
    let entry = codex_archive_entry(
        "1.0.1",
        &sha,
        vec![release("1.0.1", ProviderReleaseStatus::Supported, None)],
    );
    let mut cfg = managed_archive_cfg(&runtime, "1.0.1", &sha);
    let dependency_id = "runtime-node-linux-x86_64";
    let linux_runtime_sha = "44836872d9aec49f1e6b52a9a922872db9a2b02d235a616a5681b6a85fec8d89";
    cfg.managed_provider_targets
        .get_mut("codex")
        .and_then(|targets| targets.get_mut(InstallTarget::LinuxX8664.as_str()))
        .expect("codex linux target")
        .dependencies = vec![dependency_id.to_string()];
    cfg.managed_install_targets.insert(
        dependency_id.to_string(),
        HashMap::from([(
            InstallTarget::LinuxX8664.as_str().to_string(),
            ManagedInstallMetadata {
                package: Some("node-runtime".to_string()),
                version: Some(crate::NODE_VERSION.to_string()),
                artifact_fingerprint: Some(format!(
                    "runtime:node:{}:sha256:{linux_runtime_sha}",
                    crate::NODE_VERSION
                )),
                archive_sha256: Some(linux_runtime_sha.to_string()),
                target: None,
                install_dir_rel: Some("runtimes/node/linux".to_string()),
                bin_dir_rel: Some("runtimes/node/linux/bin".to_string()),
                last_success_at: None,
                last_error: None,
            },
        )]),
    );
    let mut status = installed_status("codex", InstallTarget::LinuxX8664);

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert!(matches!(status.health, ProviderHealth::Ok));
    assert!(
        !status
            .details
            .contains_key("managed_dependency_update_available"),
        "matching linux runtime metadata with missing target must not be compared as host"
    );
    assert!(
        !status.details.contains_key("matrix_update_available"),
        "matching implicit runtime dependency should not mark matrix update available"
    );
}

#[tokio::test]
async fn provider_status_matrix_marks_hybrid_container_archive_updates_available() {
    let old_sha = sha256_hex(b"old-container-archive");
    let new_sha = sha256_hex(b"new-container-archive");
    let mut entry = codex_npm_entry(vec![
        release("1.0.0", ProviderReleaseStatus::Supported, None),
        release("1.0.1", ProviderReleaseStatus::Supported, None),
    ]);
    if let Some(ProviderInstall::Npm { targets, .. }) = entry.managed_install.as_mut() {
        targets.insert(
            "linux-x86_64".to_string(),
            ProviderArchiveTarget {
                url: "https://example.invalid/codex-container.tar.gz".to_string(),
                sha256: Some(new_sha.clone()),
                size_bytes: None,
                archive: ProviderArchiveKind::TarGz,
                bin_path: "codex-crp".to_string(),
            },
        );
    }
    let cfg = managed_hybrid_npm_container_cfg("1.0.0", &old_sha);
    let mut status = installed_status("codex", InstallTarget::Container);

    apply_matrix_to_status(
        Path::new("/tmp"),
        &cfg,
        &entry,
        &mut status,
        CURRENT_CTX_VERSION,
    )
    .await;

    assert_eq!(status.version.as_deref(), Some("1.0.0"));
    assert!(matches!(status.health, ProviderHealth::Ok));
    assert_eq!(
        status
            .details
            .get("matrix_recommended_version")
            .map(String::as_str),
        Some("1.0.1")
    );
    assert_eq!(
        status
            .details
            .get("matrix_update_available")
            .map(String::as_str),
        Some("true")
    );
}

#[tokio::test]
async fn provider_status_matrix_marks_latest_ctx_incompatible_release_as_requires_ctx_update_only()
{
    let entry = codex_npm_entry(vec![
        release("1.2.2", ProviderReleaseStatus::Supported, None),
        release("1.2.3", ProviderReleaseStatus::Supported, Some("0.99.0")),
    ]);
    let cfg = managed_npm_cfg("1.2.2");
    let mut status = installed_status("codex", InstallTarget::Host);

    apply_matrix_to_status(
        Path::new("/tmp"),
        &cfg,
        &entry,
        &mut status,
        CURRENT_CTX_VERSION,
    )
    .await;

    assert_eq!(status.version.as_deref(), Some("1.2.2"));
    assert!(matches!(status.health, ProviderHealth::Ok));
    assert_eq!(
        status
            .details
            .get("matrix_recommended_version")
            .map(String::as_str),
        Some("1.2.2")
    );
    assert_eq!(
        status
            .details
            .get("matrix_latest_version")
            .map(String::as_str),
        Some("1.2.3")
    );
    assert_eq!(
        status
            .details
            .get("matrix_update_requires_context")
            .map(String::as_str),
        Some("true")
    );
    assert!(!status.details.contains_key("matrix_update_available"));
}

#[tokio::test]
async fn provider_status_matrix_marks_blocked_installed_release_unsupported() {
    let entry = codex_npm_entry(vec![
        release("1.2.2", ProviderReleaseStatus::Blocked, None),
        release("1.2.3", ProviderReleaseStatus::Supported, None),
    ]);
    let cfg = managed_npm_cfg("1.2.2");
    let mut status = installed_status("codex", InstallTarget::Host);

    apply_matrix_to_status(
        Path::new("/tmp"),
        &cfg,
        &entry,
        &mut status,
        CURRENT_CTX_VERSION,
    )
    .await;

    assert_eq!(status.version.as_deref(), Some("1.2.2"));
    assert!(matches!(status.health, ProviderHealth::UnsupportedVersion));
    assert_eq!(
        status
            .details
            .get("matrix_update_available")
            .map(String::as_str),
        Some("true")
    );
    assert!(status
        .diagnostics
        .iter()
        .any(|message| message.contains("blocked by the support matrix")));
}
