use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McpAuthCapabilities {
    pub(crate) subagents: bool,
    pub(crate) artifacts: bool,
    pub(crate) merge_queue_submit: bool,
}

impl McpAuthCapabilities {
    pub(crate) fn provider_session() -> Self {
        Self {
            subagents: true,
            artifacts: true,
            merge_queue_submit: false,
        }
    }

    pub(crate) fn provider_turn_default() -> Self {
        Self {
            subagents: true,
            artifacts: true,
            merge_queue_submit: true,
        }
    }

    pub(crate) fn names(self) -> Vec<&'static str> {
        let mut values = Vec::new();
        if self.subagents {
            values.push("subagents");
        }
        if self.artifacts {
            values.push("artifacts");
        }
        if self.merge_queue_submit {
            values.push("merge_queue_submit");
        }
        values
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McpAuthContext {
    pub(crate) session_id: SessionId,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) worktree_id: WorktreeId,
    pub(crate) capabilities: McpAuthCapabilities,
}

impl McpAuthContext {
    pub(crate) fn provider_session(
        session_id: SessionId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        capabilities: McpAuthCapabilities,
    ) -> Self {
        Self {
            session_id,
            workspace_id,
            worktree_id,
            capabilities,
        }
    }

    pub(crate) fn allows_subagents(self, session_id: SessionId) -> bool {
        self.capabilities.subagents && self.session_id == session_id
    }

    pub(crate) fn allows_artifacts(self, session_id: SessionId) -> bool {
        self.capabilities.artifacts && self.session_id == session_id
    }

    pub(crate) fn allows_merge_queue_submit(
        self,
        session_id: SessionId,
        worktree_id: WorktreeId,
    ) -> bool {
        self.capabilities.merge_queue_submit
            && self.session_id == session_id
            && self.worktree_id == worktree_id
    }
}
