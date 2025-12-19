use agent_client_protocol::ModelId;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct SessionState {
    pub cwd: PathBuf,
    pub panel_id: String,
    pub chat_id: Option<String>,
    pub last_assistant_text: String,
    pub pending_cancel: Option<oneshot::Sender<()>>,
    pub processes: HashMap<String, ProcessSnapshot>,
    pub current_model: Option<ModelId>,
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub state: Option<String>,
    pub _title: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WebviewPostMessage {
    pub id: String,
    pub message: TranscriptEnvelope,
}

#[derive(Debug, Deserialize)]
pub struct TranscriptEnvelope {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub messages: Vec<TranscriptMessage>,
    #[serde(default, rename = "isMessageInProgress")]
    pub _is_message_in_progress: bool,
    #[serde(rename = "chatID")]
    pub chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TranscriptMessage {
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub processes: Option<Vec<TranscriptProcess>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TranscriptProcess {
    pub id: Option<String>,
    pub state: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
}

pub fn extract_assistant_text(messages: &[TranscriptMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.speaker.as_deref() == Some("assistant"))
        .and_then(|message| message.text.clone())
}

pub fn extract_processes(messages: &[TranscriptMessage]) -> Vec<TranscriptProcess> {
    messages
        .iter()
        .rev()
        .find_map(|message| message.processes.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extracts_assistant_text() {
        let messages = vec![
            TranscriptMessage {
                speaker: Some("human".to_string()),
                text: Some("hi".to_string()),
                processes: None,
            },
            TranscriptMessage {
                speaker: Some("assistant".to_string()),
                text: Some("hello".to_string()),
                processes: None,
            },
        ];
        assert_eq!(extract_assistant_text(&messages), Some("hello".to_string()));
    }

    #[test]
    fn extracts_processes() {
        let messages = vec![TranscriptMessage {
            speaker: Some("assistant".to_string()),
            text: Some("done".to_string()),
            processes: Some(vec![TranscriptProcess {
                id: Some("step".to_string()),
                state: Some("success".to_string()),
                title: Some("Step".to_string()),
                content: None,
            }]),
        }];
        let processes = extract_processes(&messages);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].id.as_deref(), Some("step"));
    }
}
