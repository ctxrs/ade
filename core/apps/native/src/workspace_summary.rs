use ctx_core::ids::{SessionId, TaskId};
use ctx_core::models::{
    Session, SessionCatchupSummary, SessionMetadata, SessionSnapshotSummary, SessionStatus, Task,
    WorkspaceActiveSnapshot, WorkspaceActiveTaskSummary, WorkspaceCatchupSnapshot,
    WorkspaceCatchupTaskSummary,
};

#[derive(Debug, Clone)]
pub struct TaskSummaryItem {
    pub id: TaskId,
    pub task: Task,
    pub primary_session: Option<SessionSnapshotSummary>,
    pub sessions: Vec<SessionSnapshotSummary>,
    pub sort_at_ms: i64,
}

impl TaskSummaryItem {
    pub fn from_active(summary: &WorkspaceActiveTaskSummary) -> Self {
        let sort_at_ms = summary.sort_at.timestamp_millis();
        Self {
            id: summary.task.id,
            task: summary.task.clone(),
            primary_session: Some(summary.primary_session.clone()),
            sessions: summary.sessions.clone(),
            sort_at_ms,
        }
    }

    pub fn from_archived(summary: &WorkspaceCatchupTaskSummary) -> Self {
        let sort_at_ms = summary
            .task
            .archived_at
            .unwrap_or(summary.task.created_at)
            .timestamp_millis();
        let (primary_session, sessions) = archived_session_summaries(summary);
        Self {
            id: summary.task.id,
            task: summary.task.clone(),
            primary_session,
            sessions,
            sort_at_ms,
        }
    }

    pub fn with_task(&self, task: Task) -> Self {
        let sort_at = task.archived_at.unwrap_or(task.created_at);
        Self {
            id: self.id,
            task,
            primary_session: self.primary_session.clone(),
            sessions: self.sessions.clone(),
            sort_at_ms: sort_at.timestamp_millis(),
        }
    }

    pub fn is_archived(&self) -> bool {
        self.task.archived_at.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummaryItem {
    pub session_id: SessionId,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCatchupCounts {
    pub active_total: i64,
    pub archived_total: Option<i64>,
}

pub fn catchup_counts(
    snapshot: &WorkspaceActiveSnapshot,
    archived: Option<&WorkspaceCatchupSnapshot>,
) -> WorkspaceCatchupCounts {
    WorkspaceCatchupCounts {
        active_total: snapshot.active.total_count,
        archived_total: archived.and_then(|page| page.archived.as_ref().map(|page| page.total_count)),
    }
}

#[cfg(test)]
pub fn task_summaries(snapshot: &WorkspaceActiveSnapshot) -> Vec<TaskSummaryItem> {
    snapshot
        .active
        .tasks
        .iter()
        .map(TaskSummaryItem::from_active)
        .collect()
}

#[cfg(test)]
pub fn session_summaries(snapshot: &WorkspaceActiveSnapshot) -> Vec<SessionSummaryItem> {
    snapshot
        .active
        .tasks
        .iter()
        .flat_map(|task| task_session_summaries(&TaskSummaryItem::from_active(task)))
        .collect()
}

pub fn task_session_summaries(task: &TaskSummaryItem) -> Vec<SessionSummaryItem> {
    let mut items = Vec::new();
    let mut seen_primary = None;
    if let Some(primary) = &task.primary_session {
        items.push(session_summary_item(primary));
        seen_primary = Some(primary.session.id);
    }
    for session in &task.sessions {
        if Some(session.session.id) == seen_primary {
            continue;
        }
        items.push(session_summary_item(session));
    }
    items
}

fn session_summary_item(summary: &SessionSnapshotSummary) -> SessionSummaryItem {
    let title = if summary.session.title.is_empty() {
        "Session".to_string()
    } else {
        summary.session.title.clone()
    };
    let status = session_status_text(&summary.session.status, summary.activity.is_working);
    SessionSummaryItem {
        session_id: summary.session.id,
        title,
        status: status.to_string(),
    }
}

fn session_status_text(status: &SessionStatus, is_working: bool) -> &'static str {
    if is_working {
        "Working"
    } else {
        session_status_label(status)
    }
}

fn session_status_label(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "Active",
        SessionStatus::Completed => "Completed",
        SessionStatus::Failed => "Failed",
        SessionStatus::Cancelled => "Cancelled",
    }
}

fn session_metadata_from_session(session: &Session) -> SessionMetadata {
    SessionMetadata {
        id: session.id,
        task_id: session.task_id,
        workspace_id: session.workspace_id,
        worktree_id: session.worktree_id,
        parent_session_id: session.parent_session_id,
        relationship: session.relationship.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        title: session.title.clone(),
        agent_role: session.agent_role.clone(),
        status: session.status.clone(),
        provider_session_ref: session.provider_session_ref.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

fn session_summary_from_catchup(summary: &SessionCatchupSummary) -> SessionSnapshotSummary {
    SessionSnapshotSummary {
        session: session_metadata_from_session(&summary.session),
        last_message_at: summary.last_message_at,
        last_message_preview: summary.last_message_preview.clone(),
        last_event_seq: summary.last_event_seq,
        activity: summary.activity.clone(),
        unread: summary.unread,
    }
}

fn archived_session_summaries(
    summary: &WorkspaceCatchupTaskSummary,
) -> (Option<SessionSnapshotSummary>, Vec<SessionSnapshotSummary>) {
    let mut sessions = Vec::new();
    for track in &summary.tracks {
        for session in &track.sessions {
            sessions.push(session_summary_from_catchup(session));
        }
    }

    let mut primary_id = summary.task.primary_session_id;
    if primary_id.is_none() {
        primary_id = summary.tracks.iter().find_map(|track| track.primary_session_id);
    }

    let mut primary = None;
    if let Some(id) = primary_id {
        primary = sessions.iter().find(|item| item.session.id == id).cloned();
    }
    if primary.is_none() {
        primary = sessions.first().cloned();
    }

    let primary_id = primary.as_ref().map(|item| item.session.id);
    let remaining = sessions
        .into_iter()
        .filter(|item| Some(item.session.id) != primary_id)
        .collect();

    (primary, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_snapshot_lists_and_counts() {
        let json = r#"{
            "workspace_id": "00000000-0000-0000-0000-000000000001",
            "snapshot_rev": 1,
            "active": {
                "tasks": [
                    {
                        "task": {
                            "id": "00000000-0000-0000-0000-000000000010",
                            "workspace_id": "00000000-0000-0000-0000-000000000001",
                            "title": "Fix login",
                            "status": "running",
                            "created_at": "2024-01-01T00:00:00Z",
                            "updated_at": "2024-01-01T01:00:00Z"
                        },
                        "primary_session": {
                            "session": {
                                "id": "00000000-0000-0000-0000-000000000040",
                                "task_id": "00000000-0000-0000-0000-000000000010",
                                "workspace_id": "00000000-0000-0000-0000-000000000001",
                                "worktree_id": "00000000-0000-0000-0000-000000000030",
                                "provider_id": "codex",
                                "model_id": "gpt-5",
                                "title": "Primary",
                                "agent_role": "implementer",
                                "status": "active",
                                "created_at": "2024-01-01T00:00:00Z",
                                "updated_at": "2024-01-01T01:00:00Z"
                            }
                        },
                        "primary_session_head": {
                            "session": {
                                "id": "00000000-0000-0000-0000-000000000040",
                                "task_id": "00000000-0000-0000-0000-000000000010",
                                "workspace_id": "00000000-0000-0000-0000-000000000001",
                                "worktree_id": "00000000-0000-0000-0000-000000000030",
                                "provider_id": "codex",
                                "model_id": "gpt-5",
                                "title": "Primary",
                                "agent_role": "implementer",
                                "status": "active",
                                "created_at": "2024-01-01T00:00:00Z",
                                "updated_at": "2024-01-01T01:00:00Z"
                            },
                            "last_event_seq": 12,
                            "has_more_turns": false
                        },
                        "sessions": [],
                        "sort_at": "2024-01-01T01:00:00Z"
                    }
                ],
                "total_count": 1
            }
        }"#;

        let snapshot: WorkspaceActiveSnapshot = serde_json::from_str(json).unwrap();
        let counts = catchup_counts(&snapshot, None);
        assert_eq!(counts.active_total, 1);
        assert_eq!(counts.archived_total, None);

        let tasks = task_summaries(&snapshot);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task.title, "Fix login");
        assert_eq!(tasks[0].id.0.to_string(), "00000000-0000-0000-0000-000000000010");

        let sessions = session_summaries(&snapshot);
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].session_id.0.to_string(),
            "00000000-0000-0000-0000-000000000040"
        );
        assert_eq!(sessions[0].title, "Primary");
        assert_eq!(sessions[0].status, "Active");
    }
}
