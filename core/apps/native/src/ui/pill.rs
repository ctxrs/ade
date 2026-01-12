use gpui::{Div, IntoElement, prelude::*, px, div};

use crate::theme::{ThemeColors, ThemeMetrics};

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum PillTone {
    Neutral,
    Accent,
    Success,
    Warning,
    Error,
}

fn tone_color(colors: ThemeColors, tone: PillTone) -> gpui::Rgba {
    match tone {
        PillTone::Neutral => colors.muted,
        PillTone::Accent => colors.accent,
        PillTone::Success => colors.success,
        PillTone::Warning => colors.warning,
        PillTone::Error => colors.error,
    }
}

#[allow(dead_code)]
pub fn pill<E: IntoElement>(colors: ThemeColors, label: E, tone: PillTone) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .px(px(metrics.spacing.md))
        .py(px(metrics.spacing.xs))
        .text_sm()
        .bg(colors.panel)
        .border_1()
        .border_color(colors.border)
        .rounded_full()
        .text_color(tone_color(colors, tone))
        .child(label)
}

#[allow(dead_code)]
pub fn count(colors: ThemeColors, n: usize) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .px(px(metrics.spacing.sm))
        .py(px(metrics.spacing.xs))
        .text_sm()
        .border_1()
        .border_color(colors.border)
        .rounded_full()
        .bg(colors.panel)
        .text_color(colors.muted)
        .child(n.to_string())
}
