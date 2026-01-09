use gpui::{Div, IntoElement, prelude::*, div};

use crate::theme::ThemeColors;

pub fn header<E: IntoElement>(colors: ThemeColors, left: E) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .text_sm()
        .text_color(colors.muted)
        .child(left)
}

pub fn header_lr<L: IntoElement, R: IntoElement>(colors: ThemeColors, left: L, right: R) -> Div {
    header(colors, div().flex().items_center().justify_between().w_full().child(left).child(right))
}
