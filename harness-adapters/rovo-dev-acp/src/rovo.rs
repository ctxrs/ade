use anyhow::{anyhow, Context, Result};
use reqwest::StatusCode;
use serde_json::Value;
use std::fmt;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug)]
pub struct RovoApiError {
    status: StatusCode,
    body: String,
}

impl RovoApiError {
    pub fn is_auth_required(&self) -> bool {
        if self.status == StatusCode::UNAUTHORIZED || self.status == StatusCode::FORBIDDEN {
            return true;
        }
        let hay = self.body.to_lowercase();
        hay.contains("auth") || hay.contains("login") || hay.contains("unauthorized")
    }
}

impl fmt::Display for RovoApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rovo dev server error ({}): {}", self.status, self.body)
    }
}

impl std::error::Error for RovoApiError {}

#[derive(Clone)]
pub struct RovoClient {
    base_url: String,
    client: reqwest::Client,
}

impl RovoClient {
    pub fn new(base_url: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn healthcheck(&self) -> Result<Value> {
        self.get_json("/healthcheck").await
    }

    pub async fn create_session(&self) -> Result<String> {
        let value = self.post_json("/v3/sessions/create", Value::Null).await?;
        extract_session_id(&value)
            .ok_or_else(|| anyhow!("missing session id in response: {value}"))
    }

    pub async fn restore_session(&self, session_id: &str) -> Result<()> {
        let path = format!("/v3/sessions/{}/restore", session_id);
        let _ = self.post_json(&path, Value::Null).await?;
        Ok(())
    }

    pub async fn set_chat_message(&self, message: &str, enable_deep_plan: bool) -> Result<()> {
        let payload = serde_json::json!({
            "message": message,
            "enable_deep_plan": enable_deep_plan,
        });
        let _ = self.post_json("/v3/set_chat_message", payload).await?;
        Ok(())
    }

    pub async fn stream_chat(&self, pause_on_tool_calls: bool) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, "/v3/stream_chat");
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .query(&[("pause_on_call_tools_start", pause_on_tool_calls)])
            .send()
            .await
            .context("stream_chat request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RovoApiError { status, body }.into());
        }
        Ok(resp)
    }

    pub async fn cancel(&self) -> Result<()> {
        let _ = self.post_json("/v3/cancel", Value::Null).await?;
        Ok(())
    }

    async fn get_json(&self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("GET request failed")?;
        parse_response(resp).await
    }

    async fn post_json(&self, path: &str, payload: Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let req = if payload.is_null() {
            self.client.post(url)
        } else {
            self.client.post(url).json(&payload)
        };
        let resp = req.send().await.context("POST request failed")?;
        parse_response(resp).await
    }
}

pub struct RovoProcess {
    child: Child,
}

impl RovoProcess {
    pub async fn spawn(command: &str, args: &[String], cwd: &Path) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn rovo dev command: {command}"))?;

        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(pipe_output(stdout, "stdout"));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(pipe_output(stderr, "stderr"));
        }

        Ok(Self { child })
    }

}

impl Drop for RovoProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

async fn pipe_output<R>(stream: R, label: &str)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        eprintln!("[rovo-dev {label}] {line}");
    }
}

async fn parse_response(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(RovoApiError { status, body }.into());
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body).context("decoding response JSON")
}

fn extract_session_id(value: &Value) -> Option<String> {
    if let Some(id) = value.as_str() {
        return Some(id.to_string());
    }
    let candidates = ["session_id", "sessionId", "id"];
    for key in candidates {
        if let Some(id) = value.get(key).and_then(|v| v.as_str()) {
            return Some(id.to_string());
        }
    }
    if let Some(session) = value.get("session") {
        for key in candidates {
            if let Some(id) = session.get(key).and_then(|v| v.as_str()) {
                return Some(id.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_session_id_from_variants() {
        let value = serde_json::json!({"sessionId": "abc"});
        assert_eq!(extract_session_id(&value), Some("abc".to_string()));

        let value = serde_json::json!({"session": {"id": "def"}});
        assert_eq!(extract_session_id(&value), Some("def".to_string()));
    }
}
