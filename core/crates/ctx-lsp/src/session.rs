use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use lsp_types::{
    ApplyWorkspaceEditParams, ApplyWorkspaceEditResponse, Diagnostic, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, ExecuteCommandParams, InitializeParams, InitializedParams,
    PublishDiagnosticsParams, TextDocumentContentChangeEvent, TextDocumentItem, Uri,
    VersionedTextDocumentIdentifier, WorkspaceEdit, WorkspaceFolder,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify};

use crate::config::LspManagerConfig;
use crate::workspace_edit::apply_workspace_edit_to_disk;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Html,
    Css,
    Json,
    Yaml,
    Bash,
    Dockerfile,
    CCpp,
    Lua,
    Toml,
    Markdown,
    Custom(String),
}

impl Language {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "rust" => Some(Language::Rust),
            "typescript" => Some(Language::TypeScript),
            "javascript" => Some(Language::JavaScript),
            "python" => Some(Language::Python),
            "go" => Some(Language::Go),
            "html" => Some(Language::Html),
            "css" => Some(Language::Css),
            "json" => Some(Language::Json),
            "yaml" => Some(Language::Yaml),
            "bash" => Some(Language::Bash),
            "dockerfile" => Some(Language::Dockerfile),
            "cpp" | "c" | "ccpp" => Some(Language::CCpp),
            "lua" => Some(Language::Lua),
            "toml" => Some(Language::Toml),
            "markdown" => Some(Language::Markdown),
            _ => None,
        }
    }

    pub fn detect(path: &Path, cfg: &LspManagerConfig) -> Option<Self> {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if name == "Dockerfile" {
                return Some(Language::Dockerfile);
            }
            if let Some(lang) = cfg.custom_filename_map.get(name) {
                return Language::from_id(lang).or_else(|| Some(Language::Custom(lang.clone())));
            }
        }

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !ext.is_empty() {
            if let Some(lang) = cfg.custom_extension_map.get(&ext) {
                return Language::from_id(lang).or_else(|| Some(Language::Custom(lang.clone())));
            }
        }

        if path.file_name().and_then(|s| s.to_str()) == Some("Dockerfile") {
            return Some(Language::Dockerfile);
        }
        match ext.as_str() {
            "rs" => Some(Language::Rust),
            "ts" | "tsx" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "py" => Some(Language::Python),
            "go" => Some(Language::Go),
            "html" | "htm" => Some(Language::Html),
            "css" | "scss" | "less" => Some(Language::Css),
            "json" | "jsonc" => Some(Language::Json),
            "yml" | "yaml" => Some(Language::Yaml),
            "sh" | "bash" | "zsh" => Some(Language::Bash),
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(Language::CCpp),
            "lua" => Some(Language::Lua),
            "toml" => Some(Language::Toml),
            "md" | "mdx" => Some(Language::Markdown),
            _ => None,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Html => "html",
            Language::Css => "css",
            Language::Json => "json",
            Language::Yaml => "yaml",
            Language::Bash => "bash",
            Language::Dockerfile => "dockerfile",
            Language::CCpp => "cpp",
            Language::Lua => "lua",
            Language::Toml => "toml",
            Language::Markdown => "markdown",
            Language::Custom(id) => id.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosticsUpdate {
    pub uri: Uri,
    pub diagnostics: Vec<Diagnostic>,
}

async fn sync_open_doc_text(
    open_docs: &Arc<Mutex<HashMap<Uri, i32>>>,
    tx: &mpsc::Sender<Value>,
    lang_id: &str,
    uri: Uri,
    text: String,
) {
    let (method, params) = {
        let mut docs = open_docs.lock().await;
        if let Some(v) = docs.get_mut(&uri) {
            *v += 1;
            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: *v,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text,
                }],
            };
            (
                "textDocument/didChange",
                serde_json::to_value(params).unwrap_or(Value::Null),
            )
        } else {
            docs.insert(uri.clone(), 1);
            let params = DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: lang_id.to_string(),
                    version: 1,
                    text,
                },
            };
            (
                "textDocument/didOpen",
                serde_json::to_value(params).unwrap_or(Value::Null),
            )
        }
    };
    let _ = tx
        .send(json!({
            "jsonrpc":"2.0",
            "method": method,
            "params": params
        }))
        .await;
}

#[derive(Debug, Clone)]
pub(crate) struct OpenDoc {
    pub(crate) uri: Uri,
}

pub(crate) struct LspSession {
    root: PathBuf,
    lang: Language,
    tx: mpsc::Sender<Value>,
    next_id: Arc<std::sync::atomic::AtomicI64>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    open_docs: Arc<Mutex<HashMap<Uri, i32>>>,
    diagnostics: Arc<Mutex<HashMap<Uri, Vec<Diagnostic>>>>,
    notify: Arc<Notify>,
    diag_tx: broadcast::Sender<DiagnosticsUpdate>,
    apply_edit_capture: Arc<Mutex<Option<oneshot::Sender<WorkspaceEdit>>>>,
    request_timeout: Duration,
    _child: Mutex<Child>,
}

impl LspSession {
    pub(crate) async fn spawn(
        cfg: &LspManagerConfig,
        root: PathBuf,
        lang: Language,
    ) -> Result<Self> {
        let root = root.canonicalize().unwrap_or(root);
        let (child, stdin, stdout) = spawn_server(cfg, lang.clone(), &root).await?;

        let (tx, mut rx) = mpsc::channel::<Value>(256);
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let open_docs: Arc<Mutex<HashMap<Uri, i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<Mutex<HashMap<Uri, Vec<Diagnostic>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let notify = Arc::new(Notify::new());
        let (diag_tx, _) = broadcast::channel(512);
        let next_id = Arc::new(std::sync::atomic::AtomicI64::new(1));
        let apply_edit_capture: Arc<Mutex<Option<oneshot::Sender<WorkspaceEdit>>>> =
            Arc::new(Mutex::new(None));

        // Writer task
        let pending_w = pending.clone();
        let mut writer = tokio::io::BufWriter::new(stdin);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let body = match serde_json::to_vec(&msg) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                if writer.write_all(header.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(&body).await.is_err() {
                    break;
                }
                if writer.flush().await.is_err() {
                    break;
                }
            }
            // Drain pending to unblock waiters.
            let mut p = pending_w.lock().await;
            p.clear();
        });

        // Reader task
        let pending_r = pending.clone();
        let diagnostics_r = diagnostics.clone();
        let notify_r = notify.clone();
        let diag_tx_r = diag_tx.clone();
        let tx_r = tx.clone();
        let apply_edit_capture_r = apply_edit_capture.clone();
        let root_r = root.clone();
        let open_docs_r = open_docs.clone();
        let lang_id_r = lang.id().to_string();
        let mut reader = BufReader::new(stdout);
        tokio::spawn(async move {
            loop {
                let msg = match read_lsp_message(&mut reader).await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::debug!(target: "ctx_lsp", "lsp_read_error: {e:#}");
                        break;
                    }
                };
                let id = msg.get("id").and_then(|v| v.as_i64());
                let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

                // Server -> client request (has both id and method).
                if let Some(id) = id {
                    if !method.is_empty() {
                        if method == "workspace/applyEdit" {
                            let mut applied = false;
                            let mut failure_reason: Option<String> = None;
                            if let Some(params) = msg.get("params") {
                                match serde_json::from_value::<ApplyWorkspaceEditParams>(
                                    params.clone(),
                                ) {
                                    Ok(p) => {
                                        let edit = p.edit;
                                        let capture = apply_edit_capture_r.lock().await.take();
                                        if let Some(tx) = capture {
                                            if tx.send(edit).is_ok() {
                                                applied = true;
                                            } else {
                                                failure_reason = Some(
                                                    "failed to deliver WorkspaceEdit to capture consumer".to_string(),
                                                );
                                            }
                                        } else {
                                            match apply_workspace_edit_to_disk(&root_r, &edit).await
                                            {
                                                Ok(changed) => {
                                                    for (uri, text) in changed {
                                                        sync_open_doc_text(
                                                            &open_docs_r,
                                                            &tx_r,
                                                            &lang_id_r,
                                                            uri,
                                                            text,
                                                        )
                                                        .await;
                                                    }
                                                    applied = true;
                                                }
                                                Err(e) => {
                                                    failure_reason = Some(format!(
                                                        "failed to apply WorkspaceEdit: {e:#}"
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        failure_reason = Some(format!(
                                            "invalid workspace/applyEdit params: {e}"
                                        ));
                                    }
                                }
                            } else {
                                failure_reason =
                                    Some("missing workspace/applyEdit params".to_string());
                            }
                            let _ = tx_r
                                .send(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": ApplyWorkspaceEditResponse { applied, failure_reason, failed_change: None }
                                }))
                                .await;
                        } else {
                            let _ = tx_r
                                .send(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": Value::Null
                                }))
                                .await;
                        }
                        continue;
                    }

                    // Regular response (id but no method).
                    if let Some(tx) = pending_r.lock().await.remove(&id) {
                        let _ = tx.send(msg);
                    }
                    continue;
                }

                if method == "textDocument/publishDiagnostics" {
                    if let Some(params) = msg.get("params") {
                        if let Ok(pd) =
                            serde_json::from_value::<PublishDiagnosticsParams>(params.clone())
                        {
                            let uri = pd.uri;
                            let diagnostics = pd.diagnostics;
                            diagnostics_r
                                .lock()
                                .await
                                .insert(uri.clone(), diagnostics.clone());
                            let _ = diag_tx_r.send(DiagnosticsUpdate { uri, diagnostics });
                            notify_r.notify_waiters();
                        }
                    }
                }
            }
        });

        let mut session = Self {
            root,
            lang,
            tx,
            next_id,
            pending,
            open_docs,
            diagnostics,
            notify,
            diag_tx,
            apply_edit_capture,
            request_timeout: cfg.request_timeout,
            _child: Mutex::new(child),
        };
        session.initialize().await?;
        Ok(session)
    }

    async fn initialize(&mut self) -> Result<()> {
        let root_url = url::Url::from_directory_path(&self.root)
            .map_err(|_| anyhow!("invalid root path for URL"))?;
        let root_uri: Uri = root_url
            .as_str()
            .parse()
            .map_err(|_| anyhow!("invalid root URI"))?;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri.clone(),
                name: self.root.to_string_lossy().to_string(),
            }]),
            capabilities: lsp_types::ClientCapabilities::default(),
            ..Default::default()
        };

        let _ = self.request("initialize", &params).await?;
        self.notify("initialized", &InitializedParams {}).await?;
        Ok(())
    }

    pub(crate) fn subscribe_diagnostics(&self) -> broadcast::Receiver<DiagnosticsUpdate> {
        self.diag_tx.subscribe()
    }

    pub(crate) async fn diagnostics_for_file(
        &self,
        file: &Path,
        wait: Duration,
    ) -> Result<Vec<Diagnostic>> {
        let uri = self.open_doc(file).await?.uri;

        // Wait for diagnostics for this URI.
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(d) = self.diagnostics.lock().await.get(&uri).cloned() {
                return Ok(d);
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(vec![]);
            }
            let remaining = deadline - now;
            tokio::time::timeout(
                remaining.min(Duration::from_millis(250)),
                self.notify.notified(),
            )
            .await
            .ok();
        }
    }

    pub(crate) async fn open_doc(&self, file: &Path) -> Result<OpenDoc> {
        let abs = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        if !abs.starts_with(&self.root) {
            anyhow::bail!("file outside LSP root");
        }
        let url =
            url::Url::from_file_path(&abs).map_err(|_| anyhow!("invalid file path for URL"))?;
        let uri: Uri = url
            .as_str()
            .parse()
            .map_err(|_| anyhow!("invalid file URI"))?;

        // If the document is already open, do not overwrite server state with a disk read.
        // Buffer-backed callers keep the doc in sync via `open_doc_with_text`.
        if self.open_docs.lock().await.contains_key(&uri) {
            return Ok(OpenDoc { uri });
        }

        let text = tokio::fs::read_to_string(&abs)
            .await
            .with_context(|| format!("reading {}", abs.to_string_lossy()))?;

        // First open: didOpen with disk text.
        let mut open_docs = self.open_docs.lock().await;
        if open_docs.contains_key(&uri) {
            return Ok(OpenDoc { uri });
        }
        open_docs.insert(uri.clone(), 1);
        drop(open_docs);
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: self.lang.id().to_string(),
                version: 1,
                text,
            },
        };
        self.notify("textDocument/didOpen", &params).await?;

        Ok(OpenDoc { uri })
    }

    pub(crate) async fn open_doc_with_text(&self, file: &Path, text: String) -> Result<OpenDoc> {
        let abs = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        if !abs.starts_with(&self.root) {
            anyhow::bail!("file outside LSP root");
        }
        let url =
            url::Url::from_file_path(&abs).map_err(|_| anyhow!("invalid file path for URL"))?;
        let uri: Uri = url
            .as_str()
            .parse()
            .map_err(|_| anyhow!("invalid file URI"))?;

        let mut open_docs = self.open_docs.lock().await;
        if let Some(v) = open_docs.get_mut(&uri) {
            *v += 1;
            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: *v,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: text.clone(),
                }],
            };
            drop(open_docs);
            self.notify("textDocument/didChange", &params).await?;
        } else {
            open_docs.insert(uri.clone(), 1);
            drop(open_docs);
            let params = DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: self.lang.id().to_string(),
                    version: 1,
                    text: text.clone(),
                },
            };
            self.notify("textDocument/didOpen", &params).await?;
        }

        Ok(OpenDoc { uri })
    }

    pub(crate) async fn request<T: serde::Serialize>(
        &self,
        method: &str,
        params: &T,
    ) -> Result<Value> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel::<Value>();
        self.pending.lock().await.insert(id, tx);
        self.tx
            .send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }))
            .await
            .context("sending LSP request")?;
        let msg = tokio::time::timeout(self.request_timeout, rx)
            .await
            .context("timeout waiting for LSP response")?
            .context("waiting for LSP response")?;
        if let Some(err) = msg.get("error") {
            anyhow::bail!("LSP error: {}", err);
        }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }

    pub(crate) async fn request_typed<R, T>(&self, method: &str, params: &T) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
        T: serde::Serialize,
    {
        let v = self.request(method, params).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub(crate) async fn execute_command_capture_edit(
        &self,
        params: &ExecuteCommandParams,
    ) -> Result<(Value, Option<WorkspaceEdit>)> {
        let (tx, rx) = oneshot::channel::<WorkspaceEdit>();
        {
            let mut cap = self.apply_edit_capture.lock().await;
            if cap.is_some() {
                anyhow::bail!("executeCommand already in progress");
            }
            *cap = Some(tx);
        }

        let request_fut = self.request("workspace/executeCommand", params);
        let capture_fut = tokio::time::timeout(self.request_timeout, rx);
        let (result, captured) = tokio::join!(request_fut, capture_fut);

        // Always clear capture.
        *self.apply_edit_capture.lock().await = None;

        let result = result?;
        let captured = match captured {
            Ok(Ok(edit)) => Some(edit),
            _ => None,
        };
        Ok((result, captured))
    }

    async fn notify<T: serde::Serialize>(&self, method: &str, params: &T) -> Result<()> {
        self.tx
            .send(json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params
            }))
            .await
            .context("sending LSP notify")?;
        Ok(())
    }
}

async fn spawn_server(
    cfg: &LspManagerConfig,
    lang: Language,
    root: &Path,
) -> Result<(
    Child,
    tokio::process::ChildStdin,
    tokio::process::ChildStdout,
)> {
    let (cmd, args) = match &lang {
        Language::Rust => (cfg.rust_command.clone(), cfg.rust_args.clone()),
        Language::TypeScript | Language::JavaScript => {
            (cfg.ts_command.clone(), cfg.ts_args.clone())
        }
        Language::Python => (cfg.py_command.clone(), cfg.py_args.clone()),
        Language::Go => (cfg.go_command.clone(), cfg.go_args.clone()),
        Language::Html => (cfg.html_command.clone(), cfg.html_args.clone()),
        Language::Css => (cfg.css_command.clone(), cfg.css_args.clone()),
        Language::Json => (cfg.json_command.clone(), cfg.json_args.clone()),
        Language::Yaml => (cfg.yaml_command.clone(), cfg.yaml_args.clone()),
        Language::Bash => (cfg.bash_command.clone(), cfg.bash_args.clone()),
        Language::Dockerfile => (cfg.dockerfile_command.clone(), cfg.dockerfile_args.clone()),
        Language::CCpp => (cfg.clangd_command.clone(), cfg.clangd_args.clone()),
        Language::Lua => (cfg.lua_command.clone(), cfg.lua_args.clone()),
        Language::Toml => (cfg.toml_command.clone(), cfg.toml_args.clone()),
        Language::Markdown => (cfg.markdown_command.clone(), cfg.markdown_args.clone()),
        Language::Custom(id) => cfg
            .custom_servers
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("no command configured for custom language: {id}"))?,
    };

    let mut c = Command::new(&cmd);
    c.args(args)
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = c
        .spawn()
        .with_context(|| format!("spawning LSP server: {cmd}"))?;
    let stdin = child.stdin.take().context("missing LSP stdin")?;
    let stdout = child.stdout.take().context("missing LSP stdout")?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = r.next_line().await {
                tracing::debug!(target: "ctx_lsp", "lsp_stderr: {}", line);
            }
        });
    }
    Ok((child, stdin, stdout))
}

async fn read_lsp_message<R: tokio::io::AsyncBufRead + Unpin>(r: &mut R) -> Result<Value> {
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("EOF");
        }
        let line_trim = line.trim_end_matches(['\r', '\n']);
        if line_trim.is_empty() {
            break;
        }
        let lower = line_trim.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            let len = rest.trim().parse::<usize>()?;
            content_len = Some(len);
        }
    }
    let len = content_len.context("missing Content-Length")?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}
