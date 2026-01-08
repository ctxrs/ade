use gpui::{
    ClickEvent, Context, CursorStyle, FocusHandle, ObjectFit, div, img, prelude::*, px,
};
use ctx_core::models::SessionEvent;

use ctx_core::ids::WorkspaceId;

use ctx_core::models::Artifact;

use crate::theme::ThemeColors;

use super::models::{
    artifact_label, is_image_artifact, is_text_artifact, session_event_type_label, MessageItem,
    SessionInfo,
};
use super::state::{ArtifactPreviewState, DataLoadState, ProviderItem, ShellView, WorkspaceItem};
use super::workspace_summary::{SessionSummaryItem, TaskSummaryItem, TaskSummaryStatus};

pub(super) struct WorkspaceListView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) workspaces: &'a [WorkspaceItem],
    pub(super) selected_workspace: Option<WorkspaceId>,
}

impl<'a> WorkspaceListView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let list = if self.workspaces.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No workspaces yet.")
        } else {
            self.workspaces
                .iter()
                .enumerate()
                .fold(div().flex().flex_col().gap_2(), |list, (index, workspace)| {
                    let is_selected = self.selected_workspace == Some(workspace.id);
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
                    let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_workspace(index, cx);
                    });
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(item_border)
                            .rounded_sm()
                            .bg(item_bg)
                            .text_sm()
                            .child(workspace.name.as_str())
                            .cursor_pointer()
                            .on_click(on_click),
                    )
                })
        };

        div()
            .id("workspace-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Workspaces")
                    .child(format!("{}", self.workspaces.len())),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}

pub(super) struct ProviderListView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) providers: &'a [ProviderItem],
}

impl<'a> ProviderListView<'a> {
    pub(super) fn render(&self) -> impl IntoElement {
        let list = if self.providers.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No providers detected.")
        } else {
            self.providers
                .iter()
                .fold(div().flex().flex_col().gap_2(), |list, provider| {
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .bg(self.colors.panel_2)
                            .text_sm()
                            .child(provider.name.as_str())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child(provider.status.as_str()),
                            ),
                    )
                })
        };

        div()
            .id("provider-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Providers")
                    .child(format!("{}", self.providers.len())),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}

pub(super) struct SidebarView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) workspaces: &'a [WorkspaceItem],
    pub(super) selected_workspace: Option<WorkspaceId>,
    pub(super) providers: &'a [ProviderItem],
    pub(super) tasks: &'a [TaskSummaryItem],
    pub(super) selected_task: Option<usize>,
}

impl<'a> SidebarView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(px(260.0))
            .bg(self.colors.panel_2)
            .border_r_1()
            .border_color(self.colors.border)
            .p_3()
            .gap_2()
            .child(
                WorkspaceListView {
                    colors: self.colors,
                    workspaces: self.workspaces,
                    selected_workspace: self.selected_workspace,
                }
                .render(cx),
            )
            .child(
                ProviderListView {
                    colors: self.colors,
                    providers: self.providers,
                }
                .render(),
            )
            .child(
                TaskListView {
                    colors: self.colors,
                    tasks: self.tasks,
                    selected_task: self.selected_task,
                }
                .render(),
            )
    }
}

pub(super) struct TaskListView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) tasks: &'a [TaskSummaryItem],
    pub(super) selected_task: Option<usize>,
}

impl<'a> TaskListView<'a> {
    pub(super) fn render(&self) -> impl IntoElement {
        let list = self
            .tasks
            .iter()
            .enumerate()
            .fold(div().flex().flex_col(), |list, (index, task)| {
                let is_selected = self.selected_task == Some(index);
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
                let status_color = match task.status {
                    TaskSummaryStatus::Pending => self.colors.muted,
                    TaskSummaryStatus::Running => self.colors.accent,
                    TaskSummaryStatus::Completed => self.colors.success,
                    TaskSummaryStatus::Failed => self.colors.error,
                    TaskSummaryStatus::Cancelled => self.colors.warning,
                };
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(item_border)
                        .rounded_sm()
                        .bg(item_bg)
                        .text_sm()
                        .child(task.title.as_str())
                        .child(
                            div()
                                .px_2()
                                .py_0()
                                .text_sm()
                                .text_color(status_color)
                                .child(task.status.label()),
                        ),
                )
            });

        div()
            .id("task-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Tasks")
                    .child(format!("{}", self.tasks.len())),
            )
            .child(div().h(px(12.0)))
            .child(list)
    }
}

struct SessionListView<'a> {
    colors: ThemeColors,
    sessions: &'a [SessionSummaryItem],
    selected_session: Option<usize>,
}

impl<'a> SessionListView<'a> {
    fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let list = self
            .sessions
            .iter()
            .enumerate()
            .fold(div().flex().flex_col().gap_2(), |list, (index, session)| {
                let is_selected = self.selected_session == Some(index);
                let item_bg = if is_selected {
                    self.colors.panel_2
                } else {
                    self.colors.panel
                };
                let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.select_session(index, cx);
                });
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .bg(item_bg)
                        .text_sm()
                        .child(session.title.as_str())
                        .child(
                            div()
                                .px_2()
                                .py_0()
                                .text_sm()
                                .text_color(self.colors.muted)
                                .child(session.status.as_str()),
                        )
                        .on_click(on_click),
                )
            });

        div()
            .id("session-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Sessions")
                    .child(format!("{}", self.sessions.len())),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}

struct MessagesView<'a> {
    colors: ThemeColors,
    messages: &'a [MessageItem],
}

impl<'a> MessagesView<'a> {
    fn render(&self) -> impl IntoElement {
        let list = self
            .messages
            .iter()
            .fold(div().flex().flex_col().gap_2(), |list, message| {
                list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_2()
                        .py_2()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .bg(self.colors.panel_2)
                        .text_sm()
                        .child(
                            div()
                                .text_sm()
                                .text_color(self.colors.muted)
                                .child(message.role.as_str()),
                        )
                        .child(message.content.as_str()),
                )
            });

        div()
            .id("message-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Messages"),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}

struct EventsView<'a> {
    colors: ThemeColors,
    events: &'a [SessionEvent],
}

impl<'a> EventsView<'a> {
    fn render(&self) -> impl IntoElement {
        let list = if self.events.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No events yet.")
        } else {
            self.events
                .iter()
                .fold(div().flex().flex_col().gap_2(), |list, event| {
                    let left = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_color(self.colors.muted)
                                .child(format!("#{}", event.seq)),
                        )
                        .child(session_event_type_label(&event.event_type));
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .bg(self.colors.panel_2)
                            .text_sm()
                            .child(left)
                            .child(
                                div()
                                    .text_color(self.colors.muted)
                                    .child(event.created_at.to_rfc3339()),
                            ),
                    )
                })
        };

        div()
            .id("events")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Events")
                    .child(format!("{}", self.events.len())),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}

struct ArtifactsView<'a> {
    colors: ThemeColors,
    artifacts: &'a [Artifact],
    selected_artifact: Option<usize>,
    artifact_preview: &'a ArtifactPreviewState,
}

impl<'a> ArtifactsView<'a> {
    fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
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
                    .child("Artifacts")
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

struct ComposerView<'a> {
    colors: ThemeColors,
    composer_text: &'a str,
    can_send: bool,
    focus_handle: &'a FocusHandle,
}

impl<'a> ComposerView<'a> {
    fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let placeholder = self.composer_text.is_empty();
        let input_text = if placeholder {
            "Type a message..."
        } else {
            self.composer_text
        };
        let input_color = if placeholder {
            self.colors.muted
        } else {
            self.colors.text
        };

        let input = div()
            .flex_1()
            .text_sm()
            .text_color(input_color)
            .cursor(CursorStyle::IBeam)
            .track_focus(self.focus_handle)
            .on_click(cx.listener(ShellView::focus_composer))
            .on_key_down(cx.listener(ShellView::on_composer_key_down))
            .child(input_text);

        let mut send_button = div()
            .px_3()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child("Send");

        if self.can_send {
            send_button = send_button
                .bg(self.colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(ShellView::on_send_click));
        } else {
            send_button = send_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }

        div()
            .id("composer")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .px_2()
            .py_2()
            .bg(self.colors.panel_2)
            .child(input)
            .child(send_button)
    }
}

pub(super) struct SessionView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) workspaces: &'a [WorkspaceItem],
    pub(super) selected_workspace: Option<WorkspaceId>,
    pub(super) catchup_active_total: Option<i64>,
    pub(super) catchup_archived_total: Option<i64>,
    pub(super) tasks: &'a [TaskSummaryItem],
    pub(super) session: &'a SessionInfo,
    pub(super) sessions: &'a [SessionSummaryItem],
    pub(super) selected_session: Option<usize>,
    pub(super) messages: &'a [MessageItem],
    pub(super) session_events: &'a [SessionEvent],
    pub(super) artifacts: &'a [Artifact],
    pub(super) selected_artifact: Option<usize>,
    pub(super) artifact_preview: &'a ArtifactPreviewState,
    pub(super) data_state: &'a DataLoadState,
    pub(super) composer_text: &'a str,
    pub(super) composer_focus: &'a FocusHandle,
}

impl<'a> SessionView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let mut data_block = div().flex().flex_col().text_sm().text_color(self.colors.muted);
        match self.data_state {
            DataLoadState::Loading => {
                data_block = data_block.child("Loading workspace data...");
            }
            DataLoadState::Error(err) => {
                data_block = data_block.child("Workspace data unavailable");
                data_block = data_block.child(format!("Error: {err}"));
            }
            DataLoadState::Loaded => {
                data_block = data_block.child(format!("Workspaces: {}", self.workspaces.len()));
                if self.workspaces.is_empty() {
                    data_block = data_block.child("Workspace list: none");
                } else {
                    data_block = data_block.child("Workspace list:");
                    for workspace in self.workspaces {
                        data_block = data_block.child(format!("- {}", workspace.name));
                    }
                }
                let selected = self
                    .selected_workspace
                    .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id))
                    .map(|ws| ws.name.as_str())
                    .unwrap_or("None");
                data_block = data_block.child(format!("Selected workspace: {selected}"));
                if let Some(active_total) = self.catchup_active_total {
                    data_block = data_block.child(format!("Active tasks (total): {active_total}"));
                }
                if let Some(archived_total) = self.catchup_archived_total {
                    data_block = data_block.child(format!("Archived tasks (total): {archived_total}"));
                }
                if self.tasks.is_empty() {
                    data_block = data_block.child("Catchup tasks: none");
                } else {
                    data_block = data_block.child("Catchup tasks:");
                    for task in self.tasks {
                        data_block = data_block.child(format!(
                            "- {} ({})",
                            task.title,
                            task.status.label()
                        ));
                    }
                }
            }
        }
        let has_session = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
            .is_some();
        let can_send = has_session && !self.composer_text.trim().is_empty();
        let mut interrupt_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child("Interrupt");

        if has_session {
            interrupt_button = interrupt_button
                .bg(self.colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(ShellView::on_interrupt_click));
        } else {
            interrupt_button = interrupt_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }

        let mut cancel_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child("Cancel");

        if has_session {
            cancel_button = cancel_button
                .bg(self.colors.panel)
                .text_color(self.colors.warning)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(ShellView::on_cancel_click));
        } else {
            cancel_button = cancel_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }
        div()
            .id("session-view")
            .flex()
            .flex_col()
            .flex_1()
            .p_4()
            .bg(self.colors.panel)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(self.colors.text)
                            .child(self.session.title.as_str()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .bg(self.colors.panel_2)
                                    .border_1()
                                    .border_color(self.colors.border)
                                    .rounded_sm()
                                    .child(self.session.status.as_str()),
                            )
                            .child(interrupt_button)
                            .child(cancel_button),
                    ),
            )
            .child(div().h(px(12.0)))
            .child(
                SessionListView {
                    colors: self.colors,
                    sessions: self.sessions,
                    selected_session: self.selected_session,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(self.colors.border)
                    .rounded_sm()
                    .p_3()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(self.session.detail.as_str())
                    .child(div().h(px(8.0)))
                    .child("Session stream placeholder"),
            )
            .child(div().h(px(16.0)))
            .child(
                EventsView {
                    colors: self.colors,
                    events: self.session_events,
                }
                .render(),
            )
            .child(div().h(px(16.0)))
            .child(
                MessagesView {
                    colors: self.colors,
                    messages: self.messages,
                }
                .render(),
            )
            .child(div().h(px(16.0)))
            .child(
                ArtifactsView {
                    colors: self.colors,
                    artifacts: self.artifacts,
                    selected_artifact: self.selected_artifact,
                    artifact_preview: self.artifact_preview,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(
                ComposerView {
                    colors: self.colors,
                    composer_text: self.composer_text,
                    can_send,
                    focus_handle: self.composer_focus,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(data_block)
    }
}
