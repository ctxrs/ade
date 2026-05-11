use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserStreamAuthScope {
    WorkspaceActiveSnapshot { workspace_id: String },
    WorkspaceStream { workspace_id: String },
    WorkspaceVcs { workspace_id: String },
    ExecutionLaunch { job_id: String },
    DictationLivekit,
    ProviderInstall { install_id: String },
}

impl BrowserStreamAuthScope {
    pub(super) fn serialize(&self) -> String {
        match self {
            Self::WorkspaceActiveSnapshot { workspace_id } => {
                format!("workspace_active_snapshot:{workspace_id}")
            }
            Self::WorkspaceStream { workspace_id } => format!("workspace_stream:{workspace_id}"),
            Self::WorkspaceVcs { workspace_id } => format!("workspace_vcs:{workspace_id}"),
            Self::ExecutionLaunch { job_id } => format!("execution_launch:{job_id}"),
            Self::DictationLivekit => "dictation_livekit".to_string(),
            Self::ProviderInstall { install_id } => format!("provider_install:{install_id}"),
        }
    }
}

pub(super) fn browser_stream_scope(req: &Request<Body>) -> Option<BrowserStreamAuthScope> {
    let path = req.uri().path();
    if let Some(scope) = workspace_stream_scope(path) {
        return Some(scope);
    }
    if let Some(scope) = provider_install_stream_scope(path) {
        return Some(scope);
    }
    match path {
        "/api/execution/launch/stream" => {
            let job_id = query_param(req, "job_id")?;
            let job_id = job_id.trim();
            if job_id.is_empty() {
                return None;
            }
            Some(BrowserStreamAuthScope::ExecutionLaunch {
                job_id: job_id.to_string(),
            })
        }
        "/api/dictation/livekit/stream" => Some(BrowserStreamAuthScope::DictationLivekit),
        _ => None,
    }
}

fn workspace_stream_scope(path: &str) -> Option<BrowserStreamAuthScope> {
    let remainder = path.strip_prefix("/api/workspaces/")?;
    let (workspace_id, suffix) = remainder.split_once('/')?;
    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        return None;
    }
    match suffix {
        "active_snapshot/stream" => Some(BrowserStreamAuthScope::WorkspaceActiveSnapshot {
            workspace_id: workspace_id.to_string(),
        }),
        "stream" => Some(BrowserStreamAuthScope::WorkspaceStream {
            workspace_id: workspace_id.to_string(),
        }),
        "vcs/stream" => Some(BrowserStreamAuthScope::WorkspaceVcs {
            workspace_id: workspace_id.to_string(),
        }),
        _ => None,
    }
}

fn provider_install_stream_scope(path: &str) -> Option<BrowserStreamAuthScope> {
    let remainder = path.strip_prefix("/api/providers/install/")?;
    let (install_id, suffix) = remainder.split_once('/')?;
    let install_id = install_id.trim();
    if install_id.is_empty() || suffix != "stream" {
        return None;
    }
    Some(BrowserStreamAuthScope::ProviderInstall {
        install_id: install_id.to_string(),
    })
}
