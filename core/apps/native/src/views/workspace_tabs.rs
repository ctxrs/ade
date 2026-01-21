use gpui::{
    BoxShadow,
    ClickEvent,
    Context,
    FontWeight,
    Hsla,
    MouseButton,
    MouseDownEvent,
    Rgba,
    div,
    point,
    prelude::*,
    px,
};
use gpui_component::scroll::ScrollableElement;

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
            .flex_1()
            .items_end()
            .gap_0()
            .min_w(px(0.0))
            .overflow_x_scrollbar();

        for workspace_id in &shell.workspace_tabs {
            let id = *workspace_id;
            let active = active_id == Some(id);
            let label = shell
                .workspaces
                .iter()
                .find(|ws| ws.id == id)
                .map(|ws| ws.name.clone())
                .unwrap_or_else(|| id.0.to_string().chars().take(8).collect::<String>());

            let tab_bg = if active { shell.colors.bg } else { shell.colors.panel };
            let tab_border = if active { shell.colors.border_strong } else { shell.colors.border };
            let tab_text = if active { shell.colors.text } else { shell.colors.muted };
            let show_close = active || shell.workspace_tab_hovered == Some(id);

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

            let close_slot = if show_close {
                close_btn.into_any_element()
            } else {
                div().w(px(18.0)).h(px(18.0)).into_any_element()
            };

            let mut tab = div()
                .h(px(28.0))
                .px(px(12.0))
                .gap(px(10.0))
                .flex()
                .items_center()
                .rounded_tl(px(10.0))
                .rounded_tr(px(10.0))
                .rounded_bl(px(0.0))
                .rounded_br(px(0.0))
                .border_1()
                .border_b_0()
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
                .on_hover(cx.listener(move |view, hovered, _window, cx| {
                    if *hovered {
                        view.workspace_tab_hovered = Some(id);
                    } else if view.workspace_tab_hovered == Some(id) {
                        view.workspace_tab_hovered = None;
                    }
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |view, _event: &MouseDownEvent, _window, cx| {
                    view.close_workspace_tab(id, cx);
                    cx.stop_propagation();
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_sm()
                        .font_weight(FontWeight(500.0))
                        .text_color(tab_text)
                        .child(label),
                )
                .child(close_slot);

            if active {
                tab = tab
                    .mt(px(0.0))
                    .shadow(vec![BoxShadow {
                        color: Hsla::from(rgba(0, 0, 0, 0.22)),
                        offset: point(px(0.0), px(1.0)),
                        blur_radius: px(8.0),
                        spread_radius: px(0.0),
                    }]);
            } else {
                tab = tab.mt(px(4.0));
            }

            tabs_row = tabs_row.child(tab);
        }

        let add_btn = div()
            .w(px(28.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(rgba(255, 255, 255, 0.0))
            .bg(rgba(255, 255, 255, 0.0))
            .cursor_pointer()
            .id("workspace-tab-add")
            .hover(|style| style.bg(shell.colors.panel).border_color(shell.colors.border))
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
            .bg(shell.colors.panel_2)
            .child(tabs_row)
            .child(add_btn)
    }
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}
