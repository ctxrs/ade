use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, oneshot, Mutex};

#[derive(Debug, Clone)]
pub struct JsonRpcNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub _data: Option<Value>,
}

impl JsonRpcError {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            code: value.get("code")?.as_i64()?,
            message: value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error")
                .to_string(),
            _data: value.get("data").cloned(),
        })
    }
}

type PendingMap = HashMap<u64, oneshot::Sender<Result<Value, JsonRpcError>>>;

type BoxedWriter = Box<dyn AsyncWrite + Send + Unpin>;

pub struct JsonRpcClient {
    writer: Arc<Mutex<BoxedWriter>>,
    pending: Arc<Mutex<PendingMap>>,
    notifications: broadcast::Sender<JsonRpcNotification>,
    next_id: AtomicU64,
}

impl JsonRpcClient {
    pub fn new<R, W>(reader: R, writer: W) -> Arc<Self>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (notifications, _) = broadcast::channel(256);
        let client = Arc::new(Self {
            writer: Arc::new(Mutex::new(Box::new(writer))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notifications,
            next_id: AtomicU64::new(1),
        });

        let reader_client = Arc::clone(&client);
        tokio::spawn(async move {
            if let Err(err) = reader_client.read_loop(reader).await {
                eprintln!("[cody-acp] JSON-RPC read loop ended: {err:#}");
            }
        });

        client
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notifications.subscribe()
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.send_message(&payload).await?;

        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(anyhow!("JSON-RPC error {}: {}", err.code, err.message)),
            Err(err) => Err(anyhow!("JSON-RPC response channel closed: {err}")),
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_message(&payload).await
    }

    async fn send_message(&self, payload: &Value) -> Result<()> {
        let body = serde_json::to_vec(payload)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut writer = self.writer.lock().await;
        writer.write_all(header.as_bytes()).await?;
        writer.write_all(&body).await?;
        writer.flush().await?;
        Ok(())
    }

    async fn read_loop<R>(&self, reader: R) -> Result<()>
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let mut reader = BufReader::new(reader);
        loop {
            let Some(message) = read_message(&mut reader).await? else {
                return Ok(());
            };

            if let Some(id) = message.get("id") {
                if let Some(method) = message.get("method").and_then(|v| v.as_str()) {
                    self.handle_request(id, method, message.get("params").cloned())
                        .await?;
                    continue;
                }
                let id = id.as_u64().ok_or_else(|| anyhow!("invalid JSON-RPC id"))?;
                let result = if let Some(result) = message.get("result") {
                    Ok(result.clone())
                } else if let Some(error) = message.get("error") {
                    Err(JsonRpcError::from_value(error).unwrap_or(JsonRpcError {
                        code: -32000,
                        message: "Unknown error".to_string(),
                        _data: Some(error.clone()),
                    }))
                } else {
                    Err(JsonRpcError {
                        code: -32000,
                        message: "Missing result".to_string(),
                        _data: Some(message.clone()),
                    })
                };

                if let Some(tx) = self.pending.lock().await.remove(&id) {
                    let _ = tx.send(result);
                }
                continue;
            }

            if let Some(method) = message.get("method").and_then(|v| v.as_str()) {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let _ = self
                    .notifications
                    .send(JsonRpcNotification {
                        method: method.to_string(),
                        params,
                    });
            }
        }
    }

    async fn handle_request(&self, id: &Value, method: &str, params: Option<Value>) -> Result<()> {
        match method {
            "window/showMessage" => {
                if let Some(params) = params {
                    eprintln!("[cody-acp] window/showMessage: {params}");
                }
                let response = json!({"jsonrpc": "2.0", "id": id, "result": Value::Null});
                self.send_message(&response).await?;
            }
            _ => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("Method not supported: {method}"),
                    }
                });
                self.send_message(&response).await?;
            }
        }
        Ok(())
    }
}

async fn read_message<R>(reader: &mut BufReader<R>) -> Result<Option<Value>>
where
    R: AsyncRead + Unpin,
{
    let mut content_length = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).await?;
        if bytes == 0 {
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
        if trimmed.is_empty() {
            break;
        }

        if let Some(rest) = trimmed.split_once(':') {
            let key = rest.0.trim().to_ascii_lowercase();
            if key == "content-length" {
                content_length = Some(rest.1.trim().parse::<usize>()?);
            }
        }
    }

    let length = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    let mut buf = vec![0u8; length];
    reader.read_exact(&mut buf).await?;
    let message: Value = serde_json::from_slice(&buf)?;
    Ok(Some(message))
}
