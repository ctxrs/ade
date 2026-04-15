use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use ctx_core::ids::{SessionId, WorktreeId};
use lsp_types::{TextEdit, Uri, WorkspaceEdit};
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EditPlanId(pub uuid::Uuid);

impl EditPlanId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for EditPlanId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPlanSummary {
    pub id: EditPlanId,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub remaining_files: usize,
    pub remaining_hunks: usize,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPlan {
    pub id: EditPlanId,
    pub session_id: SessionId,
    pub worktree_id: WorktreeId,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub worktree_root: PathBuf,
    pub files: Vec<PlanFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanFile {
    pub key: String,
    pub old_path: String,
    pub new_path: String,
    /// SHA256 of the file contents used as the base when the plan was created.
    #[serde(default)]
    pub base_sha256: String,
    pub header_lines: Vec<String>,
    pub hunks: Vec<PlanHunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanHunk {
    pub header_line: String,
    pub lines: Vec<String>,
}

impl EditPlan {
    pub fn to_summary(&self) -> EditPlanSummary {
        let remaining_files = self.files.iter().filter(|f| !f.hunks.is_empty()).count();
        let remaining_hunks = self.files.iter().map(|f| f.hunks.len()).sum::<usize>();
        EditPlanSummary {
            id: self.id,
            title: self.title.clone(),
            created_at: self.created_at,
            remaining_files,
            remaining_hunks,
            diff: render_unified_diff(&self.files),
        }
    }

    pub fn remove_patch(&mut self, patch: &str) {
        let parsed = parse_unified_diff(patch);
        for pf in parsed {
            for f in &mut self.files {
                if !paths_match(&pf.old_path, &pf.new_path, &f.old_path, &f.new_path) {
                    continue;
                }
                // Remove hunks by exact header+lines match.
                f.hunks.retain(|h| {
                    !pf.hunks.iter().any(|ph| {
                        ph.header_line == h.header_line && hunk_lines_match(&ph.lines, &h.lines)
                    })
                });
            }
        }
        self.files.retain(|f| !f.hunks.is_empty());
    }
}

fn hunk_lines_match(a: &[String], b: &[String]) -> bool {
    fn trim_len(lines: &[String]) -> usize {
        let mut n = lines.len();
        while n > 0 && lines[n - 1].is_empty() {
            n -= 1;
        }
        n
    }
    let al = trim_len(a);
    let bl = trim_len(b);
    a[..al] == b[..bl]
}

fn paths_match(a_old: &str, a_new: &str, b_old: &str, b_new: &str) -> bool {
    (!a_new.is_empty() || a_old == b_old) && a_new == b_new
}

pub fn render_unified_diff(files: &[PlanFile]) -> String {
    let mut out = String::new();
    for f in files {
        if f.hunks.is_empty() {
            continue;
        }
        for (idx, line) in f.header_lines.iter().enumerate() {
            out.push_str(line);
            if idx + 1 < f.header_lines.len() || !f.header_lines.is_empty() {
                out.push('\n');
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for h in &f.hunks {
            out.push_str(&h.header_line);
            out.push('\n');
            for l in &h.lines {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    out
}

pub fn parse_unified_diff(diff_text: &str) -> Vec<PlanFile> {
    let lines = diff_text.split('\n').collect::<Vec<_>>();
    let mut files: Vec<PlanFile> = Vec::new();
    let mut current: Option<PlanFile> = None;
    let mut in_header = false;
    let mut current_hunk: Option<PlanHunk> = None;

    let push_current = |files: &mut Vec<PlanFile>,
                        current: &mut Option<PlanFile>,
                        current_hunk: &mut Option<PlanHunk>| {
        if let Some(mut c) = current.take() {
            if let Some(h) = current_hunk.take() {
                c.hunks.push(h);
            }
            if !c.header_lines.is_empty() || !c.hunks.is_empty() {
                files.push(c);
            }
        }
    };

    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("diff --git ") {
            push_current(&mut files, &mut current, &mut current_hunk);
            let m = line
                .strip_prefix("diff --git ")
                .and_then(|rest| rest.split_once(' '));
            let (a, b) = m.unwrap_or(("", ""));
            let old_path = a.strip_prefix("a/").unwrap_or(a).to_string();
            let new_path = b.strip_prefix("b/").unwrap_or(b).to_string();
            current = Some(PlanFile {
                key: format!("{old_path}=>{new_path}:{i}"),
                old_path,
                new_path,
                base_sha256: String::new(),
                header_lines: vec![line.to_string()],
                hunks: Vec::new(),
            });
            in_header = true;
            continue;
        }

        let Some(c) = current.as_mut() else {
            continue;
        };

        if line.starts_with("@@ ") {
            if let Some(h) = current_hunk.take() {
                c.hunks.push(h);
            }
            current_hunk = Some(PlanHunk {
                header_line: line.to_string(),
                lines: Vec::new(),
            });
            in_header = false;
            continue;
        }

        if in_header {
            c.header_lines.push(line.to_string());
        } else if let Some(h) = current_hunk.as_mut() {
            h.lines.push(line.to_string());
        }
    }

    push_current(&mut files, &mut current, &mut current_hunk);
    files.retain(|f| f.header_lines.iter().any(|l| !l.trim().is_empty()) || !f.hunks.is_empty());
    files
}

pub fn workspace_edit_to_plan(
    root: &Path,
    worktree_root: &Path,
    session_id: SessionId,
    worktree_id: WorktreeId,
    title: String,
    edit: WorkspaceEdit,
) -> Result<EditPlan> {
    let (mut file_edits, ops) = collect_workspace_edit(root, &edit)?;
    let mut files = Vec::new();

    // Apply operations first (create/rename/delete).
    for op in ops {
        match op {
            LspFileOp::Create { path } => {
                let edits = file_edits.remove(&path).unwrap_or_default();
                let old = String::new();
                let base_sha256 = sha256_hex(&old);
                let new = apply_text_edits_utf16(&old, &edits)
                    .with_context(|| format!("applying edits for {path}"))?;
                let patch = git_unified_diff_create(&path, &new);
                for mut pf in parse_unified_diff(&patch) {
                    pf.base_sha256 = base_sha256.clone();
                    files.push(pf);
                }
            }
            LspFileOp::Delete { path } => {
                let abs = worktree_root.join(&path);
                let old = std::fs::read_to_string(&abs).unwrap_or_default();
                let base_sha256 = sha256_hex(&old);
                let patch = git_unified_diff_delete(&path, &old);
                for mut pf in parse_unified_diff(&patch) {
                    pf.base_sha256 = base_sha256.clone();
                    files.push(pf);
                }
                // Any edits targeting deleted file are ignored.
                file_edits.remove(&path);
            }
            LspFileOp::Rename { old_path, new_path } => {
                let abs = worktree_root.join(&old_path);
                let old = std::fs::read_to_string(&abs).unwrap_or_default();
                let base_sha256 = sha256_hex(&old);

                let mut new_text = old.clone();
                if let Some(edits) = file_edits.remove(&old_path) {
                    new_text = apply_text_edits_utf16(&new_text, &edits)
                        .with_context(|| format!("applying edits for {old_path}"))?;
                }
                if let Some(edits) = file_edits.remove(&new_path) {
                    new_text = apply_text_edits_utf16(&new_text, &edits)
                        .with_context(|| format!("applying edits for {new_path}"))?;
                }

                let patch_del = git_unified_diff_delete(&old_path, &old);
                for mut pf in parse_unified_diff(&patch_del) {
                    pf.base_sha256 = base_sha256.clone();
                    files.push(pf);
                }

                let patch_add = git_unified_diff_create(&new_path, &new_text);
                for mut pf in parse_unified_diff(&patch_add) {
                    pf.base_sha256 = sha256_hex("");
                    files.push(pf);
                }
            }
        }
    }

    // Now apply remaining edits as modifications (stable order for deterministic diffs).
    let mut remaining = file_edits.into_iter().collect::<Vec<_>>();
    remaining.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (rel, edits) in remaining {
        let abs = worktree_root.join(&rel);
        let old = std::fs::read_to_string(&abs).unwrap_or_default();
        let base_sha256 = sha256_hex(&old);
        let new = apply_text_edits_utf16(&old, &edits)
            .with_context(|| format!("applying edits for {rel}"))?;
        if old == new {
            continue;
        }
        let patch = git_unified_diff_modify(&rel, &old, &new);
        for mut pf in parse_unified_diff(&patch) {
            pf.base_sha256 = base_sha256.clone();
            files.push(pf);
        }
    }

    Ok(EditPlan {
        id: EditPlanId::new(),
        session_id,
        worktree_id,
        title,
        created_at: Utc::now(),
        worktree_root: worktree_root.to_path_buf(),
        files,
    })
}

pub fn text_edits_to_plan(
    worktree_root: &Path,
    session_id: SessionId,
    worktree_id: WorktreeId,
    title: String,
    rel_path: String,
    edits: Vec<TextEdit>,
) -> Result<EditPlan> {
    let abs = worktree_root.join(&rel_path);
    let old = std::fs::read_to_string(&abs).unwrap_or_default();
    let base_sha256 = sha256_hex(&old);
    let new = apply_text_edits_utf16(&old, &edits)
        .with_context(|| format!("applying edits for {rel_path}"))?;
    if old == new {
        return Ok(EditPlan {
            id: EditPlanId::new(),
            session_id,
            worktree_id,
            title,
            created_at: Utc::now(),
            worktree_root: worktree_root.to_path_buf(),
            files: vec![],
        });
    }
    let patch = git_unified_diff_modify(&rel_path, &old, &new);
    let mut files = parse_unified_diff(&patch);
    for f in &mut files {
        f.base_sha256 = base_sha256.clone();
    }
    Ok(EditPlan {
        id: EditPlanId::new(),
        session_id,
        worktree_id,
        title,
        created_at: Utc::now(),
        worktree_root: worktree_root.to_path_buf(),
        files,
    })
}

#[derive(Debug, Clone)]
enum LspFileOp {
    Create { path: String },
    Delete { path: String },
    Rename { old_path: String, new_path: String },
}

type WorkspaceEditFileMap = HashMap<String, Vec<TextEdit>>;
type WorkspaceEditOps = Vec<LspFileOp>;
type WorkspaceEditCollection = (WorkspaceEditFileMap, WorkspaceEditOps);

fn collect_workspace_edit(root: &Path, edit: &WorkspaceEdit) -> Result<WorkspaceEditCollection> {
    let mut out: WorkspaceEditFileMap = HashMap::new();
    let mut ops: WorkspaceEditOps = Vec::new();

    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            let rel = uri_to_relpath(root, uri)?;
            out.entry(rel).or_default().extend(edits.clone());
        }
    }

    if let Some(doc_changes) = &edit.document_changes {
        match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => {
                for e in edits {
                    let rel = uri_to_relpath(root, &e.text_document.uri)?;
                    out.entry(rel)
                        .or_default()
                        .extend(e.edits.iter().cloned().map(|x| match x {
                            lsp_types::OneOf::Left(te) => te,
                            lsp_types::OneOf::Right(annot) => annot.text_edit,
                        }));
                }
            }
            lsp_types::DocumentChanges::Operations(changes) => {
                for ch in changes {
                    match ch {
                        lsp_types::DocumentChangeOperation::Edit(edit) => {
                            let rel = uri_to_relpath(root, &edit.text_document.uri)?;
                            out.entry(rel)
                                .or_default()
                                .extend(edit.edits.iter().cloned().map(|x| match x {
                                    lsp_types::OneOf::Left(te) => te,
                                    lsp_types::OneOf::Right(annot) => annot.text_edit,
                                }));
                        }
                        lsp_types::DocumentChangeOperation::Op(op) => match op {
                            lsp_types::ResourceOp::Create(cf) => {
                                let rel = uri_to_relpath(root, &cf.uri)?;
                                ops.push(LspFileOp::Create { path: rel });
                            }
                            lsp_types::ResourceOp::Rename(rf) => {
                                let old_rel = uri_to_relpath(root, &rf.old_uri)?;
                                let new_rel = uri_to_relpath(root, &rf.new_uri)?;
                                ops.push(LspFileOp::Rename {
                                    old_path: old_rel,
                                    new_path: new_rel,
                                });
                            }
                            lsp_types::ResourceOp::Delete(df) => {
                                let rel = uri_to_relpath(root, &df.uri)?;
                                ops.push(LspFileOp::Delete { path: rel });
                            }
                        },
                    }
                }
            }
        }
    }

    Ok((out, ops))
}

fn uri_to_relpath(root: &Path, uri: &Uri) -> Result<String> {
    let url = url::Url::parse(uri.as_str()).map_err(|_| anyhow!("invalid uri"))?;
    let path = url
        .to_file_path()
        .map_err(|_| anyhow!("uri is not a file path"))?;

    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canon = if let Ok(canon) = path.canonicalize() {
        canon
    } else {
        // For file operations like create/rename, the target file may not exist yet.
        // Canonicalize the nearest existing ancestor (parent dir), then join the file name.
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("uri has no parent directory"))?;
        let file_name = path
            .file_name()
            .ok_or_else(|| anyhow!("uri has no file name"))?;
        let parent_canon = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        parent_canon.join(file_name)
    };

    if !canon.starts_with(&root) {
        anyhow::bail!("uri outside root");
    }
    let rel = canon.strip_prefix(&root).unwrap_or(&canon);
    Ok(rel
        .to_string_lossy()
        .trim_start_matches(std::path::MAIN_SEPARATOR)
        .to_string())
}

fn git_unified_diff_modify(path: &str, old: &str, new: &str) -> String {
    let a = format!("a/{}", path);
    let b = format!("b/{}", path);
    let diff = similar::TextDiff::from_lines(old, new);
    let body = diff.unified_diff().header(&a, &b).to_string();
    format!("diff --git {a} {b}\n{body}")
}

fn git_unified_diff_create(path: &str, new: &str) -> String {
    let a = format!("a/{}", path);
    let b = format!("b/{}", path);
    if new.is_empty() {
        return format!(
            "diff --git {a} {b}\nnew file mode 100644\n--- /dev/null\n+++ {b}\n@@ -0,0 +0,0 @@\n"
        );
    }
    let diff = similar::TextDiff::from_lines("", new);
    let body = diff.unified_diff().header("/dev/null", &b).to_string();
    format!("diff --git {a} {b}\nnew file mode 100644\n{body}")
}

fn git_unified_diff_delete(path: &str, old: &str) -> String {
    let a = format!("a/{}", path);
    let b = format!("b/{}", path);
    if old.is_empty() {
        return format!(
            "diff --git {a} {b}\ndeleted file mode 100644\n--- {a}\n+++ /dev/null\n@@ -0,0 +0,0 @@\n"
        );
    }
    let diff = similar::TextDiff::from_lines(old, "");
    let body = diff.unified_diff().header(&a, "/dev/null").to_string();
    format!("diff --git {a} {b}\ndeleted file mode 100644\n{body}")
}

fn apply_text_edits_utf16(text: &str, edits: &[TextEdit]) -> Result<String> {
    let mut out = text.to_string();
    let mut edits = edits.to_vec();
    // Apply from bottom to top (stable offsets).
    edits.sort_by(|a, b| {
        let ar = &a.range;
        let br = &b.range;
        (
            br.start.line,
            br.start.character,
            br.end.line,
            br.end.character,
        )
            .cmp(&(
                ar.start.line,
                ar.start.character,
                ar.end.line,
                ar.end.character,
            ))
    });
    for e in edits {
        let start = pos_to_byte_offset_utf16(&out, e.range.start)?;
        let end = pos_to_byte_offset_utf16(&out, e.range.end)?;
        if start > end || end > out.len() {
            anyhow::bail!("invalid edit range");
        }
        out.replace_range(start..end, &e.new_text);
    }
    Ok(out)
}

fn pos_to_byte_offset_utf16(text: &str, pos: lsp_types::Position) -> Result<usize> {
    let mut lines = text.split_inclusive('\n');
    let mut offset = 0usize;
    for _ in 0..pos.line {
        let Some(l) = lines.next() else {
            return Ok(text.len());
        };
        offset += l.len();
    }
    let line = lines.next().unwrap_or("");
    let target_units = pos.character as usize;
    let mut units = 0usize;
    let mut byte = 0usize;
    for ch in line.chars() {
        if units >= target_units {
            break;
        }
        units += ch.len_utf16();
        byte += ch.len_utf8();
    }
    Ok(std::cmp::min(offset + byte, text.len()))
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}
