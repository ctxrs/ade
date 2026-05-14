use std::path::{Path, PathBuf};

use tokio::fs;

use crate::daemon::DaemonState;
use ctx_core::models::Artifact;
use ctx_session_tools::{NormalizedToolEvent, ToolOutputArtifactRef};

#[cfg(test)]
mod tests;

pub(super) struct ToolOutputArtifactScope {
    pub(super) session_id: ctx_core::ids::SessionId,
    pub(super) task_id: ctx_core::ids::TaskId,
    pub(super) workspace_id: ctx_core::ids::WorkspaceId,
    pub(super) worktree_id: ctx_core::ids::WorktreeId,
    pub(super) turn_id: ctx_core::ids::TurnId,
}

pub(super) fn cwd_outside_worktree(
    cwd: &str,
    workdir_root: &Path,
    workdir_canonical: Option<&PathBuf>,
) -> bool {
    if cwd.trim().is_empty() {
        return false;
    }
    let cwd_path = Path::new(cwd);
    if cwd_path.is_relative() {
        return false;
    }
    if cwd_path.starts_with(workdir_root) {
        return false;
    }
    if let Some(root) = workdir_canonical {
        if cwd_path.starts_with(root) {
            return false;
        }
    }
    true
}

pub(super) async fn maybe_spool_tool_output(
    state: &DaemonState,
    store: &ctx_store::Store,
    tool_event: &NormalizedToolEvent,
    scope: ToolOutputArtifactScope,
) -> Option<ToolOutputArtifactRef> {
    if !state.core.tool_output_spool_enabled {
        return None;
    }
    let tool_call_id = tool_event.tool_call_id.as_deref()?;
    let output = tool_event.raw_output_text.as_deref()?;
    if output.trim().is_empty() {
        return None;
    }
    if !tool_event
        .output_preview
        .as_ref()
        .is_some_and(|preview| preview.truncated)
    {
        return None;
    }

    let dir = state
        .core
        .tool_output_spool_dir
        .join(scope.session_id.0.to_string())
        .join(scope.turn_id.0.to_string());
    if let Err(err) = fs::create_dir_all(&dir).await {
        tracing::warn!(
            "failed to create tool output spool dir {}: {err}",
            dir.to_string_lossy()
        );
        return None;
    }

    let file_name = format!("{}.txt", sanitize_spool_segment(tool_call_id));
    let path = dir.join(file_name);
    if let Err(err) = fs::write(&path, output.as_bytes()).await {
        tracing::warn!(
            "failed to write tool output spool {}: {err}",
            path.to_string_lossy()
        );
        return None;
    }
    let name = format!("tool-output-{}.txt", sanitize_spool_segment(tool_call_id));
    let artifact = Artifact {
        id: ctx_core::ids::ArtifactId::new(),
        session_id: scope.session_id,
        task_id: scope.task_id,
        workspace_id: scope.workspace_id,
        worktree_id: scope.worktree_id,
        name: Some(name.clone()),
        absolute_path: path.to_string_lossy().to_string(),
        mime_type: "text/plain".to_string(),
        bytes: output.len() as i64,
        created_at: chrono::Utc::now(),
        missing: None,
    };
    let artifact = match store.upsert_session_artifact_by_path(&artifact).await {
        Ok(artifact) => artifact,
        Err(err) => {
            tracing::warn!(
                "failed to register tool output artifact {}: {err}",
                path.to_string_lossy()
            );
            return None;
        }
    };

    Some(ToolOutputArtifactRef {
        artifact_id: artifact.id.0.to_string(),
        name: artifact.name,
        mime_type: artifact.mime_type,
        bytes: artifact.bytes,
    })
}

fn sanitize_spool_segment(raw: &str) -> String {
    let mut output: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if output.is_empty() {
        output.push_str("tool_output");
    }
    if output.len() > 80 {
        output.truncate(80);
    }
    output
}
