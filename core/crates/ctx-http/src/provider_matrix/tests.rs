use super::*;
use crate::installer::{AgentServerCommand, ManagedInstallMetadata};
use tempfile::tempdir;

#[test]
fn parse_version_loose_accepts_two_part_versions() {
    let v = parse_version_loose("0.62").expect("expected version");
    assert_eq!(v.to_string(), "0.62.0");
}

#[test]
fn select_latest_release_prefers_semver() {
    let releases = [
        ProviderRelease {
            version: "0.7.1".to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: None,
            context_min: None,
            context_max: None,
            notes: None,
            provenance: None,
        },
        ProviderRelease {
            version: "0.7.3".to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: None,
            context_min: None,
            context_max: None,
            notes: None,
            provenance: None,
        },
    ];
    let refs = releases.iter().collect::<Vec<_>>();
    let latest = select_latest_release(&refs).expect("latest");
    assert_eq!(latest.version, "0.7.3");
}

#[test]
fn release_matches_context_min() {
    let release = ProviderRelease {
        version: "1.0.0".to_string(),
        status: ProviderReleaseStatus::Supported,
        upstream_version: None,
        context_min: Some("1.2.0".to_string()),
        context_max: None,
        notes: None,
        provenance: None,
    };
    let ctx = Version::parse("1.1.0").ok();
    assert!(!release_matches_context(&release, ctx.as_ref()));
}

#[test]
fn version_matches_suffix_release() {
    assert!(version_matches("1.0.1-cli", "1.0.1"));
    assert!(version_matches("1.0.1", "1.0.1-cli"));
}

#[tokio::test]
async fn load_matrix_returns_cached_when_present() {
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path();

    let cached = ProviderMatrix {
        version: MATRIX_SCHEMA_VERSION,
        generated_at: Some("2026-02-23T00:00:00Z".to_string()),
        providers: vec![ProviderMatrixEntry {
            id: "cached-provider".to_string(),
            kind: ProviderMatrixEntryKind::Harness,
            display_name: Some("Cached Provider".to_string()),
            tier: Some("tier3".to_string()),
            command: None,
            managed_install: None,
            provider_dependencies: vec![],
            dependencies: vec![],
            version_probe: None,
            releases: vec![],
        }],
    };
    save_cached_matrix(data_root, &cached)
        .await
        .expect("save cached matrix");

    let loaded = load_matrix(data_root).await;
    assert_eq!(loaded.version, MATRIX_SCHEMA_VERSION);
    assert_eq!(loaded.providers.len(), 1);
    assert_eq!(loaded.providers[0].id, "cached-provider");
}

#[tokio::test]
async fn load_matrix_returns_builtin_when_cache_missing() {
    let dir = tempdir().expect("tempdir");
    let loaded = load_matrix(dir.path()).await;
    let builtin = builtin_matrix();
    assert_eq!(loaded.version, builtin.version);
    assert_eq!(loaded.providers.len(), builtin.providers.len());
}

#[test]
fn provider_matrix_entry_kind_defaults_to_harness_when_missing_from_json() {
    let entry: ProviderMatrixEntry = serde_json::from_str(
        r#"{
          "id": "example-provider",
          "managed_install": {
            "kind": "npm",
            "package": "example",
            "entrypoint": "bin/example.js",
            "args": []
          },
          "releases": []
        }"#,
    )
    .expect("entry parses");

    assert_eq!(entry.kind, ProviderMatrixEntryKind::Harness);
}

fn create_gemini_probe_layout(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
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
    let cli_pkg = cli_entry
        .parent()
        .expect("cli dist")
        .parent()
        .expect("cli root")
        .join("package.json");

    std::fs::create_dir_all(node_bin.parent().expect("node parent")).expect("mkdir node");
    std::fs::create_dir_all(cli_entry.parent().expect("cli parent")).expect("mkdir cli");
    std::fs::create_dir_all(core_entry.parent().expect("core parent")).expect("mkdir core");
    std::fs::write(&node_bin, b"node").expect("write node");
    std::fs::write(&cli_entry, b"cli").expect("write cli");
    std::fs::write(&core_entry, b"core").expect("write core");
    std::fs::write(
        &cli_pkg,
        r#"{"name":"@google/gemini-cli","version":"0.32.1"}"#,
    )
    .expect("write cli package");

    (node_bin, cli_entry, core_entry)
}

#[test]
fn probe_node_package_version_uses_explicit_gemini_entrypoint() {
    let temp = tempdir().expect("tempdir");
    let (node_bin, cli_entry, _) = create_gemini_probe_layout(temp.path());
    let command = ProviderCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec![cli_entry.to_string_lossy().to_string()],
    };

    let version = probe_node_package_version(&command, "@google/gemini-cli", temp.path());

    assert_eq!(version.as_deref(), Some("0.32.1"));
}

#[test]
fn probe_node_package_version_rejects_path_style_gemini_command() {
    let temp = tempdir().expect("tempdir");
    let command = ProviderCommand {
        command: "gemini".to_string(),
        args: vec!["--experimental-acp".to_string()],
    };

    let version = probe_node_package_version(&command, "@google/gemini-cli", temp.path());

    assert!(version.is_none());
}

#[test]
fn probe_node_package_version_rejects_relative_gemini_entrypoint() {
    let temp = tempdir().expect("tempdir");
    let (node_bin, _, _) = create_gemini_probe_layout(temp.path());
    let command = ProviderCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec!["node_modules/@google/gemini-cli/dist/index.js".to_string()],
    };

    let version = probe_node_package_version(&command, "@google/gemini-cli", temp.path());

    assert!(version.is_none());
}

#[test]
fn builtin_matrix_marks_dependencies_and_omits_cagent() {
    let matrix = builtin_matrix();
    let bridge = get_entry(&matrix, "acp-crp-bridge").expect("bridge entry");
    let claude_cli = get_entry(&matrix, "claude-cli").expect("claude-cli entry");

    assert_eq!(bridge.kind, ProviderMatrixEntryKind::Dependency);
    assert_eq!(claude_cli.kind, ProviderMatrixEntryKind::Dependency);
    assert!(get_entry(&matrix, "cagent").is_none());
}

#[test]
fn builtin_matrix_uses_kimi_acp_subcommand() {
    let matrix = builtin_matrix();
    let kimi = matrix
        .providers
        .iter()
        .find(|entry| entry.id == "kimi")
        .expect("kimi entry");

    let command = kimi.command.as_ref().expect("kimi command");
    assert_eq!(command.command, "kimi");
    assert_eq!(command.args, vec!["acp".to_string()]);

    let managed_install = kimi.managed_install.as_ref().expect("kimi managed install");
    match managed_install {
        ProviderInstall::Python { args, .. } => {
            assert_eq!(args, &vec!["acp".to_string()]);
        }
        other => panic!("expected kimi python managed install, got {other:?}"),
    }
}

#[test]
fn user_facing_harness_filter_excludes_known_dependencies_only() {
    let matrix = builtin_matrix();

    assert!(is_user_facing_harness_id(&matrix, "codex"));
    assert!(!is_user_facing_harness_id(&matrix, "acp-crp-bridge"));
    assert!(!is_user_facing_harness_id(&matrix, "claude-cli"));
    assert!(is_user_facing_harness_id(&matrix, "unknown-provider"));
}

#[test]
fn managed_dependency_update_available_when_runtime_dependency_missing() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex".to_string(),
        AgentServerCommand {
            command: "/tmp/codex".to_string(),
            args: Vec::new(),
            dependencies: vec!["runtime-node-host".to_string()],
            managed: None,
        },
    );
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
    };
    assert!(managed_dependency_update_available(&cfg, &status));
}

#[test]
fn managed_dependency_update_available_when_runtime_dependency_version_mismatched() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex".to_string(),
        AgentServerCommand {
            command: "/tmp/codex".to_string(),
            args: Vec::new(),
            dependencies: vec!["runtime-node-host".to_string()],
            managed: None,
        },
    );
    cfg.managed_installs.insert(
        "runtime-node-host".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some("0.0.1".to_string()),
            target: None,
            install_dir_rel: None,
            bin_dir_rel: None,
            last_success_at: None,
            last_error: None,
        },
    );
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
    };
    assert!(managed_dependency_update_available(&cfg, &status));
}

#[test]
fn managed_dependency_update_unavailable_when_runtime_dependency_matches_expected() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex".to_string(),
        AgentServerCommand {
            command: "/tmp/codex".to_string(),
            args: Vec::new(),
            dependencies: vec!["runtime-node-host".to_string()],
            managed: None,
        },
    );
    let expected = crate::installer::expected_managed_dependency_version("runtime-node-host")
        .expect("runtime node version");
    cfg.managed_installs.insert(
        "runtime-node-host".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some(expected.to_string()),
            target: None,
            install_dir_rel: None,
            bin_dir_rel: None,
            last_success_at: None,
            last_error: None,
        },
    );
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
    };
    assert!(!managed_dependency_update_available(&cfg, &status));
}
