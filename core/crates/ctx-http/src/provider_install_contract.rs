use std::fmt;
use std::path::Path;

use crate::daemon;
use crate::installer::{self, AgentServerConfigFile};
use crate::installs::InstallTarget;
use crate::provider_matrix::ProviderMatrix;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderInstallViabilityIssue {
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
pub(crate) struct ProviderInstallPrerequisite {
    pub provider_id: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderInstallContract {
    pub resolved_target_key: &'static str,
    pub prerequisites: Vec<ProviderInstallPrerequisite>,
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

fn resolve_acp_bridge_prerequisites(
    cfg: &AgentServerConfigFile,
    matrix: &ProviderMatrix,
    provider_id: &str,
    target: InstallTarget,
) -> Result<Vec<ProviderInstallPrerequisite>, ProviderInstallViabilityIssue> {
    match installer::resolve_runtime_provider_command_for_target(
        cfg,
        "acp-crp-bridge",
        Some(target),
    ) {
        Ok(Some(_)) => Ok(Vec::new()),
        Ok(None) => {
            if matches!(target, InstallTarget::Host) {
                return Err(acp_bridge_missing_issue(provider_id, target));
            }
            if installer::is_supported_managed_provider_for_target(matrix, "acp-crp-bridge", target)
            {
                return Ok(vec![ProviderInstallPrerequisite {
                    provider_id: "acp-crp-bridge",
                }]);
            }
            Err(acp_bridge_missing_issue(provider_id, target))
        }
        Err(err) => Err(ProviderInstallViabilityIssue {
            code: "acp_bridge_invalid",
            message: format!(
                "ACP bridge runtime is invalid for target '{}' required by provider '{}': {err}",
                target.as_str(),
                provider_id
            ),
        }),
    }
}

pub(crate) fn resolve_provider_install_contract(
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

    let prerequisites = if daemon::is_acp_provider_id(provider_id) {
        resolve_acp_bridge_prerequisites(cfg, matrix, provider_id, target)?
    } else {
        Vec::new()
    };

    Ok(ProviderInstallContract {
        resolved_target_key,
        prerequisites,
    })
}

pub(crate) fn provider_install_viability_issue(
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
    use std::collections::HashMap;

    use crate::installer::{AgentServerCommand, AgentServerConfigFile};
    use crate::provider_matrix::{
        ProviderArchiveKind, ProviderArchiveTarget, ProviderInstall, ProviderMatrix,
        ProviderMatrixEntry, ProviderRelease, ProviderReleaseStatus,
    };

    use super::{
        provider_install_viability_issue, resolve_provider_install_contract,
        ProviderInstallPrerequisite, ProviderInstallViabilityIssue,
    };

    fn matrix_with_providers(provider_ids: &[&str]) -> ProviderMatrix {
        ProviderMatrix {
            version: 2,
            generated_at: None,
            providers: provider_ids
                .iter()
                .map(|provider_id| ProviderMatrixEntry {
                    id: (*provider_id).to_string(),
                    display_name: None,
                    tier: None,
                    command: None,
                    managed_install: Some(ProviderInstall::Archive {
                        version: "1.0.0".to_string(),
                        args: Vec::new(),
                        targets: HashMap::from([
                            (
                                "linux-x86_64".to_string(),
                                ProviderArchiveTarget {
                                    url: "https://example.invalid/provider-x86_64.tar.gz"
                                        .to_string(),
                                    sha256: None,
                                    size_bytes: None,
                                    archive: ProviderArchiveKind::TarGz,
                                    bin_path: "provider".to_string(),
                                },
                            ),
                            (
                                "linux-aarch64".to_string(),
                                ProviderArchiveTarget {
                                    url: "https://example.invalid/provider-aarch64.tar.gz"
                                        .to_string(),
                                    sha256: None,
                                    size_bytes: None,
                                    archive: ProviderArchiveKind::TarGz,
                                    bin_path: "provider".to_string(),
                                },
                            ),
                        ]),
                    }),
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
                })
                .collect(),
        }
    }

    #[test]
    fn acp_provider_requires_bridge_runtime() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = AgentServerConfigFile::default();
        let err = resolve_provider_install_contract(
            root.path(),
            &cfg,
            &matrix_with_providers(&["kimi"]),
            "kimi",
            crate::installs::InstallTarget::Container,
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
            &matrix_with_providers(&["kimi", "acp-crp-bridge"]),
            "kimi",
            crate::installs::InstallTarget::Container,
        )
        .expect("missing installable bridge should become a prerequisite");

        assert_eq!(
            contract.prerequisites,
            vec![ProviderInstallPrerequisite {
                provider_id: "acp-crp-bridge",
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
            &matrix_with_providers(&["codex"]),
            "codex",
            crate::installs::InstallTarget::Container,
        )
        .expect("native provider should be viable without ACP bridge");
        assert!(matches!(
            contract.resolved_target_key,
            "linux-x86_64" | "linux-aarch64"
        ));
    }

    #[test]
    fn invalid_bridge_runtime_is_reported() {
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
        let issue = provider_install_viability_issue(
            root.path(),
            &cfg,
            &matrix_with_providers(&["kimi", "acp-crp-bridge"]),
            "kimi",
            crate::installs::InstallTarget::Container,
        )
        .expect("invalid bridge should be reported");
        assert_eq!(issue.code, "acp_bridge_invalid");
        assert!(issue.message.contains("relative-bridge"));
    }
}
