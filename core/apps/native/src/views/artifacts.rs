use gpui::{ClickEvent, Context, ObjectFit, div, img, prelude::*, px};
use ctx_core::models::Artifact;

use crate::theme::ThemeColors;

use super::super::icons::{Icon, IconName};
use super::super::models::{artifact_label, is_image_artifact, is_text_artifact};
use super::super::state::{ArtifactPreviewState, ShellView};

pub(super) struct ArtifactsView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) artifacts: &'a [Artifact],
    pub(super) selected_artifact: Option<usize>,
    pub(super) artifact_preview: &'a ArtifactPreviewState,
}

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
                            .h(px(220.0))
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
                            let mut lines = div().flex().flex_col().gap_1().text_sm();
                            for line in &preview.lines {
                                lines = lines.child(line.clone());
                            }
                            if preview.truncated {
                                lines = lines.child(
                                    div()
                                        .text_sm()
                                        .text_color(self.colors.muted)
                                        .child("Preview truncated."),
                                );
                            }
                            lines
                        }
                    }
                    ArtifactPreviewState::Loading { artifact_id }
                        if *artifact_id == artifact.id =>
                    {
                        div()
                            .text_sm()
                            .text_color(self.colors.muted)
                            .child("Loading preview...")
                    }
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
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("No preview available for this artifact type.")
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

            if is_text {
                let can_open = !is_missing;
                let can_download = !is_missing;
                let open_artifact = artifact.clone();
                let download_artifact = artifact.clone();
                let on_open = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.open_artifact(open_artifact.clone(), cx);
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
                        .child(download_button),
                );
            }

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
                    .text_color(self.colors.muted)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(
                                IconName::Artifact,
                                12.0,
                                self.colors.muted,
                            ))
                            .child("Artifacts"),
                    )
                    .child(format!("{}", self.artifacts.len())),
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
