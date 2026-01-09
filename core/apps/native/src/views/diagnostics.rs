use gpui::{div, prelude::*};

use crate::theme::ThemeColors;

pub(super) struct DiagnosticsPanelView {
    pub(super) colors: ThemeColors,
    pub(super) frame_time_ms: Option<f32>,
    pub(super) message_count: usize,
    pub(super) event_count: usize,
    pub(super) memory_mb: Option<u64>,
}

impl DiagnosticsPanelView {
    pub(super) fn render(&self) -> impl IntoElement {
        let frame_text = match self.frame_time_ms {
            Some(value) => format!("Frame: {value:.1} ms"),
            None => "Frame: -- ms".to_string(),
        };
        let memory_text = match self.memory_mb {
            Some(value) => format!("Memory: {value} MB"),
            None => "Memory: -- MB".to_string(),
        };

        div()
            .id("diagnostics-panel")
            .flex()
            .flex_col()
            .gap_2()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .bg(self.colors.panel_2)
            .p_3()
            .child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Diagnostics"),
            )
            .child(div().text_sm().child(frame_text))
            .child(div().text_sm().child(format!("Messages: {}", self.message_count)))
            .child(div().text_sm().child(format!("Events: {}", self.event_count)))
            .child(div().text_sm().child(memory_text))
    }
}
