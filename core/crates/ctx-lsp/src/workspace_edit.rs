use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use lsp_types::{Position, TextEdit, Uri, WorkspaceEdit};

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

pub(crate) async fn apply_workspace_edit_to_disk(
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
