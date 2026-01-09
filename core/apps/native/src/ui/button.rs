use gpui::{Div, IntoElement, prelude::*, px, div};

use crate::theme::{ThemeColors, ThemeMetrics};

// Minimal native button primitives wired to ThemeMetrics/Colors.

pub fn primary<E: IntoElement>(colors: ThemeColors, label: E) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .px(px(metrics.spacing.xxl))
        .py(px(metrics.spacing.lg))
        .text_sm()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel)
        .child(label)
}

pub fn ghost<E: IntoElement>(colors: ThemeColors, label: E) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .px(px(metrics.spacing.md))
        .py(px(metrics.spacing.sm))
        .text_sm()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel_2)
        .child(label)
}

pub fn ghost_icon<E: IntoElement>(colors: ThemeColors, icon: E) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .w(px(metrics.controls.h_md))
        .h(px(metrics.controls.h_md))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel_2)
        .child(icon)
}
