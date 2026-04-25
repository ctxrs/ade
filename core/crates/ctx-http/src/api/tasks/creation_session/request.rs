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
        deserialize_with = "sessions::deserialize_optional_reasoning_effort"
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
