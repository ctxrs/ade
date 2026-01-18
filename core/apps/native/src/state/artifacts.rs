use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui::{AsyncApp, Context, Image, ImageFormat, WeakEntity};
use gpui_tokio::Tokio;

use ctx_core::ids::ArtifactId;
use ctx_core::models::{Artifact, SessionEvent, SessionEventType};
use ctx_client;
use serde::Deserialize;

use super::ShellView;
use super::super::models::{
    is_absolute_path, is_diff_artifact, is_image_artifact, is_text_artifact, is_video_artifact,
};

const TEXT_PREVIEW_LINE_LIMIT: usize = 20;
const TEXT_PREVIEW_BYTE_LIMIT: u64 = 64 * 1024;
const ARTIFACT_PREFETCH_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct ArtifactsSetPayload {
    artifacts: Vec<Artifact>,
}

#[derive(Clone)]
pub(crate) struct ArtifactContentCache {
    budget_bytes: u64,
    total_bytes: u64,
    entries: HashMap<ArtifactId, ArtifactCacheEntry>,
    access_counter: u64,
}

#[derive(Clone)]
struct ArtifactCacheEntry {
    bytes: Arc<Vec<u8>>,
    size: u64,
    last_used: u64,
}

impl ArtifactContentCache {
    pub(super) fn new(budget_bytes: u64) -> Self {
        Self {
            budget_bytes,
            total_bytes: 0,
            entries: HashMap::new(),
            access_counter: 0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    pub(super) fn contains(&self, artifact_id: ArtifactId) -> bool {
        self.entries.contains_key(&artifact_id)
    }

    pub(super) fn get(&mut self, artifact_id: ArtifactId) -> Option<Arc<Vec<u8>>> {
        let entry = self.entries.get_mut(&artifact_id)?;
        self.access_counter = self.access_counter.saturating_add(1);
        entry.last_used = self.access_counter;
        Some(entry.bytes.clone())
    }

    pub(super) fn insert(&mut self, artifact_id: ArtifactId, bytes: Vec<u8>) {
        let size = bytes.len() as u64;
        if size == 0 || size > self.budget_bytes {
            return;
        }
        if let Some(existing) = self.entries.remove(&artifact_id) {
            self.total_bytes = self.total_bytes.saturating_sub(existing.size);
        }
        while self.total_bytes.saturating_add(size) > self.budget_bytes {
            let Some(oldest_id) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            if let Some(oldest) = self.entries.remove(&oldest_id) {
                self.total_bytes = self.total_bytes.saturating_sub(oldest.size);
            }
        }
        self.access_counter = self.access_counter.saturating_add(1);
        self.total_bytes = self.total_bytes.saturating_add(size);
        self.entries.insert(
            artifact_id,
            ArtifactCacheEntry {
                bytes: Arc::new(bytes),
                size,
                last_used: self.access_counter,
            },
        );
    }

    pub(super) fn remaining_budget(&self) -> u64 {
        self.budget_bytes.saturating_sub(self.total_bytes)
    }
}

impl Default for ArtifactContentCache {
    fn default() -> Self {
        Self::new(ARTIFACT_PREFETCH_BUDGET_BYTES)
    }
}

#[derive(Clone)]
pub(crate) struct TextArtifactPreview {
    pub(crate) lines: Vec<String>,
    pub(crate) truncated: bool,
    pub(crate) is_diff: bool,
}

#[derive(Clone)]
pub(crate) enum ArtifactPreviewState {
    None,
    Loading {
        artifact_id: ArtifactId,
    },
    Text {
        artifact_id: ArtifactId,
        preview: TextArtifactPreview,
    },
    Image {
        artifact_id: ArtifactId,
        image: Arc<Image>,
    },
    Error {
        artifact_id: ArtifactId,
        message: String,
    },
}

impl ShellView {
    pub(super) fn apply_artifacts_set_event(
        &mut self,
        session_id: ctx_core::ids::SessionId,
        event: &SessionEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event.event_type, SessionEventType::ArtifactsSet) {
            return;
        }
        let Ok(payload) = serde_json::from_value::<ArtifactsSetPayload>(event.payload_json.clone())
        else {
            return;
        };
        self.apply_artifacts_update(session_id, payload.artifacts, cx);
    }

    pub(super) fn prefetch_artifacts(
        &mut self,
        session_id: ctx_core::ids::SessionId,
        cx: &mut Context<Self>,
    ) {
        if self.selected_task_is_archived() {
            self.clear_artifact_prefetch_cache();
            return;
        }
        if self.artifact_prefetch_session_id != Some(session_id) {
            self.artifact_prefetch_session_id = Some(session_id);
            self.artifact_prefetch_cache.clear();
            self.artifact_prefetch_inflight.clear();
        }

        let inflight_bytes = self
            .artifacts
            .iter()
            .filter(|artifact| self.artifact_prefetch_inflight.contains(&artifact.id))
            .filter_map(|artifact| if artifact.bytes > 0 { Some(artifact.bytes as u64) } else { None })
            .sum::<u64>();
        let mut remaining = self
            .artifact_prefetch_cache
            .remaining_budget()
            .saturating_sub(inflight_bytes);
        if remaining == 0 {
            return;
        }

        let artifacts = self.artifacts.clone();
        for artifact in artifacts {
            if artifact.missing.unwrap_or(false) {
                continue;
            }
            if !(is_image_artifact(&artifact) || is_video_artifact(&artifact)) {
                continue;
            }
            let size = if artifact.bytes > 0 {
                artifact.bytes as u64
            } else {
                0
            };
            if size == 0 || size > remaining {
                continue;
            }
            if self.artifact_prefetch_cache.contains(artifact.id)
                || self.artifact_prefetch_inflight.contains(&artifact.id)
            {
                continue;
            }
            remaining = remaining.saturating_sub(size);
            self.artifact_prefetch_inflight.insert(artifact.id);
            let artifact_id = artifact.id;
            let session_id = session_id;
            let task = Tokio::spawn_result(cx, async move {
                let config = ctx_client::resolve_daemon_config()?;
                let client = ctx_client::Client::new(config)?;
                let bytes = client.get_artifact_bytes(artifact_id, None).await?;
                Ok((artifact_id, bytes))
            });

            cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        view.artifact_prefetch_inflight.remove(&artifact_id);
                        if view.artifact_prefetch_session_id != Some(session_id) {
                            return;
                        }
                        if let Ok((artifact_id, bytes)) = result {
                            view.artifact_prefetch_cache.insert(artifact_id, bytes);
                        }
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
    }

    pub(super) fn clear_artifact_prefetch_cache(&mut self) {
        self.artifact_prefetch_session_id = None;
        self.artifact_prefetch_inflight.clear();
        self.artifact_prefetch_cache.clear();
    }

    fn selected_task_is_archived(&self) -> bool {
        match self.selected_task.and_then(|task_id| self.tasks_by_id.get(&task_id)) {
            Some(item) => item.is_archived(),
            None => true,
        }
    }

    fn apply_artifacts_update(
        &mut self,
        session_id: ctx_core::ids::SessionId,
        artifacts: Vec<Artifact>,
        cx: &mut Context<Self>,
    ) {
        self.artifacts = artifacts;
        self.artifacts_session_id = Some(session_id);
        if let Some(selected) = self.selected_artifact {
            if selected >= self.artifacts.len() {
                self.selected_artifact = if self.artifacts.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
        } else if !self.artifacts.is_empty() {
            self.selected_artifact = Some(0);
        }
        if self.show_artifacts_pane {
            self.load_artifact_preview(cx);
        } else {
            self.artifact_preview = ArtifactPreviewState::None;
        }
        self.prefetch_artifacts(session_id, cx);
    }

    pub(crate) fn select_artifact(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.artifacts.len() {
            return;
        }
        self.selected_artifact = Some(index);
        self.load_artifact_preview(cx);
        cx.notify();
    }

    fn selected_artifact(&self) -> Option<&Artifact> {
        self.selected_artifact
            .and_then(|index| self.artifacts.get(index))
    }

    fn selected_artifact_id(&self) -> Option<ArtifactId> {
        self.selected_artifact().map(|artifact| artifact.id)
    }

    pub(super) fn load_artifact_preview(&mut self, cx: &mut Context<Self>) {
        let Some(artifact) = self.selected_artifact().cloned() else {
            self.artifact_preview = ArtifactPreviewState::None;
            return;
        };
        if artifact.missing.unwrap_or(false) {
            self.artifact_preview = ArtifactPreviewState::None;
            return;
        }
        if is_image_artifact(&artifact) {
            self.fetch_image_preview(artifact, cx);
            return;
        }
        if is_text_artifact(&artifact) {
            self.fetch_text_preview(artifact, cx);
            return;
        }
        self.artifact_preview = ArtifactPreviewState::None;
    }

    fn fetch_text_preview(&mut self, artifact: Artifact, cx: &mut Context<Self>) {
        let artifact_id = artifact.id;
        let is_diff_hint = is_diff_artifact(&artifact);
        self.artifact_preview = ArtifactPreviewState::Loading { artifact_id };
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let bytes = client
                .get_artifact_bytes(
                    artifact_id,
                    Some((0, TEXT_PREVIEW_BYTE_LIMIT.saturating_sub(1))),
                )
                .await?;
            Ok(bytes)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if view.selected_artifact_id() != Some(artifact_id) {
                        return;
                    }
                    match result {
                        Ok(bytes) => {
                            let preview = build_text_preview(&bytes, is_diff_hint);
                            view.artifact_preview = ArtifactPreviewState::Text {
                                artifact_id,
                                preview,
                            };
                        }
                        Err(err) => {
                            view.artifact_preview = ArtifactPreviewState::Error {
                                artifact_id,
                                message: err.to_string(),
                            };
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn fetch_image_preview(&mut self, artifact: Artifact, cx: &mut Context<Self>) {
        let Some(format) = image_format_for_artifact(&artifact) else {
            self.artifact_preview = ArtifactPreviewState::Error {
                artifact_id: artifact.id,
                message: "Unsupported image format.".to_string(),
            };
            return;
        };
        let artifact_id = artifact.id;
        if let Some(bytes) = self.artifact_prefetch_cache.get(artifact_id) {
            let image = Arc::new(Image::from_bytes(format, bytes.as_ref().clone()));
            self.artifact_preview = ArtifactPreviewState::Image { artifact_id, image };
            return;
        }
        self.artifact_preview = ArtifactPreviewState::Loading { artifact_id };
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let bytes = client.get_artifact_bytes(artifact_id, None).await?;
            Ok(bytes)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if view.selected_artifact_id() != Some(artifact_id) {
                        return;
                    }
                    match result {
                        Ok(bytes) => {
                            view.artifact_prefetch_cache
                                .insert(artifact_id, bytes.clone());
                            let image = Arc::new(Image::from_bytes(format, bytes));
                            view.artifact_preview = ArtifactPreviewState::Image { artifact_id, image };
                        }
                        Err(err) => {
                            view.artifact_preview = ArtifactPreviewState::Error {
                                artifact_id,
                                message: err.to_string(),
                            };
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn open_artifact(&mut self, artifact: Artifact, cx: &mut Context<Self>) {
        if artifact.missing.unwrap_or(false) {
            return;
        }
        if let Some(url) = file_url_from_path(&artifact.absolute_path) {
            cx.open_url(&url);
            return;
        }
        if let Some(url) = self.cached_artifact_file_url(&artifact) {
            cx.open_url(&url);
            return;
        }
        if let Some(url) = self.artifact_url(artifact.id) {
            cx.open_url(&url);
        }
    }

    pub(crate) fn open_artifact_in_app(&mut self, artifact: Artifact, cx: &mut Context<Self>) {
        if artifact.missing.unwrap_or(false) {
            return;
        }
        if let Some(url) = ctx_open_url_from_path(&artifact.absolute_path) {
            cx.open_url(&url);
        }
    }

    pub(crate) fn download_artifact(&mut self, artifact: Artifact, cx: &mut Context<Self>) {
        if artifact.missing.unwrap_or(false) {
            return;
        }
        if let Some(url) = self.cached_artifact_file_url(&artifact) {
            cx.open_url(&url);
            return;
        }
        if let Some(url) = self.artifact_url(artifact.id) {
            cx.open_url(&url);
        }
    }

    fn artifact_url(&self, artifact_id: ArtifactId) -> Option<String> {
        let base = self.base_url.trim();
        if base.is_empty() || base == "unknown" {
            return None;
        }
        let base = base.trim_end_matches('/');
        Some(format!("{}/api/artifacts/{}", base, artifact_id.0))
    }

    fn cached_artifact_file_url(&mut self, artifact: &Artifact) -> Option<String> {
        let bytes = self.artifact_prefetch_cache.get(artifact.id)?;
        let mut path = std::env::temp_dir();
        let mut name = format!("ctx-artifact-{}", artifact.id.0);
        if let Some(ext) = artifact_file_extension(artifact) {
            name.push('.');
            name.push_str(&ext);
        }
        path.push(name);
        std::fs::write(&path, bytes.as_ref()).ok()?;
        file_url_from_path(path.to_str()?)
    }
}

fn build_text_preview(bytes: &[u8], is_diff_hint: bool) -> TextArtifactPreview {
    let content = String::from_utf8_lossy(bytes);
    let mut lines = Vec::new();
    for line in content.lines().take(TEXT_PREVIEW_LINE_LIMIT + 1) {
        lines.push(line.to_string());
    }
    let mut truncated = lines.len() > TEXT_PREVIEW_LINE_LIMIT;
    if truncated {
        lines.truncate(TEXT_PREVIEW_LINE_LIMIT);
    }
    if bytes.len() as u64 >= TEXT_PREVIEW_BYTE_LIMIT {
        truncated = true;
    }
    let is_diff = is_diff_hint || looks_like_diff(&lines);
    TextArtifactPreview {
        lines,
        truncated,
        is_diff,
    }
}

fn looks_like_diff(lines: &[String]) -> bool {
    for line in lines.iter().take(8) {
        if line.starts_with("diff --git ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("@@ ")
            || line.starts_with("index ")
        {
            return true;
        }
    }
    false
}

fn image_format_for_artifact(artifact: &Artifact) -> Option<ImageFormat> {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/png" => return Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => return Some(ImageFormat::Jpeg),
        "image/webp" => return Some(ImageFormat::Webp),
        "image/gif" => return Some(ImageFormat::Gif),
        _ => {}
    }

    let extension = std::path::Path::new(&artifact.absolute_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;

    match extension.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        _ => None,
    }
}

fn artifact_file_extension(artifact: &Artifact) -> Option<String> {
    let ext = Path::new(&artifact.absolute_path)
        .extension()
        .and_then(|value| value.to_str())
        .or_else(|| {
            artifact
                .name
                .as_deref()
                .and_then(|name| Path::new(name).extension())
                .and_then(|value| value.to_str())
        })?;
    let trimmed = ext.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn file_url_from_path(path: &str) -> Option<String> {
    if path.trim().is_empty() {
        return None;
    }
    let normalized = path.replace('\\', "/");
    let prefix = if normalized.starts_with('/') {
        "file://"
    } else if normalized.len() > 1 && normalized.as_bytes()[1] == b':' {
        "file:///"
    } else {
        return None;
    };
    Some(format!("{prefix}{}", encode_url_path(&normalized)))
}

fn ctx_open_url_from_path(path: &str) -> Option<String> {
    if !is_absolute_path(path) {
        return None;
    }
    Some(format!("ctx://open?path={}", encode_url_component(path)))
}

fn encode_url_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.bytes() {
        match ch {
            b' ' => out.push_str("%20"),
            b'#' => out.push_str("%23"),
            b'%' => out.push_str("%25"),
            b'?' => out.push_str("%3F"),
            b'<' => out.push_str("%3C"),
            b'>' => out.push_str("%3E"),
            b'"' => out.push_str("%22"),
            b'{' => out.push_str("%7B"),
            b'}' => out.push_str("%7D"),
            b'|' => out.push_str("%7C"),
            _ => out.push(ch as char),
        }
    }
    out
}

fn encode_url_component(value: &str) -> String {
    let mut out = String::new();
    for ch in value.bytes() {
        match ch {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~' => out.push(ch as char),
            _ => out.push_str(&format!("%{:02X}", ch)),
        }
    }
    out
}
