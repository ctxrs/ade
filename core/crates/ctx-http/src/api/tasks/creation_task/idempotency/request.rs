use super::*;

pub(in crate::api::tasks::creation::task_creation) struct CreateTaskRequestParts {
    pub(in crate::api::tasks::creation::task_creation) task_id: Option<TaskId>,
    pub(in crate::api::tasks::creation::task_creation) requested_title: String,
    pub(in crate::api::tasks::creation::task_creation) requested_description: Option<String>,
    pub(in crate::api::tasks::creation::task_creation) requested_default_session:
        Option<CreateTaskDefaultSessionReq>,
}

impl CreateTaskRequestParts {
    pub(in crate::api::tasks::creation::task_creation) fn from_request(
        req: CreateTaskReq,
    ) -> Result<Self, CreateTaskApiError> {
        let task_id = match req.id.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid task id".to_string(),
                    }),
                )
            })?)),
        };
        Ok(Self {
            task_id,
            requested_title: req.title,
            requested_description: req.description,
            requested_default_session: req.default_session,
        })
    }

    pub(in crate::api::tasks::creation::task_creation) fn should_preflight_default_session(
        &self,
        existing_task: &Option<Task>,
    ) -> bool {
        existing_task.is_none() && self.requested_default_session.is_none()
    }
}
