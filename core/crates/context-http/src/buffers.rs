use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use context_core::ids::{SessionId, WorktreeId};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BufferId(pub uuid::Uuid);

impl BufferId {
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

#[derive(Debug, Clone)]
pub struct BufferKey {
    pub worktree_id: WorktreeId,
    pub path: PathBuf,
}

impl BufferKey {
    pub fn new(worktree_id: WorktreeId, path: PathBuf) -> Self {
        Self { worktree_id, path }
    }
}

impl std::hash::Hash for BufferKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.worktree_id.hash(state);
        self.path.hash(state);
    }
}

impl PartialEq for BufferKey {
    fn eq(&self, other: &Self) -> bool {
        self.worktree_id == other.worktree_id && self.path == other.path
    }
}

impl Eq for BufferKey {}

#[derive(Debug, Clone)]
pub struct BufferState {
    pub id: BufferId,
    pub worktree_id: WorktreeId,
    pub root: PathBuf,
    pub path: PathBuf,
    pub text: String,
    pub version: u64,
    pub last_disk_sha256: String,
    pub open_count: u32,
    pub watchers: HashSet<SessionId>,
}

#[derive(Debug, Serialize)]
pub struct BufferOpenResp {
    pub buffer_id: String,
    pub path: String,
    pub version: u64,
    pub text: String,
    pub last_disk_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct BufferUpdateResp {
    pub buffer_id: String,
    pub version: u64,
    pub last_disk_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct BufferConflictResp {
    pub error: String,
    pub disk_sha256: String,
    pub disk_text: String,
}

#[derive(Debug, Deserialize)]
pub struct BufferOpenReq {
    pub session_id: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct BufferUpdateReq {
    pub buffer_id: String,
    pub version: u64,
    pub text: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default = "default_true")]
    pub persist: bool,
}

#[derive(Debug, Deserialize)]
pub struct BufferCloseReq {
    pub session_id: String,
    pub buffer_id: String,
}

pub struct BufferStore {
    by_id: Mutex<HashMap<BufferId, BufferState>>,
    by_key: Mutex<HashMap<BufferKey, BufferId>>,
}

impl Default for BufferStore {
    fn default() -> Self {
        Self {
            by_id: Mutex::new(HashMap::new()),
            by_key: Mutex::new(HashMap::new()),
        }
    }
}

impl BufferStore {
    pub async fn open_or_reuse(
        &self,
        session_id: SessionId,
        worktree_id: WorktreeId,
        root: PathBuf,
        abs_path: PathBuf,
        text: String,
        sha256: String,
    ) -> BufferState {
        let key = BufferKey::new(worktree_id, abs_path.clone());
        if let Some(existing_id) = self.by_key.lock().await.get(&key).cloned() {
            let mut by_id = self.by_id.lock().await;
            if let Some(st) = by_id.get_mut(&existing_id) {
                st.open_count = st.open_count.saturating_add(1);
                st.watchers.insert(session_id);
                return st.clone();
            }
        }

        let id = BufferId::new_v4();
        let mut watchers = HashSet::new();
        watchers.insert(session_id);
        let state = BufferState {
            id,
            worktree_id,
            root,
            path: abs_path.clone(),
            text,
            version: 1,
            last_disk_sha256: sha256,
            open_count: 1,
            watchers,
        };
        self.by_id.lock().await.insert(id, state.clone());
        self.by_key.lock().await.insert(key, id);
        state
    }

    pub async fn get(&self, id: BufferId) -> Option<BufferState> {
        self.by_id.lock().await.get(&id).cloned()
    }

    pub async fn update(
        &self,
        id: BufferId,
        version: u64,
        text: String,
        new_sha256: Option<String>,
    ) -> Result<BufferState> {
        let mut by_id = self.by_id.lock().await;
        let st = by_id
            .get_mut(&id)
            .ok_or_else(|| anyhow!("buffer not found"))?;
        if version <= st.version {
            anyhow::bail!("stale buffer version");
        }
        st.version = version;
        st.text = text;
        if let Some(sha) = new_sha256 {
            st.last_disk_sha256 = sha;
        }
        Ok(st.clone())
    }

    pub async fn close(&self, id: BufferId, session_id: SessionId) -> Option<BufferState> {
        let mut by_id = self.by_id.lock().await;
        let st = by_id.get_mut(&id)?;
        st.open_count = st.open_count.saturating_sub(1);
        st.watchers.remove(&session_id);
        if st.open_count > 0 {
            return Some(st.clone());
        }

        let st = by_id.remove(&id)?;
        drop(by_id);
        self.by_key
            .lock()
            .await
            .retain(|k, v| !(k.worktree_id == st.worktree_id && v == &id));
        Some(st)
    }

    pub async fn watchers_for_abs_path(&self, abs_path: &Path) -> Vec<(SessionId, PathBuf)> {
        let by_id = self.by_id.lock().await;
        by_id
            .values()
            .filter(|st| st.path == abs_path)
            .flat_map(|st| {
                st.watchers
                    .iter()
                    .copied()
                    .map(|sid| {
                        let rel = st
                            .path
                            .strip_prefix(&st.root)
                            .unwrap_or(&st.path)
                            .to_path_buf();
                        (sid, rel)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    }

    pub async fn resolve_path(root: &Path, path: &str) -> Result<PathBuf> {
        let candidate = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            root.join(path)
        };
        let file = candidate
            .canonicalize()
            .with_context(|| "canonicalize buffer path")?;
        if !file.starts_with(root) {
            anyhow::bail!("path outside root");
        }
        Ok(file)
    }
}

pub fn sha256_hex(text: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn default_true() -> bool {
    true
}
