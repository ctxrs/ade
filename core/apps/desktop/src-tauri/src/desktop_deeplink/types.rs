#[derive(Debug)]
pub(super) enum DeepLinkAction {
    Open(DeepLinkOpen),
    Reveal(DeepLinkReveal),
    Workspace(DeepLinkWorkspace),
    Task(DeepLinkTask),
    Focus,
}

#[derive(Debug)]
pub(super) struct DeepLinkOpen {
    pub(super) target: DeepLinkTarget,
    pub(super) line: Option<u32>,
    pub(super) col: Option<u32>,
    pub(super) open_with: DeepLinkOpenWith,
    pub(super) editor_override: Option<DesktopEditorTarget>,
    pub(super) token: Option<String>,
}

#[derive(Debug)]
pub(super) struct DeepLinkReveal {
    pub(super) target: DeepLinkTarget,
    pub(super) token: Option<String>,
}

#[derive(Debug)]
pub(super) struct DeepLinkWorkspace {
    pub(super) workspace_id: Option<String>,
    pub(super) path: Option<String>,
}

#[derive(Debug)]
pub(super) struct DeepLinkTask {
    pub(super) session_id: Option<String>,
    pub(super) task_id: String,
    pub(super) workspace_id: String,
}

#[derive(Debug)]
pub(super) enum DeepLinkTarget {
    WorktreeFile { worktree_id: String, file: String },
    Path { path: String },
}

#[derive(Debug)]
pub(super) struct WorktreeInfo {
    pub(super) root: PathBuf,
    pub(super) workspace_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeepLinkOpenWith {
    Ctx,
    Editor,
    System,
}
