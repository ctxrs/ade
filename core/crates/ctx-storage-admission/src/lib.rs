use std::fmt;

use serde::Serialize;

const GIB: u64 = 1024 * 1024 * 1024;
const EMERGENCY_FREE_BYTES: u64 = GIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAdmissionOperation {
    DiskIsolatedWorktreeMaterialization,
    DiskIsolatedWorkspaceMaterialization,
}

impl StorageAdmissionOperation {
    fn action_label(self) -> &'static str {
        match self {
            Self::DiskIsolatedWorktreeMaterialization => "creating an isolated task worktree",
            Self::DiskIsolatedWorkspaceMaterialization => "creating an isolated workspace copy",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StorageAdmissionPathStatus {
    pub label: String,
    pub path: String,
    pub mount_point: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageAdmissionSample {
    pub label: String,
    pub path: String,
    pub mount_point: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageAdmissionFailure {
    operation: StorageAdmissionOperation,
    required_bytes: u64,
    active: StorageAdmissionPathStatus,
}

impl StorageAdmissionFailure {
    pub fn operation(&self) -> StorageAdmissionOperation {
        self.operation
    }

    pub fn required_bytes(&self) -> u64 {
        self.required_bytes
    }

    pub fn active(&self) -> &StorageAdmissionPathStatus {
        &self.active
    }
}

impl fmt::Display for StorageAdmissionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            storage_admission_message(self.operation, self.required_bytes, &self.active)
        )
    }
}

impl std::error::Error for StorageAdmissionFailure {}

pub fn storage_admission_required_bytes(estimated_write_bytes: u64) -> u64 {
    estimated_write_bytes.saturating_add(EMERGENCY_FREE_BYTES)
}

pub fn storage_admission_message(
    operation: StorageAdmissionOperation,
    required_bytes: u64,
    active: &StorageAdmissionPathStatus,
) -> String {
    format!(
        "Insufficient storage capacity for {} on {}. CTX needs {} free before starting this operation, but only {} is available. Free space, then retry.",
        operation.action_label(),
        format_path_label(active),
        format_storage_bytes(required_bytes),
        format_storage_bytes(active.free_bytes),
    )
}

pub fn check_storage_admission(
    operation: StorageAdmissionOperation,
    required_bytes: u64,
    samples: &[StorageAdmissionSample],
) -> std::result::Result<(), StorageAdmissionFailure> {
    let mut active: Option<StorageAdmissionPathStatus> = None;
    for sample in samples {
        let path = StorageAdmissionPathStatus {
            label: sample.label.clone(),
            path: sample.path.clone(),
            mount_point: sample.mount_point.clone(),
            free_bytes: sample.free_bytes,
            total_bytes: sample.total_bytes,
        };
        let should_replace = active
            .as_ref()
            .map(|current| path.free_bytes < current.free_bytes)
            .unwrap_or(true);
        if should_replace {
            active = Some(path);
        }
    }

    let Some(active) = active else {
        return Ok(());
    };
    if active.free_bytes >= required_bytes {
        return Ok(());
    }
    Err(StorageAdmissionFailure {
        operation,
        required_bytes,
        active,
    })
}

fn format_storage_bytes(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_path_label(path: &StorageAdmissionPathStatus) -> String {
    if path.label.is_empty() {
        path.path.clone()
    } else {
        format!("{} ({})", path.label, path.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_admission_denial_mentions_task_worktree() {
        let required_bytes = storage_admission_required_bytes(512 * 1024 * 1024);
        let err = check_storage_admission(
            StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization,
            required_bytes,
            &[
                StorageAdmissionSample {
                    label: "CTX data root".to_string(),
                    path: "/ctx-data".to_string(),
                    mount_point: "/".to_string(),
                    free_bytes: 5 * 1024 * 1024 * 1024,
                    total_bytes: 20 * 1024 * 1024 * 1024,
                },
                StorageAdmissionSample {
                    label: "sandbox workspace volume".to_string(),
                    path: "/ctx/ws/worktrees".to_string(),
                    mount_point: "/ctx/ws".to_string(),
                    free_bytes: 256 * 1024 * 1024,
                    total_bytes: 20 * 1024 * 1024 * 1024,
                },
            ],
        )
        .expect_err("low sandbox capacity should deny admission");
        assert_eq!(
            err.operation(),
            StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization
        );
        assert!(err.to_string().contains("isolated task worktree"));
        assert!(err.to_string().contains("sandbox workspace volume"));
    }

    #[test]
    fn storage_admission_denial_mentions_workspace_copy() {
        let required_bytes = storage_admission_required_bytes(2 * 1024 * 1024 * 1024);
        let err = check_storage_admission(
            StorageAdmissionOperation::DiskIsolatedWorkspaceMaterialization,
            required_bytes,
            &[StorageAdmissionSample {
                label: "sandbox workspace volume".to_string(),
                path: "/ctx/ws".to_string(),
                mount_point: "/ctx/ws".to_string(),
                free_bytes: 512 * 1024 * 1024,
                total_bytes: 20 * 1024 * 1024 * 1024,
            }],
        )
        .expect_err("low sandbox capacity should deny admission");
        assert!(err.to_string().contains("isolated workspace copy"));
    }
}

