use ctx_core::ids::SessionId;

#[derive(Debug, Clone, Copy)]
pub struct CreateSessionRequestPolicy<'a> {
    pub requested_session_id: Option<&'a str>,
    pub parent_session_id: Option<&'a str>,
    pub relationship: Option<&'a str>,
    pub initial_prompt_present: bool,
    pub initial_message_id_present: bool,
    pub initial_turn_id_present: bool,
    pub task_primary_session_id: Option<SessionId>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CreateSessionRequestDecision {
    pub session_id: Option<SessionId>,
    pub parent_session_id: Option<SessionId>,
    pub relationship: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CreateSessionRequestError {
    InvalidSessionId,
    InvalidParentSessionId,
    RelationshipRequiresParent,
    MissingInitialPromptIds,
    PrimarySessionConflict,
}

pub fn validate_create_session_request(
    input: CreateSessionRequestPolicy<'_>,
) -> Result<CreateSessionRequestDecision, CreateSessionRequestError> {
    let session_id = parse_optional_session_id(input.requested_session_id)
        .map_err(|_| CreateSessionRequestError::InvalidSessionId)?;
    let parent_session_id = parse_optional_session_id(input.parent_session_id)
        .map_err(|_| CreateSessionRequestError::InvalidParentSessionId)?;
    let relationship = input
        .relationship
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if parent_session_id.is_some() != relationship.is_some() {
        return Err(CreateSessionRequestError::RelationshipRequiresParent);
    }
    if parent_session_id.is_none() && relationship.is_none() {
        if let Some(primary_session_id) = input.task_primary_session_id {
            if session_id.as_ref() != Some(&primary_session_id) {
                return Err(CreateSessionRequestError::PrimarySessionConflict);
            }
        }
    }
    if input.initial_prompt_present
        && (!input.initial_message_id_present || !input.initial_turn_id_present)
    {
        return Err(CreateSessionRequestError::MissingInitialPromptIds);
    }

    Ok(CreateSessionRequestDecision {
        session_id,
        parent_session_id,
        relationship,
    })
}

fn parse_optional_session_id(raw: Option<&str>) -> Result<Option<SessionId>, uuid::Error> {
    match raw.map(str::trim) {
        Some("") | None => Ok(None),
        Some(value) => uuid::Uuid::parse_str(value).map(SessionId).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requested_session_and_parent_relationship() {
        let session_id = SessionId::new();
        let parent_id = SessionId::new();

        let decision = validate_create_session_request(CreateSessionRequestPolicy {
            requested_session_id: Some(&session_id.0.to_string()),
            parent_session_id: Some(&parent_id.0.to_string()),
            relationship: Some(" branch "),
            initial_prompt_present: false,
            initial_message_id_present: false,
            initial_turn_id_present: false,
            task_primary_session_id: None,
        })
        .expect("valid request");

        assert_eq!(decision.session_id, Some(session_id));
        assert_eq!(decision.parent_session_id, Some(parent_id));
        assert_eq!(decision.relationship.as_deref(), Some("branch"));
    }

    #[test]
    fn rejects_relationship_without_parent_or_parent_without_relationship() {
        assert_eq!(
            validate_create_session_request(CreateSessionRequestPolicy {
                requested_session_id: None,
                parent_session_id: Some(&SessionId::new().0.to_string()),
                relationship: None,
                initial_prompt_present: false,
                initial_message_id_present: false,
                initial_turn_id_present: false,
                task_primary_session_id: None,
            }),
            Err(CreateSessionRequestError::RelationshipRequiresParent)
        );
        assert_eq!(
            validate_create_session_request(CreateSessionRequestPolicy {
                requested_session_id: None,
                parent_session_id: None,
                relationship: Some("branch"),
                initial_prompt_present: false,
                initial_message_id_present: false,
                initial_turn_id_present: false,
                task_primary_session_id: None,
            }),
            Err(CreateSessionRequestError::RelationshipRequiresParent)
        );
    }

    #[test]
    fn rejects_missing_initial_prompt_ids() {
        assert_eq!(
            validate_create_session_request(CreateSessionRequestPolicy {
                requested_session_id: None,
                parent_session_id: None,
                relationship: None,
                initial_prompt_present: true,
                initial_message_id_present: true,
                initial_turn_id_present: false,
                task_primary_session_id: None,
            }),
            Err(CreateSessionRequestError::MissingInitialPromptIds)
        );
    }

    #[test]
    fn rejects_conflicting_primary_session_creation() {
        let primary_id = SessionId::new();
        assert_eq!(
            validate_create_session_request(CreateSessionRequestPolicy {
                requested_session_id: None,
                parent_session_id: None,
                relationship: None,
                initial_prompt_present: false,
                initial_message_id_present: false,
                initial_turn_id_present: false,
                task_primary_session_id: Some(primary_id),
            }),
            Err(CreateSessionRequestError::PrimarySessionConflict)
        );
    }
}
