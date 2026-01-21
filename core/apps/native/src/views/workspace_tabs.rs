use gpui::{ClickEvent, Context, FontWeight, div, prelude::*, px};

use crate::theme::ThemeMetrics;

use super::super::icons::{Icon, IconName};
use super::super::state::{ShellRoute, ShellView};

pub(crate) struct WorkspaceTabsView<'a> {
    pub(crate) shell: &'a ShellView,
}

impl<'a> WorkspaceTabsView<'a> {
    pub(crate) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let shell = self.shell;
        let metrics = ThemeMetrics::default();
        let active_id = shell.active_workspace_tab.or(shell.selected_workspace);

        let mut tabs_row = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w(px(0.0));

        for workspace_id in &shell.workspace_tabs {
            let id = *workspace_id;
            let active = active_id == Some(id);
            let label = shell
                .workspaces
                .iter()
                .find(|ws| ws.id == id)
                .map(|ws| ws.name.clone())
                .unwrap_or_else(|| id.0.to_string().chars().take(8).collect::<String>());

            let tab_bg = if active {
                shell.colors.panel
            } else {
                shell.colors.bg
            };
            let tab_border = if active {
                shell.colors.border
            } else {
                shell.colors.border
            };
            let tab_text = if active {
                shell.colors.text
            } else {
                shell.colors.muted
            };

            let close_btn = div()
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .id(format!("workspace-tab-close-{}", id.0))
                .hover(|style| style.bg(shell.colors.panel_2))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |view, _event: &ClickEvent, _window, cx| {
                    view.close_workspace_tab(id, cx);
                    cx.stop_propagation();
                }))
                .child(Icon::new(IconName::Cancel, 12.0, tab_text));

            let tab = div()
                .h(px(26.0))
                .px(px(10.0))
                .gap(px(8.0))
                .flex()
                .items_center()
                .rounded(px(10.0))
                .border_1()
                .border_color(tab_border)
                .bg(tab_bg)
                .cursor_pointer()
                .id(format!("workspace-tab-{}", id.0))
                .on_click(cx.listener(move |view, event: &ClickEvent, _window, cx| {
                    if event.is_right_click() {
                        return;
                    }
                    view.activate_workspace_tab(id, cx);
                }))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight(600.0))
                        .text_color(tab_text)
                        .child(label),
                )
                .child(close_btn);

            tabs_row = tabs_row.child(tab);
        }

        let add_btn = div()
            .w(px(26.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.0))
            .border_1()
            .border_color(shell.colors.border)
            .bg(shell.colors.bg)
            .cursor_pointer()
            .id("workspace-tab-add")
            .hover(|style| style.bg(shell.colors.panel_2))
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, event: &ClickEvent, _window, cx| {
                if event.is_right_click() {
                    return;
                }
                view.set_route(ShellRoute::Workspaces, cx);
            }))
            .child(Icon::new(IconName::LayersPlus, 14.0, shell.colors.muted));

        div()
            .id("workspace-tabs")
            .h(px(36.0))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(metrics.spacing.xl))
            .border_b_1()
            .border_color(shell.colors.border)
            .bg(shell.colors.bg)
            .child(tabs_row)
            .child(add_btn)
    }
}

