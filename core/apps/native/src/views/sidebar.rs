use gpui::{ClickEvent, Context, ElementId, div, prelude::*, px};
use ctx_core::ids::WorkspaceId;

use crate::theme::{ThemeColors, ThemeMetrics};
use crate::ui::{
    button,
    list::{self, RowDensity},
    pill::{self, PillTone},
    section,
};

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
        let metrics = ThemeMetrics::default();
        let _ = metrics;
        let list = if self.workspaces.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No workspaces yet.")
        } else {
            self.workspaces
                .iter()
                .enumerate()
                .fold(div().flex().flex_col(), |list, (index, workspace)| {
                    let is_selected = self.selected_workspace == Some(workspace.id);
                    let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_workspace(index, cx);
                    });
                    list.child(
                        list::row(
                            self.colors,
                            is_selected,
                            RowDensity::Tight,
                            workspace.name.clone(),
                        )
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
                section::header_lr(
                    self.colors,
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(IconName::Refresh, 12.0, self.colors.muted))
                        .child("Workspaces"),
                    pill::count(self.colors, self.workspaces.len()),
                ),
            )
            .child(div().h(px(ThemeMetrics::default().spacing.lg)))
            .child(list)
    }
}

pub(super) struct ProviderListView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) providers: &'a [ProviderItem],
}

impl<'a> ProviderListView<'a> {
    pub(super) fn render(&self) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let _ = metrics;
        let list = if self.providers.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No providers detected.")
        } else {
            self.providers
                .iter()
                .fold(div().flex().flex_col(), |list, provider| {
                    list.child(list::row_lr(
                        self.colors,
                        false,
                        RowDensity::Regular,
                        provider.name.clone(),
                        div().text_sm().text_color(self.colors.muted).child(provider.status.clone()),
                    ))
                })
        };

        div()
            .id("provider-list")
            .flex()
            .flex_col()
            .child(section::header_lr(
                self.colors,
                div().child("Agent Harnesses"),
                pill::count(self.colors, self.providers.len()),
            ))
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
        let metrics = ThemeMetrics::default();
        let _ = metrics;
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
            .fold(div().flex().flex_col(), |list, (index, (route, label))| {
                let active = self.current_route == *route;
                let next_route = *route;
                let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.set_route(next_route, cx);
                });
                list.child(
                    list::row(self.colors, active, RowDensity::Regular, *label)
                        .cursor_pointer()
                        .id(ElementId::named_usize("nav-route", index))
                        .on_click(on_click),
                )
            });

        div()
            .id("navigation-list")
            .flex()
            .flex_col()
            .child(section::header(self.colors, div().child("Navigation")))
            .child(div().h(px(ThemeMetrics::default().spacing.lg)))
            .child(list)
    }
}

impl<'a> SidebarView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let mut root = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(px(260.0))
            .bg(self.colors.panel_2)
            .border_r_1()
            .border_color(self.colors.border)
            .p(px(metrics.spacing.gutter))
            .gap_3();

        if self.current_route == ShellRoute::Workbench {
            // Web Workbench sidebar shows header + task list
            let header = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    button::primary(self.colors, "New Task")
                        .cursor_pointer()
                        .id("sidebar-new-task")
                        .on_click(cx.listener(ShellView::focus_composer)),
                )
                .child(div().h(px(metrics.spacing.md)));

            root = root
                .child(header)
                .child(
                    TaskListView {
                        colors: self.colors,
                        tasks: self.tasks,
                        selected_task: self.selected_task,
                    }
                    .render(),
                );
        } else {
            // Non-workbench routes: show navigation menu + route-specific list.
            root = root.child(
                NavigationListView {
                    colors: self.colors,
                    current_route: self.current_route,
                }
                .render(cx),
            );

            match self.current_route {
                ShellRoute::Workspaces => {
                    root = root.child(
                        WorkspaceListView {
                            colors: self.colors,
                            workspaces: self.workspaces,
                            selected_workspace: self.selected_workspace,
                        }
                        .render(cx),
                    );
                }
                ShellRoute::Providers => {
                    root = root.child(
                        ProviderListView {
                            colors: self.colors,
                            providers: self.providers,
                        }
                        .render(),
                    );
                }
                _ => {}
            }
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
        let metrics = ThemeMetrics::default();
        let _ = metrics;
        let list = self
            .tasks
            .iter()
            .enumerate()
            .fold(div().flex().flex_col(), |list, (index, task)| {
                let is_selected = self.selected_task == Some(index);
                let tone = match task.status {
                    TaskSummaryStatus::Pending => PillTone::Neutral,
                    TaskSummaryStatus::Running => PillTone::Accent,
                    TaskSummaryStatus::Completed => PillTone::Success,
                    TaskSummaryStatus::Failed => PillTone::Error,
                    TaskSummaryStatus::Cancelled => PillTone::Warning,
                };
                list.child(
                    list::row_lr(
                        self.colors,
                        is_selected,
                        RowDensity::Tight,
                        task.title.clone(),
                        pill::pill(self.colors, task.status.label(), tone),
                    ),
                )
            });

        div()
            .id("task-list")
            .flex()
            .flex_col()
            // Web workbench shows just the list with no section header
            .child(list)
    }
}
