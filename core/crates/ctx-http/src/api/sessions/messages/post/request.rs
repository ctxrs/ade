use super::super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct PostMessageReq {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) turn_id: Option<String>,
    pub(super) content: String,
    pub(super) delivery: Option<MessageDelivery>,
    #[serde(default)]
    pub(super) attachments: Vec<MessageAttachment>,
}
