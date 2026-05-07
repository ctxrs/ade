use super::*;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct CreateSessionReq {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_id")]
    pub(super) provider_id: String,
    #[serde(deserialize_with = "deserialize_concrete_model_id")]
    pub(super) model_id: String,
    #[serde(
        default,
        deserialize_with = "ctx_session_tools::model_resolution::deserialize_optional_reasoning_effort"
    )]
    pub(super) reasoning_effort: Option<String>,
    #[serde(default)]
    pub(super) remember_model_preference: bool,
    pub(super) parent_session_id: Option<String>,
    pub(super) relationship: Option<String>,
    #[serde(default)]
    pub(super) initial_prompt: Option<String>,
    #[serde(default)]
    pub(super) initial_message_id: Option<String>,
    #[serde(default)]
    pub(super) initial_turn_id: Option<String>,
    #[serde(default)]
    pub(super) worktree_id: Option<String>,
    #[serde(default)]
    pub(crate) execution_environment: Option<ExecutionEnvironment>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api::tasks) struct CreateTaskDefaultSessionReq {
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
    execution_environment: Option<ExecutionEnvironment>,
}

impl CreateTaskDefaultSessionReq {
    pub(super) fn into_create_session_req(self) -> CreateSessionReq {
        CreateSessionReq {
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
            execution_environment: self.execution_environment,
        }
    }

    pub(super) fn into_replay_create_session_req(
        self,
        primary_session_id: SessionId,
    ) -> CreateSessionReq {
        let mut req = self.into_create_session_req();
        if req
            .id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            req.id = Some(primary_session_id.0.to_string());
        }
        req
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
