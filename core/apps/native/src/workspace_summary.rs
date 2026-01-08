use ctx_core::ids::SessionId;
use ctx_core::models::{SessionStatus, TaskStatus, WorkspaceCatchupSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSummaryStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskSummaryStatus {
    pub fn label(self) -> &'static str {
        match self {
            TaskSummaryStatus::Pending => "Pending",
            TaskSummaryStatus::Running => "Running",
            TaskSummaryStatus::Completed => "Done",
            TaskSummaryStatus::Failed => "Failed",
            TaskSummaryStatus::Cancelled => "Cancelled",
        }
    }

    fn from_task_status(status: &TaskStatus) -> Self {
        match status {
            TaskStatus::Pending => TaskSummaryStatus::Pending,
            TaskStatus::Running => TaskSummaryStatus::Running,
            TaskStatus::Completed => TaskSummaryStatus::Completed,
            TaskStatus::Failed => TaskSummaryStatus::Failed,
            TaskStatus::Cancelled => TaskSummaryStatus::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummaryItem {
    pub title: String,
    pub status: TaskSummaryStatus,
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

pub fn task_summaries(snapshot: &WorkspaceCatchupSnapshot) -> Vec<TaskSummaryItem> {
    snapshot
        .active
        .tasks
        .iter()
        .map(|summary| TaskSummaryItem {
            title: summary.task.title.clone(),
            status: TaskSummaryStatus::from_task_status(&summary.task.status),
        })
        .collect()
}

pub fn session_summaries(snapshot: &WorkspaceCatchupSnapshot) -> Vec<SessionSummaryItem> {
    let mut items = Vec::new();
    for task in &snapshot.active.tasks {
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
        assert_eq!(tasks[0].title, "Fix login");
        assert_eq!(tasks[0].status, TaskSummaryStatus::Running);

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
