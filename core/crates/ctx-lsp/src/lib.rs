use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use lsp_types::{
    ApplyWorkspaceEditParams, ApplyWorkspaceEditResponse, CallHierarchyPrepareParams,
    CodeActionContext, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeLensParams,
    CompletionParams, Diagnostic, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentHighlightParams, DocumentLinkParams, DocumentSymbolParams,
    ExecuteCommandParams, HoverParams, InitializeParams, InitializedParams, InlayHintParams,
    Location, Position, PublishDiagnosticsParams, Range, ReferenceContext, ReferenceParams,
    RenameParams, SelectionRangeParams, SemanticTokensParams, SignatureHelpParams,
    SymbolInformation, TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, TextEdit, TypeHierarchyPrepareParams, TypeHierarchySubtypesParams,
    TypeHierarchySupertypesParams, Uri, VersionedTextDocumentIdentifier, WorkspaceEdit,
    WorkspaceFolder, WorkspaceSymbolParams,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify};

fn ctx_env(name: &str) -> std::result::Result<String, std::env::VarError> {
    std::env::var(format!("CTX_{name}")).or_else(|_| std::env::var(format!("CONTEXT_{name}")))
}

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
    pub html_command: String,
    pub html_args: Vec<String>,
    pub css_command: String,
    pub css_args: Vec<String>,
    pub json_command: String,
    pub json_args: Vec<String>,
    pub yaml_command: String,
    pub yaml_args: Vec<String>,
    pub bash_command: String,
    pub bash_args: Vec<String>,
    pub dockerfile_command: String,
    pub dockerfile_args: Vec<String>,
    pub clangd_command: String,
    pub clangd_args: Vec<String>,
    pub lua_command: String,
    pub lua_args: Vec<String>,
    pub toml_command: String,
    pub toml_args: Vec<String>,
    pub markdown_command: String,
    pub markdown_args: Vec<String>,
    /// User-provided servers keyed by language id (BYO LSP).
    pub custom_servers: HashMap<String, (String, Vec<String>)>,
    /// Extension -> language id mapping for BYO LSP (no dot, lowercased).
    pub custom_extension_map: HashMap<String, String>,
    /// Filename -> language id mapping for BYO LSP (exact match).
    pub custom_filename_map: HashMap<String, String>,
    pub diagnostics_wait: Duration,
    pub execute_commands_enabled: bool,
    pub execute_command_allowlist: Vec<String>,
    pub request_timeout: Duration,
}

impl Default for LspManagerConfig {
    fn default() -> Self {
        let enabled = ctx_env("LSP_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let rust_command =
            ctx_env("LSP_RUST_COMMAND").unwrap_or_else(|_| "rust-analyzer".to_string());
        let rust_args = vec!["--stdio".to_string()];

        let ts_command =
            ctx_env("LSP_TS_COMMAND").unwrap_or_else(|_| "typescript-language-server".to_string());
        let ts_args = vec!["--stdio".to_string()];

        let py_command =
            ctx_env("LSP_PY_COMMAND").unwrap_or_else(|_| "pyright-langserver".to_string());
        let py_args = vec!["--stdio".to_string()];

        let go_command = ctx_env("LSP_GO_COMMAND").unwrap_or_else(|_| "gopls".to_string());
        let go_args = vec!["-mode=stdio".to_string()];

        let html_command = ctx_env("LSP_HTML_COMMAND")
            .unwrap_or_else(|_| "vscode-html-language-server".to_string());
        let html_args = vec!["--stdio".to_string()];

        let css_command =
            ctx_env("LSP_CSS_COMMAND").unwrap_or_else(|_| "vscode-css-language-server".to_string());
        let css_args = vec!["--stdio".to_string()];

        let json_command = ctx_env("LSP_JSON_COMMAND")
            .unwrap_or_else(|_| "vscode-json-language-server".to_string());
        let json_args = vec!["--stdio".to_string()];

        let yaml_command =
            ctx_env("LSP_YAML_COMMAND").unwrap_or_else(|_| "yaml-language-server".to_string());
        let yaml_args = vec!["--stdio".to_string()];

        let bash_command =
            ctx_env("LSP_BASH_COMMAND").unwrap_or_else(|_| "bash-language-server".to_string());
        let bash_args = vec!["start".to_string(), "--stdio".to_string()];

        let dockerfile_command =
            ctx_env("LSP_DOCKERFILE_COMMAND").unwrap_or_else(|_| "docker-langserver".to_string());
        let dockerfile_args = vec!["--stdio".to_string()];

        let clangd_command = ctx_env("LSP_CLANGD_COMMAND").unwrap_or_else(|_| "clangd".to_string());
        let clangd_args = vec!["--stdio".to_string()];

        let lua_command =
            ctx_env("LSP_LUA_COMMAND").unwrap_or_else(|_| "lua-language-server".to_string());
        let lua_args = vec![];

        let toml_command = ctx_env("LSP_TOML_COMMAND").unwrap_or_else(|_| "taplo".to_string());
        let toml_args = vec!["lsp".to_string(), "stdio".to_string()];

        let markdown_command =
            ctx_env("LSP_MARKDOWN_COMMAND").unwrap_or_else(|_| "marksman".to_string());
        let markdown_args = vec!["server".to_string()];

        let diagnostics_wait = Duration::from_secs(
            ctx_env("LSP_DIAGNOSTICS_WAIT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(6),
        );

        let execute_commands_enabled = ctx_env("LSP_EXECUTE_COMMANDS_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let execute_command_allowlist = ctx_env("LSP_EXECUTE_COMMAND_ALLOWLIST")
            .ok()
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let request_timeout = Duration::from_secs(
            ctx_env("LSP_REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
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
            html_command,
            html_args,
            css_command,
            css_args,
            json_command,
            json_args,
            yaml_command,
            yaml_args,
            bash_command,
            bash_args,
            dockerfile_command,
            dockerfile_args,
            clangd_command,
            clangd_args,
            lua_command,
            lua_args,
            toml_command,
            toml_args,
            markdown_command,
            markdown_args,
            custom_servers: HashMap::new(),
            custom_extension_map: HashMap::new(),
            custom_filename_map: HashMap::new(),
            diagnostics_wait,
            execute_commands_enabled,
            execute_command_allowlist,
            request_timeout,
        }
    }
}

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

type LspSessionKey = (PathBuf, Language);
type LspSessionMap = HashMap<LspSessionKey, Arc<LspSession>>;
type SharedLspSessionMap = Arc<Mutex<LspSessionMap>>;

#[derive(Clone)]
pub struct LspManager {
    cfg: LspManagerConfig,
    sessions: SharedLspSessionMap,
}

#[derive(Debug, Clone)]
pub struct DiagnosticsUpdate {
    pub uri: Uri,
    pub diagnostics: Vec<Diagnostic>,
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let lang =
            Language::detect(file, &self.cfg).ok_or_else(|| anyhow!("no LSP language for file"))?;
        let session = self.get_or_spawn(root, lang).await?;
        session
            .diagnostics_for_file(file, self.cfg.diagnostics_wait)
            .await
    }

    pub async fn definition(&self, root: &Path, file: &Path, position: Position) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                position,
            };
            session.request("textDocument/definition", &params).await
        })
        .await
    }

    pub async fn type_definition(
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
            session
                .request("textDocument/typeDefinition", &params)
                .await
        })
        .await
    }

    pub async fn implementation(
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
            session
                .request("textDocument/implementation", &params)
                .await
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
                context: ReferenceContext {
                    include_declaration,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session
                .request_typed("textDocument/references", &params)
                .await
        })
        .await
    }

    pub async fn hover(&self, root: &Path, file: &Path, position: Position) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                work_done_progress_params: Default::default(),
            };
            session.request("textDocument/hover", &params).await
        })
        .await
    }

    pub async fn signature_help(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = SignatureHelpParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                context: None,
                work_done_progress_params: Default::default(),
            };
            session.request("textDocument/signatureHelp", &params).await
        })
        .await
    }

    pub async fn completion(&self, root: &Path, file: &Path, position: Position) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                context: None,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("textDocument/completion", &params).await
        })
        .await
    }

    pub async fn completion_resolve(
        &self,
        root: &Path,
        file: &Path,
        completion_item: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            session
                .request("completionItem/resolve", &completion_item)
                .await
        })
        .await
    }

    pub async fn code_action_resolve(
        &self,
        root: &Path,
        file: &Path,
        code_action: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            session.request("codeAction/resolve", &code_action).await
        })
        .await
    }

    pub async fn inlay_hints(&self, root: &Path, file: &Path, range: Range) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = InlayHintParams {
                work_done_progress_params: Default::default(),
                text_document: TextDocumentIdentifier { uri: doc.uri },
                range,
            };
            session.request("textDocument/inlayHint", &params).await
        })
        .await
    }

    pub async fn document_highlight(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = DocumentHighlightParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session
                .request("textDocument/documentHighlight", &params)
                .await
        })
        .await
    }

    pub async fn selection_ranges(
        &self,
        root: &Path,
        file: &Path,
        positions: Vec<Position>,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = SelectionRangeParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                positions,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session
                .request("textDocument/selectionRange", &params)
                .await
        })
        .await
    }

    pub async fn call_hierarchy_prepare(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = CallHierarchyPrepareParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                work_done_progress_params: Default::default(),
            };
            session
                .request("textDocument/prepareCallHierarchy", &params)
                .await
        })
        .await
    }

    pub async fn call_hierarchy_incoming(
        &self,
        root: &Path,
        file: &Path,
        item: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            let params = json!({ "item": item });
            session
                .request("callHierarchy/incomingCalls", &params)
                .await
        })
        .await
    }

    pub async fn call_hierarchy_outgoing(
        &self,
        root: &Path,
        file: &Path,
        item: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            let params = json!({ "item": item });
            session
                .request("callHierarchy/outgoingCalls", &params)
                .await
        })
        .await
    }

    pub async fn code_lens(&self, root: &Path, file: &Path) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = CodeLensParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("textDocument/codeLens", &params).await
        })
        .await
    }

    pub async fn code_lens_resolve(
        &self,
        root: &Path,
        file: &Path,
        code_lens: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            session.request("codeLens/resolve", &code_lens).await
        })
        .await
    }

    pub async fn prepare_rename(
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
            session.request("textDocument/prepareRename", &params).await
        })
        .await
    }

    pub async fn document_links(&self, root: &Path, file: &Path) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = DocumentLinkParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("textDocument/documentLink", &params).await
        })
        .await
    }

    pub async fn document_link_resolve(
        &self,
        root: &Path,
        file: &Path,
        link: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            session.request("documentLink/resolve", &link).await
        })
        .await
    }

    pub async fn semantic_tokens_full(&self, root: &Path, file: &Path) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = SemanticTokensParams {
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                text_document: TextDocumentIdentifier { uri: doc.uri },
            };
            session
                .request("textDocument/semanticTokens/full", &params)
                .await
        })
        .await
    }

    pub async fn semantic_tokens_delta(
        &self,
        root: &Path,
        file: &Path,
        previous_result_id: String,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = json!({
                "textDocument": { "uri": doc.uri },
                "previousResultId": previous_result_id,
            });
            session
                .request("textDocument/semanticTokens/full/delta", &params)
                .await
        })
        .await
    }

    pub async fn folding_ranges(&self, root: &Path, file: &Path) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = json!({
                "textDocument": { "uri": doc.uri },
            });
            session.request("textDocument/foldingRange", &params).await
        })
        .await
    }

    pub async fn linked_editing_range(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = json!({
                "textDocument": { "uri": doc.uri },
                "position": position,
            });
            session
                .request("textDocument/linkedEditingRange", &params)
                .await
        })
        .await
    }

    pub async fn type_hierarchy_prepare(
        &self,
        root: &Path,
        file: &Path,
        position: Position,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = TypeHierarchyPrepareParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: doc.uri },
                    position,
                },
                work_done_progress_params: Default::default(),
            };
            session
                .request("textDocument/prepareTypeHierarchy", &params)
                .await
        })
        .await
    }

    pub async fn type_hierarchy_supertypes(
        &self,
        root: &Path,
        file: &Path,
        item: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            let params = TypeHierarchySupertypesParams {
                item: serde_json::from_value(item)?,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("typeHierarchy/supertypes", &params).await
        })
        .await
    }

    pub async fn type_hierarchy_subtypes(
        &self,
        root: &Path,
        file: &Path,
        item: Value,
    ) -> Result<Value> {
        self.with_open_doc(root, file, |session, _doc| async move {
            let params = TypeHierarchySubtypesParams {
                item: serde_json::from_value(item)?,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session.request("typeHierarchy/subtypes", &params).await
        })
        .await
    }

    pub async fn execute_command_for_file(
        &self,
        root: &Path,
        file: &Path,
        command: String,
        arguments: Vec<Value>,
    ) -> Result<(Value, Option<WorkspaceEdit>)> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        if !self.cfg.execute_commands_enabled {
            anyhow::bail!(
                "LSP executeCommand disabled (set CTX_LSP_EXECUTE_COMMANDS_ENABLED=1 (or CONTEXT_LSP_EXECUTE_COMMANDS_ENABLED=1))"
            );
        }
        if self.cfg.execute_command_allowlist.is_empty()
            || !self
                .cfg
                .execute_command_allowlist
                .iter()
                .any(|c| c == &command)
        {
            anyhow::bail!("executeCommand not allowlisted: {command}");
        }

        self.with_open_doc(root, file, |session, _doc| async move {
            let params = ExecuteCommandParams {
                command,
                arguments,
                work_done_progress_params: Default::default(),
            };
            session.execute_command_capture_edit(&params).await
        })
        .await
    }

    pub async fn sync_document_text(&self, root: &Path, file: &Path, text: String) -> Result<()> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let lang =
            Language::detect(file, &self.cfg).ok_or_else(|| anyhow!("no LSP language for file"))?;
        let session = self.get_or_spawn(root, lang).await?;
        let _ = session.open_doc_with_text(file, text).await?;
        Ok(())
    }

    pub async fn subscribe_diagnostics_for_file(
        &self,
        root: &Path,
        file: &Path,
    ) -> Result<broadcast::Receiver<DiagnosticsUpdate>> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let lang =
            Language::detect(file, &self.cfg).ok_or_else(|| anyhow!("no LSP language for file"))?;
        let session = self.get_or_spawn(root, lang).await?;
        Ok(session.subscribe_diagnostics())
    }

    pub async fn subscribe_diagnostics_for_language(
        &self,
        root: &Path,
        lang: Language,
    ) -> Result<broadcast::Receiver<DiagnosticsUpdate>> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let session = self.get_or_spawn(root, lang).await?;
        Ok(session.subscribe_diagnostics())
    }

    pub async fn document_symbols(&self, root: &Path, file: &Path) -> Result<Value> {
        self.with_open_doc(root, file, |session, doc| async move {
            let params = DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri: doc.uri },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            session
                .request("textDocument/documentSymbol", &params)
                .await
        })
        .await
    }

    pub async fn workspace_symbols(
        &self,
        root: &Path,
        query: String,
    ) -> Result<Vec<SymbolInformation>> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let candidates = detect_workspace_languages(root, &self.cfg);
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

    pub async fn workspace_symbol_resolve(&self, root: &Path, item: Value) -> Result<Value> {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let candidates = detect_workspace_languages(root, &self.cfg);

        let mut last_err: Option<anyhow::Error> = None;
        for lang in candidates {
            match self.get_or_spawn(root, lang).await {
                Ok(session) => match session.request("workspace/symbol/resolve", &item).await {
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

    pub async fn format_document(&self, root: &Path, file: &Path) -> Result<Vec<TextEdit>> {
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
            session
                .request_typed("textDocument/formatting", &params)
                .await
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
            session
                .request_typed("textDocument/codeAction", &params)
                .await
        })
        .await
    }

    async fn get_or_spawn(&self, root: &Path, lang: Language) -> Result<Arc<LspSession>> {
        let key = (root.to_path_buf(), lang.clone());
        let mut map = self.sessions.lock().await;
        if let Some(s) = map.get(&key) {
            return Ok(s.clone());
        }
        let created = Arc::new(LspSession::spawn(&self.cfg, root.to_path_buf(), lang).await?);
        map.insert(key, created.clone());
        Ok(created)
    }

    async fn with_open_doc<F, Fut, T>(&self, root: &Path, file: &Path, f: F) -> Result<T>
    where
        F: FnOnce(Arc<LspSession>, OpenDoc) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        if !self.cfg.enabled {
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1 (or CONTEXT_LSP_ENABLED=1))");
        }
        let lang =
            Language::detect(file, &self.cfg).ok_or_else(|| anyhow!("no LSP language for file"))?;
        let session = self.get_or_spawn(root, lang).await?;
        let doc = session.open_doc(file).await?;
        f(session, doc).await
    }
}

fn detect_workspace_languages(root: &Path, cfg: &LspManagerConfig) -> Vec<Language> {
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
    if root.join("Dockerfile").exists() {
        push_unique(&mut langs, Language::Dockerfile);
    }
    if root.join("compile_commands.json").exists() {
        push_unique(&mut langs, Language::CCpp);
    }
    if root.join(".luarc.json").exists() {
        push_unique(&mut langs, Language::Lua);
    }
    if root.join("README.md").exists() {
        push_unique(&mut langs, Language::Markdown);
    }

    for id in cfg.custom_servers.keys() {
        push_unique(&mut langs, Language::Custom(id.clone()));
    }

    if langs.is_empty() {
        push_unique(&mut langs, Language::Rust);
    }
    langs
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::Prefix(p) => out.push(p.as_os_str()),
            std::path::Component::RootDir => out.push(std::path::MAIN_SEPARATOR_STR),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(x) => out.push(x),
        }
    }
    out
}

fn uri_to_abs_path_under_root(root: &Path, uri: &Uri) -> Result<PathBuf> {
    let url = url::Url::parse(uri.as_str())
        .map_err(|e| anyhow!("invalid URI {:?}: {e}", uri.as_str()))?;
    let p = url
        .to_file_path()
        .map_err(|_| anyhow!("unsupported URI (expected file://): {:?}", uri.as_str()))?;
    let p = normalize_path(&p);
    if !p.starts_with(root) {
        anyhow::bail!(
            "refusing to apply edit outside root: {}",
            p.to_string_lossy()
        );
    }
    Ok(p)
}

fn build_line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    for (i, b) in text.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            out.push(i + 1);
        }
    }
    out
}

fn byte_offset_for_position_utf16(
    text: &str,
    line_starts: &[usize],
    pos: Position,
) -> Result<usize> {
    let line = pos.line as usize;
    let character_u16 = pos.character as usize;
    if line >= line_starts.len() {
        anyhow::bail!("position line out of bounds");
    }
    let start = line_starts[line];
    let end = if line + 1 < line_starts.len() {
        line_starts[line + 1]
    } else {
        text.len()
    };
    let mut u16 = 0usize;
    for (rel, ch) in text[start..end].char_indices() {
        if u16 == character_u16 {
            return Ok(start + rel);
        }
        u16 += ch.len_utf16();
        if u16 > character_u16 {
            return Ok(start + rel);
        }
    }
    if u16 == character_u16 {
        return Ok(end);
    }
    anyhow::bail!("position character out of bounds");
}

fn apply_text_edits_to_string(original: &str, edits: &[TextEdit]) -> Result<String> {
    if edits.is_empty() {
        return Ok(original.to_string());
    }
    let line_starts = build_line_starts(original);
    let mut computed: Vec<(usize, usize, String)> = Vec::with_capacity(edits.len());
    for e in edits {
        let start = byte_offset_for_position_utf16(original, &line_starts, e.range.start)?;
        let end = byte_offset_for_position_utf16(original, &line_starts, e.range.end)?;
        if start > end || end > original.len() {
            anyhow::bail!("invalid text edit range");
        }
        computed.push((start, end, e.new_text.clone()));
    }
    computed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for pair in computed.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        if cur.1 > prev.0 {
            anyhow::bail!("overlapping text edits are not supported");
        }
    }
    let mut out = original.to_string();
    for (start, end, new_text) in computed {
        out.replace_range(start..end, &new_text);
    }
    Ok(out)
}

async fn apply_workspace_edit_to_disk(
    root: &Path,
    edit: &WorkspaceEdit,
) -> Result<Vec<(Uri, String)>> {
    let mut changed: Vec<(Uri, String)> = Vec::new();

    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            let path = uri_to_abs_path_under_root(root, uri)?;
            let before = match tokio::fs::read_to_string(&path).await {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => {
                    return Err(e).with_context(|| format!("reading {}", path.to_string_lossy()))
                }
            };
            let after = apply_text_edits_to_string(&before, edits)?;
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::write(&path, &after)
                .await
                .with_context(|| format!("writing {}", path.to_string_lossy()))?;
            changed.push((uri.clone(), after));
        }
        return Ok(changed);
    }

    if let Some(document_changes) = &edit.document_changes {
        match document_changes {
            lsp_types::DocumentChanges::Edits(edits) => {
                for tde in edits {
                    let uri = &tde.text_document.uri;
                    let path = uri_to_abs_path_under_root(root, uri)?;
                    let before = match tokio::fs::read_to_string(&path).await {
                        Ok(s) => s,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                        Err(e) => {
                            return Err(e)
                                .with_context(|| format!("reading {}", path.to_string_lossy()))
                        }
                    };
                    let edits: Vec<TextEdit> = tde
                        .edits
                        .iter()
                        .map(|e| match e {
                            lsp_types::OneOf::Left(te) => te.clone(),
                            lsp_types::OneOf::Right(ate) => ate.text_edit.clone(),
                        })
                        .collect();
                    let after = apply_text_edits_to_string(&before, &edits)?;
                    if let Some(parent) = path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    tokio::fs::write(&path, &after)
                        .await
                        .with_context(|| format!("writing {}", path.to_string_lossy()))?;
                    changed.push((uri.clone(), after));
                }
            }
            lsp_types::DocumentChanges::Operations(ops) => {
                for op in ops {
                    match op {
                        lsp_types::DocumentChangeOperation::Edit(tde) => {
                            let uri = &tde.text_document.uri;
                            let path = uri_to_abs_path_under_root(root, uri)?;
                            let before = match tokio::fs::read_to_string(&path).await {
                                Ok(s) => s,
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                                Err(e) => {
                                    return Err(e).with_context(|| {
                                        format!("reading {}", path.to_string_lossy())
                                    })
                                }
                            };
                            let edits: Vec<TextEdit> = tde
                                .edits
                                .iter()
                                .map(|e| match e {
                                    lsp_types::OneOf::Left(te) => te.clone(),
                                    lsp_types::OneOf::Right(ate) => ate.text_edit.clone(),
                                })
                                .collect();
                            let after = apply_text_edits_to_string(&before, &edits)?;
                            if let Some(parent) = path.parent() {
                                let _ = tokio::fs::create_dir_all(parent).await;
                            }
                            tokio::fs::write(&path, &after)
                                .await
                                .with_context(|| format!("writing {}", path.to_string_lossy()))?;
                            changed.push((uri.clone(), after));
                        }
                        lsp_types::DocumentChangeOperation::Op(op) => match op {
                            lsp_types::ResourceOp::Create(cf) => {
                                let uri = &cf.uri;
                                let path = uri_to_abs_path_under_root(root, uri)?;
                                if path.exists()
                                    && !cf
                                        .options
                                        .as_ref()
                                        .and_then(|o| o.overwrite)
                                        .unwrap_or(false)
                                {
                                    anyhow::bail!(
                                        "refusing to overwrite existing file {}",
                                        path.to_string_lossy()
                                    );
                                }
                                if let Some(parent) = path.parent() {
                                    tokio::fs::create_dir_all(parent).await.with_context(|| {
                                        format!("creating {}", parent.to_string_lossy())
                                    })?;
                                }
                                tokio::fs::write(&path, "").await.with_context(|| {
                                    format!("creating {}", path.to_string_lossy())
                                })?;
                            }
                            lsp_types::ResourceOp::Rename(rf) => {
                                let old_path = uri_to_abs_path_under_root(root, &rf.old_uri)?;
                                let new_path = uri_to_abs_path_under_root(root, &rf.new_uri)?;
                                if new_path.exists()
                                    && !rf
                                        .options
                                        .as_ref()
                                        .and_then(|o| o.overwrite)
                                        .unwrap_or(false)
                                {
                                    anyhow::bail!(
                                        "refusing to overwrite existing file {}",
                                        new_path.to_string_lossy()
                                    );
                                }
                                if let Some(parent) = new_path.parent() {
                                    let _ = tokio::fs::create_dir_all(parent).await;
                                }
                                tokio::fs::rename(&old_path, &new_path).await.with_context(
                                    || format!("renaming {}", old_path.to_string_lossy()),
                                )?;
                            }
                            lsp_types::ResourceOp::Delete(df) => {
                                let path = uri_to_abs_path_under_root(root, &df.uri)?;
                                if !path.exists()
                                    && df
                                        .options
                                        .as_ref()
                                        .and_then(|o| o.ignore_if_not_exists)
                                        .unwrap_or(false)
                                {
                                    continue;
                                }
                                tokio::fs::remove_file(&path).await.with_context(|| {
                                    format!("deleting {}", path.to_string_lossy())
                                })?;
                            }
                        },
                    }
                }
            }
        }
    }

    Ok(changed)
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
    diag_tx: broadcast::Sender<DiagnosticsUpdate>,
    apply_edit_capture: Arc<Mutex<Option<oneshot::Sender<WorkspaceEdit>>>>,
    request_timeout: Duration,
    _child: Mutex<Child>,
}

impl LspSession {
    async fn spawn(cfg: &LspManagerConfig, root: PathBuf, lang: Language) -> Result<Self> {
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

    fn subscribe_diagnostics(&self) -> broadcast::Receiver<DiagnosticsUpdate> {
        self.diag_tx.subscribe()
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
            tokio::time::timeout(
                remaining.min(Duration::from_millis(250)),
                self.notify.notified(),
            )
            .await
            .ok();
        }
    }

    async fn open_doc(&self, file: &Path) -> Result<OpenDoc> {
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

    async fn open_doc_with_text(&self, file: &Path, text: String) -> Result<OpenDoc> {
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
        let msg = tokio::time::timeout(self.request_timeout, rx)
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

    async fn execute_command_capture_edit(
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
