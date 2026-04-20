use super::*;
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

#[tokio::test]
async fn load_matrix_prefers_explicit_bundle_manifest_over_cache() {
    let dir = tempdir().expect("tempdir");
    let bundle_dir = tempdir().expect("bundle tempdir");
    let bundle_matrix = ProviderMatrix {
        version: MATRIX_SCHEMA_VERSION,
        generated_at: Some("2026-04-20T00:00:00Z".to_string()),
        providers: vec![ProviderMatrixEntry {
            id: "bundled-provider".to_string(),
            kind: ProviderMatrixEntryKind::Harness,
            display_name: None,
            tier: None,
            command: None,
            managed_install: None,
            provider_dependencies: vec![],
            dependencies: vec![],
            version_probe: None,
            releases: vec![],
        }],
    };
    let cached = ProviderMatrix {
        version: MATRIX_SCHEMA_VERSION,
        generated_at: Some("2026-04-19T00:00:00Z".to_string()),
        providers: vec![ProviderMatrixEntry {
            id: "cached-provider".to_string(),
            kind: ProviderMatrixEntryKind::Harness,
            display_name: None,
            tier: None,
            command: None,
            managed_install: None,
            provider_dependencies: vec![],
            dependencies: vec![],
            version_probe: None,
            releases: vec![],
        }],
    };
    save_cached_matrix(dir.path(), &cached)
        .await
        .expect("save cached matrix");
    std::fs::write(
        bundle_dir.path().join(MATRIX_CACHE_FILENAME),
        serde_json::to_string_pretty(&bundle_matrix).expect("serialize bundle matrix"),
    )
    .expect("write bundle matrix");

    let previous_bundle_dir = std::env::var("CTX_BUNDLE_DIR").ok();
    unsafe {
        std::env::set_var("CTX_BUNDLE_DIR", bundle_dir.path());
    }
    let loaded = load_matrix(dir.path()).await;
    match previous_bundle_dir {
        Some(value) => unsafe {
            std::env::set_var("CTX_BUNDLE_DIR", value);
        },
        None => unsafe {
            std::env::remove_var("CTX_BUNDLE_DIR");
        },
    }

    assert_eq!(loaded.providers.len(), 1);
    assert_eq!(loaded.providers[0].id, "bundled-provider");
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

    let managed_install = codex_cli
        .managed_install
        .as_ref()
        .expect("codex-cli managed install");
    match managed_install {
        ProviderInstall::Archive {
            version, targets, ..
        } => {
            assert_eq!(version, "rust-v0.121.0");
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
        } => {
            assert_eq!(package, "openhands");
            assert_eq!(version, "1.14.0");
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
