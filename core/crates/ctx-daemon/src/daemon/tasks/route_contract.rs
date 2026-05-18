use chrono::{DateTime, Utc};
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Session, SessionStatus, SessionSummary, Task, TaskStatus,
    WorkspaceArchivedPage, WorkspaceIndexCursor, WorkspaceTaskSummary,
};
use ctx_observability::logs;
use serde::{Deserialize, Serialize};

use crate::daemon::{TasksHandle, WorkspaceStoreAccessError};

use super::{
    ArchiveTaskOutcome, CreateTaskInput, CreateTaskSessionInput, TaskCreateError,
    TaskLifecycleError, TaskSessionCreateError,
};

#[derive(Debug)]
pub struct ListWorkspaceTasksRouteParams {
    workspace_id: String,
}

impl ListWorkspaceTasksRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListWorkspaceArchivedTasksRouteRequest {
    limit: Option<u32>,
    cursor_sort_at: Option<String>,
    cursor_task_id: Option<String>,
}

#[derive(Debug)]
pub struct ListWorkspaceArchivedTasksRouteParams {
    workspace_id: String,
    query: ListWorkspaceArchivedTasksRouteRequest,
}

impl ListWorkspaceArchivedTasksRouteParams {
    pub fn new(
        workspace_id: impl Into<String>,
        query: ListWorkspaceArchivedTasksRouteRequest,
    ) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            query,
        }
    }
}

#[derive(Debug)]
pub struct TaskRouteParams {
    task_id: String,
}

impl TaskRouteParams {
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskRouteRequest {
    #[serde(default)]
    id: Option<String>,
    title: String,
    description: Option<String>,
    #[serde(default)]
    default_session: Option<CreateTaskDefaultSessionRouteRequest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskDefaultSessionRouteRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_id")]
    provider_id: String,
    #[serde(deserialize_with = "deserialize_concrete_model_id")]
    model_id: String,
    #[serde(
        default,
        deserialize_with = "ctx_session_tools::model_resolution::deserialize_optional_reasoning_effort"
    )]
    reasoning_effort: Option<String>,
    #[serde(default)]
    remember_model_preference: bool,
    #[serde(default)]
    initial_prompt: Option<String>,
    #[serde(default)]
    initial_message_id: Option<String>,
    #[serde(default)]
    initial_turn_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    execution_environment: Option<ExecutionEnvironmentRouteValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskSessionRouteRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_id")]
    provider_id: String,
    #[serde(deserialize_with = "deserialize_concrete_model_id")]
    model_id: String,
    #[serde(
        default,
        deserialize_with = "ctx_session_tools::model_resolution::deserialize_optional_reasoning_effort"
    )]
    reasoning_effort: Option<String>,
    #[serde(default)]
    remember_model_preference: bool,
    parent_session_id: Option<String>,
    relationship: Option<String>,
    #[serde(default)]
    initial_prompt: Option<String>,
    #[serde(default)]
    initial_message_id: Option<String>,
    #[serde(default)]
    initial_turn_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    execution_environment: Option<ExecutionEnvironmentRouteValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTaskTitleRouteRequest {
    title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskRouteResponse {
    pub id: TaskId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatusRouteResponse,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub exec_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_worktree_id: Option<WorktreeId>,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_seen_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_message_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_active_session: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatusRouteResponse {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRouteResponse {
    pub id: SessionId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub execution_environment: ExecutionEnvironmentRouteValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub title: String,
    pub agent_role: String,
    pub status: SessionStatusRouteResponse,
    pub provider_session_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummaryRouteResponse {
    pub id: SessionId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub execution_environment: ExecutionEnvironmentRouteValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub title: String,
    pub status: SessionStatusRouteResponse,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatusRouteResponse {
    Active,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironmentRouteValue {
    Host,
    Sandbox,
}

impl<'de> Deserialize<'de> for ExecutionEnvironmentRouteValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("host") {
            return Ok(Self::Host);
        }
        if trimmed.eq_ignore_ascii_case("sandbox") || trimmed.starts_with("container_") {
            return Ok(Self::Sandbox);
        }
        Err(serde::de::Error::custom(format!(
            "unknown execution environment: {trimmed}"
        )))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceIndexCursorRouteResponse {
    pub sort_at: DateTime<Utc>,
    pub task_id: TaskId,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceArchivedPageRouteResponse {
    pub workspace_id: WorkspaceId,
    #[serde(default)]
    pub archived_rev: i64,
    pub tasks: Vec<WorkspaceTaskSummaryRouteResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<WorkspaceIndexCursorRouteResponse>,
    pub total_archived: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceTaskSummaryRouteResponse {
    pub task: TaskRouteResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionSummaryRouteResponse>,
    pub sort_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveTaskRouteResponse {
    #[serde(flatten)]
    pub task: TaskRouteResponse,
    pub cleanup_failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRouteErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Forbidden,
    InsufficientStorage,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRouteError {
    kind: TaskRouteErrorKind,
    message: String,
}

impl TaskRouteError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::Conflict,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: TaskRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    fn classified_internal(error: &anyhow::Error, message: impl Into<String>) -> Self {
        Self {
            kind: route_error_kind_for_internal_error(error),
            message: message.into(),
        }
    }

    pub fn kind(&self) -> TaskRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl TasksHandle {
    pub async fn create_task_for_route(
        &self,
        raw_workspace_id: &str,
        req: CreateTaskRouteRequest,
    ) -> Result<TaskRouteResponse, TaskRouteError> {
        let workspace_id = parse_workspace_id(raw_workspace_id)?;
        let input = req.into_create_task_input()?;
        self.create_task_for_workspace(workspace_id, input)
            .await
            .map(TaskRouteResponse::from)
            .map_err(TaskRouteError::from_task_create)
    }

    pub async fn create_session_for_task_route(
        &self,
        params: TaskRouteParams,
        req: CreateTaskSessionRouteRequest,
        run_id_header: Option<String>,
    ) -> Result<SessionRouteResponse, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        self.create_session_for_task(task_id, req.into_task_session_input(run_id_header))
            .await
            .map(SessionRouteResponse::from)
            .map_err(TaskRouteError::from_task_session_create)
    }

    pub async fn list_workspace_tasks_for_route(
        &self,
        params: ListWorkspaceTasksRouteParams,
    ) -> Result<Vec<TaskRouteResponse>, TaskRouteError> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        self.list_workspace_tasks(workspace_id)
            .await
            .map(|tasks| tasks.into_iter().map(TaskRouteResponse::from).collect())
            .map_err(TaskRouteError::from_workspace_store)
    }

    pub async fn list_workspace_archived_page_for_route(
        &self,
        params: ListWorkspaceArchivedTasksRouteParams,
    ) -> Result<WorkspaceArchivedPageRouteResponse, TaskRouteError> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let limit = params.query.limit.unwrap_or(50) as i64;
        let cursor = parse_archived_cursor(
            params.query.cursor_sort_at.as_deref(),
            params.query.cursor_task_id.as_deref(),
        )?;
        self.list_workspace_archived_page(workspace_id, cursor, limit)
            .await
            .map(WorkspaceArchivedPageRouteResponse::from)
            .map_err(TaskRouteError::from_workspace_store)
    }

    pub async fn list_task_sessions_for_route(
        &self,
        params: TaskRouteParams,
    ) -> Result<Vec<SessionRouteResponse>, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        let sessions = self
            .list_task_sessions(task_id)
            .await
            .map_err(|_| TaskRouteError::not_found("task not found"))?
            .ok_or_else(|| TaskRouteError::not_found("task not found"))?;
        Ok(sessions
            .into_iter()
            .map(SessionRouteResponse::from)
            .collect())
    }

    pub async fn mark_task_read_for_route(
        &self,
        params: TaskRouteParams,
    ) -> Result<TaskRouteResponse, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        let task = self
            .mark_task_read(task_id)
            .await
            .map_err(|error| TaskRouteError::internal(error.to_string()))?
            .ok_or_else(|| TaskRouteError::not_found("task not found"))?;
        Ok(TaskRouteResponse::from(task))
    }

    pub async fn mark_task_unread_for_route(
        &self,
        params: TaskRouteParams,
    ) -> Result<TaskRouteResponse, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        let task = self
            .mark_task_unread(task_id)
            .await
            .map_err(|error| TaskRouteError::internal(error.to_string()))?
            .ok_or_else(|| TaskRouteError::not_found("task not found"))?;
        Ok(TaskRouteResponse::from(task))
    }

    pub async fn update_task_title_for_route(
        &self,
        params: TaskRouteParams,
        req: UpdateTaskTitleRouteRequest,
    ) -> Result<TaskRouteResponse, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        let title = req.validated_title()?;
        let task = self
            .update_task_title(task_id, title)
            .await
            .map_err(|error| {
                let message = logs::redact_sensitive(&error.to_string());
                TaskRouteError::classified_internal(&error, message)
            })?
            .ok_or_else(|| TaskRouteError::not_found("task not found"))?;
        Ok(TaskRouteResponse::from(task))
    }

    pub async fn archive_task_for_route(
        &self,
        params: TaskRouteParams,
    ) -> Result<ArchiveTaskRouteResponse, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        self.archive_task(task_id)
            .await
            .map(ArchiveTaskRouteResponse::from)
            .map_err(TaskRouteError::from_task_lifecycle)
    }

    pub async fn unarchive_task_for_route(
        &self,
        params: TaskRouteParams,
    ) -> Result<TaskRouteResponse, TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        self.unarchive_task(task_id)
            .await
            .map(TaskRouteResponse::from)
            .map_err(TaskRouteError::from_task_lifecycle)
    }

    pub async fn delete_task_for_route(
        &self,
        params: TaskRouteParams,
    ) -> Result<(), TaskRouteError> {
        let task_id = parse_task_id(&params.task_id)?;
        self.delete_task(task_id)
            .await
            .map_err(TaskRouteError::from_task_lifecycle)
    }
}

impl TaskRouteError {
    fn from_workspace_store(error: WorkspaceStoreAccessError) -> Self {
        match error {
            WorkspaceStoreAccessError::NotFound => Self::not_found("workspace not found"),
            WorkspaceStoreAccessError::Unavailable(error) => {
                tracing::warn!("workspace store unavailable for task route: {error:#}");
                Self::internal("workspace store unavailable")
            }
        }
    }

    fn from_task_create(error: TaskCreateError) -> Self {
        match error {
            TaskCreateError::BadRequest(error) => Self::bad_request(error),
            TaskCreateError::NotFound(error) => Self::not_found(error),
            TaskCreateError::Conflict(error) => Self::conflict(error),
            TaskCreateError::Internal(error) => {
                let message = logs::redact_sensitive(&error.to_string());
                Self::classified_internal(&error, message)
            }
            TaskCreateError::DefaultSessionFailed(error) => {
                let kind = Self::from_task_session_create(error).kind;
                Self {
                    kind,
                    message: "failed to create default session".to_string(),
                }
            }
            TaskCreateError::DefaultSessionConflict => {
                Self::conflict("task id already exists with a different default session")
            }
        }
    }

    fn from_task_session_create(error: TaskSessionCreateError) -> Self {
        match error {
            TaskSessionCreateError::BadRequest => Self::bad_request("bad request"),
            TaskSessionCreateError::NotFound => Self::not_found("task not found"),
            TaskSessionCreateError::Conflict => Self::conflict("session conflict"),
            TaskSessionCreateError::Internal(error) => {
                tracing::warn!("task session creation failed: {error:#}");
                let message = logs::redact_sensitive(&error.to_string());
                Self::classified_internal(&error, message)
            }
        }
    }

    fn from_task_lifecycle(error: TaskLifecycleError) -> Self {
        match error {
            TaskLifecycleError::NotFound => Self::not_found("task not found"),
            TaskLifecycleError::Internal(error) => {
                tracing::warn!("task lifecycle operation failed: {error:#}");
                Self::classified_internal(&error, "task lifecycle operation failed")
            }
        }
    }
}

impl CreateTaskRouteRequest {
    fn into_create_task_input(self) -> Result<CreateTaskInput, TaskRouteError> {
        let task_id = match self.id.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(raw) => {
                Some(TaskId(uuid::Uuid::parse_str(raw).map_err(|_| {
                    TaskRouteError::bad_request("invalid task id")
                })?))
            }
        };
        Ok(CreateTaskInput {
            task_id,
            title: self.title,
            description: self.description,
            default_session: self
                .default_session
                .map(|default_session| default_session.into_task_session_input(None)),
        })
    }
}

impl CreateTaskDefaultSessionRouteRequest {
    fn into_task_session_input(self, run_id_header: Option<String>) -> CreateTaskSessionInput {
        CreateTaskSessionInput {
            id: self.id,
            provider_id: self.provider_id,
            model_id: self.model_id,
            reasoning_effort: self.reasoning_effort,
            remember_model_preference: self.remember_model_preference,
            parent_session_id: None,
            relationship: None,
            initial_prompt: self.initial_prompt,
            initial_message_id: self.initial_message_id,
            initial_turn_id: self.initial_turn_id,
            worktree_id: self.worktree_id,
            execution_environment: self.execution_environment.map(Into::into),
            run_id_header,
        }
    }
}

impl CreateTaskSessionRouteRequest {
    fn into_task_session_input(self, run_id_header: Option<String>) -> CreateTaskSessionInput {
        CreateTaskSessionInput {
            id: self.id,
            provider_id: self.provider_id,
            model_id: self.model_id,
            reasoning_effort: self.reasoning_effort,
            remember_model_preference: self.remember_model_preference,
            parent_session_id: self.parent_session_id,
            relationship: self.relationship,
            initial_prompt: self.initial_prompt,
            initial_message_id: self.initial_message_id,
            initial_turn_id: self.initial_turn_id,
            worktree_id: self.worktree_id,
            execution_environment: self.execution_environment.map(Into::into),
            run_id_header,
        }
    }
}

impl UpdateTaskTitleRouteRequest {
    fn validated_title(self) -> Result<String, TaskRouteError> {
        let title = self.title.trim().to_string();
        if title.is_empty() {
            return Err(TaskRouteError::bad_request("title is required"));
        }
        if title.len() > 120 {
            return Err(TaskRouteError::bad_request("title is too long"));
        }
        Ok(title)
    }
}

impl From<Task> for TaskRouteResponse {
    fn from(task: Task) -> Self {
        Self {
            id: task.id,
            workspace_id: task.workspace_id,
            title: task.title,
            description: task.description,
            status: task.status.into(),
            created_at: task.created_at,
            updated_at: task.updated_at,
            exec_plan_id: task.exec_plan_id,
            primary_session_id: task.primary_session_id,
            primary_worktree_id: task.primary_worktree_id,
            archived_at: task.archived_at,
            assistant_seen_at: task.assistant_seen_at,
            last_activity_at: task.last_activity_at,
            last_assistant_message_at: task.last_assistant_message_at,
            has_active_session: task.has_active_session,
        }
    }
}

impl From<TaskStatus> for TaskStatusRouteResponse {
    fn from(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Pending => Self::Pending,
            TaskStatus::Running => Self::Running,
            TaskStatus::Completed => Self::Completed,
            TaskStatus::Failed => Self::Failed,
            TaskStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl From<Session> for SessionRouteResponse {
    fn from(session: Session) -> Self {
        Self {
            id: session.id,
            task_id: session.task_id,
            workspace_id: session.workspace_id,
            worktree_id: session.worktree_id,
            execution_environment: session.execution_environment.into(),
            parent_session_id: session.parent_session_id,
            relationship: session.relationship,
            provider_id: session.provider_id,
            model_id: session.model_id,
            reasoning_effort: session.reasoning_effort,
            title: session.title,
            agent_role: session.agent_role,
            status: session.status.into(),
            provider_session_ref: session.provider_session_ref,
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

impl From<SessionSummary> for SessionSummaryRouteResponse {
    fn from(session: SessionSummary) -> Self {
        Self {
            id: session.id,
            task_id: session.task_id,
            workspace_id: session.workspace_id,
            execution_environment: session.execution_environment.into(),
            parent_session_id: session.parent_session_id,
            relationship: session.relationship,
            provider_id: session.provider_id,
            model_id: session.model_id,
            reasoning_effort: session.reasoning_effort,
            title: session.title,
            status: session.status.into(),
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

impl From<SessionStatus> for SessionStatusRouteResponse {
    fn from(status: SessionStatus) -> Self {
        match status {
            SessionStatus::Active => Self::Active,
            SessionStatus::Completed => Self::Completed,
            SessionStatus::Failed => Self::Failed,
            SessionStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl From<ExecutionEnvironment> for ExecutionEnvironmentRouteValue {
    fn from(value: ExecutionEnvironment) -> Self {
        match value {
            ExecutionEnvironment::Host => Self::Host,
            ExecutionEnvironment::Sandbox => Self::Sandbox,
        }
    }
}

impl From<ExecutionEnvironmentRouteValue> for ExecutionEnvironment {
    fn from(value: ExecutionEnvironmentRouteValue) -> Self {
        match value {
            ExecutionEnvironmentRouteValue::Host => Self::Host,
            ExecutionEnvironmentRouteValue::Sandbox => Self::Sandbox,
        }
    }
}

impl From<WorkspaceIndexCursor> for WorkspaceIndexCursorRouteResponse {
    fn from(cursor: WorkspaceIndexCursor) -> Self {
        Self {
            sort_at: cursor.sort_at,
            task_id: cursor.task_id,
        }
    }
}

impl From<WorkspaceTaskSummary> for WorkspaceTaskSummaryRouteResponse {
    fn from(summary: WorkspaceTaskSummary) -> Self {
        Self {
            task: summary.task.into(),
            provider_ids: summary.provider_ids,
            sessions: summary.sessions.into_iter().map(Into::into).collect(),
            sort_at: summary.sort_at,
        }
    }
}

impl From<WorkspaceArchivedPage> for WorkspaceArchivedPageRouteResponse {
    fn from(page: WorkspaceArchivedPage) -> Self {
        Self {
            workspace_id: page.workspace_id,
            archived_rev: page.archived_rev,
            tasks: page.tasks.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor.map(Into::into),
            total_archived: page.total_archived,
        }
    }
}

impl From<ArchiveTaskOutcome> for ArchiveTaskRouteResponse {
    fn from(outcome: ArchiveTaskOutcome) -> Self {
        Self {
            task: outcome.task.into(),
            cleanup_failed: outcome.cleanup_failed,
        }
    }
}

fn parse_workspace_id(value: &str) -> Result<WorkspaceId, TaskRouteError> {
    uuid::Uuid::parse_str(value)
        .map(WorkspaceId)
        .map_err(|_| TaskRouteError::bad_request("invalid workspace id"))
}

fn parse_task_id(value: &str) -> Result<TaskId, TaskRouteError> {
    uuid::Uuid::parse_str(value)
        .map(TaskId)
        .map_err(|_| TaskRouteError::bad_request("invalid task id"))
}

fn parse_archived_cursor(
    cursor_sort_at: Option<&str>,
    cursor_task_id: Option<&str>,
) -> Result<Option<WorkspaceIndexCursor>, TaskRouteError> {
    match (cursor_sort_at, cursor_task_id) {
        (None, None) => Ok(None),
        (Some(sort_at), Some(task_id)) => {
            let sort_at = DateTime::parse_from_rfc3339(sort_at)
                .map_err(|_| TaskRouteError::bad_request("invalid cursor"))?
                .with_timezone(&Utc);
            let task_id = parse_task_id(task_id)?;
            Ok(Some(WorkspaceIndexCursor { sort_at, task_id }))
        }
        _ => Err(TaskRouteError::bad_request("invalid cursor")),
    }
}

fn deserialize_concrete_model_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(serde::de::Error::custom("model_id must not be empty"));
    }
    if trimmed.eq_ignore_ascii_case("default") {
        return Err(serde::de::Error::custom(
            "model_id must be a concrete model id",
        ));
    }
    Ok(trimmed.to_string())
}

fn deserialize_provider_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(serde::de::Error::custom("provider_id must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn route_error_kind_for_internal_error(error: &anyhow::Error) -> TaskRouteErrorKind {
    if ctx_settings_service::is_execution_policy_denial(error) {
        TaskRouteErrorKind::Forbidden
    } else if error
        .chain()
        .any(|cause| ctx_storage_admission::is_storage_exhaustion_error(&cause.to_string()))
    {
        TaskRouteErrorKind::InsufficientStorage
    } else {
        TaskRouteErrorKind::Internal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn now(offset: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 17, 10, offset, 0).unwrap()
    }

    fn task_with_optional_fields(status: TaskStatus) -> Task {
        Task {
            id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            title: "task".to_string(),
            description: Some("description".to_string()),
            status,
            created_at: now(0),
            updated_at: now(1),
            exec_plan_id: Some("plan".to_string()),
            primary_session_id: Some(SessionId::new()),
            primary_worktree_id: Some(WorktreeId::new()),
            archived_at: Some(now(2)),
            assistant_seen_at: Some(now(3)),
            last_activity_at: Some(now(4)),
            last_assistant_message_at: Some(now(5)),
            has_active_session: true,
        }
    }

    fn session_with_optional_fields(status: SessionStatus) -> Session {
        Session {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Sandbox,
            parent_session_id: Some(SessionId::new()),
            relationship: Some("follow_up".to_string()),
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: Some("high".to_string()),
            title: "session".to_string(),
            agent_role: "default".to_string(),
            status,
            provider_session_ref: Some("provider-ref".to_string()),
            created_at: now(0),
            updated_at: now(1),
        }
    }

    fn session_summary_with_optional_fields(status: SessionStatus) -> SessionSummary {
        SessionSummary {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            execution_environment: ExecutionEnvironment::Sandbox,
            parent_session_id: Some(SessionId::new()),
            relationship: Some("follow_up".to_string()),
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: Some("medium".to_string()),
            title: "summary".to_string(),
            status,
            created_at: now(2),
            updated_at: now(3),
        }
    }

    #[test]
    fn task_route_response_matches_raw_task_wire_shape_with_optional_fields() {
        let task = task_with_optional_fields(TaskStatus::Running);

        assert_eq!(
            serde_json::to_value(TaskRouteResponse::from(task.clone())).unwrap(),
            serde_json::to_value(task).unwrap()
        );
    }

    #[test]
    fn task_route_response_matches_raw_task_wire_shape_without_optional_fields() {
        let mut task = task_with_optional_fields(TaskStatus::Completed);
        task.description = None;
        task.exec_plan_id = None;
        task.primary_session_id = None;
        task.primary_worktree_id = None;
        task.archived_at = None;
        task.assistant_seen_at = None;
        task.last_activity_at = None;
        task.last_assistant_message_at = None;
        task.has_active_session = false;

        assert_eq!(
            serde_json::to_value(TaskRouteResponse::from(task.clone())).unwrap(),
            serde_json::to_value(task).unwrap()
        );
    }

    #[test]
    fn session_route_response_matches_raw_session_wire_shape() {
        let session = session_with_optional_fields(SessionStatus::Active);

        assert_eq!(
            serde_json::to_value(SessionRouteResponse::from(session.clone())).unwrap(),
            serde_json::to_value(session).unwrap()
        );
    }

    #[test]
    fn session_route_response_matches_raw_session_wire_shape_without_optional_fields() {
        let mut session = session_with_optional_fields(SessionStatus::Completed);
        session.execution_environment = ExecutionEnvironment::Host;
        session.parent_session_id = None;
        session.relationship = None;
        session.reasoning_effort = None;
        session.provider_session_ref = None;

        assert_eq!(
            serde_json::to_value(SessionRouteResponse::from(session.clone())).unwrap(),
            serde_json::to_value(session).unwrap()
        );
    }

    #[test]
    fn archived_page_response_matches_raw_page_wire_shape() {
        let task = task_with_optional_fields(TaskStatus::Pending);
        let summary = WorkspaceTaskSummary {
            task,
            provider_ids: vec!["fake".to_string()],
            sessions: vec![session_summary_with_optional_fields(SessionStatus::Active)],
            sort_at: now(6),
        };
        let page = WorkspaceArchivedPage {
            workspace_id: WorkspaceId::new(),
            archived_rev: 42,
            tasks: vec![summary],
            next_cursor: Some(WorkspaceIndexCursor {
                sort_at: now(7),
                task_id: TaskId::new(),
            }),
            total_archived: 3,
        };

        assert_eq!(
            serde_json::to_value(WorkspaceArchivedPageRouteResponse::from(page.clone())).unwrap(),
            serde_json::to_value(page).unwrap()
        );
    }

    #[test]
    fn archived_page_response_matches_raw_page_wire_shape_without_cursor_or_nested_lists() {
        let page = WorkspaceArchivedPage {
            workspace_id: WorkspaceId::new(),
            archived_rev: 0,
            tasks: vec![WorkspaceTaskSummary {
                task: task_with_optional_fields(TaskStatus::Failed),
                provider_ids: Vec::new(),
                sessions: Vec::new(),
                sort_at: now(8),
            }],
            next_cursor: None,
            total_archived: 1,
        };

        assert_eq!(
            serde_json::to_value(WorkspaceArchivedPageRouteResponse::from(page.clone())).unwrap(),
            serde_json::to_value(page).unwrap()
        );
    }

    #[test]
    fn archive_response_preserves_flattened_task_shape() {
        let task = task_with_optional_fields(TaskStatus::Cancelled);
        let response = ArchiveTaskRouteResponse::from(ArchiveTaskOutcome {
            task: task.clone(),
            cleanup_failed: true,
        });
        let mut expected = serde_json::to_value(task).unwrap();
        expected["cleanup_failed"] = json!(true);

        assert_eq!(serde_json::to_value(response).unwrap(), expected);
    }

    #[test]
    fn create_session_request_parses_and_normalizes_fields() {
        let req: CreateTaskSessionRouteRequest = serde_json::from_value(json!({
            "id": "session-id",
            "provider_id": " fake ",
            "model_id": " fake-model ",
            "reasoning_effort": "high",
            "remember_model_preference": true,
            "parent_session_id": "parent",
            "relationship": "follow_up",
            "initial_prompt": "hello",
            "initial_message_id": "message",
            "initial_turn_id": "turn",
            "worktree_id": "worktree",
            "execution_environment": "container_default"
        }))
        .unwrap();

        let input = req.into_task_session_input(Some("run".to_string()));
        assert_eq!(input.id.as_deref(), Some("session-id"));
        assert_eq!(input.provider_id, "fake");
        assert_eq!(input.model_id, "fake-model");
        assert_eq!(input.reasoning_effort.as_deref(), Some("high"));
        assert!(input.remember_model_preference);
        assert_eq!(input.parent_session_id.as_deref(), Some("parent"));
        assert_eq!(input.relationship.as_deref(), Some("follow_up"));
        assert_eq!(input.initial_prompt.as_deref(), Some("hello"));
        assert_eq!(input.initial_message_id.as_deref(), Some("message"));
        assert_eq!(input.initial_turn_id.as_deref(), Some("turn"));
        assert_eq!(input.worktree_id.as_deref(), Some("worktree"));
        assert_eq!(
            input.execution_environment,
            Some(ExecutionEnvironment::Sandbox)
        );
        assert_eq!(input.run_id_header.as_deref(), Some("run"));
    }

    #[test]
    fn create_session_request_rejects_legacy_env_target_alias() {
        let error = serde_json::from_value::<CreateTaskSessionRouteRequest>(json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "env_target": "local"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn create_session_request_rejects_invalid_execution_environment() {
        let error = serde_json::from_value::<CreateTaskSessionRouteRequest>(json!({
            "provider_id": "fake",
            "model_id": "fake-model",
            "execution_environment": "worktree"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown execution environment"));
    }

    #[test]
    fn create_session_request_rejects_empty_model_id() {
        let error = serde_json::from_value::<CreateTaskSessionRouteRequest>(json!({
            "provider_id": "fake",
            "model_id": "   "
        }))
        .unwrap_err();

        assert!(error.to_string().contains("model_id must not be empty"));
    }

    #[test]
    fn create_session_request_rejects_default_placeholder_model_id() {
        let error = serde_json::from_value::<CreateTaskSessionRouteRequest>(json!({
            "provider_id": "fake",
            "model_id": "default"
        }))
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("model_id must be a concrete model id"));
    }

    #[test]
    fn create_session_request_rejects_empty_provider_id() {
        let error = serde_json::from_value::<CreateTaskSessionRouteRequest>(json!({
            "provider_id": "   ",
            "model_id": "fake-model"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("provider_id must not be empty"));
    }

    #[test]
    fn create_task_request_rejects_legacy_default_session_flag() {
        let error = serde_json::from_value::<CreateTaskRouteRequest>(json!({
            "title": "task",
            "create_default_session": false
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn create_task_request_accepts_default_session_options() {
        let task_id = TaskId::new();
        let req: CreateTaskRouteRequest = serde_json::from_value(json!({
            "id": format!(" {} ", task_id.0),
            "title": "task",
            "description": "description",
            "default_session": {
                "provider_id": "fake",
                "model_id": "fake-model",
                "execution_environment": "host"
            }
        }))
        .unwrap();

        let input = req.into_create_task_input().unwrap();
        assert_eq!(input.task_id, Some(task_id));
        assert_eq!(input.title, "task");
        assert_eq!(input.description.as_deref(), Some("description"));
        assert!(input.default_session.is_some());
    }

    #[test]
    fn create_task_request_rejects_invalid_task_id() {
        let error = CreateTaskRouteRequest {
            id: Some("not-a-task".to_string()),
            title: "task".to_string(),
            description: None,
            default_session: None,
        }
        .into_create_task_input()
        .unwrap_err();

        assert_eq!(error.kind(), TaskRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid task id");
    }

    #[test]
    fn archived_cursor_parses_complete_cursor() {
        let task_id = TaskId::new();
        let sort_at = now(9);
        let cursor =
            parse_archived_cursor(Some(&sort_at.to_rfc3339()), Some(&task_id.0.to_string()))
                .unwrap()
                .unwrap();

        assert_eq!(cursor.sort_at, sort_at);
        assert_eq!(cursor.task_id, task_id);
    }

    #[test]
    fn archived_cursor_rejects_half_cursor() {
        let error = parse_archived_cursor(Some(&now(9).to_rfc3339()), None).unwrap_err();

        assert_eq!(error.kind(), TaskRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid cursor");
    }

    #[test]
    fn archived_cursor_rejects_bad_timestamp() {
        let error = parse_archived_cursor(Some("not-a-date"), Some("not-a-task")).unwrap_err();

        assert_eq!(error.kind(), TaskRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid cursor");
    }

    #[test]
    fn archived_cursor_rejects_bad_task_id() {
        let error =
            parse_archived_cursor(Some(&now(9).to_rfc3339()), Some("not-a-task")).unwrap_err();

        assert_eq!(error.kind(), TaskRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid task id");
    }

    #[test]
    fn title_request_trims_and_validates() {
        let title = UpdateTaskTitleRouteRequest {
            title: "  New title  ".to_string(),
        }
        .validated_title()
        .unwrap();
        assert_eq!(title, "New title");

        let empty = UpdateTaskTitleRouteRequest {
            title: "  ".to_string(),
        }
        .validated_title()
        .unwrap_err();
        assert_eq!(empty.kind(), TaskRouteErrorKind::BadRequest);
        assert_eq!(empty.message(), "title is required");

        let too_long = UpdateTaskTitleRouteRequest {
            title: "x".repeat(121),
        }
        .validated_title()
        .unwrap_err();
        assert_eq!(too_long.kind(), TaskRouteErrorKind::BadRequest);
        assert_eq!(too_long.message(), "title is too long");
    }

    #[test]
    fn route_error_kind_preserves_non_http_internal_status_categories() {
        let storage_error =
            anyhow::anyhow!("Insufficient storage capacity for creating an isolated task worktree");
        assert_eq!(
            route_error_kind_for_internal_error(&storage_error),
            TaskRouteErrorKind::InsufficientStorage
        );

        let policy_error = ctx_settings_service::HostExecutionPolicy::SandboxOnly
            .validate_execution_environment(ExecutionEnvironment::Host)
            .expect_err("host execution should be denied");
        assert_eq!(
            route_error_kind_for_internal_error(&policy_error),
            TaskRouteErrorKind::Forbidden
        );

        assert_eq!(
            route_error_kind_for_internal_error(&anyhow::anyhow!("plain internal failure")),
            TaskRouteErrorKind::Internal
        );
    }
}
