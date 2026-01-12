use std::collections::HashSet;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt, BoxShadow, ClickEvent, Context, ElementId, FontWeight, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, Pixels, Point, Rgba, Transformation,
    div, ease_in_out, img, percentage, point, px, radians, size, svg,
    InteractiveElement as _,
    StatefulInteractiveElement as _, prelude::*,
};
use gpui_component::{checkbox::Checkbox, input::Input, tooltip::Tooltip, v_virtual_list};
use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::SessionStatus;

use crate::theme::{ThemeColors, ThemeMetrics};

use super::super::harness_catalog::harness_entry;
use super::super::icons::{Icon, IconName};
use super::super::relative_time::format_relative_age_short;
use super::super::state::{
    AnchorRect, ProviderItem, ShellRoute, ShellView, TaskArchiveAction, TaskFetchState,
    WorkspaceItem,
};

const SIDEBAR_BG: Rgba = rgba(24, 24, 24, 1.0);
const SEARCH_BG: Rgba = rgba(255, 255, 255, 0.04);
const BUTTON_BG: Rgba = rgba(255, 255, 255, 0.04);
const COLLAPSE_BG: Rgba = rgba(255, 255, 255, 0.03);
const HOVER_BG: Rgba = rgba(255, 255, 255, 0.04);
const ACTIVE_BG: Rgba = rgba(255, 255, 255, 0.06);
const MUTED_ICON: Rgba = rgba(255, 255, 255, 0.62);
const ACTIVE_ICON: Rgba = rgba(255, 255, 255, 0.78);
const ACTION_ICON_HOVER: Rgba = rgba(255, 255, 255, 0.92);
const ACTION_HOVER_BG: Rgba = rgba(255, 255, 255, 0.06);
const SPINNER_BG: Rgba = rgba(255, 255, 255, 0.22);
const SPINNER_ACCENT: Rgba = rgba(78, 163, 255, 0.90);
const SPINNER_ARCHIVE: Rgba = rgba(251, 191, 36, 0.32);

const MENU_BG: Rgba = rgba(34, 34, 34, 0.92);
const MENU_BORDER: Rgba = rgba(255, 255, 255, 0.10);
const MENU_ITEM_HOVER_BG: Rgba = rgba(255, 255, 255, 0.06);
const MENU_ITEM_HOVER_BORDER: Rgba = rgba(255, 255, 255, 0.08);
const MENU_ITEM_DANGER: Rgba = rgba(255, 120, 120, 0.96);
const MENU_ITEM_DANGER_BG: Rgba = rgba(255, 69, 58, 0.10);
const MENU_ITEM_DANGER_BORDER: Rgba = rgba(255, 69, 58, 0.16);

const ARCHIVE_CONFIRM_BG: Rgba = rgba(34, 34, 34, 0.96);
const ARCHIVE_CONFIRM_BORDER: Rgba = rgba(255, 255, 255, 0.10);
const ARCHIVE_CONFIRM_BODY: Rgba = rgba(255, 255, 255, 0.76);
const ARCHIVE_CONFIRM_TITLE: Rgba = rgba(255, 255, 255, 0.95);
const ARCHIVE_TOGGLE_TEXT: Rgba = rgba(255, 255, 255, 0.70);

const TASK_ROW_HEIGHT: f32 = 28.0;
const TASK_ROW_GAP: f32 = 2.0;
const HEADER_HEIGHT: f32 = 22.0;
const HEADER_GAP: f32 = 12.0;
const ARCHIVED_HEADER_MARGIN: f32 = 12.0;
const STATUS_ROW_HEIGHT: f32 = 18.0;

const SIDEBAR_ANIM_DURATION: Duration = Duration::from_millis(180);
const SIDEBAR_FADE_RATIO: f32 = 140.0 / 180.0;
const HOVER_FADE_DURATION: Duration = Duration::from_millis(120);

const SPINNER_DURATION: Duration = Duration::from_millis(800);
static SPINNER_ANCHOR: OnceLock<Instant> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
enum TaskListItem {
    ActiveHeader,
    ActiveEmpty,
    ActiveTask(TaskId),
    ArchivedHeader,
    ArchivedLoading,
    ArchivedError,
    ArchivedEmpty,
    ArchivedTask(TaskId),
}

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
                        div()
                            .px(px(metrics.spacing.xl))
                            .py(px(metrics.spacing.sm))
                            .rounded_lg()
                            .border_1()
                            .border_color(self.colors.border)
                            .bg(if is_selected { self.colors.panel } else { self.colors.panel_2 })
                            .text_sm()
                            .child(workspace.name.clone())
                            .cursor_pointer()
                            .id(ElementId::named_usize("workspace-item", index))
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
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Workspaces"),
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
        let list = if self.providers.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No providers detected.")
        } else {
            self.providers
                .iter()
                .fold(div().flex().flex_col(), |list, provider| {
                    let label = harness_entry(&provider.provider_id)
                        .map(|entry| entry.label.to_string())
                        .unwrap_or_else(|| provider.provider_id.clone());
                    let status = format!("{:?}", provider.health);
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .rounded_lg()
                            .border_1()
                            .border_color(self.colors.border)
                            .px(px(12.0))
                            .py(px(6.0))
                            .child(label)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child(status),
                            ),
                    )
                })
        };

        div()
            .id("provider-list")
            .flex()
            .flex_col()
            .child(div().text_sm().text_color(self.colors.muted).child("Agent Harnesses"))
            .child(div().h(px(10.0)))
            .child(list)
    }
}

pub(crate) struct SidebarView<'a> {
    pub(super) shell: &'a ShellView,
}

pub(super) struct NavigationListView {
    pub(super) colors: ThemeColors,
    pub(super) current_route: ShellRoute,
}

impl NavigationListView {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
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
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded_lg()
                        .border_1()
                        .border_color(if active {
                            self.colors.border_strong
                        } else {
                            self.colors.border
                        })
                        .bg(if active { self.colors.panel } else { self.colors.panel_2 })
                        .text_sm()
                        .child(*label)
                        .cursor_pointer()
                        .id(ElementId::named_usize("nav-item", index))
                        .on_click(on_click),
                )
            });

        div()
            .id("navigation-list")
            .flex()
            .flex_col()
            .child(div().text_sm().text_color(self.colors.muted).child("Navigation"))
            .child(div().h(px(10.0)))
            .child(list)
    }
}

impl<'a> SidebarView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let shell = self.shell;
        let sidebar_width = shell.sidebar_width;
        let sidebar_anim_epoch = shell.sidebar_anim_epoch;
        let sidebar_collapsed = shell.sidebar_collapsed;
        let is_workbench = shell.route == ShellRoute::Workbench;
        let mut root = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(px(sidebar_width))
            .min_w(px(0.0))
            .overflow_hidden()
            .bg(SIDEBAR_BG)
            .border_r_1()
            .border_color(shell.colors.border);

        if is_workbench {
            root = root
                .child(self.render_workbench_header(cx))
                .child(self.render_task_list(cx));
        } else {
            root = root.child(
                NavigationListView {
                    colors: shell.colors,
                    current_route: shell.route,
                }
                .render(cx),
            );

            match shell.route {
                ShellRoute::Workspaces => {
                    root = root.child(
                        WorkspaceListView {
                            colors: shell.colors,
                            workspaces: &shell.workspaces,
                            selected_workspace: shell.selected_workspace,
                        }
                        .render(cx),
                    );
                }
                ShellRoute::Providers => {
                    root = root.child(
                        ProviderListView {
                            colors: shell.colors,
                            providers: &shell.providers,
                        }
                        .render(),
                    );
                }
                _ => {}
            }
        }

        let root = root
            .when(is_workbench, |this| this.bg(SIDEBAR_BG))
            .when(!is_workbench, |this| this.bg(shell.colors.panel_2))
            .when(!is_workbench, |this| this.p(px(metrics.spacing.xxs)));

        if sidebar_anim_epoch == 0 {
            let root = if sidebar_collapsed {
                root.w(px(0.0))
                    .opacity(0.0)
                    .border_r_0()
                    .invisible()
            } else {
                root
            };
            root.into_any_element()
        } else {
            let target_width = sidebar_width;
            root.with_animation(
                ElementId::named_usize("sidebar-collapse", sidebar_anim_epoch as usize),
                Animation::new(SIDEBAR_ANIM_DURATION).with_easing(ease_in_out),
                move |this, delta| {
                    let width = if sidebar_collapsed {
                        target_width * (1.0 - delta)
                    } else {
                        target_width * delta
                    };
                    let fade = if SIDEBAR_FADE_RATIO <= f32::EPSILON {
                        delta
                    } else {
                        (delta / SIDEBAR_FADE_RATIO).min(1.0)
                    };
                    let opacity = if sidebar_collapsed { 1.0 - fade } else { fade };
                    this.w(px(width.max(0.0)))
                        .opacity(opacity)
                        .when(sidebar_collapsed && delta >= 0.99, |this| {
                            this.invisible().border_r_0()
                        })
                },
            )
            .into_any_element()
        }
    }

    fn render_workbench_header(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let shell = self.shell;

        let new_task = div()
            .w_full()
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(shell.colors.border)
            .bg(BUTTON_BG)
            .text_size(px(13.0))
            .text_center()
            .child("New Task")
            .cursor_pointer()
            .id("sidebar-new-task")
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(ShellView::focus_new_task));

        let collapse = div()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(shell.colors.border)
            .bg(COLLAPSE_BG)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(16.0))
            .text_color(shell.colors.text)
            .child("‹")
            .cursor_pointer()
            .id("sidebar-collapse")
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_sidebar_collapsed(true, cx);
            }));

        let search = Input::new(&shell.task_search_input)
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .px(px(8.0))
            .py(px(6.0))
            .h(px(28.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(shell.colors.border)
            .bg(SEARCH_BG)
            .text_size(px(13.0))
            .text_color(shell.colors.text)
            .w_full();

        div()
            .px(px(12.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(new_task)
                    .child(collapse),
            )
            .child(search)
    }

    fn render_task_list(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let shell = self.shell;
        let model = TaskListModel::from_shell(shell);
        let items = Rc::new(model.items);
        let item_sizes = Rc::new(model.item_sizes);
        let active_last_index = model.active_last_index;
        let list_len = items.len();

        let list = v_virtual_list(
            cx.entity().clone(),
            "task-list",
            item_sizes.clone(),
            move |view, visible_range, _window, cx| {
                if let Some(last_index) = active_last_index {
                    if visible_range.end.saturating_sub(1) >= last_index
                        && view.task_fetch_active != TaskFetchState::Loading
                        && view.task_has_more_active
                    {
                        view.load_more_active_tasks(cx);
                    }
                }

                if !view.archived_collapsed
                    && view.task_fetch_archived != TaskFetchState::Loading
                    && view.task_has_more_archived
                    && list_len > 0
                    && visible_range.end >= list_len.saturating_sub(2)
                {
                    view.load_more_archived_tasks(false, cx);
                }

                visible_range
                    .map(|ix| render_task_list_item(view, items[ix], cx))
                    .collect()
            },
        )
        .track_scroll(&shell.task_list_scroll_handle)
        .pr(px(4.0))
        .min_h(px(0.0))
        .w_full();

        div()
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(list)
    }
}

struct TaskListModel {
    items: Vec<TaskListItem>,
    item_sizes: Vec<gpui::Size<Pixels>>,
    active_last_index: Option<usize>,
}

impl TaskListModel {
    fn from_shell(shell: &ShellView) -> Self {
        let query = shell.task_query.trim().to_lowercase();
        let mut active_ids = Vec::new();
        let mut archived_ids = Vec::new();

        for task_id in &shell.task_active_order {
            let Some(task) = shell.tasks_by_id.get(task_id) else {
                continue;
            };
            if query.is_empty() || task.task.title.to_lowercase().contains(&query) {
                active_ids.push(*task_id);
            }
        }

        for task_id in &shell.task_archived_order {
            let Some(task) = shell.tasks_by_id.get(task_id) else {
                continue;
            };
            if query.is_empty() || task.task.title.to_lowercase().contains(&query) {
                archived_ids.push(*task_id);
            }
        }

        let mut items = Vec::new();
        items.push(TaskListItem::ActiveHeader);

        if active_ids.is_empty()
            && shell.task_store_initialized
            && shell.task_fetch_active != TaskFetchState::Loading
        {
            items.push(TaskListItem::ActiveEmpty);
        } else {
            for id in active_ids {
                items.push(TaskListItem::ActiveTask(id));
            }
        }

        items.push(TaskListItem::ArchivedHeader);

        if !shell.archived_collapsed {
            if shell.task_fetch_archived == TaskFetchState::Loading {
                items.push(TaskListItem::ArchivedLoading);
            }
            if shell.task_fetch_archived == TaskFetchState::Error {
                items.push(TaskListItem::ArchivedError);
            }
            for id in archived_ids.iter().copied() {
                items.push(TaskListItem::ArchivedTask(id));
            }
            if archived_ids.is_empty()
                && shell.task_archived_loaded
                && shell.task_fetch_archived != TaskFetchState::Loading
            {
                items.push(TaskListItem::ArchivedEmpty);
            }
        }

        let mut active_last_index = None;
        for (idx, item) in items.iter().enumerate() {
            match item {
                TaskListItem::ActiveTask(_) | TaskListItem::ActiveEmpty => {
                    active_last_index = Some(idx)
                }
                TaskListItem::ArchivedHeader => break,
                _ => {}
            }
        }

        let item_sizes = items
            .iter()
            .map(|item| {
                let height = match item {
                    TaskListItem::ActiveHeader => HEADER_HEIGHT + HEADER_GAP,
                    TaskListItem::ActiveTask(_) | TaskListItem::ArchivedTask(_) => {
                        TASK_ROW_HEIGHT + TASK_ROW_GAP
                    }
                    TaskListItem::ArchivedHeader => HEADER_HEIGHT + ARCHIVED_HEADER_MARGIN,
                    TaskListItem::ActiveEmpty
                    | TaskListItem::ArchivedLoading
                    | TaskListItem::ArchivedError
                    | TaskListItem::ArchivedEmpty => STATUS_ROW_HEIGHT + TASK_ROW_GAP,
                };
                size(px(0.0), px(height))
            })
            .collect();

        Self {
            items,
            item_sizes,
            active_last_index,
        }
    }
}

fn render_task_list_item(
    view: &mut ShellView,
    item: TaskListItem,
    cx: &mut Context<ShellView>,
) -> gpui::AnyElement {
    match item {
        TaskListItem::ActiveHeader => render_active_header(view).into_any_element(),
        TaskListItem::ActiveEmpty => render_status_row(view, "No active tasks.").into_any_element(),
        TaskListItem::ActiveTask(task_id) => render_task_row(view, task_id, false, cx).into_any_element(),
        TaskListItem::ArchivedHeader => render_archived_header(view, cx).into_any_element(),
        TaskListItem::ArchivedLoading => {
            render_status_row(view, "Loading archived tasks…").into_any_element()
        }
        TaskListItem::ArchivedError => {
            render_status_row(view, "Failed to load archived tasks. Retry.").into_any_element()
        }
        TaskListItem::ArchivedEmpty => render_status_row(view, "No archived tasks.").into_any_element(),
        TaskListItem::ArchivedTask(task_id) => render_task_row(view, task_id, true, cx).into_any_element(),
    }
}

fn render_active_header(view: &ShellView) -> impl IntoElement {
    div()
        .h(px(HEADER_HEIGHT))
        .flex()
        .flex_col()
        .pb(px(8.0))
        .child(
            div()
                .text_size(px(12.0))
                .line_height(px(14.0))
                .text_color(view.colors.muted)
                .child("Active".to_string().to_uppercase()),
        )
}

fn render_archived_header(view: &ShellView, cx: &mut Context<ShellView>) -> impl IntoElement {
    let is_collapsed = view.archived_collapsed;
    let chevron = Icon::new(IconName::ChevronDown, 14.0, rgba(255, 255, 255, 0.55))
        .with_animation(
            ElementId::named_usize("archived-chevron", is_collapsed as usize),
            Animation::new(HOVER_FADE_DURATION).with_easing(ease_in_out),
            move |icon, delta| {
                let angle = if is_collapsed {
                    -std::f32::consts::FRAC_PI_2 * delta
                } else {
                    -std::f32::consts::FRAC_PI_2 * (1.0 - delta)
                };
                icon.transform(Transformation::rotate(radians(angle)))
            },
        );

    let toggle = div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .cursor_pointer()
        .id("sidebar-archived-toggle")
        .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
            let next = !view.archived_collapsed;
            view.set_archived_collapsed(next, cx);
            if !next {
                view.ensure_archived_loaded(cx);
            }
        }))
        .child(
            div()
                .text_size(px(12.0))
                .line_height(px(14.0))
                .text_color(view.colors.muted)
                .child("Archived".to_string().to_uppercase()),
        )
        .child(chevron);

    div()
        .h(px(HEADER_HEIGHT + ARCHIVED_HEADER_MARGIN))
        .pt(px(ARCHIVED_HEADER_MARGIN))
        .child(toggle)
}

fn render_status_row(view: &ShellView, label: &str) -> impl IntoElement {
    div()
        .h(px(STATUS_ROW_HEIGHT + TASK_ROW_GAP))
        .flex()
        .items_center()
        .text_size(px(12.0))
        .line_height(px(14.0))
        .text_color(view.colors.muted)
        .child(label.to_string())
}

fn render_task_row(
    view: &ShellView,
    task_id: TaskId,
    archived: bool,
    cx: &mut Context<ShellView>,
) -> impl IntoElement {
    let Some(task) = view.tasks_by_id.get(&task_id) else {
        return div().h(px(TASK_ROW_HEIGHT + TASK_ROW_GAP)).into_any_element();
    };

    let title = if task.task.title.trim().is_empty() {
        "New Task".to_string()
    } else {
        task.task.title.clone()
    };

    let selected = view.selected_task == Some(task_id);
    let hovered = view.task_hovered == Some(task_id);
    let is_renaming = view.renaming_task_id == Some(task_id);

    let pending_action = view.archive_pending.get(&task_id).copied();
    let archive_pending = pending_action.is_some();
    let archiving = matches!(pending_action, Some(TaskArchiveAction::Archive));

    let mut provider_ids = Vec::new();
    let mut provider_set = HashSet::new();
    let mut working = false;
    let mut has_error = false;
    let mut unread_session = false;

    for track in &task.tracks {
        for session in &track.sessions {
            if session.activity.is_working {
                working = true;
            }
            if matches!(
                session.session.status,
                SessionStatus::Failed | SessionStatus::Cancelled
            ) {
                has_error = true;
            }
            if session.unread.unwrap_or(false) {
                unread_session = true;
            }
            let provider_id = session.session.provider_id.trim();
            if !provider_id.is_empty() && provider_set.insert(provider_id.to_string()) {
                provider_ids.push(provider_id.to_string());
            }
        }
    }

    let last_assistant = task.task.last_assistant_message_at;
    let seen = task.task.assistant_seen_at;
    let unread_task = match last_assistant {
        Some(last) => seen.map(|seen| last > seen).unwrap_or(true),
        None => false,
    };

    let unread = !working && (unread_session || unread_task);
    let dot_kind = if has_error {
        Some("error")
    } else if unread {
        Some("unread")
    } else {
        None
    };

    let age_at = task
        .task
        .last_activity_at
        .or(Some(task.task.updated_at))
        .or(Some(task.task.created_at));
    let age_label = {
        let formatted = format_relative_age_short(age_at, view.relative_now);
        if formatted.is_empty() {
            "Now".to_string()
        } else {
            formatted
        }
    };

    let show_actions = hovered || view.task_menu.map(|menu| menu.task_id == task_id).unwrap_or(false);

    let leading = render_task_leading(view, provider_ids, selected);
    let meta = render_task_meta(
        view,
        task_id,
        archived,
        archiving,
        archive_pending,
        working,
        dot_kind,
        age_label,
        show_actions,
        cx,
    );

    let body = if is_renaming {
        render_task_rename(view, task_id, cx)
    } else {
        div()
            .flex_1()
            .min_w(px(0.0))
            .child(
                div()
                    .text_size(px(12.5))
                    .line_height(px(14.0))
                    .text_color(view.colors.text)
                    .truncate()
                    .child(title),
            )
            .into_any_element()
    };

    let mut row = div()
        .w_full()
        .px(px(8.0))
        .py(px(5.0))
        .gap(px(8.0))
        .flex()
        .items_center()
        .rounded(px(10.0))
        .border_1()
        .border_color(if selected { view.colors.border } else { rgba(0, 0, 0, 0.0) })
        .bg(if selected { ACTIVE_BG } else { rgba(0, 0, 0, 0.0) })
        .cursor_pointer()
        .id(ElementId::from((ElementId::from(task_id.0), "task-row")))
        .child(leading)
        .child(body)
        .child(meta)
        .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
            if event.is_right_click() {
                return;
            }
            view.focus_task(task_id, window, cx);
        }))
        .on_hover(cx.listener(move |view, hovered, _window, cx| {
            if *hovered {
                view.task_hovered = Some(task_id);
            } else if view.task_hovered == Some(task_id) {
                view.task_hovered = None;
            }
            cx.notify();
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(move |view, event: &MouseDownEvent, window, cx| {
            window.prevent_default();
            let anchor = anchor_from_point(event.position, 0.0);
            view.toggle_task_menu(task_id, anchor, cx);
            cx.stop_propagation();
        }))
        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
            let key = event.keystroke.key.to_lowercase();
            if key == "enter" || key == " " {
                view.focus_task(task_id, window, cx);
                cx.stop_propagation();
            }
        }));

    if hovered && !selected {
        row = row.bg(HOVER_BG);
    }
    if archived {
        row = row.opacity(0.85);
    }

    div()
        .h(px(TASK_ROW_HEIGHT + TASK_ROW_GAP))
        .child(row)
        .into_any_element()
}

fn render_task_leading(view: &ShellView, provider_ids: Vec<String>, selected: bool) -> impl IntoElement {
    let provider_count = provider_ids.len();
    let color = if selected { ACTIVE_ICON } else { MUTED_ICON };

    let leading_icon = if provider_count > 1 {
        Icon::new(IconName::LayersPlus, 16.0, color).into_any_element()
    } else if let Some(provider_id) = provider_ids.first() {
        if let Some(entry) = harness_entry(provider_id) {
            let image = if view.is_dark && entry.invert_in_dark {
                entry.inverted_image.clone()
            } else {
                entry.image.clone()
            };
            img(image)
                .w(px(16.0))
                .h(px(16.0))
                .object_fit(ObjectFit::Contain)
                .rounded(px(6.0))
                .into_any_element()
        } else {
            render_harness_fallback().into_any_element()
        }
    } else {
        render_harness_fallback().into_any_element()
    };

    div()
        .w(px(18.0))
        .h(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .child(leading_icon)
}

fn render_harness_fallback() -> impl IntoElement {
    div()
        .w(px(16.0))
        .h(px(16.0))
        .rounded(px(6.0))
        .bg(rgba(255, 255, 255, 0.10))
        .border_1()
        .border_color(rgba(18, 18, 18, 0.80))
}

fn render_task_meta(
    view: &ShellView,
    task_id: TaskId,
    archived: bool,
    archiving: bool,
    archive_pending: bool,
    working: bool,
    dot_kind: Option<&'static str>,
    age_label: String,
    show_actions: bool,
    cx: &mut Context<ShellView>,
) -> impl IntoElement {
    let status = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(0.0))
        .child(
            div()
                .min_w(px(28.0))
                .text_right()
                .text_size(px(12.0))
                .line_height(px(14.0))
                .text_color(view.colors.muted)
                .child(age_label),
        )
        .child(render_task_spinner(
            task_id,
            "work",
            working,
            SPINNER_BG,
            SPINNER_ACCENT,
        ))
        .child(render_task_spinner(
            task_id,
            "archive",
            archiving,
            SPINNER_ARCHIVE,
            view.colors.warning,
        ));

    let status = if let Some(kind) = dot_kind {
        status.child(render_status_dot(kind))
    } else {
        status
    };

    let actions = render_task_actions(
        view,
        task_id,
        archived,
        archiving,
        archive_pending,
        cx,
    );

    let status_id = ElementId::from((
        ElementId::from(task_id.0),
        if show_actions { "task-status-hide" } else { "task-status-show" },
    ));
    let actions_id = ElementId::from((
        ElementId::from(task_id.0),
        if show_actions { "task-actions-show" } else { "task-actions-hide" },
    ));

    let status = status.with_animation(
        status_id,
        Animation::new(HOVER_FADE_DURATION).with_easing(ease_in_out),
        move |this, delta| {
            let opacity = if show_actions { 1.0 - delta } else { delta };
            this.opacity(opacity)
        },
    );

    let actions = actions.with_animation(
        actions_id,
        Animation::new(HOVER_FADE_DURATION).with_easing(ease_in_out),
        move |this, delta| {
            let opacity = if show_actions { delta } else { 1.0 - delta };
            this.opacity(opacity).when(opacity <= 0.01, |this| this.invisible())
        },
    );

    div()
        .min_w(px(44.0))
        .flex_none()
        .relative()
        .flex()
        .items_center()
        .justify_end()
        .child(status)
        .child(actions)
}

fn render_task_spinner(
    task_id: TaskId,
    kind: &'static str,
    active: bool,
    base: Rgba,
    accent: Rgba,
) -> impl IntoElement {
    if !active {
        return div().into_any_element();
    }

    let base_id = ElementId::from(task_id.0);
    let spinner_id = ElementId::from((base_id, kind));
    let arc = Icon::new(IconName::SpinnerArc, 12.0, accent).with_animation(
        spinner_id,
        Animation::new(SPINNER_DURATION).repeat(),
        |icon, _delta| {
            let phase = spinner_phase();
            icon.transform(Transformation::rotate(percentage(phase)))
        },
    );

    div()
        .w(px(12.0))
        .h(px(12.0))
        .ml(px(8.0))
        .relative()
        .child(
            div()
                .w(px(12.0))
                .h(px(12.0))
                .rounded_full()
                .border_2()
                .border_color(base),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .items_center()
                .justify_center()
                .child(arc),
        )
        .into_any_element()
}

fn render_status_dot(kind: &str) -> impl IntoElement {
    let color = match kind {
        "error" => rgba(255, 69, 58, 0.95),
        _ => rgba(78, 163, 255, 0.95),
    };

    div()
        .w(px(6.0))
        .h(px(6.0))
        .ml(px(8.0))
        .rounded_full()
        .bg(color)
}

fn render_task_actions(
    _view: &ShellView,
    task_id: TaskId,
    archived: bool,
    archiving: bool,
    archive_pending: bool,
    cx: &mut Context<ShellView>,
) -> gpui::Div {
    let menu_button = task_action_button(
        cx,
        ElementId::from((ElementId::from(task_id.0), "task-menu-trigger")),
        IconName::Ellipsis,
        false,
        move |view, event, _window, cx| {
            let anchor = anchor_from_click(event, 20.0);
            view.toggle_task_menu(task_id, anchor, cx);
        },
    )
    .tooltip(|window, cx| Tooltip::new("More actions").build(window, cx));

    let archive_label = if archive_pending {
        if archiving {
            "Archiving..."
        } else {
            "Unarchiving..."
        }
    } else if archived {
        "Unarchive"
    } else {
        "Archive"
    };

    let archive_button = task_action_button(
        cx,
        ElementId::from((ElementId::from(task_id.0), "task-archive-trigger")),
        IconName::Archive,
        archive_pending,
        move |view, event, _window, cx| {
            if view.archive_pending.contains_key(&task_id) {
                return;
            }
            let anchor = anchor_from_click(event, 20.0);
            view.toggle_task_archive(task_id, !archived, anchor, cx);
        },
    )
    .tooltip({
        let label = archive_label.to_string();
        move |window, cx| Tooltip::new(label.clone()).build(window, cx)
    });

    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .flex()
        .items_center()
        .gap(px(2.0))
        .child(menu_button)
        .child(archive_button)
}

fn task_action_button(
    cx: &mut Context<ShellView>,
    id: ElementId,
    icon: IconName,
    disabled: bool,
    handler: impl Fn(&mut ShellView, &ClickEvent, &mut gpui::Window, &mut Context<ShellView>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let icon = svg()
        .path(icon.path())
        .w(px(14.0))
        .h(px(14.0))
        .flex_none();

    let mut button = div()
        .w(px(20.0))
        .h(px(20.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .text_color(MUTED_ICON)
        .child(icon)
        .cursor_pointer()
        .id(id)
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        });

    if disabled {
        button = button.opacity(0.55).cursor_not_allowed();
    } else {
        button = button
            .hover(|this| this.bg(ACTION_HOVER_BG).text_color(ACTION_ICON_HOVER))
            .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                handler(view, event, window, cx);
            }));
    }

    button
}

fn render_task_rename(
    view: &ShellView,
    _task_id: TaskId,
    cx: &mut Context<ShellView>,
) -> gpui::AnyElement {
    let input = Input::new(&view.rename_input)
        .appearance(false)
        .bordered(false)
        .focus_bordered(false)
        .px(px(0.0))
        .py(px(0.0))
        .h(px(18.0))
        .line_height(px(18.0))
        .text_size(px(12.5))
        .text_color(view.colors.text)
        .w_full();

    div()
        .flex_1()
        .min_w(px(0.0))
        .w_full()
        .child(input)
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _window, cx| {
            if event.keystroke.key == "Escape" {
                view.cancel_task_rename(cx);
            }
        }))
        .into_any_element()
}

pub(crate) struct SidebarOverlays<'a> {
    pub(crate) shell: &'a ShellView,
    pub(crate) viewport: gpui::Size<Pixels>,
}

impl<'a> SidebarOverlays<'a> {
    pub(crate) fn render(&self, cx: &mut Context<ShellView>) -> gpui::AnyElement {
        if self.shell.task_menu.is_none() && self.shell.archive_confirm.is_none() {
            return div().into_any_element();
        }

        let mut overlay = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(MouseButton::Left, cx.listener(|view, _: &MouseDownEvent, _window, cx| {
                view.close_task_menu(cx);
                view.cancel_archive_confirm(cx);
            }));

        if let Some(menu) = self.shell.task_menu {
            overlay = overlay.child(render_task_menu(self.shell, menu, self.viewport, cx));
        }

        if let Some(confirm) = self.shell.archive_confirm {
            overlay = overlay.child(render_archive_confirm(self.shell, confirm, self.viewport, cx));
        }

        overlay.into_any_element()
    }
}

fn render_task_menu(
    view: &ShellView,
    menu: super::super::state::TaskMenuState,
    viewport: gpui::Size<Pixels>,
    cx: &mut Context<ShellView>,
) -> gpui::AnyElement {
    let viewport_w = f32::from(viewport.width);
    let viewport_h = f32::from(viewport.height);

    let anchor = menu.anchor;
    let margin = 8.0;
    let base_left = if anchor.width <= 0.0 {
        clamp(anchor.left, margin, viewport_w - margin)
    } else {
        anchor.left
    };
    let base_top = if anchor.height <= 0.0 {
        clamp(anchor.top, margin, viewport_h - margin)
    } else {
        anchor.bottom + 6.0
    };

    let left = base_left.min(viewport_w - 240.0).max(margin);
    let top = base_top.min(viewport_h - 260.0).max(margin);

    let Some(summary) = view.tasks_by_id.get(&menu.task_id) else {
        return div().into_any_element();
    };

    let task = &summary.task;
    let task_id = menu.task_id;
    let task_archived = task.archived_at.is_some();
    let last_assistant = task.last_assistant_message_at;
    let seen = task.assistant_seen_at;
    let unread = match last_assistant {
        Some(last) => seen.map(|seen| last > seen).unwrap_or(true),
        None => false,
    };

    let mark_label = if unread { "Mark as Read" } else { "Mark as Unread" };
    let archive_label = if task.archived_at.is_some() { "Unarchive" } else { "Archive" };

    fn menu_item(
        cx: &mut Context<ShellView>,
        id: ElementId,
        label: String,
        disabled: bool,
        danger: bool,
        handler: impl Fn(&mut ShellView, &ClickEvent, &mut gpui::Window, &mut Context<ShellView>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let mut item = div()
            .w_full()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgba(0, 0, 0, 0.0))
            .text_size(px(12.0))
            .text_color(if danger { MENU_ITEM_DANGER } else { rgba(255, 255, 255, 0.92) })
            .child(label)
            .cursor_pointer()
            .id(id);

        if disabled {
            item = item.opacity(0.55).cursor_not_allowed();
        } else if danger {
            item = item
                .hover(|this| this.bg(MENU_ITEM_DANGER_BG).border_color(MENU_ITEM_DANGER_BORDER))
                .on_click(cx.listener(handler));
        } else {
            item = item
                .hover(|this| this.bg(MENU_ITEM_HOVER_BG).border_color(MENU_ITEM_HOVER_BORDER))
                .on_click(cx.listener(handler));
        }

        item
    }

    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .min_w(px(220.0))
        .max_w(px(440.0_f32.min(viewport_w * 0.78_f32)))
        .rounded(px(12.0))
        .border_1()
        .border_color(MENU_BORDER)
        .bg(MENU_BG)
        .shadow(vec![BoxShadow {
            color: Hsla::from(rgba(0, 0, 0, 0.55)),
            offset: point(px(0.0), px(16.0)),
            blur_radius: px(40.0),
            spread_radius: px(0.0),
        }])
        .p(px(6.0))
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(menu_item(
            cx,
            ElementId::from((ElementId::from(task_id.0), "menu-rename")),
            "Rename Task".to_string(),
            false,
            false,
            move |view, _event, window, cx| {
                view.close_task_menu(cx);
                view.begin_task_rename(task_id, window, cx);
            },
        ))
        .child(menu_item(
            cx,
            ElementId::from((ElementId::from(task_id.0), "menu-archive")),
            archive_label.to_string(),
            view.archive_pending.contains_key(&task_id),
            false,
            move |view, event, _window, cx| {
                let anchor = anchor_from_click(event, 20.0);
                view.close_task_menu(cx);
                view.toggle_task_archive(task_id, !task_archived, anchor, cx);
            },
        ))
        .child(menu_item(
            cx,
            ElementId::from((ElementId::from(task_id.0), "menu-mark")),
            mark_label.to_string(),
            last_assistant.is_none(),
            false,
            move |view, _event, _window, cx| {
                view.close_task_menu(cx);
                if unread {
                    view.mark_task_read(task_id, cx);
                } else {
                    view.mark_task_unread(task_id, cx);
                }
            },
        ))
        .child(menu_item(
            cx,
            ElementId::from((ElementId::from(task_id.0), "menu-delete")),
            "Delete Task".to_string(),
            false,
            true,
            move |view, _event, _window, cx| {
                view.close_task_menu(cx);
                view.delete_task(task_id, cx);
            },
        ))
        .into_any_element()
}

fn render_archive_confirm(
    view: &ShellView,
    confirm: super::super::state::ArchiveConfirmState,
    viewport: gpui::Size<Pixels>,
    cx: &mut Context<ShellView>,
) -> impl IntoElement {
    let viewport_w = f32::from(viewport.width);
    let viewport_h = f32::from(viewport.height);
    let margin = 12.0;
    let width = (360.0_f32)
        .min(viewport_w - margin * 2.0)
        .max(160.0_f32);
    let left = clamp(
        confirm.anchor.left + confirm.anchor.width / 2.0 - width / 2.0,
        margin,
        viewport_w - width - margin,
    );
    let top = clamp(confirm.anchor.bottom + 10.0, margin, viewport_h - 180.0);
    let view_handle = cx.entity();

    let checkbox = Checkbox::new("archive-confirm")
        .checked(view.archive_confirm_dont_remind)
        .label("Don't remind me again")
        .text_sm()
        .text_color(ARCHIVE_TOGGLE_TEXT)
        .on_click(move |checked, _window, cx| {
            let _ = view_handle.update(cx, |view, cx| {
                view.archive_confirm_dont_remind = *checked;
                cx.notify();
            });
        });

    let cancel_button = div()
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(rgba(255, 255, 255, 0.16))
        .bg(rgba(255, 255, 255, 0.04))
        .text_size(px(12.0))
        .text_color(rgba(255, 255, 255, 0.92))
        .child("Cancel")
        .cursor_pointer()
        .id("archive-confirm-cancel")
        .hover(|this| this.bg(rgba(255, 255, 255, 0.14)))
        .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
            view.cancel_archive_confirm(cx);
        }));

    let archive_button = div()
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(rgba(255, 120, 120, 0.35))
        .bg(rgba(255, 69, 58, 0.18))
        .text_size(px(12.0))
        .text_color(MENU_ITEM_DANGER)
        .child("Archive")
        .cursor_pointer()
        .id("archive-confirm-apply")
        .hover(|this| this.bg(rgba(255, 69, 58, 0.26)))
        .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
            view.confirm_archive(cx);
        }));

    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .rounded(px(12.0))
        .border_1()
        .border_color(ARCHIVE_CONFIRM_BORDER)
        .bg(ARCHIVE_CONFIRM_BG)
        .shadow(vec![BoxShadow {
            color: Hsla::from(rgba(0, 0, 0, 0.55)),
            offset: point(px(0.0), px(16.0)),
            blur_radius: px(40.0),
            spread_radius: px(0.0),
        }])
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .text_size(px(12.0))
        .text_color(rgba(255, 255, 255, 0.92))
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ARCHIVE_CONFIRM_TITLE)
                .child("Archive conversation?"),
        )
        .child(
            div()
                .text_size(px(12.5))
                .line_height(px(16.0))
                .text_color(ARCHIVE_CONFIRM_BODY)
                .child(
                    "Archiving removes the worktree on disk. You can unarchive to recreate it. Uncommitted changes will be lost.",
                ),
        )
        .child(checkbox)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(10.0))
                .child(cancel_button)
                .child(archive_button),
        )
}

fn anchor_from_click(event: &ClickEvent, size: f32) -> AnchorRect {
    let pos = event.position();
    anchor_from_point(pos, size)
}

fn anchor_from_point(pos: Point<Pixels>, size: f32) -> AnchorRect {
    let half = size / 2.0;
    let x = f32::from(pos.x);
    let y = f32::from(pos.y);
    let left = x - half;
    let top = y - half;
    AnchorRect {
        left,
        top,
        bottom: top + size,
        width: size,
        height: size,
    }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        return min;
    }
    value.min(max).max(min)
}

fn spinner_phase() -> f32 {
    let anchor = SPINNER_ANCHOR.get_or_init(Instant::now);
    let duration = SPINNER_DURATION.as_secs_f32();
    if duration <= f32::EPSILON {
        return 0.0;
    }
    let elapsed = anchor.elapsed().as_secs_f32();
    (elapsed / duration).rem_euclid(1.0)
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}
