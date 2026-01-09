use gpui::{Div, IntoElement, prelude::*, px, div};

use crate::theme::ThemeColors;

#[derive(Clone, Copy)]
pub enum RowDensity {
    Tight,
    Regular,
}

fn density_padding(density: RowDensity) -> (f32, f32) {
    match density {
        RowDensity::Tight => (5.0, 8.0),
        RowDensity::Regular => (6.0, 10.0),
    }
}

pub fn row<E: IntoElement>(colors: ThemeColors, active: bool, density: RowDensity, left: E) -> Div {
    let (pad_y, pad_x) = density_padding(density);
    div()
        .flex()
        .items_center()
        .px(px(pad_x))
        .py(px(pad_y))
        .rounded_sm()
        .bg(if active { colors.panel } else { colors.panel_2 })
        .text_sm()
        .child(left)
}

pub fn row_lr<L: IntoElement, R: IntoElement>(
    colors: ThemeColors,
    active: bool,
    density: RowDensity,
    left: L,
    right: R,
) -> Div {
    row(colors, active, density, div().flex().items_center().justify_between().w_full().child(left).child(right))
}
