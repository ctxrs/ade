use gpui::{Div, IntoElement, prelude::*, px, div};

use crate::theme::{ThemeColors, ThemeMetrics};

#[allow(dead_code)]
pub fn row<E: IntoElement>(colors: ThemeColors, content: E) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .flex()
        .items_center()
        .gap(px(metrics.spacing.md))
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel)
        .p(px(metrics.spacing.md))
        .child(content)
}
