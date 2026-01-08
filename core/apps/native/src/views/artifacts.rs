use gpui::{ClickEvent, Context, ObjectFit, div, img, prelude::*, px};
use ctx_core::models::Artifact;

use crate::theme::ThemeColors;

use super::super::icons::{Icon, IconName};
use super::super::models::{
    artifact_label, is_absolute_path, is_image_artifact, is_pdf_artifact, is_text_artifact,
    is_video_artifact,
};
use super::super::state::{ArtifactPreviewState, ShellView};

pub(super) struct ArtifactsView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) artifacts: &'a [Artifact],
    pub(super) selected_artifact: Option<usize>,
    pub(super) artifact_preview: &'a ArtifactPreviewState,
}

const PREVIEW_HEIGHT: f32 = 220.0;
const MONO_FONT_FAMILY: &str = "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace";

impl<'a> ArtifactsView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let list = self
            .artifacts
            .iter()
            .enumerate()
            .fold(div().flex().flex_col().gap_2(), |list, (index, artifact)| {
                let is_selected = self.selected_artifact == Some(index);
                let item_bg = if is_selected {
                    self.colors.panel
                } else {
                    self.colors.panel_2
                };
                let item_border = if is_selected {
                    self.colors.border_strong
                } else {
                    self.colors.border
                };
                let label = artifact_label(artifact);
                let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.select_artifact(index, cx);
                });
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(item_border)
                        .rounded_sm()
                        .bg(item_bg)
                        .text_sm()
                        .child(label)
                        .on_click(on_click),
                )
            });

        let detail = if let Some(artifact) = self
            .selected_artifact
            .and_then(|index| self.artifacts.get(index))
        {
            let name = artifact.name.as_deref().unwrap_or("N/A");
            let path = if artifact.absolute_path.is_empty() {
                "N/A"
            } else {
                artifact.absolute_path.as_str()
            };
            let mime_type = if artifact.mime_type.is_empty() {
                "unknown"
            } else {
                artifact.mime_type.as_str()
            };
            let created_at = artifact.created_at.to_rfc3339();
            let bytes = if artifact.bytes > 0 {
                format!("{} bytes", artifact.bytes)
            } else {
                "0 bytes".to_string()
            };
            let is_missing = artifact.missing.unwrap_or(false);
            let is_image = is_image_artifact(artifact);
            let is_text = is_text_artifact(artifact);
            let is_video = is_video_artifact(artifact);
            let is_pdf = is_pdf_artifact(artifact);
            let preview_height = px(PREVIEW_HEIGHT);

            let preview_content = if is_missing {
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Missing on disk.")
            } else if is_image {
                match self.artifact_preview {
                    ArtifactPreviewState::Image { artifact_id, image }
                        if *artifact_id == artifact.id =>
                    {
                        let colors = self.colors;
                        div()
                            .h(preview_height)
                            .w_full()
                            .child(
                                img(image.clone())
                                    .w_full()
                                    .h_full()
                                    .object_fit(ObjectFit::Contain)
                                    .with_loading(move || {
                                        div().text_sm().text_color(colors.muted).child(
                                            "Loading image preview...",
                                        )
                                    })
                                    .with_fallback(move || {
                                        div().text_sm().text_color(colors.muted).child(
                                            "Image preview unavailable.",
                                        )
                                    }),
                            )
                    }
                    ArtifactPreviewState::Loading { artifact_id }
                        if *artifact_id == artifact.id =>
                    {
                        div()
                            .text_sm()
                            .text_color(self.colors.muted)
                            .child("Loading image preview...")
                    }
                    ArtifactPreviewState::Error {
                        artifact_id,
                        message,
                    } if *artifact_id == artifact.id => div()
                        .text_sm()
                        .text_color(self.colors.error)
                        .child(format!("Image preview unavailable: {message}")),
                    _ => div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Image preview unavailable."),
                }
            } else if is_text {
                match self.artifact_preview {
                    ArtifactPreviewState::Text { artifact_id, preview }
                        if *artifact_id == artifact.id =>
                    {
                        if preview.lines.is_empty() {
                            div()
                                .text_sm()
                                .text_color(self.colors.muted)
                                .child("Empty file.")
                        } else {
                            let mut lines = div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .text_sm()
                                .font_family(MONO_FONT_FAMILY)
                                .text_color(self.colors.text)
                                .whitespace_nowrap()
                                .overflow_x_scroll()
                                .overflow_y_scroll()
                                .h(preview_height)
                                .w_full();
                            for line in &preview.lines {
                                let mut row = div().child(line.clone());
                                if preview.is_diff {
                                    row = row.text_color(diff_line_color(line, self.colors));
                                }
                                lines = lines.child(row);
                            }
                            let mut body = div().flex().flex_col().gap_1().child(lines);
                            if preview.truncated {
                                body = body.child(
                                    div()
                                        .text_sm()
                                        .text_color(self.colors.muted)
                                        .child("Preview truncated."),
                                );
                            }
                            body
                        }
                    },
                    ArtifactPreviewState::Loading { artifact_id }
                        if *artifact_id == artifact.id =>
                    {
                        div()
                            .text_sm()
                            .text_color(self.colors.muted)
                            .child("Loading preview...")
                    },
                    ArtifactPreviewState::Error {
                        artifact_id,
                        message,
                    } if *artifact_id == artifact.id => div()
                        .text_sm()
                        .text_color(self.colors.error)
                        .child(format!("Preview unavailable: {message}")),
                    _ => div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Preview unavailable."),
                }
            } else {
                let fallback_message = if is_video {
                    "Video preview unavailable. Use Open to play."
                } else if is_pdf {
                    "PDF preview unavailable. Use Open to view."
                } else {
                    "No preview available for this artifact type."
                };
                let fallback_meta = preview_metadata_block(
                    name,
                    path,
                    mime_type,
                    &bytes,
                    &created_at,
                    self.colors,
                );
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .h(preview_height)
                    .w_full()
                    .child(
                        div()
                            .text_sm()
                            .text_color(self.colors.muted)
                            .child(fallback_message),
                    )
                    .child(fallback_meta)
            };

            let mut detail = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .p_3()
                        .bg(self.colors.panel)
                        .child(preview_content),
                );

            let can_open = !is_missing;
            let can_open_in_app = !is_missing && is_absolute_path(&artifact.absolute_path);
            let can_download = !is_missing;
            let open_artifact = artifact.clone();
            let open_in_app_artifact = artifact.clone();
            let download_artifact = artifact.clone();
            let on_open = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.open_artifact(open_artifact.clone(), cx);
            });
            let on_open_in_app = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.open_artifact_in_app(open_in_app_artifact.clone(), cx);
            });
            let on_download = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.download_artifact(download_artifact.clone(), cx);
            });

            let mut open_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child("Open");
            if can_open {
                open_button = open_button
                    .bg(self.colors.panel)
                    .cursor_pointer()
                    .active(|this| this.opacity(0.85))
                    .on_click(on_open);
            } else {
                open_button = open_button
                    .bg(self.colors.panel_2)
                    .text_color(self.colors.muted);
            }

            let mut open_in_app_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child("Open in app");
            if can_open_in_app {
                open_in_app_button = open_in_app_button
                    .bg(self.colors.panel)
                    .cursor_pointer()
                    .active(|this| this.opacity(0.85))
                    .on_click(on_open_in_app);
            } else {
                open_in_app_button = open_in_app_button
                    .bg(self.colors.panel_2)
                    .text_color(self.colors.muted);
            }

            let mut download_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child("Download");
            if can_download {
                download_button = download_button
                    .bg(self.colors.panel)
                    .cursor_pointer()
                    .active(|this| this.opacity(0.85))
                    .on_click(on_download);
            } else {
                download_button = download_button
                    .bg(self.colors.panel_2)
                    .text_color(self.colors.muted);
            }

            detail = detail.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(open_button)
                    .child(open_in_app_button)
                    .child(download_button),
            );

            let mut meta = div()
                .flex()
                .flex_col()
                .gap_1()
                .text_sm()
                .text_color(self.colors.muted)
                .child(format!("Id: {}", artifact.id.0))
                .child(format!("Type: {}", mime_type))
                .child(format!("Name: {}", name))
                .child(format!("Path: {}", path))
                .child(format!("Size: {}", bytes));
            if !created_at.is_empty() {
                meta = meta.child(format!("Created: {}", created_at));
            }
            if is_missing {
                meta = meta.child("Status: Missing on disk");
            }
            detail.child(meta)
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .text_sm()
                .text_color(self.colors.muted)
                .child("Select an artifact to view details.")
        };

        div()
            .id("artifacts")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(IconName::Image, 12.0, self.colors.muted))
                            .child(div().text_color(self.colors.text).child("Artifacts")),
                    )
                    .child(
                        div()
                            .px_1()
                            .py_0()
                            .text_sm()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .bg(self.colors.panel)
                            .text_color(self.colors.muted)
                            .child(format!("{}", self.artifacts.len())),
                    ),
            )
            .child(div().h(px(8.0)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(220.0))
                            .child(if self.artifacts.is_empty() {
                                div()
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child("No artifacts yet.")
                            } else {
                                list
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .p_3()
                            .bg(self.colors.panel_2)
                            .child(detail),
                    ),
            )
    }
}

fn preview_metadata_block(
    name: &str,
    path: &str,
    mime_type: &str,
    bytes: &str,
    created_at: &str,
    colors: ThemeColors,
) -> gpui::Div {
    let mut meta = div()
        .flex()
        .flex_col()
        .gap_1()
        .text_sm()
        .text_color(colors.muted)
        .child(format!("Type: {mime_type}"))
        .child(format!("Size: {bytes}"));
    if name != "N/A" {
        meta = meta.child(format!("Name: {name}"));
    }
    if path != "N/A" {
        meta = meta.child(format!("Path: {path}"));
    }
    if !created_at.is_empty() {
        meta = meta.child(format!("Created: {created_at}"));
    }
    meta
}

fn diff_line_color(line: &str, colors: ThemeColors) -> gpui::Rgba {
    if line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        return colors.muted;
    }
    if line.starts_with("@@") {
        return colors.accent;
    }
    if line.starts_with('+') {
        return colors.success;
    }
    if line.starts_with('-') {
        return colors.error;
    }
    colors.text
}
