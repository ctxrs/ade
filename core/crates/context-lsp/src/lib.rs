use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use lsp_types::{
    CodeActionContext, CodeActionKind, CodeActionOrCommand, CodeActionParams, Diagnostic, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, DocumentSymbolParams, InitializeParams,
    InitializedParams, Location, Position, PublishDiagnosticsParams, Range,
    ReferenceContext, ReferenceParams, RenameParams, SymbolInformation, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, TextEdit, Uri,
    VersionedTextDocumentIdentifier, WorkspaceFolder, WorkspaceEdit, WorkspaceSymbolParams,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

#[derive(Debug, Clone)]
pub struct LspManagerConfig {
    pub enabled: bool,
    pub rust_command: String,
    pub rust_args: Vec<String>,
    pub ts_command: String,
    pub ts_args: Vec<String>,
    pub py_command: String,
    pub py_args: Vec<String>,
    pub go_command: String,
    pub go_args: Vec<String>,
    pub diagnostics_wait: Duration,
}

impl Default for LspManagerConfig {
    fn default() -> Self {
        let enabled = std::env::var("CONTEXT_LSP_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let rust_command =
            std::env::var("CONTEXT_LSP_RUST_COMMAND").unwrap_or_else(|_| "rust-analyzer".to_string());
        let rust_args = vec!["--stdio".to_string()];

        let ts_command = std::env::var("CONTEXT_LSP_TS_COMMAND")
            .unwrap_or_else(|_| "typescript-language-server".to_string());
        let ts_args = vec!["--stdio".to_string()];

        let py_command = std::env::var("CONTEXT_LSP_PY_COMMAND")
            .unwrap_or_else(|_| "pyright-langserver".to_string());
        let py_args = vec!["--stdio".to_string()];

        let go_command =
            std::env::var("CONTEXT_LSP_GO_COMMAND").unwrap_or_else(|_| "gopls".to_string());
        let go_args = vec!["-mode=stdio".to_string()];

        let diagnostics_wait = Duration::from_secs(
            std::env::var("CONTEXT_LSP_DIAGNOSTICS_WAIT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(6),
        );

        Self {
            enabled,
            rust_command,
            rust_args,
            ts_command,
            ts_args,
            py_command,
            py_args,
            go_command,
            go_args,
            diagnostics_wait,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
}

impl Language {
    pub fn detect(path: &Path) -> Option<Self> {
        match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
            "rs" => Some(Language::Rust),
            "ts" | "tsx" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "py" => Some(Language::Python),
            "go" => Some(Language::Go),
            _ => None,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
        }
    }
}

#[derive(Clone)]
pub struct LspManager {
    cfg: LspManagerConfig,
    sessions: Arc<Mutex<HashMap<(PathBuf, Language), Arc<LspSession>>>>,
}

impl LspManager {
    pub fn new(cfg: LspManagerConfig) -> Self {
        Self {
            cfg,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    pub async fn diagnostics_for_file(&self, root: &Path, file: &Path) -> Result<Vec<Diagnostic>> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CONTEXT_LSP_ENABLED=1)");
        }
        let lang = Language::detect(file).ok_or_else(|| anyhow!("no LSP language for file"))?;
        let session = self.get_or_spawn(root, lang).await?;
        session.diagnostics_for_file(file, self.cfg.diagnostics_wait).await
    }

    pub async fn definition(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                position,
            };
            session.request("textDocument/definition", &params).await
        })
        .await
    }

    pub async fn references(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
        include_declaration: bool,
    ) -> Result<Vec<Location>> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = ReferenceParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                context: ReferenceContext { include_declaration },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request_typed("textDocument/references", &params).await
        })
        .await
    }

    pub async fn document_symbols(
        &self,
        root: &Path,
        file: &Path,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("textDocument/documentSymbol", &params).await
        })
        .await
    }

    pub async fn workspace_symbols(
        &self,
        root: &Path,
        query: String,
    ) -> Result<Vec<SymbolInformation>> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CONTEXT_LSP_ENABLED=1)");
        }
        let candidates = detect_workspace_languages(root);
        let params = WorkspaceSymbolParams {
            query,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let mut last_err: Option<anyhow::Error> = None;
        for lang in candidates {
            match self.get_or_spawn(root, lang).await {
                Ok(session) => match session.request_typed("workspace/symbol", &params).await {
                    Ok(v) => return Ok(v),
                    Err(e) => last_err = Some(e),
                },
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("no LSP language for workspace")))
    }

    pub async fn rename(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
        new_name: String,
    ) -> Result<WorkspaceEdit> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = RenameParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                new_name,
                work_done_progress_params: Default::default(),
            };
            session.request_typed("textDocument/rename", &params).await
        })
        .await
    }

    pub async fn format_document(
        &self,
        root: &Path,
        file: &Path,
    ) -> Result<Vec<TextEdit>> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                options: lsp_types::FormattingOptions {
                    tab_size: 2,
                    insert_spaces: true,
                    ..Default::default()
                },
                work_done_progress_params: Default::default(),
            };
            session.request_typed("textDocument/formatting", &params).await
        })
        .await
    }

    pub async fn code_actions(
        &self,
        root: &Path,
        file: &Path,
        range: Range,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = CodeActionParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                range,
                context: CodeActionContext {
                    diagnostics,
                    only: None,
                    trigger_kind: None,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("textDocument/codeAction", &params).await
        })
        .await
    }

    pub async fn code_actions_typed(
        &self,
        root: &Path,
        file: &Path,
        range: Range,
        diagnostics: Vec<Diagnostic>,
        only: Option<Vec<CodeActionKind>>,
    ) -> Result<Vec<CodeActionOrCommand>> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = CodeActionParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                range,
                context: CodeActionContext {
                    diagnostics,
                    only,
                    trigger_kind: None,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request_typed("textDocument/codeAction", &params).await
        })
        .await
    }

    async fn get_or_spawn(&self, root: &Path, lang: Language) -> Result<Arc<LspSession>> {
        let key = (root.to_path_buf(), lang);
        let mut map = self.sessions.lock().await;
        if let Some(s) = map.get(&key) {
            return Ok(s.clone());
        }
        let created = Arc::new(LspSession::spawn(&self.cfg, root.to_path_buf(), lang).await?);
        map.insert(key, created.clone());
        Ok(created)
    }

    async fn with_open_doc<F, Fut, T>(
        &self,
        root: &Path,
        file: &Path,
        f: F,
    ) -> Result<T>
    where
        F: FnOnce(Arc<LspSession>, OpenDoc) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CONTEXT_LSP_ENABLED=1)");
        }
        let lang = Language::detect(file).ok_or_else(|| anyhow!("no LSP language for file"))?;
        let session = self.get_or_spawn(root, lang).await?;
        let doc = session.open_doc(file).await?;
        f(session, doc).await
    }
}

fn detect_workspace_languages(root: &Path) -> Vec<Language> {
    let mut langs: Vec<Language> = Vec::new();
    let push_unique = |langs: &mut Vec<Language>, l: Language| {
        if !langs.contains(&l) {
            langs.push(l);
        }
    };

    if root.join("Cargo.toml").exists() {
        push_unique(&mut langs, Language::Rust);
    }
    if root.join("package.json").exists() || root.join("tsconfig.json").exists() {
        push_unique(&mut langs, Language::TypeScript);
    }
    if root.join("pyproject.toml").exists()
        || root.join("requirements.txt").exists()
        || root.join("setup.py").exists()
    {
        push_unique(&mut langs, Language::Python);
    }
    if root.join("go.mod").exists() {
        push_unique(&mut langs, Language::Go);
    }

    if langs.is_empty() {
        push_unique(&mut langs, Language::Rust);
    }
    langs
}

#[derive(Debug, Clone)]
struct OpenDoc {
    uri: Uri,
}

struct LspSession {
    root: PathBuf,
    lang: Language,
    tx: mpsc::Sender<Value>,
    next_id: Arc<std::sync::atomic::AtomicI64>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    open_docs: Arc<Mutex<HashMap<Uri, i32>>>,
    diagnostics: Arc<Mutex<HashMap<Uri, Vec<Diagnostic>>>>,
    notify: Arc<Notify>,
    _child: Mutex<Child>,
}

impl LspSession {
    async fn spawn(cfg: &LspManagerConfig, root: PathBuf, lang: Language) -> Result<Self> {
        let (child, stdin, stdout) = spawn_server(cfg, lang, &root).await?;

        let (tx, mut rx) = mpsc::channel::<Value>(256);
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let open_docs: Arc<Mutex<HashMap<Uri, i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<Mutex<HashMap<Uri, Vec<Diagnostic>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let notify = Arc::new(Notify::new());
        let next_id = Arc::new(std::sync::atomic::AtomicI64::new(1));

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
        let mut reader = BufReader::new(stdout);
        tokio::spawn(async move {
            loop {
                let msg = match read_lsp_message(&mut reader).await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::debug!(target: "context_lsp", "lsp_read_error: {e:#}");
                        break;
                    }
                };
                let id = msg.get("id").and_then(|v| v.as_i64());
                if let Some(id) = id {
                    if let Some(tx) = pending_r.lock().await.remove(&id) {
                        let _ = tx.send(msg);
                    }
                    continue;
                }

                let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
                if method == "textDocument/publishDiagnostics" {
                    if let Some(params) = msg.get("params") {
                        if let Ok(pd) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone()) {
                            diagnostics_r.lock().await.insert(pd.uri, pd.diagnostics);
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

    async fn diagnostics_for_file(&self, file: &Path, wait: Duration) -> Result<Vec<Diagnostic>> {
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
            tokio::time::timeout(remaining.min(Duration::from_millis(250)), self.notify.notified())
                .await
                .ok();
        }
    }

    async fn open_doc(&self, file: &Path) -> Result<OpenDoc> {
        let abs = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        if !abs.starts_with(&self.root) {
            anyhow::bail!("file outside LSP root");
        }
        let url = url::Url::from_file_path(&abs).map_err(|_| anyhow!("invalid file path for URL"))?;
        let uri: Uri = url.as_str().parse().map_err(|_| anyhow!("invalid file URI"))?;

        let text = tokio::fs::read_to_string(&abs)
            .await
            .with_context(|| format!("reading {}", abs.to_string_lossy()))?;

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

    async fn request<T: serde::Serialize>(&self, method: &str, params: &T) -> Result<Value> {
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
        let msg = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .context("timeout waiting for LSP response")?
            .context("waiting for LSP response")?;
        if let Some(err) = msg.get("error") {
            anyhow::bail!("LSP error: {}", err);
        }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn request_typed<R, T>(&self, method: &str, params: &T) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
        T: serde::Serialize,
    {
        let v = self.request(method, params).await?;
        Ok(serde_json::from_value(v)?)
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
) -> Result<(Child, tokio::process::ChildStdin, tokio::process::ChildStdout)> {
    let (cmd, args) = match lang {
        Language::Rust => (cfg.rust_command.clone(), cfg.rust_args.clone()),
        Language::TypeScript | Language::JavaScript => (cfg.ts_command.clone(), cfg.ts_args.clone()),
        Language::Python => (cfg.py_command.clone(), cfg.py_args.clone()),
        Language::Go => (cfg.go_command.clone(), cfg.go_args.clone()),
    };

    let mut c = Command::new(&cmd);
    c.args(args)
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = c.spawn().with_context(|| format!("spawning LSP server: {cmd}"))?;
    let stdin = child.stdin.take().context("missing LSP stdin")?;
    let stdout = child.stdout.take().context("missing LSP stdout")?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = r.next_line().await {
                tracing::debug!(target: "context_lsp", "lsp_stderr: {}", line);
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
