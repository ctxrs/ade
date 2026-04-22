use super::*;
use crate::installer::{AgentServerCommand, AgentServerConfigFile, ManagedInstallMetadata};
use ctx_provider_install::install_state::InstallTarget;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const CURRENT_CTX_VERSION: Option<&str> = Some("0.59.0-canary.deadbeefcafe");

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

    let previous_bundle_dir = std::env::var("CTX_BUNDLE_DIR").ok();
    let previous_bundle_matrix = std::env::var("CTX_BUNDLE_MATRIX_JSON").ok();
    unsafe {
        std::env::remove_var("CTX_BUNDLE_DIR");
        std::env::remove_var("CTX_BUNDLE_MATRIX_JSON");
    }
    let loaded = load_matrix(data_root).await;
    match previous_bundle_dir {
        Some(value) => unsafe {
            std::env::set_var("CTX_BUNDLE_DIR", value);
        },
        None => unsafe {
            std::env::remove_var("CTX_BUNDLE_DIR");
        },
    }
    match previous_bundle_matrix {
        Some(value) => unsafe {
            std::env::set_var("CTX_BUNDLE_MATRIX_JSON", value);
        },
        None => unsafe {
            std::env::remove_var("CTX_BUNDLE_MATRIX_JSON");
        },
    }
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
            "version": "1.0.0",
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

fn create_gemini_probe_layout(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
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
        .join("bundle")
        .join("gemini.js");
    let core_entry = cli_entry
        .parent()
        .expect("bundle dir")
        .join("core-ctx-test.js");
    let cli_pkg = cli_entry
        .parent()
        .expect("bundle dir")
        .parent()
        .expect("cli root")
        .join("package.json");

    std::fs::create_dir_all(node_bin.parent().expect("node parent")).expect("mkdir node");
    std::fs::create_dir_all(cli_entry.parent().expect("cli parent")).expect("mkdir cli");
    std::fs::write(&node_bin, b"node").expect("write node");
    std::fs::write(&cli_entry, b"cli").expect("write cli");
    std::fs::write(
        &core_entry,
        "export const coreEvents = {}; export const CoreEvent = {}; export const writeToStdout = () => {}; export const writeToStderr = () => {};",
    )
    .expect("write core");
    std::fs::write(
        &cli_pkg,
        r#"{"name":"@google/gemini-cli","version":"0.38.2"}"#,
    )
    .expect("write cli package");

    (node_bin, cli_entry, core_entry, cli_pkg)
}

#[test]
fn probe_node_package_version_uses_explicit_gemini_entrypoint() {
    let temp = tempdir().expect("tempdir");
    let (node_bin, cli_entry, _, _) = create_gemini_probe_layout(temp.path());
    let command = ProviderCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec![cli_entry.to_string_lossy().to_string()],
    };

    let version = probe_node_package_version(&command, "@google/gemini-cli", temp.path());

    assert_eq!(version.as_deref(), Some("0.38.2"));
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
    let (node_bin, _, _, _) = create_gemini_probe_layout(temp.path());
    let command = ProviderCommand {
        command: node_bin.to_string_lossy().to_string(),
        args: vec!["node_modules/@google/gemini-cli/bundle/gemini.js".to_string()],
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
fn builtin_matrix_uses_raw_upstream_cline_acp_runtime() {
    let matrix = builtin_matrix();
    let cline = matrix
        .providers
        .iter()
        .find(|entry| entry.id == "cline")
        .expect("cline entry");

    let command = cline.command.as_ref().expect("cline command");
    assert_eq!(command.command, "cline");
    assert_eq!(command.args, vec!["--acp".to_string()]);

    let managed_install = cline
        .managed_install
        .as_ref()
        .expect("cline managed install");
    match managed_install {
        ProviderInstall::Npm {
            package,
            entrypoint,
            args,
            ..
        } => {
            assert_eq!(package, "cline");
            assert_eq!(entrypoint, "node_modules/cline/dist/cli.mjs");
            assert_eq!(args, &vec!["--acp".to_string()]);
        }
        other => panic!("expected cline npm managed install, got {other:?}"),
    }
}

#[test]
fn builtin_matrix_uses_goose_upstream_acp_archive() {
    let matrix = builtin_matrix();
    let goose = matrix
        .providers
        .iter()
        .find(|entry| entry.id == "goose")
        .expect("goose entry");

    let command = goose.command.as_ref().expect("goose command");
    assert_eq!(command.command, "goose");
    assert_eq!(command.args, vec!["acp".to_string()]);

    let managed_install = goose
        .managed_install
        .as_ref()
        .expect("goose managed install");
    match managed_install {
        ProviderInstall::Archive {
            version,
            args,
            targets,
        } => {
            assert_eq!(version, "1.31.1");
            assert_eq!(args, &vec!["acp".to_string()]);

            let darwin = targets
                .get("darwin-aarch64")
                .expect("goose darwin-aarch64 target");
            assert!(matches!(darwin.archive, ProviderArchiveKind::TarBz2));
            assert_eq!(darwin.bin_path, "goose");
            assert_eq!(
                darwin.url,
                "https://github.com/aaif-goose/goose/releases/download/v1.31.1/goose-aarch64-apple-darwin.tar.bz2"
            );
            assert_eq!(
                darwin.sha256.as_deref(),
                Some("fd7cad6b0405fbea267d6ae3a7e5b17a096a28d33a8019779c242a290ec1e16e")
            );
        }
        other => panic!("expected goose archive managed install, got {other:?}"),
    }

    let release = goose.releases.first().expect("goose release");
    assert_eq!(release.version, "1.31.1");
    assert_eq!(release.upstream_version.as_deref(), Some("1.31.1"));
}

#[test]
fn builtin_matrix_tracks_target_specific_codex_cli_archive_binaries() {
    let matrix = builtin_matrix();
    let codex_cli = matrix
        .providers
        .iter()
        .find(|entry| entry.id == "codex-cli")
        .expect("codex-cli entry");
    let release = codex_cli.releases.first().expect("codex-cli release");

    let managed_install = codex_cli
        .managed_install
        .as_ref()
        .expect("codex-cli managed install");
    match managed_install {
        ProviderInstall::Archive {
            version, targets, ..
        } => {
            assert_eq!(release.version, "rust-v0.121.0");
            assert_eq!(version, &release.version);
            assert_eq!(
                targets
                    .get("darwin-aarch64")
                    .expect("codex-cli darwin-aarch64 target")
                    .bin_path,
                "codex-aarch64-apple-darwin"
            );
            assert_eq!(
                targets
                    .get("darwin-x86_64")
                    .expect("codex-cli darwin-x86_64 target")
                    .bin_path,
                "codex-x86_64-apple-darwin"
            );
            assert_eq!(
                targets
                    .get("linux-aarch64")
                    .expect("codex-cli linux-aarch64 target")
                    .bin_path,
                "codex-aarch64-unknown-linux-gnu"
            );
            assert_eq!(
                targets
                    .get("linux-x86_64")
                    .expect("codex-cli linux-x86_64 target")
                    .bin_path,
                "codex-x86_64-unknown-linux-gnu"
            );
        }
        other => panic!("expected codex-cli archive managed install, got {other:?}"),
    }
}

#[test]
fn builtin_matrix_routes_codex_cli_prerequisite_same_as_provider() {
    let matrix = builtin_matrix();
    let codex = matrix
        .providers
        .iter()
        .find(|entry| entry.id == "codex")
        .expect("codex entry");

    let dependency = codex
        .provider_dependencies
        .iter()
        .find(|dependency| dependency.id == "codex-cli")
        .expect("codex-cli prerequisite");

    assert_eq!(dependency.role, ProviderInstallDependencyRole::Prerequisite);
    assert_eq!(
        dependency.target,
        ProviderInstallDependencyTarget::SameAsProvider
    );
}

#[test]
fn builtin_matrix_uses_upstream_openhands_python_acp_runtime() {
    let matrix = builtin_matrix();
    let openhands = matrix
        .providers
        .iter()
        .find(|entry| entry.id == "openhands")
        .expect("openhands entry");
    let release = openhands.releases.first().expect("openhands release");

    let command = openhands.command.as_ref().expect("openhands command");
    assert_eq!(command.command, "openhands");
    assert_eq!(command.args, vec!["acp".to_string()]);

    let managed_install = openhands
        .managed_install
        .as_ref()
        .expect("openhands managed install");
    match managed_install {
        ProviderInstall::Python {
            package,
            version,
            entrypoint,
            args,
            python_version,
            python_build_tag,
            ..
        } => {
            assert_eq!(package, "openhands");
            assert_eq!(release.version, "1.14.0");
            assert_eq!(version, &release.version);
            assert_eq!(entrypoint, "openhands");
            assert_eq!(args, &vec!["acp".to_string()]);
            assert_eq!(python_version.as_deref(), Some("3.12.13"));
            assert_eq!(python_build_tag.as_deref(), Some("20260303"));
        }
        other => panic!("expected openhands python managed install, got {other:?}"),
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
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(managed_dependency_update_available(
        &cfg,
        &codex_archive_test_entry("0.114.0-ctx.2", &sha256_hex(b"unused")),
        &status,
    ));
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
            artifact_fingerprint: None,
            archive_sha256: None,
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
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(managed_dependency_update_available(
        &cfg,
        &codex_archive_test_entry("0.114.0-ctx.2", &sha256_hex(b"unused")),
        &status,
    ));
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
            artifact_fingerprint: None,
            archive_sha256: None,
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
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(!managed_dependency_update_available(
        &cfg,
        &codex_archive_test_entry("0.114.0-ctx.2", &sha256_hex(b"unused")),
        &status,
    ));
}

#[test]
fn managed_dependency_update_unavailable_for_container_provider_with_matching_dual_node_runtimes() {
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_provider_targets.insert(
        "amp".to_string(),
        HashMap::from([(
            "container".to_string(),
            AgentServerCommand {
                command: "/tmp/amp-acp.js".to_string(),
                args: Vec::new(),
                dependencies: vec![
                    "runtime-node-container".to_string(),
                    "runtime-node-host".to_string(),
                ],
                managed: Some(ManagedInstallMetadata {
                    package: Some("https://example.com/amp.tar.gz".to_string()),
                    version: Some("0.1.2".to_string()),
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
    let expected = crate::installer::expected_managed_dependency_version("runtime-node-host")
        .expect("runtime node version");
    cfg.managed_installs.insert(
        "runtime-node-host".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some(expected.to_string()),
            artifact_fingerprint: Some(format!("runtime:node:{expected}")),
            archive_sha256: None,
            target: Some(InstallTarget::Host),
            install_dir_rel: None,
            bin_dir_rel: None,
            last_success_at: None,
            last_error: None,
        },
    );
    cfg.managed_installs.insert(
        "runtime-node-container".to_string(),
        ManagedInstallMetadata {
            package: Some("node-runtime".to_string()),
            version: Some(expected.to_string()),
            artifact_fingerprint: Some(format!("runtime:node:{expected}")),
            archive_sha256: None,
            target: Some(InstallTarget::Container),
            install_dir_rel: None,
            bin_dir_rel: None,
            last_success_at: None,
            last_error: None,
        },
    );
    let status = ctx_providers::adapters::ProviderStatus {
        provider_id: "amp".to_string(),
        installed: true,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([(
            "install_target".to_string(),
            InstallTarget::Container.as_str().to_string(),
        )]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };
    assert!(!managed_dependency_update_available(
        &cfg,
        &ProviderMatrixEntry {
            id: "amp".to_string(),
            kind: ProviderMatrixEntryKind::Harness,
            display_name: Some("Amp".to_string()),
            tier: Some("tier2".to_string()),
            command: None,
            managed_install: Some(ProviderInstall::Archive {
                version: "0.1.2".to_string(),
                args: Vec::new(),
                targets: HashMap::from([(
                    "linux-aarch64".to_string(),
                    ProviderArchiveTarget {
                        url: "https://example.com/amp.tar.gz".to_string(),
                        sha256: None,
                        size_bytes: None,
                        archive: ProviderArchiveKind::None,
                        bin_path: "dist/bin/amp-acp.js".to_string(),
                    },
                )]),
            }),
            provider_dependencies: Vec::new(),
            dependencies: Vec::new(),
            version_probe: None,
            releases: vec![ProviderRelease {
                version: "0.1.2".to_string(),
                status: ProviderReleaseStatus::Supported,
                upstream_version: Some("0.1.0-fixture".to_string()),
                context_min: None,
                context_max: None,
                notes: None,
                provenance: None,
            }],
        },
        &status,
    ));
}

#[tokio::test]
async fn apply_matrix_to_status_uses_target_scoped_dependency_metadata() {
    let temp = tempdir().expect("tempdir");
    let target = InstallTarget::LinuxX8664;
    let target_key = target.as_str().to_string();
    let provider_sha = sha256_hex(b"provider-archive");
    let dependency_sha = sha256_hex(b"dependency-archive");

    let entry = ProviderMatrixEntry {
        id: "targeted-provider".to_string(),
        kind: ProviderMatrixEntryKind::Harness,
        display_name: Some("Targeted Provider".to_string()),
        tier: Some("tier2".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Archive {
            version: "1.0.0".to_string(),
            args: Vec::new(),
            targets: HashMap::from([(
                target_key.clone(),
                ProviderArchiveTarget {
                    url: "https://example.com/targeted-provider.tar.gz".to_string(),
                    sha256: Some(provider_sha.clone()),
                    size_bytes: None,
                    archive: ProviderArchiveKind::TarGz,
                    bin_path: "targeted-provider".to_string(),
                },
            )]),
        }),
        provider_dependencies: Vec::new(),
        dependencies: vec![ProviderDependency {
            id: "targeted-dependency".to_string(),
            install: DependencyInstall::Archive {
                version: "2.0.0".to_string(),
                targets: HashMap::from([(
                    target_key.clone(),
                    ProviderArchiveTarget {
                        url: "https://example.com/targeted-dependency.tar.gz".to_string(),
                        sha256: Some(dependency_sha.clone()),
                        size_bytes: None,
                        archive: ProviderArchiveKind::TarGz,
                        bin_path: "targeted-dependency".to_string(),
                    },
                )]),
            },
        }],
        version_probe: None,
        releases: vec![ProviderRelease {
            version: "1.0.0".to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: Some("1.0.0".to_string()),
            context_min: None,
            context_max: None,
            notes: None,
            provenance: None,
        }],
    };

    let mut cfg = AgentServerConfigFile::default();
    let provider_meta = ManagedInstallMetadata {
        package: Some("https://example.com/targeted-provider.tar.gz".to_string()),
        version: Some("1.0.0".to_string()),
        artifact_fingerprint: Some(provider_sha.clone()),
        archive_sha256: Some(provider_sha),
        target: Some(target),
        install_dir_rel: Some("providers/agent-servers/targeted-provider/1.0.0".to_string()),
        bin_dir_rel: None,
        last_success_at: None,
        last_error: None,
    };
    cfg.managed_provider_targets.insert(
        "targeted-provider".to_string(),
        HashMap::from([(
            target_key.clone(),
            AgentServerCommand {
                command: "/tmp/targeted-provider".to_string(),
                args: Vec::new(),
                dependencies: vec!["targeted-dependency".to_string()],
                managed: Some(provider_meta.clone()),
            },
        )]),
    );
    cfg.managed_install_targets.insert(
        "targeted-provider".to_string(),
        HashMap::from([(target_key.clone(), provider_meta)]),
    );
    cfg.managed_install_targets.insert(
        "targeted-dependency".to_string(),
        HashMap::from([(
            target_key.clone(),
            ManagedInstallMetadata {
                package: Some("https://example.com/targeted-dependency.tar.gz".to_string()),
                version: Some("2.0.0".to_string()),
                artifact_fingerprint: Some(dependency_sha.clone()),
                archive_sha256: Some(dependency_sha),
                target: Some(target),
                install_dir_rel: Some(
                    "providers/agent-servers/targeted-dependency/2.0.0".to_string(),
                ),
                bin_dir_rel: None,
                last_success_at: None,
                last_error: None,
            },
        )]),
    );

    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "targeted-provider".to_string(),
        installed: true,
        detected_path: Some("/tmp/targeted-provider".to_string()),
        version: Some("1.0.0".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([("install_target".to_string(), target_key)]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert!(!status
        .details
        .contains_key("managed_dependency_update_available"));
    assert!(!status.details.contains_key("matrix_update_available"));
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn codex_archive_test_entry(version: &str, sha256: &str) -> ProviderMatrixEntry {
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
                    url: "https://example.com/codex.tar.gz".to_string(),
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
        releases: vec![ProviderRelease {
            version: version.to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: Some("0.114.0".to_string()),
            context_min: None,
            context_max: None,
            notes: None,
            provenance: None,
        }],
    }
}

fn codex_npm_test_entry(version: &str) -> ProviderMatrixEntry {
    ProviderMatrixEntry {
        id: "codex".to_string(),
        kind: ProviderMatrixEntryKind::Harness,
        display_name: Some("Codex".to_string()),
        tier: Some("tier1".to_string()),
        command: None,
        managed_install: Some(ProviderInstall::Npm {
            package: "@openai/codex".to_string(),
            version: version.to_string(),
            entrypoint: "node_modules/@openai/codex/bin.js".to_string(),
            args: Vec::new(),
            targets: std::collections::HashMap::new(),
        }),
        provider_dependencies: Vec::new(),
        dependencies: Vec::new(),
        version_probe: None,
        releases: vec![ProviderRelease {
            version: version.to_string(),
            status: ProviderReleaseStatus::Supported,
            upstream_version: Some(version.to_string()),
            context_min: None,
            context_max: None,
            notes: None,
            provenance: None,
        }],
    }
}

fn managed_archive_cfg(
    command_path: &Path,
    version: &str,
    installed_sha256: &str,
) -> AgentServerConfigFile {
    let meta = ManagedInstallMetadata {
        package: Some("https://example.com/codex.tar.gz".to_string()),
        version: Some(version.to_string()),
        artifact_fingerprint: Some(installed_sha256.to_string()),
        archive_sha256: Some(installed_sha256.to_string()),
        target: Some(InstallTarget::LinuxX8664),
        install_dir_rel: Some(format!("providers/agent-servers/codex/{version}")),
        bin_dir_rel: None,
        last_success_at: None,
        last_error: None,
    };
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([("linux-x86_64".to_string(), meta.clone())]),
    );
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([(
            "linux-x86_64".to_string(),
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

#[tokio::test]
async fn apply_matrix_to_status_flags_managed_archive_checksum_mismatch() {
    let temp = tempdir().expect("tempdir");
    let runtime = temp.path().join("codex-crp");
    std::fs::write(&runtime, b"runtime-bytes-do-not-matter").expect("write runtime");

    let expected_sha256 = sha256_hex(b"new-codex-runtime");
    let actual_sha256 = sha256_hex(b"previous-archive");
    let entry = codex_archive_test_entry("0.114.0-ctx.2", &expected_sha256);
    let cfg = managed_archive_cfg(&runtime, "0.114.0-ctx.2", &actual_sha256);
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some(runtime.to_string_lossy().to_string()),
        version: None,
        capabilities: Some(ctx_providers::adapters::ProviderCapabilities {
            stream_events: true,
            stream_format: "jsonl".to_string(),
            has_turn_boundaries: true,
            has_tool_call_ids: true,
            has_file_change_events: true,
            has_command_events: true,
            supports_resume: true,
            supports_stable_session_id: true,
            supports_fork_or_rewind: true,
            supports_headless: true,
            supports_server_mode: true,
            supports_interactive_tui: false,
            supports_private_state_dir: true,
            supports_sandbox_flags: true,
            supports_approval_flags: true,
            notes: Vec::new(),
        }),
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([(
            "install_target".to_string(),
            InstallTarget::LinuxX8664.as_str().to_string(),
        )]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert!(!status.installed);
    assert!(status.capabilities.is_none());
    assert!(matches!(
        status.health,
        ctx_providers::adapters::ProviderHealth::Error
    ));
    assert_eq!(
        status
            .details
            .get("managed_checksum_mismatch")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        status
            .details
            .get("managed_expected_archive_sha256")
            .map(String::as_str),
        Some(expected_sha256.as_str())
    );
    assert_eq!(
        status
            .details
            .get("managed_detected_archive_sha256")
            .map(String::as_str),
        Some(actual_sha256.as_str())
    );
    assert!(status
        .diagnostics
        .iter()
        .any(|msg| msg.contains("checksum mismatch")));
}

#[tokio::test]
async fn apply_matrix_to_status_accepts_matching_managed_archive_checksum() {
    let temp = tempdir().expect("tempdir");
    let runtime = temp.path().join("codex-crp");
    std::fs::write(&runtime, b"bridge-or-runtime-bytes-can-differ").expect("write runtime");

    let sha256 = sha256_hex(b"matching-downloaded-archive");
    let entry = codex_archive_test_entry("0.114.0-ctx.2", &sha256);
    let cfg = managed_archive_cfg(&runtime, "0.114.0-ctx.2", &sha256);
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some(runtime.to_string_lossy().to_string()),
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([(
            "install_target".to_string(),
            InstallTarget::LinuxX8664.as_str().to_string(),
        )]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert!(status.installed);
    assert!(!status.details.contains_key("managed_checksum_mismatch"));
    assert!(status
        .diagnostics
        .iter()
        .all(|msg| !msg.contains("checksum mismatch")));
}

#[tokio::test]
async fn apply_matrix_to_status_clears_stale_matrix_update_flags_when_runtime_is_current() {
    let temp = tempdir().expect("tempdir");
    let runtime = temp.path().join("codex-crp");
    std::fs::write(&runtime, b"matching-runtime").expect("write runtime");

    let sha256 = sha256_hex(b"matching-archive");
    let entry = codex_archive_test_entry("0.114.0-ctx.2", &sha256);
    let cfg = managed_archive_cfg(&runtime, "0.114.0-ctx.2", &sha256);
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some(runtime.to_string_lossy().to_string()),
        version: Some("0.114.0-ctx.1".to_string()),
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([
            (
                "install_target".to_string(),
                InstallTarget::LinuxX8664.as_str().to_string(),
            ),
            (
                "managed_dependency_update_available".to_string(),
                "true".to_string(),
            ),
            (
                "managed_fingerprint_mismatch".to_string(),
                "true".to_string(),
            ),
            (
                "matrix_detected_upstream_version".to_string(),
                "0.114.0".to_string(),
            ),
            ("matrix_latest_version".to_string(), "9.9.9".to_string()),
            (
                "matrix_recommended_version".to_string(),
                "9.9.9".to_string(),
            ),
            ("matrix_update_available".to_string(), "true".to_string()),
            (
                "matrix_update_requires_context".to_string(),
                "true".to_string(),
            ),
        ]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert_eq!(status.version.as_deref(), Some("0.114.0-ctx.2"));
    assert_eq!(
        status
            .details
            .get("matrix_recommended_version")
            .map(String::as_str),
        Some("0.114.0-ctx.2")
    );
    assert!(!status
        .details
        .contains_key("managed_dependency_update_available"));
    assert!(!status.details.contains_key("managed_fingerprint_mismatch"));
    assert!(!status.details.contains_key("matrix_update_available"));
    assert!(!status
        .details
        .contains_key("matrix_update_requires_context"));
}

#[tokio::test]
async fn apply_matrix_to_status_marks_stale_installed_provider_as_updateable_for_current_ctx() {
    let temp = tempdir().expect("tempdir");
    let runtime = temp.path().join("codex-crp");
    std::fs::write(&runtime, b"matching-runtime").expect("write runtime");

    let sha_old = sha256_hex(b"old-archive");
    let sha_new = sha256_hex(b"new-archive");
    let mut entry = codex_archive_test_entry("0.114.0-ctx.5", &sha_new);
    entry.releases = vec![ProviderRelease {
        version: "0.114.0-ctx.5".to_string(),
        status: ProviderReleaseStatus::Supported,
        upstream_version: Some("0.114.0".to_string()),
        context_min: Some("0.59.0".to_string()),
        context_max: None,
        notes: None,
        provenance: None,
    }];
    let cfg = managed_archive_cfg(&runtime, "0.114.0-ctx.3", &sha_old);
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some(runtime.to_string_lossy().to_string()),
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([(
            "install_target".to_string(),
            InstallTarget::LinuxX8664.as_str().to_string(),
        )]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert_eq!(
        status
            .details
            .get("matrix_recommended_version")
            .map(String::as_str),
        Some("0.114.0-ctx.5")
    );
    assert_eq!(
        status
            .details
            .get("matrix_update_available")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        status.health,
        ctx_providers::adapters::ProviderHealth::UnsupportedVersion
    );
}

#[tokio::test]
async fn apply_matrix_to_status_marks_out_of_matrix_runtime_as_unsupported() {
    let temp = tempdir().expect("tempdir");
    let runtime = temp.path().join("gemini");
    std::fs::write(&runtime, b"runtime").expect("write runtime");

    let entry = codex_npm_test_entry("1.2.3");
    let mut cfg = AgentServerConfigFile::default();
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([(
            "host".to_string(),
            ManagedInstallMetadata {
                package: Some("@openai/codex".to_string()),
                version: Some("0.9.0".to_string()),
                artifact_fingerprint: Some("npm:@openai/codex@0.9.0".to_string()),
                archive_sha256: None,
                target: Some(InstallTarget::Host),
                install_dir_rel: Some("providers/agent-servers/codex/0.9.0".to_string()),
                bin_dir_rel: Some("providers/agent-servers/codex/0.9.0/bin".to_string()),
                last_success_at: None,
                last_error: None,
            },
        )]),
    );
    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: Some(runtime.to_string_lossy().to_string()),
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([("install_target".to_string(), "host".to_string())]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(temp.path(), &cfg, &entry, &mut status, CURRENT_CTX_VERSION).await;

    assert_eq!(
        status.health,
        ctx_providers::adapters::ProviderHealth::UnsupportedVersion
    );
    assert!(status
        .diagnostics
        .iter()
        .any(|msg| msg.contains("not in the support matrix")));
}

#[tokio::test]
async fn apply_matrix_to_status_flags_missing_npm_artifact_fingerprint() {
    let entry = codex_npm_test_entry("1.2.3");
    let mut cfg = AgentServerConfigFile::default();
    let meta = ManagedInstallMetadata {
        package: Some("@openai/codex".to_string()),
        version: Some("1.2.3".to_string()),
        artifact_fingerprint: None,
        archive_sha256: None,
        target: Some(InstallTarget::Host),
        install_dir_rel: Some("providers/agent-servers/codex/1.2.3".to_string()),
        bin_dir_rel: None,
        last_success_at: None,
        last_error: None,
    };
    cfg.managed_install_targets.insert(
        "codex".to_string(),
        HashMap::from([("host".to_string(), meta.clone())]),
    );
    cfg.managed_provider_targets.insert(
        "codex".to_string(),
        HashMap::from([(
            "host".to_string(),
            AgentServerCommand {
                command: "/tmp/codex".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: Some(meta),
            },
        )]),
    );

    let mut status = ctx_providers::adapters::ProviderStatus {
        provider_id: "codex".to_string(),
        installed: true,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ctx_providers::adapters::ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::from([(
            "install_target".to_string(),
            InstallTarget::Host.as_str().to_string(),
        )]),
        usability: ctx_providers::adapters::ProviderUsability::default(),
    };

    apply_matrix_to_status(
        Path::new("/tmp"),
        &cfg,
        &entry,
        &mut status,
        CURRENT_CTX_VERSION,
    )
    .await;

    assert!(!status.installed);
    assert_eq!(
        status
            .details
            .get("managed_fingerprint_mismatch")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        status
            .details
            .get("managed_detected_fingerprint")
            .map(String::as_str),
        Some("<missing>")
    );
}
