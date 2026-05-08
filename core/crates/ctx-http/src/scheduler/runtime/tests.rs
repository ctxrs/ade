use super::helpers::strip_emitted_prefix;
use super::provider_env::provider_mode_id_for;
use super::runtime_provider_id_for_session_provider;
use super::turn_start::apply_crp_launch_policy_env_for_control_mode;
use crate::installer;
use crate::installer::{
    ensure_codex_cli_command_env_for_target, AgentServerCommand, AgentServerConfigFile,
    ManagedInstallMetadata,
};
use chrono::Utc;
use ctx_core::provider_policy::{CTX_CRP_LAUNCH_POLICY_ENV, CTX_CRP_LAUNCH_POLICY_FULL};
use ctx_harness_sources::{
    EndpointModelCatalogStatus, HarnessApiShape, HarnessEndpointRecord,
    HarnessEndpointVerificationStatus, HarnessSourceKind, ResolvedHarnessSource,
};
use ctx_provider_accounts::{
    codex_env_for_active_account_with_runtime_root, codex_runtime_home, ensure_codex_account_dir,
    save_codex_registry, CodexAccountEntry, CodexAccountRegistry, CodexEndpointProfile,
    CODEX_CREDENTIAL_KIND_API_KEY,
};
use ctx_provider_install::install_state::InstallTarget;
use ctx_settings_model::ProviderControlMode;
use ctx_workspace_config as workspace_config;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn without(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prev.as_deref() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn returns_full_when_no_emitted() {
    assert_eq!(strip_emitted_prefix("Hello", ""), Some("Hello".to_string()));
}

#[test]
fn full_provider_control_maps_known_full_access_modes() {
    assert_eq!(
        provider_mode_id_for("codex", &ProviderControlMode::Full),
        Some("full-access")
    );
    assert_eq!(
        provider_mode_id_for("claude-crp", &ProviderControlMode::Full),
        Some("bypassPermissions")
    );
    assert_eq!(
        provider_mode_id_for("droid", &ProviderControlMode::Full),
        Some("auto_high")
    );
}

#[test]
fn non_full_provider_control_does_not_force_provider_modes() {
    assert_eq!(
        provider_mode_id_for("droid", &ProviderControlMode::HarnessNative),
        None
    );
    assert_eq!(
        provider_mode_id_for("droid", &ProviderControlMode::CtxEnforced),
        None
    );
}

#[test]
fn full_provider_control_sets_crp_launch_policy_env() {
    let mut provider_env = HashMap::new();

    apply_crp_launch_policy_env_for_control_mode(&mut provider_env, &ProviderControlMode::Full);

    assert_eq!(
        provider_env
            .get(CTX_CRP_LAUNCH_POLICY_ENV)
            .map(String::as_str),
        Some(CTX_CRP_LAUNCH_POLICY_FULL)
    );
}

#[test]
fn non_full_provider_control_removes_spoofed_crp_launch_policy_env() {
    let mut provider_env = HashMap::from([(
        CTX_CRP_LAUNCH_POLICY_ENV.to_string(),
        CTX_CRP_LAUNCH_POLICY_FULL.to_string(),
    )]);

    apply_crp_launch_policy_env_for_control_mode(
        &mut provider_env,
        &ProviderControlMode::CtxEnforced,
    );

    assert!(
        !provider_env.contains_key(CTX_CRP_LAUNCH_POLICY_ENV),
        "daemon must strip externally supplied CRP launch policy when policy is unset"
    );
}

#[test]
fn returns_suffix_when_full_contains_emitted_prefix() {
    let full = "Planning:Done.";
    let emitted = "Planning:";
    assert_eq!(
        strip_emitted_prefix(full, emitted),
        Some("Done.".to_string())
    );
}

#[test]
fn returns_none_when_full_equals_emitted() {
    assert_eq!(strip_emitted_prefix("Same", "Same"), None);
}

#[test]
fn returns_full_when_prefix_does_not_match() {
    assert_eq!(
        strip_emitted_prefix("Hello", "Nope"),
        Some("Hello".to_string())
    );
}

#[test]
fn gemini_bearer_endpoint_keeps_gemini_runtime_provider() {
    let source = ResolvedHarnessSource {
        source_kind: HarnessSourceKind::Endpoint,
        endpoint: Some(HarnessEndpointRecord {
            id: "ep".to_string(),
            provider_id: "gemini".to_string(),
            name: "Gemini Legacy Bearer".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: HarnessApiShape::OpenaiResponses,
            auth_type: "bearer".to_string(),
            model_override: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_verification_status: HarnessEndpointVerificationStatus::Unknown,
            last_verification_at: None,
            last_error: None,
            has_api_key: true,
            model_catalog_status: EndpointModelCatalogStatus::Unknown,
            model_catalog_fetched_at: None,
            model_catalog_error: None,
            model_catalog_models: Vec::new(),
            manual_model_ids: Vec::new(),
            model_catalog_source: None,
        }),
        env: std::collections::HashMap::new(),
    };
    assert_eq!(
        runtime_provider_id_for_session_provider("gemini", &source),
        "gemini"
    );
}

#[test]
fn gemini_subscription_keeps_gemini_runtime_provider() {
    let source = ResolvedHarnessSource {
        source_kind: HarnessSourceKind::Subscription,
        endpoint: None,
        env: std::collections::HashMap::new(),
    };
    assert_eq!(
        runtime_provider_id_for_session_provider("gemini", &source),
        "gemini"
    );
}

#[test]
fn runtime_path_includes_command_parent_before_existing_path() {
    let tmp = tempdir().expect("tempdir");
    let data_root = tmp.path().join("data");
    let provider_bin_dir = tmp.path().join("provider-bin");
    std::fs::create_dir_all(&data_root).expect("data_root");
    std::fs::create_dir_all(&provider_bin_dir).expect("provider_bin_dir");
    let provider_cmd = provider_bin_dir.join("provider-cmd");
    std::fs::write(&provider_cmd, b"#!/bin/sh\n").expect("provider_cmd");

    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "test-provider".to_string(),
        AgentServerCommand {
            command: provider_cmd.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        },
    );

    let mut provider_env = HashMap::new();
    provider_env.insert("PATH".to_string(), "/usr/bin".to_string());
    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut provider_env,
        &cfg,
        "test-provider",
        &data_root,
        None,
    );

    let path_value = provider_env.get("PATH").expect("path");
    let split: Vec<PathBuf> = std::env::split_paths(std::ffi::OsStr::new(path_value)).collect();
    let expected_first =
        std::fs::canonicalize(&provider_bin_dir).expect("canonical provider_bin_dir");
    assert_eq!(split.first().expect("first path"), &expected_first);
}

#[test]
fn runtime_path_includes_dependency_bin_dirs() {
    let tmp = tempdir().expect("tempdir");
    let data_root = tmp.path().join("data");
    let provider_bin_dir = tmp.path().join("provider-bin");
    let managed_bin_rel = "managed/dep/bin";
    let managed_bin_dir = data_root.join(managed_bin_rel);
    std::fs::create_dir_all(&managed_bin_dir).expect("managed_bin_dir");
    std::fs::create_dir_all(&provider_bin_dir).expect("provider_bin_dir");
    let provider_cmd = provider_bin_dir.join("provider-cmd");
    std::fs::write(&provider_cmd, b"#!/bin/sh\n").expect("provider_cmd");

    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "test-provider".to_string(),
        AgentServerCommand {
            command: provider_cmd.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: vec!["dep-node".to_string()],
            managed: None,
        },
    );
    cfg.managed_installs.insert(
        "dep-node".to_string(),
        ManagedInstallMetadata {
            package: None,
            version: None,
            artifact_fingerprint: None,
            archive_sha256: None,
            target: None,
            install_dir_rel: None,
            bin_dir_rel: Some(managed_bin_rel.to_string()),
            last_success_at: None,
            last_error: None,
        },
    );

    let mut provider_env = HashMap::new();
    provider_env.insert("PATH".to_string(), "/usr/bin".to_string());
    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut provider_env,
        &cfg,
        "test-provider",
        &data_root,
        None,
    );

    let path_value = provider_env.get("PATH").expect("path");
    let split: Vec<PathBuf> = std::env::split_paths(std::ffi::OsStr::new(path_value)).collect();
    let expected_first =
        std::fs::canonicalize(&provider_bin_dir).expect("canonical provider_bin_dir");
    assert_eq!(split.first().expect("first path"), &expected_first);
    assert_eq!(split.get(1).expect("second path"), &managed_bin_dir);
}

#[test]
fn runtime_path_includes_target_specific_managed_provider_dependency_bin_dirs() {
    let tmp = tempdir().expect("tempdir");
    let data_root = tmp.path().join("data");
    let provider_bin_dir = tmp.path().join("provider-bin");
    let dependency_bin_dir = tmp.path().join("dependency-bin");
    std::fs::create_dir_all(&data_root).expect("data_root");
    std::fs::create_dir_all(&provider_bin_dir).expect("provider_bin_dir");
    std::fs::create_dir_all(&dependency_bin_dir).expect("dependency_bin_dir");
    let provider_cmd = provider_bin_dir.join("provider-cmd");
    let dependency_cmd = dependency_bin_dir.join("codex");
    std::fs::write(&provider_cmd, b"#!/bin/sh\n").expect("provider_cmd");
    std::fs::write(&dependency_cmd, b"#!/bin/sh\n").expect("dependency_cmd");

    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_provider_targets.insert(
        "test-provider".to_string(),
        HashMap::from([(
            InstallTarget::Container.as_str().to_string(),
            AgentServerCommand {
                command: provider_cmd.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: vec!["dep-provider".to_string()],
                managed: Some(ManagedInstallMetadata {
                    package: None,
                    version: None,
                    artifact_fingerprint: None,
                    archive_sha256: None,
                    target: Some(InstallTarget::Container),
                    install_dir_rel: None,
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                }),
            },
        )]),
    );
    cfg.managed_provider_targets.insert(
        "dep-provider".to_string(),
        HashMap::from([(
            InstallTarget::Container.as_str().to_string(),
            AgentServerCommand {
                command: dependency_cmd.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(ManagedInstallMetadata {
                    package: None,
                    version: None,
                    artifact_fingerprint: None,
                    archive_sha256: None,
                    target: Some(InstallTarget::Container),
                    install_dir_rel: None,
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                }),
            },
        )]),
    );

    let mut provider_env = HashMap::new();
    provider_env.insert("PATH".to_string(), "/usr/bin".to_string());
    provider_env.insert(
        ctx_harness_runtime::CTX_HARNESS_LINUX_SANDBOX_ENV.to_string(),
        "1".to_string(),
    );
    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut provider_env,
        &cfg,
        "test-provider",
        &data_root,
        Some(InstallTarget::Container),
    );

    let path_value = provider_env.get("PATH").expect("path");
    let split: Vec<PathBuf> = std::env::split_paths(std::ffi::OsStr::new(path_value)).collect();
    let expected_first =
        std::fs::canonicalize(&provider_bin_dir).expect("canonical provider_bin_dir");
    let expected_second =
        std::fs::canonicalize(&dependency_bin_dir).expect("canonical dependency_bin_dir");
    assert_eq!(split.first().expect("first path"), &expected_first);
    assert_eq!(split.get(1).expect("second path"), &expected_second);
}

#[tokio::test]
async fn load_system_prompt_append_for_subagent_combines_agent_and_subagent_append() {
    let temp = tempdir().expect("tempdir");
    let db_path = temp.path().join("workspace.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None)
        .await
        .expect("open workspace store");
    workspace_config::update_agent_system_prompt_append(&store, Some("Agent prompt".to_string()))
        .await
        .expect("save agent prompt append");
    workspace_config::update_subagent_system_prompt_append(
        &store,
        Some("Subagent prompt".to_string()),
    )
    .await
    .expect("save subagent prompt append");

    let append =
        super::helpers::load_system_prompt_append_for_relationship(&store, Some("sub_agent"))
            .await
            .expect("load combined prompt append");
    assert_eq!(append, Some("Agent prompt\n\nSubagent prompt".to_string()));

    store.close().await;
}

#[tokio::test]
async fn load_system_prompt_append_fails_closed_on_invalid_workspace_settings_doc() {
    let temp = tempdir().expect("tempdir");
    let db_path = temp.path().join("workspace.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None)
        .await
        .expect("open workspace store");
    store
        .upsert_runtime_settings_document(1, "{ not valid json")
        .await
        .expect("write invalid workspace settings");

    let err = super::helpers::load_system_prompt_append_for_relationship(&store, None)
        .await
        .expect_err("invalid workspace settings should fail closed");
    let message = format!("{err:#}");
    assert!(
        message.contains("loading agent system prompt append config"),
        "expected loader context in error: {message}"
    );
    assert!(
        message.contains("parsing workspace runtime settings document"),
        "expected parse context in error: {message}"
    );

    store.close().await;
}

#[test]
fn codex_env_injects_target_specific_codex_cli_command_path() {
    let tmp = tempdir().expect("tempdir");
    let host_codex = tmp.path().join("codex-host");
    let container_codex = tmp.path().join("codex-container");
    std::fs::write(&host_codex, b"#!/bin/sh\n").expect("write host codex");
    std::fs::write(&container_codex, b"#!/bin/sh\n").expect("write container codex");

    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_provider_targets.insert(
        "codex-cli".to_string(),
        HashMap::from([
            (
                InstallTarget::Host.as_str().to_string(),
                AgentServerCommand {
                    command: host_codex.to_string_lossy().to_string(),
                    args: Vec::new(),
                    dependencies: Vec::new(),
                    managed: Some(ManagedInstallMetadata {
                        package: None,
                        version: None,
                        artifact_fingerprint: None,
                        archive_sha256: None,
                        target: Some(InstallTarget::Host),
                        install_dir_rel: None,
                        bin_dir_rel: None,
                        last_success_at: None,
                        last_error: None,
                    }),
                },
            ),
            (
                InstallTarget::Container.as_str().to_string(),
                AgentServerCommand {
                    command: container_codex.to_string_lossy().to_string(),
                    args: Vec::new(),
                    dependencies: Vec::new(),
                    managed: Some(ManagedInstallMetadata {
                        package: None,
                        version: None,
                        artifact_fingerprint: None,
                        archive_sha256: None,
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

    let mut provider_env = HashMap::new();
    ensure_codex_cli_command_env_for_target(
        &mut provider_env,
        &cfg,
        "codex",
        Some(InstallTarget::Container),
    )
    .expect("inject codex env");

    assert_eq!(
        provider_env.get("CTX_CODEX_BIN_PATH"),
        Some(
            &std::fs::canonicalize(&container_codex)
                .expect("canonicalize container codex")
                .to_string_lossy()
                .to_string()
        )
    );
}

#[test]
fn codex_env_preserves_existing_explicit_codex_bin_path() {
    let tmp = tempdir().expect("tempdir");
    let codex_bin = tmp.path().join("codex");
    std::fs::write(&codex_bin, b"#!/bin/sh\n").expect("write codex");
    let expected = std::fs::canonicalize(&codex_bin)
        .expect("canonicalize codex")
        .to_string_lossy()
        .to_string();
    let mut provider_env = HashMap::from([(
        "CTX_CODEX_BIN_PATH".to_string(),
        codex_bin.to_string_lossy().to_string(),
    )]);
    ensure_codex_cli_command_env_for_target(
        &mut provider_env,
        &AgentServerConfigFile::default(),
        "codex",
        Some(InstallTarget::Host),
    )
    .expect("preserve existing codex path");
    assert_eq!(
        provider_env.get("CTX_CODEX_BIN_PATH").map(String::as_str),
        Some(expected.as_str())
    );
}

#[test]
fn codex_env_rejects_relative_explicit_codex_bin_path() {
    let mut provider_env = HashMap::from([("CTX_CODEX_BIN_PATH".to_string(), "codex".to_string())]);
    let err = ensure_codex_cli_command_env_for_target(
        &mut provider_env,
        &AgentServerConfigFile::default(),
        "codex",
        Some(InstallTarget::Host),
    )
    .expect_err("relative codex path should fail");
    assert!(
        err.to_string()
            .contains("CTX_CODEX_BIN_PATH must be an absolute path"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn codex_subscription_env_for_sandbox_uses_runtime_root_projection() {
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let data_dir = tempdir().expect("data dir");
    let runtime_dir = tempdir().expect("runtime dir");
    let root = data_dir.path();
    let runtime_root = runtime_dir.path();

    let host_home = codex_runtime_home(root);
    std::fs::create_dir_all(&host_home).expect("host codex home");
    std::fs::write(
        host_home.join("auth.json"),
        br#"{"OPENAI_API_KEY":"stale-host-key"}"#,
    )
    .expect("write stale host auth");

    let registry = CodexAccountRegistry {
        active_account_id: Some("acct-123".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-123".to_string(),
            label: "Account".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry)
        .await
        .expect("save codex registry");
    let account_dir = ensure_codex_account_dir(root, "acct-123")
        .await
        .expect("account dir");
    tokio::fs::write(
        account_dir.join("auth.json"),
        br#"{"OPENAI_API_KEY":"fresh-active-key"}"#,
    )
    .await
    .expect("write account auth");

    let env = codex_env_for_active_account_with_runtime_root(root, runtime_root)
        .await
        .expect("sandbox codex env");
    let codex_home = env.get("CODEX_HOME").expect("CODEX_HOME");
    assert_eq!(
        Path::new(codex_home),
        codex_runtime_home(runtime_root).as_path(),
        "sandbox launch should use runtime-root Codex home, not host home",
    );

    let projected = tokio::fs::read_to_string(codex_runtime_home(runtime_root).join("auth.json"))
        .await
        .expect("read projected auth");
    assert!(
        projected.contains("fresh-active-key"),
        "sandbox projection should use active account auth: {projected}"
    );
    assert!(
        !projected.contains("stale-host-key"),
        "sandbox projection must not reuse stale host auth: {projected}"
    );
}
