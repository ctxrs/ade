use gpui::{ClickEvent, Context, ElementId, div, prelude::*, px};
use ctx_core::ids::WorkspaceId;

use crate::theme::ThemeColors;

use super::super::icons::{Icon, IconName};
use super::super::state::{ProviderItem, ShellRoute, ShellView, WorkspaceItem};
use super::super::workspace_summary::{TaskSummaryItem, TaskSummaryStatus};

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
                            .px_3()
                            .py_2()
                            .border_1()
                            .border_color(item_border)
                            .rounded_sm()
                            .bg(item_bg)
                            .text_sm()
                            .child(workspace.name.clone())
                            .cursor_pointer()
                            .id(ElementId::named_usize("sidebar-workspace", index))
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(
                                IconName::Refresh,
                                12.0,
                                self.colors.muted,
                            ))
                            .child("Workspaces"),
                    )
                    .child(format!("{}", self.workspaces.len())),
            )
            .child(div().h(px(crate::theme::ThemeMetrics::default().spacing.lg)))
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
                            .px_3()
                            .py_2()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .bg(self.colors.panel_2)
                            .text_sm()
                            .child(provider.name.clone())
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child(provider.status.clone()),
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
                    .child("Agent Harnesses")
                    .child(format!("{}", self.providers.len())),
            )
            .child(div().h(px(crate::theme::ThemeMetrics::default().spacing.lg)))
            .child(list)
    }
}

pub(crate) struct SidebarView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) current_route: ShellRoute,
    pub(super) workspaces: &'a [WorkspaceItem],
    pub(super) selected_workspace: Option<WorkspaceId>,
    pub(super) providers: &'a [ProviderItem],
    pub(super) tasks: &'a [TaskSummaryItem],
    pub(super) selected_task: Option<usize>,
}

pub(super) struct NavigationListView {
    pub(super) colors: ThemeColors,
    pub(super) current_route: ShellRoute,
}

impl NavigationListView {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        // Order and labels aligned to web app nav copy
        // - "Workspaces" and "Settings" prominent
        // - "Providers" renamed to "Agent Harnesses"
        // - "App Settings" shown as "Launcher"
        let routes = [
            (ShellRoute::Workbench, "Workbench"),
            (ShellRoute::Workspaces, "Workspaces"),
            (ShellRoute::Settings, "Settings"),
            (ShellRoute::Providers, "Agent Harnesses"),
            (ShellRoute::Diagnostics, "Diagnostics"),
            (ShellRoute::AppSettings, "Launcher"),
        ];

        let list = routes
            .iter()
            .enumerate()
            .fold(div().flex().flex_col().gap_2(), |list, (index, (route, label))| {
            let active = self.current_route == *route;
            let item_bg = if active {
                self.colors.panel
            } else {
                self.colors.panel_2
            };
            let item_border = if active {
                self.colors.border_strong
            } else {
                self.colors.border
            };
            let next_route = *route;
            let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.set_route(next_route, cx);
            });
            list.child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_1()
                    .border_color(item_border)
                    .rounded_sm()
                    .bg(item_bg)
                    .text_sm()
                    .child(*label)
                    .cursor_pointer()
                    .id(ElementId::named_usize("nav-route", index))
                    .on_click(on_click),
            )
        });

        div()
            .id("navigation-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Navigation"),
            )
            .child(div().h(px(crate::theme::ThemeMetrics::default().spacing.lg)))
            .child(list)
    }
}

impl<'a> SidebarView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let mut root = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(px(260.0))
            .bg(self.colors.panel_2)
            .border_r_1()
            .border_color(self.colors.border)
            .p_3()
            .gap_3()
            .child(
                NavigationListView {
                    colors: self.colors,
                    current_route: self.current_route,
                }
                .render(cx),
            );

        if self.current_route == ShellRoute::Workbench {
            root = root
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
                );
        }

        root
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
            .fold(div().flex().flex_col().gap_2(), |list, (index, task)| {
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
                        .px_3()
                        .py_2()
                        .border_1()
                        .border_color(item_border)
                        .rounded_sm()
                        .bg(item_bg)
                        .text_sm()
                        .child(task.title.clone())
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
            .child(div().h(px(crate::theme::ThemeMetrics::default().spacing.lg)))
            .child(list)
    }
}
