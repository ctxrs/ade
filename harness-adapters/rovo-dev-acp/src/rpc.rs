use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

pub const JSONRPC_VERSION: &str = "2.0";

pub const ERR_PARSE_ERROR: i64 = -32700;
pub const ERR_INVALID_REQUEST: i64 = -32600;
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERR_INVALID_PARAMS: i64 = -32602;
pub const ERR_INTERNAL_ERROR: i64 = -32603;
pub const ERR_AUTH_REQUIRED: i64 = -32000;

#[derive(Debug, Deserialize)]
pub struct RpcEnvelope {
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: Option<String>,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone)]
pub struct RpcWriter {
    inner: Arc<Mutex<BufWriter<tokio::io::Stdout>>>,
}

impl RpcWriter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufWriter::new(tokio::io::stdout()))),
        }
    }

    pub async fn send(&self, value: &Value) -> anyhow::Result<()> {
        let mut guard = self.inner.lock().await;
        let line = serde_json::to_string(value)?;
        guard.write_all(line.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        Ok(())
    }

    pub fn clone_handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub fn response_ok(id: Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result,
    })
}

pub fn response_error(id: Value, error: RpcError) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": error,
    })
}

pub fn notification(method: &str, params: Value) -> Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": method,
        "params": params,
    })
}
