use ctx_core::ids::{SessionId, TaskId};
use ctx_core::models::{
    SessionStatus, Task, WorkspaceCatchupSnapshot, WorkspaceCatchupTrackSummary,
    WorkspaceCatchupTaskSummary,
};

#[derive(Debug, Clone)]
pub struct TaskSummaryItem {
    pub id: TaskId,
    pub task: Task,
    pub tracks: Vec<WorkspaceCatchupTrackSummary>,
    pub sort_at_ms: i64,
}

impl TaskSummaryItem {
    pub fn from_summary(summary: &WorkspaceCatchupTaskSummary) -> Self {
        let sort_at_ms = summary
            .task
            .archived_at
            .unwrap_or(summary.task.created_at)
            .timestamp_millis();
        Self {
            id: summary.task.id,
            task: summary.task.clone(),
            tracks: summary.tracks.clone(),
            sort_at_ms,
        }
    }

    pub fn with_task(&self, task: Task) -> Self {
        let sort_at = task.archived_at.unwrap_or(task.created_at);
        Self {
            id: self.id,
            task,
            tracks: self.tracks.clone(),
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

pub fn catchup_counts(snapshot: &WorkspaceCatchupSnapshot) -> WorkspaceCatchupCounts {
    WorkspaceCatchupCounts {
        active_total: snapshot.active.total_count,
        archived_total: snapshot.archived.as_ref().map(|page| page.total_count),
    }
}

#[cfg(test)]
pub fn task_summaries(snapshot: &WorkspaceCatchupSnapshot) -> Vec<TaskSummaryItem> {
    snapshot
        .active
        .tasks
        .iter()
        .map(TaskSummaryItem::from_summary)
        .collect()
}

#[cfg(test)]
pub fn session_summaries(snapshot: &WorkspaceCatchupSnapshot) -> Vec<SessionSummaryItem> {
    snapshot
        .active
        .tasks
        .iter()
        .flat_map(|task| task_session_summaries(&TaskSummaryItem::from_summary(task)))
        .collect()
}

pub fn task_session_summaries(task: &TaskSummaryItem) -> Vec<SessionSummaryItem> {
    let mut items = Vec::new();
    for track in &task.tracks {
        for session in &track.sessions {
            let title = if session.session.title.is_empty() {
                "Session".to_string()
            } else {
                session.session.title.clone()
            };
            let status = session_status_text(&session.session.status, session.activity.is_working);
            items.push(SessionSummaryItem {
                session_id: session.session.id,
                title,
                status: status.to_string(),
            });
        }
    }
    items
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
                        "tracks": [
                            {
                                "track": {
                                    "id": "00000000-0000-0000-0000-000000000020",
                                    "task_id": "00000000-0000-0000-0000-000000000010",
                                    "workspace_id": "00000000-0000-0000-0000-000000000001",
                                    "worktree_id": "00000000-0000-0000-0000-000000000030",
                                    "label": "main",
                                    "status": "running",
                                    "created_at": "2024-01-01T00:00:00Z",
                                    "updated_at": "2024-01-01T01:00:00Z"
                                },
                                "sessions": [
                                    {
                                        "session": {
                                            "id": "00000000-0000-0000-0000-000000000040",
                                            "track_id": "00000000-0000-0000-0000-000000000020",
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
                                    }
                                ]
                            }
                        ],
                        "sort_at": "2024-01-01T01:00:00Z"
                    }
                ],
                "total_count": 1
            }
        }"#;

        let snapshot: WorkspaceCatchupSnapshot = serde_json::from_str(json).unwrap();
        let counts = catchup_counts(&snapshot);
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
