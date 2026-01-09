use gpui::{Div, IntoElement, prelude::*, px, div};

use crate::theme::{ThemeColors, ThemeMetrics};

pub fn container<E: IntoElement>(colors: ThemeColors, body: E) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel_2)
        .p(px(metrics.spacing.md))
        .child(body)
}
