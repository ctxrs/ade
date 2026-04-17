use std::fmt;
use std::path::Path;

use crate::{self as installer, AgentServerConfigFile};
use ctx_provider_matrix as provider_matrix;
use ctx_provider_matrix::{
    ProviderInstallDependencyRole, ProviderInstallDependencyTarget, ProviderMatrix,
    ProviderMatrixEntry,
};
use ctx_provider_install::install_state::InstallTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInstallViabilityIssue {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for ProviderInstallViabilityIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderInstallViabilityIssue {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderInstallDependencyRoleKind {
    Prerequisite,
    Readiness,
}

impl ProviderInstallDependencyRoleKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Prerequisite => "prerequisite",
            Self::Readiness => "readiness",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInstallDependency {
    pub provider_id: String,
    pub role: ProviderInstallDependencyRoleKind,
    pub target: InstallTarget,
    pub satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInstallContract {
    pub resolved_target_key: &'static str,
    pub dependencies: Vec<ProviderInstallDependency>,
}

impl ProviderInstallContract {
    pub fn dependencies_for_role(
        &self,
        role: ProviderInstallDependencyRoleKind,
    ) -> Vec<ProviderInstallDependency> {
        self.dependencies
            .iter()
            .filter(|dependency| dependency.role == role)
            .cloned()
            .collect()
    }
}

fn acp_bridge_missing_issue(
    provider_id: &str,
    target: InstallTarget,
) -> ProviderInstallViabilityIssue {
    ProviderInstallViabilityIssue {
        code: "acp_bridge_missing",
        message: format!(
            "ACP bridge runtime is not viable for target '{}': runtime command is not configured for provider 'acp-crp-bridge' required by provider '{}'",
            target.as_str(),
            provider_id
        ),
    }
}

fn dependency_target(
    target: ProviderInstallDependencyTarget,
    provider_target: InstallTarget,
) -> InstallTarget {
    match target {
        ProviderInstallDependencyTarget::SameAsProvider => provider_target,
        ProviderInstallDependencyTarget::Host => InstallTarget::Host,
        ProviderInstallDependencyTarget::Container => InstallTarget::Container,
        ProviderInstallDependencyTarget::LinuxAarch64 => InstallTarget::LinuxAarch64,
        ProviderInstallDependencyTarget::LinuxX8664 => InstallTarget::LinuxX8664,
    }
}

fn dependency_resolution_issue(
    provider_id: &str,
    dependency_id: &str,
    dependency_target: InstallTarget,
    dependency_role: ProviderInstallDependencyRoleKind,
    code: &'static str,
    detail: impl Into<String>,
) -> ProviderInstallViabilityIssue {
    ProviderInstallViabilityIssue {
        code,
        message: format!(
            "Required {} dependency '{}' is not viable for target '{}' required by provider '{}': {}",
            dependency_role.as_str(),
            dependency_id,
            dependency_target.as_str(),
            provider_id,
            detail.into()
        ),
    }
}

#[derive(Clone, Copy)]
struct DependencyResolutionCodes {
    missing: &'static str,
    invalid: &'static str,
}

fn resolve_dependency_viability(
    cfg: &AgentServerConfigFile,
    matrix: &ProviderMatrix,
    provider_id: &str,
    dependency_id: &str,
    dependency_target: InstallTarget,
    dependency_role: ProviderInstallDependencyRoleKind,
    codes: DependencyResolutionCodes,
) -> Result<ProviderInstallDependency, ProviderInstallViabilityIssue> {
    match installer::resolve_runtime_provider_command_for_target_repairable_managed(
        cfg,
        dependency_id,
        Some(dependency_target),
    ) {
        Ok(Some(_)) => Ok(ProviderInstallDependency {
            provider_id: dependency_id.to_string(),
            role: dependency_role,
            target: dependency_target,
            satisfied: true,
        }),
        Ok(None) => {
            if installer::is_supported_managed_provider_for_target(
                matrix,
                dependency_id,
                dependency_target,
            ) {
                return Ok(ProviderInstallDependency {
                    provider_id: dependency_id.to_string(),
                    role: dependency_role,
                    target: dependency_target,
                    satisfied: false,
                });
            }
            Err(dependency_resolution_issue(
                provider_id,
                dependency_id,
                dependency_target,
                dependency_role,
                codes.missing,
                format!(
                    "runtime command is not configured for provider '{dependency_id}' and ctx cannot managed-install it for that target"
                ),
            ))
        }
        Err(err) => Err(dependency_resolution_issue(
            provider_id,
            dependency_id,
            dependency_target,
            dependency_role,
            codes.invalid,
            err.to_string(),
        )),
    }
}

fn resolve_acp_bridge_dependencies(
    cfg: &AgentServerConfigFile,
    matrix: &ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Result<Vec<ProviderInstallDependency>, ProviderInstallViabilityIssue> {
    match resolve_dependency_viability(
        cfg,
        matrix,
        provider_id,
        "acp-crp-bridge",
        target,
        ProviderInstallDependencyRoleKind::Prerequisite,
        DependencyResolutionCodes {
            missing: "acp_bridge_missing",
            invalid: "acp_bridge_invalid",
        },
    ) {
        Ok(dependency) => {
            if dependency.satisfied {
                Ok(Vec::new())
            } else {
                Ok(vec![dependency])
            }
        }
        Err(err) if err.code == "acp_bridge_missing" => {
            Err(acp_bridge_missing_issue(provider_id, target))
        }
        Err(err) => Err(err),
    }
}

fn resolve_matrix_provider_dependencies(
    cfg: &AgentServerConfigFile,
    matrix: &ProviderMatrix,
    entry: &ProviderMatrixEntry,
    provider_target: InstallTarget,
) -> Result<Vec<ProviderInstallDependency>, ProviderInstallViabilityIssue> {
    entry
        .provider_dependencies
        .iter()
        .map(|dependency| {
            let dependency_target = dependency_target(dependency.target, provider_target);
            let dependency_role = match dependency.role {
                ProviderInstallDependencyRole::Prerequisite => {
                    ProviderInstallDependencyRoleKind::Prerequisite
                }
                ProviderInstallDependencyRole::Readiness => {
                    ProviderInstallDependencyRoleKind::Readiness
                }
            };
            resolve_dependency_viability(
                cfg,
                matrix,
                &entry.id,
                &dependency.id,
                dependency_target,
                dependency_role,
                DependencyResolutionCodes {
                    missing: "dependency_missing",
                    invalid: "dependency_invalid",
                },
            )
        })
        .collect()
}

pub fn resolve_provider_install_contract(
    _data_root: &Path,
    cfg: &AgentServerConfigFile,
    matrix: &ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Result<ProviderInstallContract, ProviderInstallViabilityIssue> {
    if !installer::is_supported_managed_provider_for_target(matrix, provider_id, target) {
        return Err(ProviderInstallViabilityIssue {
            code: "install_target_unsupported",
            message: format!(
                "provider '{}' does not support managed install target '{}'",
                provider_id,
                target.as_str()
            ),
        });
    }

    let resolved_target_key = installer::resolve_matrix_target_key(target).map_err(|err| {
        ProviderInstallViabilityIssue {
            code: "install_target_invalid",
            message: format!(
                "provider '{}' has no valid managed install target mapping for '{}': {err}",
                provider_id,
                target.as_str()
            ),
        }
    })?;

    let entry = provider_matrix::get_entry(matrix, provider_id).ok_or_else(|| {
        ProviderInstallViabilityIssue {
            code: "install_target_unsupported",
            message: format!(
                "provider '{}' does not support managed install target '{}'",
                provider_id,
                target.as_str()
            ),
        }
    })?;

    let mut dependencies = resolve_matrix_provider_dependencies(cfg, matrix, entry, target)?;
    if crate::is_acp_provider_id(provider_id) {
        dependencies.extend(resolve_acp_bridge_dependencies(
            cfg,
            matrix,
            provider_id,
            target,
        )?);
    }

    Ok(ProviderInstallContract {
        resolved_target_key,
        dependencies,
    })
}

pub fn provider_install_viability_issue(
    data_root: &Path,
    cfg: &AgentServerConfigFile,
    matrix: &ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Option<ProviderInstallViabilityIssue> {
    resolve_provider_install_contract(data_root, cfg, matrix, provider_id, target).err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::AgentServerCommand;
    use ctx_provider_matrix::{
        ProviderArchiveKind, ProviderArchiveTarget, ProviderInstall, ProviderMatrixEntry,
        ProviderMatrixEntryKind, ProviderRelease, ProviderReleaseStatus,
    };

    fn matrix_with_entries(entries: Vec<ProviderMatrixEntry>) -> ProviderMatrix {
        ProviderMatrix {
            version: 2,
            generated_at: None,
            providers: entries,
        }
    }

    fn archive_entry(provider_id: &str, kind: ProviderMatrixEntryKind) -> ProviderMatrixEntry {
        let mut targets = HashMap::from([
            (
                "linux-x86_64".to_string(),
                ProviderArchiveTarget {
                    url: "https://example.invalid/provider-x86_64.tar.gz".to_string(),
                    sha256: None,
                    size_bytes: None,
                    archive: ProviderArchiveKind::TarGz,
                    bin_path: "provider".to_string(),
                },
            ),
            (
                "linux-aarch64".to_string(),
                ProviderArchiveTarget {
                    url: "https://example.invalid/provider-aarch64.tar.gz".to_string(),
                    sha256: None,
                    size_bytes: None,
                    archive: ProviderArchiveKind::TarGz,
                    bin_path: "provider".to_string(),
                },
            ),
        ]);
        if let Ok(host_target_key) = installer::resolve_matrix_target_key(InstallTarget::Host) {
            targets.insert(
                host_target_key.to_string(),
                ProviderArchiveTarget {
                    url: "https://example.invalid/provider-host.tar.gz".to_string(),
                    sha256: None,
                    size_bytes: None,
                    archive: ProviderArchiveKind::TarGz,
                    bin_path: "provider".to_string(),
                },
            );
        }
        ProviderMatrixEntry {
            id: provider_id.to_string(),
            kind,
            display_name: None,
            tier: None,
            command: None,
            managed_install: Some(ProviderInstall::Archive {
                version: "1.0.0".to_string(),
                args: Vec::new(),
                targets,
            }),
            provider_dependencies: Vec::new(),
            dependencies: Vec::new(),
            version_probe: None,
            releases: vec![ProviderRelease {
                version: "1.0.0".to_string(),
                status: ProviderReleaseStatus::Supported,
                upstream_version: None,
                provenance: None,
                context_min: None,
                context_max: None,
                notes: None,
            }],
        }
    }

    fn npm_entry(
        provider_id: &str,
        kind: ProviderMatrixEntryKind,
        package: &str,
    ) -> ProviderMatrixEntry {
        ProviderMatrixEntry {
            id: provider_id.to_string(),
            kind,
            display_name: None,
            tier: None,
            command: None,
            managed_install: Some(ProviderInstall::Npm {
                package: package.to_string(),
                entrypoint: "cli.js".to_string(),
                args: Vec::new(),
            }),
            provider_dependencies: Vec::new(),
            dependencies: Vec::new(),
            version_probe: None,
            releases: vec![ProviderRelease {
                version: "1.0.0".to_string(),
                status: ProviderReleaseStatus::Supported,
                upstream_version: None,
                provenance: None,
                context_min: None,
                context_max: None,
                notes: None,
            }],
        }
    }

    #[test]
    fn acp_provider_requires_bridge_runtime() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let err = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![archive_entry(
                "kimi",
                ProviderMatrixEntryKind::Harness,
            )]),
            "kimi",
            InstallTarget::Container,
        )
        .expect_err("missing bridge should block ACP install");
        assert_eq!(
            err,
            ProviderInstallViabilityIssue {
                code: "acp_bridge_missing",
                message: "ACP bridge runtime is not viable for target 'container': runtime command is not configured for provider 'acp-crp-bridge' required by provider 'kimi'".to_string(),
            }
        );
    }

    #[test]
    fn acp_container_install_plans_bridge_prerequisite_when_bridge_is_installable() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                archive_entry("kimi", ProviderMatrixEntryKind::Harness),
                archive_entry("acp-crp-bridge", ProviderMatrixEntryKind::Dependency),
            ]),
            "kimi",
            InstallTarget::Container,
        )
        .expect("missing installable bridge should become a prerequisite");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Prerequisite),
            vec![ProviderInstallDependency {
                provider_id: "acp-crp-bridge".to_string(),
                role: ProviderInstallDependencyRoleKind::Prerequisite,
                target: InstallTarget::Container,
                satisfied: false,
            }]
        );
    }

    #[test]
    fn acp_host_install_plans_bridge_prerequisite_when_bridge_is_installable() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                archive_entry("kimi", ProviderMatrixEntryKind::Harness),
                archive_entry("acp-crp-bridge", ProviderMatrixEntryKind::Dependency),
            ]),
            "kimi",
            InstallTarget::Host,
        )
        .expect("missing installable host bridge should become a prerequisite");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Prerequisite),
            vec![ProviderInstallDependency {
                provider_id: "acp-crp-bridge".to_string(),
                role: ProviderInstallDependencyRoleKind::Prerequisite,
                target: InstallTarget::Host,
                satisfied: false,
            }]
        );
    }

    #[test]
    fn native_provider_does_not_require_bridge_runtime() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![archive_entry(
                "codex",
                ProviderMatrixEntryKind::Harness,
            )]),
            "codex",
            InstallTarget::Container,
        )
        .expect("native provider should be viable without ACP bridge");
        assert!(contract.dependencies.is_empty());
        assert!(matches!(
            contract.resolved_target_key,
            "linux-x86_64" | "linux-aarch64"
        ));
    }

    #[test]
    fn invalid_managed_bridge_runtime_stays_repairable() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut cfg = AgentServerConfigFile::default();
        cfg.managed_provider_targets.insert(
            "acp-crp-bridge".to_string(),
            HashMap::from([(
                "container".to_string(),
                AgentServerCommand {
                    command: "relative-bridge".to_string(),
                    args: Vec::new(),
                    dependencies: Vec::new(),
                    managed: None,
                },
            )]),
        );
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                archive_entry("kimi", ProviderMatrixEntryKind::Harness),
                archive_entry("acp-crp-bridge", ProviderMatrixEntryKind::Dependency),
            ]),
            "kimi",
            InstallTarget::Container,
        )
        .expect("stale managed bridge should stay repairable");
        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Prerequisite),
            vec![ProviderInstallDependency {
                provider_id: "acp-crp-bridge".to_string(),
                role: ProviderInstallDependencyRoleKind::Prerequisite,
                target: InstallTarget::Container,
                satisfied: false,
            }]
        );
    }

    #[test]
    fn invalid_user_override_bridge_runtime_is_reported() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "acp-crp-bridge".to_string(),
            AgentServerCommand {
                command: "relative-bridge".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: None,
            },
        );
        let issue = provider_install_viability_issue(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                archive_entry("kimi", ProviderMatrixEntryKind::Harness),
                archive_entry("acp-crp-bridge", ProviderMatrixEntryKind::Dependency),
            ]),
            "kimi",
            InstallTarget::Container,
        )
        .expect("invalid user bridge override should still be reported");
        assert_eq!(issue.code, "acp_bridge_invalid");
        assert!(issue.message.contains("relative-bridge"));
    }

    #[test]
    fn claude_install_resolves_host_readiness_dependency() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let mut claude = archive_entry("claude-crp", ProviderMatrixEntryKind::Harness);
        claude.provider_dependencies = vec![provider_matrix::ProviderInstallDependency {
            id: "claude-cli".to_string(),
            role: ProviderInstallDependencyRole::Readiness,
            target: ProviderInstallDependencyTarget::Host,
        }];
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                claude,
                npm_entry(
                    "claude-cli",
                    ProviderMatrixEntryKind::Dependency,
                    "@anthropic-ai/claude-code",
                ),
            ]),
            "claude-crp",
            InstallTarget::Container,
        )
        .expect("claude should plan host claude-cli readiness dependency");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Readiness),
            vec![ProviderInstallDependency {
                provider_id: "claude-cli".to_string(),
                role: ProviderInstallDependencyRoleKind::Readiness,
                target: InstallTarget::Host,
                satisfied: false,
            }]
        );
    }

    #[test]
    fn claude_readiness_dependency_is_marked_satisfied_when_configured() {
        let root = tempfile::tempdir().expect("tempdir");
        let script_path = root.path().join("claude");
        std::fs::write(&script_path, "#!/bin/sh\nexit 0\n").expect("write script");
        let mut perms = std::fs::metadata(&script_path)
            .expect("metadata")
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).expect("set perms");
        }
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "claude-cli".to_string(),
            AgentServerCommand {
                command: script_path.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: None,
            },
        );
        let mut claude = archive_entry("claude-crp", ProviderMatrixEntryKind::Harness);
        claude.provider_dependencies = vec![provider_matrix::ProviderInstallDependency {
            id: "claude-cli".to_string(),
            role: ProviderInstallDependencyRole::Readiness,
            target: ProviderInstallDependencyTarget::Host,
        }];
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                claude,
                npm_entry(
                    "claude-cli",
                    ProviderMatrixEntryKind::Dependency,
                    "@anthropic-ai/claude-code",
                ),
            ]),
            "claude-crp",
            InstallTarget::Host,
        )
        .expect("configured claude-cli should satisfy readiness dependency");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Readiness),
            vec![ProviderInstallDependency {
                provider_id: "claude-cli".to_string(),
                role: ProviderInstallDependencyRoleKind::Readiness,
                target: InstallTarget::Host,
                satisfied: true,
            }]
        );
    }

    #[test]
    fn codex_install_resolves_same_target_prerequisite_dependency() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let mut codex = archive_entry("codex", ProviderMatrixEntryKind::Harness);
        codex.provider_dependencies = vec![provider_matrix::ProviderInstallDependency {
            id: "codex-cli".to_string(),
            role: ProviderInstallDependencyRole::Prerequisite,
            target: ProviderInstallDependencyTarget::SameAsProvider,
        }];
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                codex,
                archive_entry("codex-cli", ProviderMatrixEntryKind::Dependency),
            ]),
            "codex",
            InstallTarget::Container,
        )
        .expect("codex should plan container codex-cli prerequisite dependency");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Prerequisite),
            vec![ProviderInstallDependency {
                provider_id: "codex-cli".to_string(),
                role: ProviderInstallDependencyRoleKind::Prerequisite,
                target: InstallTarget::Container,
                satisfied: false,
            }]
        );
    }

    #[test]
    fn codex_prerequisite_dependency_is_marked_satisfied_when_configured() {
        let root = tempfile::tempdir().expect("tempdir");
        let script_path = root.path().join("codex");
        std::fs::write(&script_path, "#!/bin/sh\nexit 0\n").expect("write script");
        let mut perms = std::fs::metadata(&script_path)
            .expect("metadata")
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).expect("set perms");
        }
        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "codex-cli".to_string(),
            AgentServerCommand {
                command: script_path.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: None,
            },
        );
        let mut codex = archive_entry("codex", ProviderMatrixEntryKind::Harness);
        codex.provider_dependencies = vec![provider_matrix::ProviderInstallDependency {
            id: "codex-cli".to_string(),
            role: ProviderInstallDependencyRole::Prerequisite,
            target: ProviderInstallDependencyTarget::SameAsProvider,
        }];
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                codex,
                archive_entry("codex-cli", ProviderMatrixEntryKind::Dependency),
            ]),
            "codex",
            InstallTarget::Host,
        )
        .expect("configured codex-cli should satisfy prerequisite dependency");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Prerequisite),
            vec![ProviderInstallDependency {
                provider_id: "codex-cli".to_string(),
                role: ProviderInstallDependencyRoleKind::Prerequisite,
                target: InstallTarget::Host,
                satisfied: true,
            }]
        );
    }

    #[test]
    fn codex_container_prerequisite_dependency_is_marked_satisfied_when_configured() {
        let root = tempfile::tempdir().expect("tempdir");
        let script_path = root.path().join("codex-linux");
        std::fs::write(&script_path, "#!/bin/sh\nexit 0\n").expect("write script");
        let mut perms = std::fs::metadata(&script_path)
            .expect("metadata")
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).expect("set perms");
        }
        let mut cfg = AgentServerConfigFile::default();
        cfg.managed_provider_targets.insert(
            "codex-cli".to_string(),
            HashMap::from([(
                "container".to_string(),
                AgentServerCommand {
                    command: script_path.to_string_lossy().to_string(),
                    args: Vec::new(),
                    dependencies: Vec::new(),
                    managed: None,
                },
            )]),
        );
        let mut codex = archive_entry("codex", ProviderMatrixEntryKind::Harness);
        codex.provider_dependencies = vec![provider_matrix::ProviderInstallDependency {
            id: "codex-cli".to_string(),
            role: ProviderInstallDependencyRole::Prerequisite,
            target: ProviderInstallDependencyTarget::SameAsProvider,
        }];
        let contract = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_entries(vec![
                codex,
                archive_entry("codex-cli", ProviderMatrixEntryKind::Dependency),
            ]),
            "codex",
            InstallTarget::Container,
        )
        .expect("configured container codex-cli should satisfy prerequisite dependency");

        assert_eq!(
            contract.dependencies_for_role(ProviderInstallDependencyRoleKind::Prerequisite),
            vec![ProviderInstallDependency {
                provider_id: "codex-cli".to_string(),
                role: ProviderInstallDependencyRoleKind::Prerequisite,
                target: InstallTarget::Container,
                satisfied: true,
            }]
        );
    }
}
