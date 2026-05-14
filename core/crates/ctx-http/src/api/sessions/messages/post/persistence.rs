use super::super::attachments::normalize_message_attachments;
use super::super::*;
use super::request::PostMessageReq;
use ctx_session_service::message_delivery::{
    resolve_message_client_ids, MessageClientIdResolutionError,
};

pub(super) struct PostMessageParts {
    pub(super) message_id: MessageId,
    pub(super) turn_id: TurnId,
    pub(super) client_supplied_ids: bool,
    pub(super) content: String,
    pub(super) requested_delivery: Option<MessageDelivery>,
    pub(super) attachments: Vec<MessageAttachment>,
}

impl PostMessageParts {
    pub(super) async fn from_request(
        state: &SessionsHandle,
        req: PostMessageReq,
    ) -> Result<Self, ApiErr> {
        let request_message_id = req
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                uuid::Uuid::parse_str(value)
                    .map(MessageId)
                    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid message id."))
            })
            .transpose()?;
        let request_turn_id = req
            .turn_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                uuid::Uuid::parse_str(value)
                    .map(TurnId)
                    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid turn id."))
            })
            .transpose()?;
        let client_ids =
            resolve_message_client_ids(request_message_id, request_turn_id).map_err(|error| {
                match error {
                    MessageClientIdResolutionError::PartialClientIds => {
                        api_error(StatusCode::BAD_REQUEST, error.message())
                    }
                }
            })?;
        Ok(Self {
            message_id: client_ids.message_id,
            turn_id: client_ids.turn_id,
            client_supplied_ids: client_ids.client_supplied,
            content: req.content,
            requested_delivery: req.delivery,
            attachments: normalize_message_attachments(state, req.attachments).await?,
        })
    }
}
