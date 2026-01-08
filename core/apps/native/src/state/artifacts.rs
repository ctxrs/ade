use std::sync::Arc;

use gpui::{Context, Image, ImageFormat};
use gpui_tokio::Tokio;

use ctx_core::ids::ArtifactId;
use ctx_core::models::Artifact;

use super::ShellView;
use super::super::models::{is_image_artifact, is_text_artifact};

const TEXT_PREVIEW_LINE_LIMIT: usize = 20;
const TEXT_PREVIEW_BYTE_LIMIT: u64 = 64 * 1024;

#[derive(Clone)]
pub(crate) struct TextArtifactPreview {
    pub(crate) lines: Vec<String>,
    pub(crate) truncated: bool,
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

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.selected_artifact_id() != Some(artifact_id) {
                    return;
                }
                match result {
                    Ok(bytes) => {
                        let preview = build_text_preview(&bytes);
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
        self.artifact_preview = ArtifactPreviewState::Loading { artifact_id };
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let bytes = client.get_artifact_bytes(artifact_id, None).await?;
            Ok(bytes)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.selected_artifact_id() != Some(artifact_id) {
                    return;
                }
                match result {
                    Ok(bytes) => {
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
        if let Some(url) = self.artifact_url(artifact.id) {
            cx.open_url(&url);
        }
    }

    pub(crate) fn download_artifact(&mut self, artifact: Artifact, cx: &mut Context<Self>) {
        if artifact.missing.unwrap_or(false) {
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
}

fn build_text_preview(bytes: &[u8]) -> TextArtifactPreview {
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
    TextArtifactPreview { lines, truncated }
}

fn image_format_for_artifact(artifact: &Artifact) -> Option<ImageFormat> {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/png" => return Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => return Some(ImageFormat::Jpeg),
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
        "gif" => Some(ImageFormat::Gif),
        _ => None,
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
