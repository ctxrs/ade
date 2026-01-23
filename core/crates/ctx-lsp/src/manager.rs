use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use lsp_types::{
    CallHierarchyPrepareParams, CodeActionContext, CodeActionKind, CodeActionOrCommand,
    CodeActionParams, CodeLensParams, CompletionParams, Diagnostic, DocumentFormattingParams,
    DocumentHighlightParams, DocumentLinkParams, DocumentSymbolParams, ExecuteCommandParams,
    HoverParams, InlayHintParams, Location, Position, Range, ReferenceContext, ReferenceParams,
    RenameParams, SelectionRangeParams, SemanticTokensParams, SignatureHelpParams,
    SymbolInformation, TextDocumentIdentifier, TextDocumentPositionParams, TextEdit,
    TypeHierarchyPrepareParams, TypeHierarchySubtypesParams, TypeHierarchySupertypesParams,
    WorkspaceEdit, WorkspaceSymbolParams,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use crate::config::LspManagerConfig;
use crate::session::{DiagnosticsUpdate, Language, LspSession, OpenDoc};

type LspSessionKey = (PathBuf, Language);
type LspSessionMap = HashMap<LspSessionKey, Arc<LspSession>>;
type SharedLspSessionMap = Arc<Mutex<LspSessionMap>>;

#[derive(Clone)]
pub struct LspManager {
    cfg: LspManagerConfig,
    sessions: SharedLspSessionMap,
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
        }
        if !self.cfg.execute_commands_enabled {
            anyhow::bail!("LSP executeCommand disabled (set CTX_LSP_EXECUTE_COMMANDS_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
            anyhow::bail!("LSP disabled (set CTX_LSP_ENABLED=1)");
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
