use ctx_core::ids::TaskId;
use serde::Deserialize;

use crate::daemon::TasksHandle;

use super::super::{CreateTaskInput, CreateTaskSessionInput};
use super::common::{parse_task_id, parse_workspace_id, TaskRouteError, TaskRouteParams};
use super::responses::{ExecutionEnvironmentRouteValue, SessionRouteResponse, TaskRouteResponse};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskRouteRequest {
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) title: String,
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) default_session: Option<CreateTaskDefaultSessionRouteRequest>,
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
}

impl CreateTaskRouteRequest {
    pub(super) fn into_create_task_input(self) -> Result<CreateTaskInput, TaskRouteError> {
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
    pub(super) fn into_task_session_input(
        self,
        run_id_header: Option<String>,
    ) -> CreateTaskSessionInput {
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
