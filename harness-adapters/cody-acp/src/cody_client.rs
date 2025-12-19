use crate::config::Config;
use crate::jsonrpc::JsonRpcClient;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use url::Url;

#[derive(Debug, Clone)]
pub struct CodyAuthStatus {
    pub authenticated: bool,
    pub _endpoint: Option<String>,
}

#[derive(Clone)]
pub struct CodyClient {
    rpc: Arc<JsonRpcClient>,
    _server_info: Value,
    auth_status: CodyAuthStatus,
    _child: Arc<tokio::sync::Mutex<Child>>,
}

#[derive(Debug, Deserialize)]
pub struct CodyModelAvailability {
    pub model: CodyModel,
    #[serde(default)]
    #[serde(rename = "isModelAvailable")]
    pub is_model_available: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CodyModel {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl CodyClient {
    pub async fn spawn(config: &Config, workspace_root: &Path) -> Result<Self> {
        let mut command = Command::new(&config.cody_command);
        command
            .args(&config.cody_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(endpoint) = config.endpoint.clone() {
            command.env("SRC_ENDPOINT", endpoint);
        }
        if let Some(token) = config.access_token.clone() {
            command.env("SRC_ACCESS_TOKEN", token);
        }
        command.env("ENABLE_SENTRY", "false");

        let mut child = command.spawn().map_err(|err| {
            anyhow!(
                "failed to spawn Cody CLI at {}: {err}",
                config.cody_command
            )
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing Cody stdout"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("missing Cody stdin"))?;

        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    let bytes = match reader.read_line(&mut line).await {
                        Ok(bytes) => bytes,
                        Err(_) => break,
                    };
                    if bytes == 0 {
                        break;
                    }
                    eprintln!("[cody-cli] {}", line.trim_end());
                }
            });
        }

        let rpc = JsonRpcClient::new(stdout, stdin);
        let workspace_root_uri = path_to_uri(workspace_root)?;
        let endpoint = config.resolved_endpoint();
        let access_token = config.resolved_access_token();

        let mut extension_config = serde_json::Map::new();
        extension_config.insert("serverEndpoint".to_string(), Value::String(endpoint));
        if let Some(token) = access_token {
            extension_config.insert("accessToken".to_string(), Value::String(token));
        }
        extension_config.insert("customHeaders".to_string(), json!({}));
        extension_config.insert(
            "customConfiguration".to_string(),
            json!({
                "cody.internal.autocomplete.entirelyDisabled": true,
                "cody.experimental.symf.enabled": false,
                "cody.experimental.telemetry.enabled": false
            }),
        );

        let client_info = json!({
            "name": "cody-acp",
            "version": env!("CARGO_PKG_VERSION"),
            "workspaceRootUri": workspace_root_uri,
            "capabilities": {
                "completions": "none"
            },
            "legacyNameForServerIdentification": "jetbrains",
            "extensionConfiguration": Value::Object(extension_config),
        });

        let server_info = rpc.request("initialize", client_info).await?;
        rpc.notify("initialized", Value::Null).await?;

        let auth_status = parse_auth_status(&server_info);

        Ok(Self {
            rpc,
            _server_info: server_info,
            auth_status,
            _child: Arc::new(tokio::sync::Mutex::new(child)),
        })
    }

    pub fn auth_status(&self) -> &CodyAuthStatus {
        &self.auth_status
    }

    pub fn subscribe_notifications(&self) -> tokio::sync::broadcast::Receiver<crate::jsonrpc::JsonRpcNotification> {
        self.rpc.subscribe()
    }

    pub async fn chat_new(&self) -> Result<String> {
        let response = self.rpc.request("chat/new", Value::Null).await?;
        response
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("unexpected chat/new response: {response}"))
    }

    pub async fn chat_models(&self) -> Result<Vec<CodyModelAvailability>> {
        let response = self
            .rpc
            .request("chat/models", json!({"modelUsage": "chat"}))
            .await?;
        let models = response
            .get("models")
            .cloned()
            .ok_or_else(|| anyhow!("missing models in chat/models response"))?;
        let parsed: Vec<CodyModelAvailability> = serde_json::from_value(models)?;
        Ok(parsed)
    }

    pub async fn chat_set_model(&self, panel_id: &str, model: &str) -> Result<()> {
        self.rpc
            .request(
                "chat/setModel",
                json!({
                    "id": panel_id,
                    "model": model,
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn chat_submit_message(
        &self,
        panel_id: &str,
        text: &str,
        context_items: &[Value],
    ) -> Result<Value> {
        let message = if context_items.is_empty() {
            json!({
                "command": "submit",
                "text": text,
            })
        } else {
            json!({
                "command": "submit",
                "text": text,
                "contextItems": context_items,
            })
        };

        let response = self
            .rpc
            .request(
                "chat/submitMessage",
                json!({
                    "id": panel_id,
                    "message": message,
                }),
            )
            .await?;
        Ok(response)
    }

    pub async fn webview_receive_message(&self, panel_id: &str, message: Value) -> Result<()> {
        self.rpc
            .request(
                "webview/receiveMessage",
                json!({
                    "id": panel_id,
                    "message": message,
                }),
            )
            .await?;
        Ok(())
    }
}

fn parse_auth_status(server_info: &Value) -> CodyAuthStatus {
    let auth = server_info.get("authStatus");
    let authenticated = auth
        .and_then(|v| v.get("authenticated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let endpoint = auth
        .and_then(|v| v.get("endpoint"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    CodyAuthStatus {
        authenticated,
        _endpoint: endpoint,
    }
}

fn path_to_uri(path: &Path) -> Result<String> {
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| anyhow!("invalid workspace path for URI"))
}
