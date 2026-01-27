use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, ClickEvent, Context, ElementId, FontWeight, MouseButton, MouseUpEvent, Render, Window,
    div, img, linear_color_stop, linear_gradient, prelude::*, px, relative, ObjectFit, Rgba,
};
use gpui_component::{input::Input, select::Select};
use gpui_component::scroll::ScrollableElement;
use qrcode::{Color as QrColor, QrCode};

use crate::automation_tree;
use ctx_client::{
    InstallStateKind, MobileTunnelState, ResourceGovernanceMode, ResourceGovernanceStatusState,
    TitleGenerationMode,
};
use ctx_core::models::WorkspaceAttachmentKind;
use ctx_providers::adapters::ProviderHealth;

use super::super::harness_catalog::harness_catalog;
use crate::theme::ThemeColors;
use super::super::state::{
    LabeledOption, SettingsSection, SettingsSectionGroup, SettingsState, SETTINGS_SECTIONS, ShellRoute,
};

enum ButtonVariant {
    Primary,
    Secondary,
}

enum PillVariant {
    Default,
    Ok,
    Warn,
    Err,
}

const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";

fn rgba_u8(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

fn white(alpha: f32) -> Rgba {
    rgba_u8(255, 255, 255, alpha)
}

fn green(alpha: f32) -> Rgba {
    rgba_u8(34, 197, 94, alpha)
}

fn blue(alpha: f32) -> Rgba {
    rgba_u8(59, 130, 246, alpha)
}

fn yellow(alpha: f32) -> Rgba {
    rgba_u8(251, 191, 36, alpha)
}

fn red(alpha: f32) -> Rgba {
    rgba_u8(239, 68, 68, alpha)
}

fn settings_card<E: IntoElement>(
    colors: ThemeColors,
    title: Option<&str>,
    content: E,
) -> gpui::Div {
    let card = div()
        .bg(colors.panel)
        .border_1()
        .border_color(white(0.08))
        .rounded(px(12.0))
        .overflow_hidden();
    match title {
        Some(title) => card
            .child(
                div()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(white(0.08))
                    .bg(white(0.02))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white(0.86))
                    .child(title.to_string()),
            )
            .child(content),
        None => card.child(content),
    }
}

fn settings_rows(rows: Vec<AnyElement>) -> gpui::Div {
    div().grid().children(rows)
}

fn settings_row<E: IntoElement>(
    title: &str,
    description: Option<&str>,
    control: E,
    is_first: bool,
) -> gpui::Div {
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(18.0))
        .px(px(16.0))
        .py(px(12.0))
        .hover(|style| style.bg(white(0.02)));
    if !is_first {
        row = row.border_t_1().border_color(white(0.08));
    }

    row.child(
        div()
            .flex_1()
            .grid()
            .gap(px(2.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(relative(1.2))
                    .text_color(white(0.84))
                    .child(title.to_string()),
            )
            .when_some(description, |this, description| {
                this.child(
                    div()
                        .text_size(px(12.0))
                        .line_height(relative(1.2))
                        .text_color(white(0.42))
                        .child(description.to_string()),
                )
            }),
    )
    .child(
        div()
            .flex()
            .flex_none()
            .justify_end()
            .items_center()
            .gap(px(10.0))
            .child(control),
    )
}

fn settings_button_base(
    label: &str,
    variant: ButtonVariant,
    disabled: bool,
    progress_pct: Option<u8>,
    compact: bool,
) -> gpui::Div {
    let padding = if compact { px(10.0) } else { px(12.0) };
    let text_size = if compact { px(12.0) } else { px(13.0) };
    let mut button = div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(30.0))
        .px(padding)
        .border_1()
        .border_color(white(0.12))
        .rounded(px(8.0))
        .bg(white(0.06))
        .text_color(white(0.8))
        .text_size(text_size);

    if matches!(variant, ButtonVariant::Secondary) {
        button = button.relative().overflow_hidden();
        if let Some(pct) = progress_pct {
            let pct = (pct as f32 / 100.0).clamp(0.0, 1.0);
            button = button.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(relative(pct))
                    .bg(green(0.35)),
            );
        }
    }

    if disabled {
        button = button.opacity(0.55);
    } else {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(white(0.1)));
    }

    button.child(label.to_string())
}

fn settings_button(
    label: &str,
    variant: ButtonVariant,
    disabled: bool,
    progress_pct: Option<u8>,
) -> gpui::Div {
    settings_button_base(label, variant, disabled, progress_pct, false)
}

fn settings_button_compact(
    label: &str,
    variant: ButtonVariant,
    disabled: bool,
    progress_pct: Option<u8>,
) -> gpui::Div {
    settings_button_base(label, variant, disabled, progress_pct, true)
}

fn settings_toggle(checked: bool, disabled: bool) -> gpui::Div {
    let mut toggle = div()
        .flex()
        .items_center()
        .h(px(22.0))
        .w(px(40.0))
        .rounded(px(999.0))
        .border_1()
        .px(px(2.0));

    if checked {
        toggle = toggle
            .bg(green(0.55))
            .border_color(green(0.65))
            .justify_end();
    } else {
        toggle = toggle
            .bg(white(0.08))
            .border_color(white(0.12))
            .justify_start();
    }

    if disabled {
        toggle = toggle.opacity(0.5);
    } else {
        toggle = toggle.cursor_pointer();
    }

    toggle.child(
        div()
            .h(px(18.0))
            .w(px(18.0))
            .rounded(px(999.0))
            .bg(if checked { white(0.9) } else { white(0.75) }),
    )
}

fn settings_banner(message: &str, is_error: bool) -> gpui::Div {
    let (border, bg, text) = if is_error {
        (red(0.35), red(0.08), red(0.9))
    } else {
        (white(0.08), white(0.04), white(0.75))
    };

    div()
        .border_1()
        .border_color(border)
        .bg(bg)
        .rounded(px(12.0))
        .px(px(12.0))
        .py(px(10.0))
        .text_color(text)
        .text_size(px(13.0))
        .child(message.to_string())
}

fn settings_empty(message: &str, is_error: bool) -> gpui::Div {
    let color = if is_error { red(0.9) } else { white(0.45) };
    div()
        .px(px(2.0))
        .py(px(14.0))
        .text_color(color)
        .child(message.to_string())
}

fn settings_empty_compact(message: &str) -> gpui::Div {
    div()
        .py(px(6.0))
        .text_color(white(0.45))
        .child(message.to_string())
}

fn render_qr_from_value(value: &serde_json::Value) -> AnyElement {
    let data = match serde_json::to_string(value) {
        Ok(s) if !s.is_empty() => s,
        _ => return settings_empty("Invalid QR payload.", true).into_any_element(),
    };

    let code = match QrCode::new(data.as_bytes()) {
        Ok(code) => code,
        Err(_) => return settings_empty("Failed to generate QR code.", true).into_any_element(),
    };

    let quiet_zone = 2i32;
    let side = code.width() as i32;
    let total = side + quiet_zone * 2;
    let cell_size = (220.0 / total.max(1) as f32).floor().max(2.0);
    let size_px = px(cell_size * total as f32);

    let colors = code.into_colors();
    let mut idx = 0usize;

    let mut container = div()
        .w(size_px)
        .h(size_px)
        .border_1()
        .border_color(white(0.08))
        .rounded(px(12.0))
        .bg(white(0.04))
        .px(px(8.0))
        .py(px(8.0))
        .grid()
        .gap(px(0.0));

    for y in -quiet_zone..(side + quiet_zone) {
        let mut row = div().flex();
        for x in -quiet_zone..(side + quiet_zone) {
            let filled = if x >= 0 && y >= 0 && x < side && y < side {
                let c = colors[idx];
                idx += 1;
                matches!(c, QrColor::Dark)
            } else {
                false
            };
            row = row.child(
                div()
                    .w(px(cell_size))
                    .h(px(cell_size))
                    .bg(if filled { white(0.92) } else { rgba_u8(0, 0, 0, 0.0) }),
            );
        }
        container = container.child(row);
    }

    container.into_any_element()
}

fn settings_pill(label: &str, variant: PillVariant, mono: bool) -> gpui::Div {
    let (border, bg, text) = match variant {
        PillVariant::Default => (white(0.12), white(0.06), white(0.78)),
        PillVariant::Ok => (green(0.45), green(0.12), green(0.95)),
        PillVariant::Warn => (yellow(0.45), yellow(0.12), yellow(0.95)),
        PillVariant::Err => (red(0.45), red(0.12), red(0.95)),
    };

    div()
        .flex()
        .items_center()
        .h(px(18.0))
        .px(px(8.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .rounded(px(999.0))
        .text_color(text)
        .text_size(px(11.0))
        .when(mono, |this| {
            this.font_family("ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace")
        })
        .child(label.to_string())
}

fn settings_code_block(code: &str) -> impl IntoElement {
    div()
        .font_family(
            "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace",
        )
        .text_size(px(12.0))
        .text_color(white(0.72))
        .bg(white(0.04))
        .border_1()
        .border_color(white(0.08))
        .rounded(px(8.0))
        .px(px(10.0))
        .py(px(8.0))
        .max_w(px(520.0))
        .child(code.to_string())
}

fn settings_metric(label: &str, value: &str, sublabel: Option<&str>, pct: Option<f32>) -> gpui::Div {
    let safe_pct = pct
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
        .max(0.0)
        / 100.0;
    let label = label.to_uppercase();
    div()
        .border_1()
        .border_color(white(0.08))
        .bg(white(0.03))
        .rounded(px(10.0))
        .px(px(12.0))
        .py(px(10.0))
        .grid()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .justify_between()
                .items_end()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(white(0.55))
                        .child(label),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(white(0.9))
                        .child(value.to_string()),
                ),
        )
        .child(
            div()
                .h(px(6.0))
                .rounded(px(999.0))
                .bg(white(0.08))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .rounded(px(999.0))
                        .w(relative(safe_pct))
                        .bg(linear_gradient(
                            90.0,
                            linear_color_stop(green(0.8), 0.0),
                            linear_color_stop(blue(0.8), 1.0),
                        )),
                ),
        )
        .when_some(sublabel, |this, sublabel| {
            this.child(
                div()
                    .text_size(px(11.0))
                    .text_color(white(0.5))
                    .child(sublabel.to_string()),
            )
        })
}

fn clamp_pct(value: u8) -> u8 {
    value.min(100)
}

fn format_pct(value: Option<f32>) -> String {
    match value {
        Some(value) if value.is_finite() => format!("{}%", value.round() as i32),
        _ => "—".to_string(),
    }
}

fn format_bytes(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut idx = 0usize;
    let mut v = value as f64;
    while v >= 1024.0 && idx < units.len() - 1 {
        v /= 1024.0;
        idx += 1;
    }
    let precision = if v >= 100.0 { 0 } else if v >= 10.0 { 1 } else { 2 };
    format!("{:.*} {}", precision, v, units[idx])
}

fn format_age_ms(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return "—".to_string();
    };
    let total_seconds = ((ms as f64) / 1000.0).round() as i64;
    if total_seconds < 60 {
        return format!("{}s", total_seconds);
    }
    let mins = total_seconds / 60;
    let secs = total_seconds % 60;
    format!("{}m {}s", mins, secs)
}

fn format_age_iso(value: &DateTime<Utc>) -> String {
    let diff = Utc::now().signed_duration_since(value.clone());
    let ms = diff.num_milliseconds().max(0) as u64;
    format_age_ms(Some(ms))
}

fn truncate_text(value: &str, max_len: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_len.saturating_sub(1)).collect::<String>();
    out.push('…');
    out
}

fn guess_attachment_name(source: &str) -> String {
    let mut cleaned = source.trim().to_string();
    while cleaned.ends_with('/') || cleaned.ends_with('\\') {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        return String::new();
    }
    let slash_idx = cleaned
        .rfind('/')
        .or_else(|| cleaned.rfind('\\'))
        .or_else(|| cleaned.rfind(':'));
    let mut name = slash_idx
        .map(|idx| cleaned[idx + 1..].to_string())
        .unwrap_or(cleaned);
    if name.ends_with(".git") {
        name.truncate(name.len().saturating_sub(4));
    }
    name
}

fn format_gib(mb: Option<u32>) -> String {
    let Some(mb) = mb else {
        return String::new();
    };
    if mb == 0 {
        return String::new();
    }
    let gb = mb as f64 / 1024.0;
    if gb >= 10.0 {
        format!("{:.0}", gb)
    } else {
        format!("{:.1}", gb)
    }
}

impl Render for SettingsState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        self.sync_input_values(window, cx);

        let search_state = Self::ensure_input_state(
            &mut self.search_input,
            "Search settings ⌘F",
            super::super::state::SettingsInputKind::Search,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let query = search_state.read(cx).value().to_lowercase();

        let filtered_sections: Vec<SettingsSection> = SETTINGS_SECTIONS
            .iter()
            .copied()
            .filter(|section| {
                query.is_empty() || section.label().to_lowercase().contains(&query)
            })
            .collect();

        automation_tree::clear_prefix("settings-nav-main-item-");
        automation_tree::clear_prefix("settings-nav-advanced-item-");

        let active_section = self.active_section;
        let header_label = active_section.label();
        let any_saving = self.saving || self.editor_saving;
        let save_status = if any_saving {
            "Saving…"
        } else if self.save_error.is_some() {
            "Not saved"
        } else {
            " "
        };

        let loaded = !self.settings_loading
            && (self.settings.is_some() || self.settings_error.is_some());
        let load_error = self.settings_error.clone();

        let mut sidebar = div()
            .w(px(280.0))
            .flex_shrink_0()
            .py(px(18.0))
            .px(px(14.0))
            .bg(rgba_u8(27, 27, 27, 1.0))
            .border_r_1()
            .border_color(white(0.06));

        sidebar = sidebar
            .child(
                div()
                    .px(px(10.0))
                    .pb(px(8.0))
                    .grid()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(white(0.45))
                            .cursor_pointer()
                            .hover(|style| style.text_color(white(0.75)))
                            .child("← Back to Workspace".to_string())
                            .on_children_prepainted(automation_tree::track_children_bounds(
                                "settings-back-to-workspace",
                                "button",
                                Some("Back to Workspace"),
                                Some("settings-pane"),
                            ))
                            .id("settings-back-to-workspace")
                            .on_mouse_up(MouseButton::Left, cx.listener(|view, _: &MouseUpEvent, _window, cx| {
                                if let Some(handle) = view.shell_handle.clone() {
                                    handle.update(cx, |shell, cx| {
                                        shell.set_route(ShellRoute::Workbench, cx);
                                    });
                                }
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white(0.9))
                            .child("Settings".to_string()),
                    ),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .px(px(6.0))
                    .on_children_prepainted(automation_tree::track_children_bounds(
                        "settings-search-input",
                        "input",
                        Some("Search Settings"),
                        Some("settings-pane"),
                    ))
                    .id("settings-search-input")
                    .child(
                        Input::new(&search_state)
                            .appearance(true)
                            .bg(white(0.06))
                            .border_color(white(0.06))
                            .rounded(px(8.0))
                            .h(px(30.0))
                            .w_full()
                            .px(px(10.0))
                            .text_size(px(13.0))
                            .text_color(white(0.8)),
                    ),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .px(px(6.0))
                    .child(
                        div()
                            .grid()
                            .gap(px(2.0))
                            .on_children_prepainted(automation_tree::track_children_bounds(
                                "settings-nav-main-list",
                                "list",
                                Some("Settings"),
                                Some("settings-pane"),
                            ))
                            .id("settings-nav-main-list")
                            .children(filtered_sections.iter().copied().filter(|s| s.group() == SettingsSectionGroup::Main).enumerate().map(|(ix, section)| {
                                let is_active = section == active_section;
                                let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.set_active_section(section, cx);
                                });
                                let item_id = format!("settings-nav-main-item-{}", ix);
                                let item_label = section.label().to_string();
                                let item = div()
                                    .h(px(28.0))
                                    .px(px(10.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.0))
                                    .text_size(px(13.0))
                                    .child(item_label.clone())
                                    .text_color(if is_active { white(0.9) } else { white(0.55) })
                                    .bg(if is_active { white(0.1) } else { rgba_u8(0, 0, 0, 0.0) })
                                    .when(!is_active, |this| this.hover(|style| style.bg(white(0.06)).text_color(white(0.82))))
                                    .cursor_pointer()
                                    .id(ElementId::named_usize("settings-nav-main", ix))
                                    .on_click(on_click);
                                div()
                                    .on_children_prepainted(automation_tree::track_children_bounds_dynamic(
                                        item_id,
                                        "button".to_string(),
                                        Some(item_label),
                                        Some("settings-nav-main-list".to_string()),
                                    ))
                                    .child(item)
                            }))
                    )
                    .child(div().my(px(10.0)).h(px(1.0)).bg(white(0.06)))
                    .child(
                        div()
                            .grid()
                            .gap(px(2.0))
                            .on_children_prepainted(automation_tree::track_children_bounds(
                                "settings-nav-advanced-list",
                                "list",
                                Some("Advanced"),
                                Some("settings-pane"),
                            ))
                            .id("settings-nav-advanced-list")
                            .children(filtered_sections.iter().copied().filter(|s| s.group() == SettingsSectionGroup::Advanced).enumerate().map(|(ix, section)| {
                                let is_active = section == active_section;
                                let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.set_active_section(section, cx);
                                });
                                let item_id = format!("settings-nav-advanced-item-{}", ix);
                                let item_label = section.label().to_string();
                                let item = div()
                                    .h(px(28.0))
                                    .px(px(10.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.0))
                                    .text_size(px(13.0))
                                    .child(item_label.clone())
                                    .text_color(if is_active { white(0.9) } else { white(0.55) })
                                    .bg(if is_active { white(0.1) } else { rgba_u8(0, 0, 0, 0.0) })
                                    .when(!is_active, |this| this.hover(|style| style.bg(white(0.06)).text_color(white(0.82))))
                                    .cursor_pointer()
                                    .id(ElementId::named_usize("settings-nav-advanced", ix))
                                    .on_click(on_click);
                                div()
                                    .on_children_prepainted(automation_tree::track_children_bounds_dynamic(
                                        item_id,
                                        "button".to_string(),
                                        Some(item_label),
                                        Some("settings-nav-advanced-list".to_string()),
                                    ))
                                    .child(item)
                            }))
                    ),
            );

        let main_content = div()
            .flex_1()
            .bg(colors.bg)
            .overflow_y_scrollbar()
            .child(
                div()
                    .flex()
                    .justify_center()
                    .w_full()
                    .child(
                        div()
                            .w_full()
                            .px(px(26.0))
                            .py(px(22.0))
                            .max_w(px(980.0))
                            .child(
                                div()
                                    .grid()
                                    .gap(px(4.0))
                                    .mb(px(14.0))
                                    .child(
                                        div()
                                            .text_size(px(18.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(white(0.9))
                                            .child(header_label.to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(white(0.35))
                                            .min_h(px(16.0))
                                            .child(save_status.to_string()),
                                    ),
                            )
                            .when_some(self.save_error.clone(), |this, error| {
                                this.child(div().mt(px(14.0)).child(settings_banner(&error, true)))
                            })
                            .child(self.render_section(active_section, loaded, load_error, window, cx)),
                    ),
            );

        div()
            .on_children_prepainted(automation_tree::track_children_bounds(
                "settings-pane",
                "pane",
                Some("Settings"),
                Some("app-shell"),
            ))
            .id("settings-pane")
            .flex()
            .flex_row()
            .flex_1()
            .bg(colors.bg)
            .child(sidebar)
            .child(main_content)
    }
}

impl SettingsState {
    fn render_section(
        &mut self,
        active: SettingsSection,
        loaded: bool,
        load_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {

        eprintln!("ctx-native: settings render active={:?} loaded={} load_error={:?}", active, loaded, load_error);
        if !loaded {
            return settings_empty("Loading…", false).into_any_element();
        }
        if let Some(error) = load_error {
            return settings_empty(&error, true).into_any_element();
        }

        match active {
            SettingsSection::General => self.render_general(window, cx),
            SettingsSection::ModelsRouting => self.render_models_routing(window, cx),
            SettingsSection::Sandboxing => self.render_sandboxing(window, cx),
            SettingsSection::WorktreeBootstrap => self.render_worktree_bootstrap(window, cx),
            SettingsSection::WorkspaceAttachments => self.render_workspace_attachments(window, cx),
            SettingsSection::ContextPack => self.render_context_pack(window, cx),
            SettingsSection::ResourceGovernance => self.render_resource_governance(window, cx),
            SettingsSection::MobileAccess => self.render_mobile_access(window, cx),
            SettingsSection::ResourceUtilization => self.render_resource_utilization(window, cx),
            SettingsSection::Dictation => self.render_dictation(window, cx),
            SettingsSection::TitleGeneration => self.render_title_generation(window, cx),
            SettingsSection::Billing => self.render_billing(window, cx),
            SettingsSection::TeamEnterprise => self.render_team_enterprise(window, cx),
            SettingsSection::UsageAnalytics => self.render_usage_analytics(window, cx),
            SettingsSection::AgentHarnesses => self.render_agent_harnesses(window, cx),
        }
    }

    fn render_general(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let editor_options = vec![
            LabeledOption {
                value: "system".to_string(),
                label: "System default".to_string(),
            },
            LabeledOption {
                value: "vscode".to_string(),
                label: "Visual Studio Code".to_string(),
            },
            LabeledOption {
                value: "vscode_insiders".to_string(),
                label: "Visual Studio Code Insiders".to_string(),
            },
            LabeledOption {
                value: "cursor".to_string(),
                label: "Cursor".to_string(),
            },
            LabeledOption {
                value: "windsurf".to_string(),
                label: "Windsurf".to_string(),
            },
            LabeledOption {
                value: "antigravity".to_string(),
                label: "Google Antigravity".to_string(),
            },
            LabeledOption {
                value: "idea".to_string(),
                label: "IntelliJ IDEA".to_string(),
            },
            LabeledOption {
                value: "pycharm".to_string(),
                label: "PyCharm".to_string(),
            },
            LabeledOption {
                value: "xcode".to_string(),
                label: "Xcode".to_string(),
            },
            LabeledOption {
                value: "android_studio".to_string(),
                label: "Android Studio".to_string(),
            },
            LabeledOption {
                value: "custom".to_string(),
                label: "Custom command".to_string(),
            },
        ];
        let editor_select = Self::ensure_select_state(
            &mut self.editor_target_select,
            editor_options,
            Some(self.editor_target.clone()),
            super::super::state::SettingsSelectKind::EditorTarget,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let show_remote = [
            "vscode",
            "vscode_insiders",
            "cursor",
            "windsurf",
            "antigravity",
        ]
        .contains(&self.editor_target.as_str());

        let telemetry_disabled = self.settings.is_none();
        let telemetry_toggle = settings_toggle(self.telemetry_enabled, telemetry_disabled)
            .id("settings-telemetry-toggle")
            .when(!telemetry_disabled, |this| {
                this.on_click(cx.listener(|view, _, _window, cx| {
                    view.set_telemetry_enabled(!view.telemetry_enabled, cx);
                }))
            });

        let editor_control = Select::new(&editor_select)
            .appearance(true)
            .bg(white(0.06))
            .border_color(white(0.08))
            .rounded(px(8.0))
            .h(px(30.0))
            .px(px(10.0))
            .text_size(px(13.0))
            .disabled(!self.editor_loaded);

        let custom_input = Self::ensure_input_state(
            &mut self.editor_custom_input,
            "code --goto {path}:{line}:{col}",
            super::super::state::SettingsInputKind::EditorCustom,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let remote_input = Self::ensure_input_state(
            &mut self.editor_remote_input,
            "ssh-remote+my-host",
            super::super::state::SettingsInputKind::EditorRemote,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let mut rows = vec![
            settings_row(
                "Telemetry",
                Some("Share anonymous usage metrics (no code, prompts, or file paths)."),
                telemetry_toggle,
                true,
            )
            .into_any_element(),
            settings_row(
                "Default IDE",
                Some("Used for open-in-editor links."),
                div().id("settings-editor-select").child(editor_control),
                false,
            )
            .into_any_element(),
        ];

        if self.editor_target == "custom" {
            rows.push(
                settings_row(
                    "Custom IDE command",
                    Some("Command to run when opening files."),
                    Input::new(&custom_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .w(relative(0.48))
                        .max_w(px(460.0))
                        .disabled(!self.editor_loaded),
                    false,
                )
                .into_any_element(),
            );
        }

        if show_remote {
            rows.push(
                settings_row(
                    "VS Code Remote Authority",
                    Some("Optional: ssh-remote+my-host for remote worktrees."),
                    Input::new(&remote_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .w(relative(0.48))
                        .max_w(px(460.0))
                        .disabled(!self.editor_loaded),
                    false,
                )
                .into_any_element(),
            );
        }

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, settings_rows(rows)));
        if let Some(error) = self.editor_error.clone() {
            content = content.child(settings_banner(&error, true));
        }
        content.into_any_element()
    }

    fn render_models_routing(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let provider_count = self.providers.len();
        let catalog_count = self
            .provider_options
            .values()
            .filter(|opts| opts.models.is_some())
            .count();
        let provider_label = if provider_count == 0 {
            "No providers".to_string()
        } else {
            format!("{provider_count} detected")
        };
        let catalog_label = if catalog_count == 0 {
            "No catalogs yet".to_string()
        } else {
            format!("{catalog_count} catalogs")
        };
        let can_jump = self.shell_handle.is_some();
        let jump_button = settings_button_compact(
            "Open Workbench",
            ButtonVariant::Secondary,
            !can_jump,
            None,
        )
        .id("settings-models-open-workbench")
        .when(can_jump, |this| {
            this.on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                if let Some(handle) = view.shell_handle.clone() {
                    handle.update(cx, |shell, cx| {
                        shell.set_route(ShellRoute::Workbench, cx);
                    });
                }
            }))
        });

        let rows = vec![
            settings_row(
                "Routing policy",
                Some("Model selection is configured per task in the composer."),
                settings_pill("Per-task", PillVariant::Default, false),
                true,
            )
            .into_any_element(),
            settings_row(
                "Providers",
                Some("Providers available for routing."),
                settings_pill(&provider_label, PillVariant::Default, false),
                false,
            )
            .into_any_element(),
            settings_row(
                "Model catalogs",
                Some("Latest model lists fetched from providers."),
                settings_pill(&catalog_label, PillVariant::Default, false),
                false,
            )
            .into_any_element(),
            settings_row(
                "Jump to Workbench",
                Some("Use the composer to pick models and routing options."),
                jump_button,
                false,
            )
            .into_any_element(),
        ];

        div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, settings_rows(rows)))
            .into_any_element()
    }

    fn render_sandboxing(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let rows = vec![settings_row(
            "Provider control",
            Some("Default is full capability. Harness-native controls are not yet available here."),
            settings_pill("Full capability", PillVariant::Default, false),
            true,
        )
        .into_any_element()];

        div()
            .grid()
            .gap(px(12.0))
            .child(settings_card(colors, None, settings_rows(rows)))
            .child(settings_banner(
                "Sandboxing settings are read-only in native for now.",
                false,
            ))
            .into_any_element()
    }

    fn render_worktree_bootstrap(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let workspace_options = self
            .workspaces
            .iter()
            .map(|ws| LabeledOption {
                value: ws.id.0.to_string(),
                label: ws.name.clone(),
            })
            .collect::<Vec<_>>();

        let workspace_select = Self::ensure_select_state(
            &mut self.workspace_select,
            workspace_options,
            self.selected_workspace.map(|id| id.0.to_string()),
            super::super::state::SettingsSelectKind::Workspace,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let any_workspace = !self.workspaces.is_empty();
        let selected_workspace = self
            .selected_workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id));

        let config_path = selected_workspace
            .map(|ws| format!("{}/.ctx/config.toml", ws.root_path))
            .unwrap_or_else(|| ".ctx/config.toml".to_string());

        let example = "[worktree.bootstrap]\nsetup_worktree = [\"pnpm install\", \"cargo fetch --locked\"]\nsetup_worktree_unix = \"scripts/worktree_bootstrap_unix.sh\"\nsetup_worktree_windows = \"scripts/worktree_bootstrap_windows.ps1\"\ntimeout_sec = 60\nwait_for_completion = false\n";

        let rows = vec![
            settings_row(
                "Workspace",
                Some("Choose the repo to edit."),
                div()
                    .id("settings-worktree-workspace-select")
                    .child(
                        Select::new(&workspace_select)
                            .appearance(true)
                            .bg(white(0.06))
                            .border_color(white(0.08))
                            .rounded(px(8.0))
                            .h(px(30.0))
                            .px(px(10.0))
                            .text_size(px(13.0))
                            .disabled(!any_workspace),
                    ),
                true,
            )
            .into_any_element(),
            settings_row(
                "Config file",
                Some("Repo-scoped worktree bootstrap configuration."),
                settings_pill(&config_path, PillVariant::Default, true),
                false,
            )
            .into_any_element(),
            settings_row(
                "Example",
                Some("Add this section to enable bootstrap."),
                settings_code_block(example),
                false,
            )
            .into_any_element(),
        ];

        settings_card(colors, Some("Worktree Bootstrap"), settings_rows(rows)).into_any_element()
    }

    fn render_workspace_attachments(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let workspace_options = self
            .workspaces
            .iter()
            .map(|ws| LabeledOption {
                value: ws.id.0.to_string(),
                label: ws.name.clone(),
            })
            .collect::<Vec<_>>();

        let workspace_select = Self::ensure_select_state(
            &mut self.workspace_select,
            workspace_options,
            self.selected_workspace.map(|id| id.0.to_string()),
            super::super::state::SettingsSelectKind::Workspace,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let any_workspace = !self.workspaces.is_empty();
        let selected_workspace = self
            .selected_workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id));

        let config_path = selected_workspace
            .map(|ws| format!("{}/.ctx/attachments.toml", ws.root_path))
            .unwrap_or_else(|| ".ctx/attachments.toml".to_string());

        let attachment_source = Self::ensure_input_state(
            &mut self.attachment_source_input,
            "git@github.com:org/repo.git",
            super::super::state::SettingsInputKind::AttachmentSource,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let attachment_name_placeholder = if self.attachment_source.trim().is_empty() {
            "reference".to_string()
        } else {
            guess_attachment_name(&self.attachment_source)
        };
        let attachment_name = Self::ensure_input_state(
            &mut self.attachment_name_input,
            &attachment_name_placeholder,
            super::super::state::SettingsInputKind::AttachmentName,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let attachment_revision = Self::ensure_input_state(
            &mut self.attachment_revision_input,
            "main or tag",
            super::super::state::SettingsInputKind::AttachmentRevision,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let docs_source = Self::ensure_input_state(
            &mut self.docs_source_input,
            "https://docs.example.com/",
            super::super::state::SettingsInputKind::DocsSource,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let docs_name_placeholder = if self.docs_attachment_source.trim().is_empty() {
            "docs".to_string()
        } else {
            guess_attachment_name(&self.docs_attachment_source)
        };
        let docs_name = Self::ensure_input_state(
            &mut self.docs_name_input,
            &docs_name_placeholder,
            super::super::state::SettingsInputKind::DocsName,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let can_add_repo = self.selected_workspace.is_some() && !self.attachment_source.trim().is_empty();
        let can_add_docs = self.selected_workspace.is_some() && !self.docs_attachment_source.trim().is_empty();
        let add_repo_disabled = !can_add_repo || self.attachment_busy;
        let sync_repo_disabled = self.selected_workspace.is_none() || self.attachment_sync_busy;
        let add_docs_disabled = !can_add_docs || self.docs_attachment_busy;

        let workspace_rows = vec![
            settings_row(
                "Workspace",
                Some("Choose the repo to configure."),
                div()
                    .id("settings-attachments-workspace-select")
                    .child(
                        Select::new(&workspace_select)
                            .appearance(true)
                            .bg(white(0.06))
                            .border_color(white(0.08))
                            .rounded(px(8.0))
                            .h(px(30.0))
                            .px(px(10.0))
                            .text_size(px(13.0))
                            .disabled(!any_workspace),
                    ),
                true,
            )
            .into_any_element(),
            settings_row(
                "Config file",
                Some("Repo-scoped attachments configuration."),
                settings_pill(&config_path, PillVariant::Default, true),
                false,
            )
            .into_any_element(),
            settings_row(
                "Mount paths",
                Some("Reference repos are mounted inside each worktree."),
                settings_pill(".ctx/attachments/refs/<name>", PillVariant::Default, true),
                false,
            )
            .into_any_element(),
        ];

        let reference_form = div()
            .px(px(16.0))
            .py(px(14.0))
            .grid()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .flex_wrap()
                    .child(
                        div()
                            .grid()
                            .gap(px(6.0))
                            .flex_1()
                            .flex_basis(relative(0.44))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white(0.7))
                                    .child("Repository URL".to_string()),
                            )
                            .child(
                                Input::new(&attachment_source)
                                    .appearance(true)
                                    .bg(white(0.06))
                                    .border_color(white(0.08))
                                    .rounded(px(8.0))
                                    .h(px(30.0))
                                    .px(px(10.0))
                                    .w_full()
                                    .text_size(px(13.0)),
                            ),
                    )
                    .child(
                        div()
                            .grid()
                            .gap(px(6.0))
                            .flex_1()
                            .flex_basis(relative(0.28))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white(0.7))
                                    .child("Display name".to_string()),
                            )
                            .child(
                                Input::new(&attachment_name)
                                    .appearance(true)
                                    .bg(white(0.06))
                                    .border_color(white(0.08))
                                    .rounded(px(8.0))
                                    .h(px(30.0))
                                    .px(px(10.0))
                                    .w_full()
                                    .text_size(px(13.0)),
                            ),
                    )
                    .child(
                        div()
                            .grid()
                            .gap(px(6.0))
                            .flex_1()
                            .flex_basis(relative(0.28))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white(0.7))
                                    .child("Revision (optional)".to_string()),
                            )
                            .child(
                                Input::new(&attachment_revision)
                                    .appearance(true)
                                    .bg(white(0.06))
                                    .border_color(white(0.08))
                                    .rounded(px(8.0))
                                    .h(px(30.0))
                                    .px(px(10.0))
                                    .w_full()
                                    .text_size(px(13.0)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .flex_wrap()
                    .child(
                        settings_button(
                            if self.attachment_busy { "Adding…" } else { "Add repo" },
                            ButtonVariant::Primary,
                            add_repo_disabled,
                            None,
                        )
                        .id("settings-attachments-add-repo")
                        .when(!add_repo_disabled, |this| {
                            this.on_click(cx.listener(|view, _, _window, cx| {
                                view.handle_add_attachment(cx);
                            }))
                        }),
                    )
                    .child(
                        settings_button(
                            if self.attachment_sync_busy {
                                "Syncing…"
                            } else {
                                "Sync now"
                            },
                            ButtonVariant::Secondary,
                            sync_repo_disabled,
                            None,
                        )
                        .id("settings-attachments-sync")
                        .when(!sync_repo_disabled, |this| {
                            this.on_click(cx.listener(|view, _, _window, cx| {
                                view.sync_workspace_attachments(true, cx);
                            }))
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(white(0.45))
                    .child("Use SSH URLs for private repos. The daemon must have access to your SSH keys.".to_string()),
            );

        let attachment_table = div()
            .px(px(16.0))
            .py(px(14.0))
            .grid()
            .gap(px(12.0))
            .child(if self.attachments_loading {
                settings_empty_compact("Loading attachments…").into_any_element()
            } else if self.attachments.is_empty() {
                settings_empty_compact("No workspace attachments yet.").into_any_element()
            } else {
                let headers = div()
                    .flex()
                    .gap(px(12.0))
                    .text_size(px(11.0))
                    .text_color(white(0.45))
                    .border_b_1()
                    .border_color(white(0.08))
                    .pb(px(6.0))
                    .child(div().w(relative(0.256)).child("ATTACHMENT".to_string()))
                    .child(div().w(relative(0.372)).child("SOURCE".to_string()))
                    .child(div().w(relative(0.209)).child("MOUNT".to_string()))
                    .child(div().w(relative(0.163)).child("UPDATED".to_string()))
                    .child(div().child("".to_string()));

                let row_count = self.attachments.len();
                let rows = self.attachments.iter().enumerate().map(|(idx, attachment)| {
                    let updated_label = format_age_iso(&attachment.updated_at);
                    let updated_display = if updated_label == "—" {
                        "—".to_string()
                    } else {
                        format!("{} ago", updated_label)
                    };
                    let delete_busy = self
                        .attachment_delete_busy
                        .get(&attachment.id.0.to_string())
                        .copied()
                        .unwrap_or(false);

                    let mut row = div()
                        .flex()
                        .gap(px(12.0))
                        .py(px(8.0));
                    if idx + 1 != row_count {
                        row = row.border_b_1().border_color(white(0.06));
                    }

                    row.child(
                            div()
                                .w(relative(0.256))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(white(0.86))
                                        .child(attachment.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(white(0.45))
                                        .child(match attachment.kind {
                                            WorkspaceAttachmentKind::ReferenceRepo => "Reference repo",
                                            WorkspaceAttachmentKind::DocMirror => "Docs mirror",
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .w(relative(0.372))
                                .font_family("ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace")
                                .text_size(px(12.0))
                                .text_color(white(0.75))
                                .child(truncate_text(&attachment.source, 64)),
                        )
                        .child(
                            div()
                                .w(relative(0.209))
                                .font_family("ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace")
                                .text_size(px(12.0))
                                .text_color(white(0.75))
                                .child(truncate_text(&attachment.mount_relpath, 32)),
                        )
                        .child(
                            div()
                                .w(relative(0.163))
                                .text_size(px(11.0))
                                .text_color(white(0.45))
                                .child(updated_display),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .child(
                                    settings_button_compact(
                                        if delete_busy { "Removing…" } else { "Remove" },
                                        ButtonVariant::Secondary,
                                        delete_busy,
                                        None,
                                    )
                                    .id(format!("settings-attachment-remove-{}", attachment.id.0))
                                    .when(!delete_busy, |this| {
                                        this.on_click(cx.listener({
                                            let attachment = attachment.clone();
                                            move |view, _, _window, cx| {
                                                let req = ctx_client::DeleteWorkspaceAttachmentRequest {
                                                    kind: attachment.kind.clone(),
                                                    name: attachment.name.clone(),
                                                };
                                                view.remove_workspace_attachment(attachment.id.0.to_string(), req, cx);
                                            }
                                        }))
                                    }),
                                ),
                        )
                });

                div()
                    .grid()
                    .gap(px(6.0))
                    .child(headers)
                    .children(rows)
                    .into_any_element()
            });

        let docs_form = div()
            .px(px(16.0))
            .py(px(14.0))
            .grid()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .flex_wrap()
                    .child(
                        div()
                            .grid()
                            .gap(px(6.0))
                            .flex_1()
                            .flex_basis(relative(0.44))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white(0.7))
                                    .child("Docs URL".to_string()),
                            )
                            .child(
                                Input::new(&docs_source)
                                    .appearance(true)
                                    .bg(white(0.06))
                                    .border_color(white(0.08))
                                    .rounded(px(8.0))
                                    .h(px(30.0))
                                    .px(px(10.0))
                                    .w_full()
                                    .text_size(px(13.0)),
                            ),
                    )
                    .child(
                        div()
                            .grid()
                            .gap(px(6.0))
                            .flex_1()
                            .flex_basis(relative(0.28))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white(0.7))
                                    .child("Display name".to_string()),
                            )
                            .child(
                                Input::new(&docs_name)
                                    .appearance(true)
                                    .bg(white(0.06))
                                    .border_color(white(0.08))
                                    .rounded(px(8.0))
                                    .h(px(30.0))
                                    .px(px(10.0))
                                    .w_full()
                                    .text_size(px(13.0)),
                            ),
                    )
                    .child(div().flex_1().flex_basis(relative(0.28))),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .flex_wrap()
                    .child(
                        settings_button(
                            if self.docs_attachment_busy { "Adding…" } else { "Add docs" },
                            ButtonVariant::Primary,
                            add_docs_disabled,
                            None,
                        )
                        .id("settings-attachments-add-docs")
                        .when(!add_docs_disabled, |this| {
                            this.on_click(cx.listener(|view, _, _window, cx| {
                                view.handle_add_docs_attachment(cx);
                            }))
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(white(0.45))
                    .child("Paste any docs page URL. The daemon will infer the crawl entrypoint and mirror it.".to_string()),
            );

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, Some("Workspace Attachments"), settings_rows(workspace_rows)))
            .child(settings_card(colors, Some("Reference Repos"), div().child(reference_form).child(attachment_table)))
            .child(settings_card(colors, Some("Docs"), docs_form));

        if let Some(error) = self.attachments_error.clone() {
            content = content.child(settings_banner(&error, true));
        }

        content.into_any_element()
    }

    fn render_context_pack(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let selected_workspace = self
            .selected_workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id));
        let pack_path = selected_workspace
            .map(|ws| format!("{}/.ctx/ctx-pack", ws.root_path))
            .unwrap_or_else(|| ".ctx/ctx-pack".to_string());
        let mut body = div()
            .grid()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(white(0.7))
                    .child("ctx pack stores specs, skills, and prompts for this workspace."),
            )
            .child(settings_code_block(&pack_path));
        if selected_workspace.is_none() {
            body = body.child(settings_empty_compact("Select a workspace to see the exact path."));
        }

        div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, body))
            .into_any_element()
    }

    fn render_team_enterprise(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let body = div()
            .grid()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(white(0.7))
                    .child("Team and Enterprise features are managed through ctx Cloud."),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(white(0.45))
                    .child("Contact sales to enable SSO, org policies, and managed deployments."),
            )
            .child(settings_button(
                "Contact sales",
                ButtonVariant::Secondary,
                true,
                None,
            ));

        div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, body))
            .into_any_element()
    }

    fn render_usage_analytics(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let status_label = if self.telemetry_enabled {
            "Enabled"
        } else {
            "Disabled"
        };
        let status_variant = if self.telemetry_enabled {
            PillVariant::Ok
        } else {
            PillVariant::Warn
        };
        let endpoint_label = if self.telemetry_endpoint.trim().is_empty() {
            "Default endpoint".to_string()
        } else {
            self.telemetry_endpoint.clone()
        };

        let rows = vec![
            settings_row(
                "Telemetry status",
                Some("Matches the General telemetry setting."),
                settings_pill(status_label, status_variant, false),
                true,
            )
            .into_any_element(),
            settings_row(
                "Endpoint",
                Some("Where anonymous usage data is sent."),
                settings_code_block(&endpoint_label),
                false,
            )
            .into_any_element(),
        ];

        div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, settings_rows(rows)))
            .into_any_element()
    }

    fn render_resource_governance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let mode_options = vec![
            LabeledOption {
                value: "auto".to_string(),
                label: "Auto (recommended)".to_string(),
            },
            LabeledOption {
                value: "custom".to_string(),
                label: "Custom".to_string(),
            },
        ];
        let mode_select = Self::ensure_select_state(
            &mut self.resource_mode_select,
            mode_options,
            Some(match self.resource_mode {
                ResourceGovernanceMode::Custom => "custom".to_string(),
                _ => "auto".to_string(),
            }),
            super::super::state::SettingsSelectKind::ResourceMode,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let cpu_input = Self::ensure_input_state(
            &mut self.resource_cpu_input,
            "300",
            super::super::state::SettingsInputKind::ResourceCpu,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let mem_high_input = Self::ensure_input_state(
            &mut self.resource_memory_high_input,
            "48",
            super::super::state::SettingsInputKind::ResourceMemHigh,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let mem_max_input = Self::ensure_input_state(
            &mut self.resource_memory_max_input,
            "54",
            super::super::state::SettingsInputKind::ResourceMemMax,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let enable_disabled = !self.settings.is_some();
        let enable_toggle = settings_toggle(self.resource_enabled, enable_disabled)
            .id("settings-resource-enable")
            .when(!enable_disabled, |this| {
                this.on_click(cx.listener(|view, _, _window, cx| {
                    view.set_resource_enabled(!view.resource_enabled, cx);
                }))
            });

        let mode_control = Select::new(&mode_select)
            .appearance(true)
            .bg(white(0.06))
            .border_color(white(0.08))
            .rounded(px(8.0))
            .h(px(30.0))
            .px(px(10.0))
            .text_size(px(13.0))
            .disabled(!self.resource_enabled);

        let mut rows = vec![
            settings_row(
                "Enable resource limits",
                Some("Keep the host responsive by throttling agent workloads."),
                enable_toggle,
                true,
            )
            .into_any_element(),
            settings_row(
                "Mode",
                Some("Auto picks safe limits for this machine."),
                div().id("settings-resource-mode-select").child(mode_control),
                false,
            )
            .into_any_element(),
        ];

        if matches!(self.resource_mode, ResourceGovernanceMode::Custom) {
            rows.push(
                settings_row(
                    "CPU quota (%)",
                    Some("100% = 1 core. Leave empty to use auto."),
                    Input::new(&cpu_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .disabled(!self.resource_enabled),
                    false,
                )
                .into_any_element(),
            );
            rows.push(
                settings_row(
                    "Memory high (GiB)",
                    Some("Soft limit for reclaim pressure."),
                    Input::new(&mem_high_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .disabled(!self.resource_enabled),
                    false,
                )
                .into_any_element(),
            );
            rows.push(
                settings_row(
                    "Memory max (GiB)",
                    Some("Hard limit; processes are killed when exceeded."),
                    Input::new(&mem_max_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .disabled(!self.resource_enabled),
                    false,
                )
                .into_any_element(),
            );
        }

        let effective_cpu = self.resource_effective.as_ref().map(|v| v.cpu_quota_pct);
        let effective_high = self.resource_effective.as_ref().map(|v| v.memory_high_mb);
        let effective_max = self.resource_effective.as_ref().map(|v| v.memory_max_mb);

        let status_state = self
            .resource_status
            .as_ref()
            .map(|status| status.state.clone())
            .unwrap_or(if self.resource_enabled {
                ResourceGovernanceStatusState::Pending
            } else {
                ResourceGovernanceStatusState::Disabled
            });

        let status_label = match status_state {
            ResourceGovernanceStatusState::Disabled => "Disabled",
            ResourceGovernanceStatusState::Applied => "Applied",
            ResourceGovernanceStatusState::Unsupported => "Unsupported",
            ResourceGovernanceStatusState::Error => "Error",
            ResourceGovernanceStatusState::Pending => "Pending",
        };
        let status_message = self
            .resource_status
            .as_ref()
            .and_then(|status| status.message.clone())
            .unwrap_or_else(|| "Apply changes to update live limits.".to_string());
        let show_apply = status_state == ResourceGovernanceStatusState::Pending
            && self
                .resource_status
                .as_ref()
                .map(|status| status.can_apply_now)
                .unwrap_or(false);
        let show_restart = self
            .resource_status
            .as_ref()
            .map(|status| status.requires_restart)
            .unwrap_or(false);

        let can_save = self.resource_governance_can_save();

        let status_control = div()
            .flex()
            .gap(px(8.0))
            .child(settings_pill(status_label, PillVariant::Default, false))
            .when(show_restart, |this| {
                this.child(settings_pill("Restart required", PillVariant::Warn, false))
            })
            .when(show_apply, |this| {
                let apply_disabled = self.saving || !can_save;
                this.child(
                    settings_button(
                        "Apply now",
                        ButtonVariant::Secondary,
                        apply_disabled,
                        None,
                    )
                    .id("settings-resource-apply")
                    .when(!apply_disabled, |this| {
                        this.on_click(cx.listener(|view, _, _window, cx| {
                            view.apply_resource_governance_now(cx);
                        }))
                    }),
                )
            });

        let effective_rows = vec![
            settings_row(
                "CPU quota",
                Some("Applied to the daemon and its child processes."),
                settings_pill(
                    &effective_cpu
                        .map(|v| format!("{}%", v))
                        .unwrap_or_else(|| "—".to_string()),
                    PillVariant::Default,
                    true,
                ),
                true,
            )
            .into_any_element(),
            settings_row(
                "Memory high / max",
                Some("High is the soft threshold; max is the hard cap."),
                settings_pill(
                    &format!(
                        "{} / {}",
                        effective_high
                            .map(|v| format!("{} GiB", format_gib(Some(v))))
                            .unwrap_or_else(|| "—".to_string()),
                        effective_max
                            .map(|v| format!("{} GiB", format_gib(Some(v))))
                            .unwrap_or_else(|| "—".to_string()),
                    ),
                    PillVariant::Default,
                    true,
                ),
                false,
            )
            .into_any_element(),
            settings_row(
                "Apply status",
                Some(status_message.as_str()),
                status_control,
                false,
            )
            .into_any_element(),
        ];

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, Some("Resource Governance"), settings_rows(rows)))
            .child(settings_card(
                colors,
                Some("Effective limits"),
                settings_rows(effective_rows),
            ));

        if !can_save && matches!(self.resource_mode, ResourceGovernanceMode::Custom) {
            content = content.child(settings_banner(
                "Memory high must be less than or equal to memory max.",
                true,
            ));
        }

        content.into_any_element()
    }

    fn render_mobile_access(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let pro_enabled = false;
        let status = self.mobile_status.clone();
        let status_label = if self.mobile_status_loading {
            "Loading".to_string()
        } else if status.as_ref().map(|s| s.enabled).unwrap_or(false) {
            "Enabled".to_string()
        } else {
            "Disabled".to_string()
        };

        let tunnel_state = status
            .as_ref()
            .map(|s| match s.tunnel_state {
                MobileTunnelState::Idle => "idle",
                MobileTunnelState::Running => "running",
                MobileTunnelState::Error => "error",
            })
            .unwrap_or("idle");

        let tunnel_desc = status
            .as_ref()
            .and_then(|s| s.last_error.clone())
            .unwrap_or_else(|| "Router tunnel lifecycle state.".to_string());

        let has_billing_user = false;
        let enable_disabled = !has_billing_user || !pro_enabled || self.mobile_enable_busy;
        let disable_disabled = !has_billing_user || self.mobile_enable_busy;

        let actions = div()
            .flex()
            .gap(px(8.0))
            .flex_wrap()
            .justify_end()
            .child(
                settings_button(
                    if status.as_ref().map(|s| s.enabled).unwrap_or(false) {
                        "Show QR"
                    } else {
                        "Enable"
                    },
                    ButtonVariant::Secondary,
                    enable_disabled,
                    None,
                )
                .id("settings-mobile-enable")
                .when(!enable_disabled, |this| {
                    this.on_click(cx.listener(|view, _, _window, cx| {
                        view.enable_mobile_access(None, cx);
                    }))
                }),
            )
            .when(status.as_ref().map(|s| s.enabled).unwrap_or(false), |this| {
                this.child(
                    settings_button("Disable", ButtonVariant::Primary, disable_disabled, None)
                        .id("settings-mobile-disable")
                        .when(!disable_disabled, |this| {
                            this.on_click(cx.listener(|view, _, _window, cx| {
                                view.disable_mobile_access(None, cx);
                            }))
                        }),
                )
            });

        let mut rows = vec![
            settings_row(
                "Entitlement",
                Some(if pro_enabled { "Pro enabled" } else { "Pro required" }),
                settings_pill(if pro_enabled { "Enabled" } else { "Disabled" }, PillVariant::Default, false),
                true,
            )
            .into_any_element(),
            settings_row(
                "Status",
                Some("Mobile access tunnel status on this daemon."),
                settings_pill(&status_label, PillVariant::Default, false),
                false,
            )
            .into_any_element(),
            settings_row(
                "Tunnel state",
                Some(tunnel_desc.as_str()),
                settings_pill(tunnel_state, PillVariant::Default, false),
                false,
            )
            .into_any_element(),
        ];

        if let Some(url) = status.as_ref().and_then(|s| s.public_base_url.clone()) {
            rows.push(
                settings_row(
                    "Public URL",
                    None,
                    settings_pill(&url, PillVariant::Default, true),
                    false,
                )
                .into_any_element(),
            );
        }
        if let Some(id) = status.as_ref().and_then(|s| s.tunnel_id.clone()) {
            rows.push(
                settings_row(
                    "Tunnel ID",
                    None,
                    settings_pill(&id, PillVariant::Default, true),
                    false,
                )
                .into_any_element(),
            );
        }

        rows.push(
            settings_row(
                "Actions",
                Some("Sign in to enable or revoke mobile access."),
                actions,
                false,
            )
            .into_any_element(),
        );

        if !pro_enabled {
            rows.push(
                settings_row(
                    "Upgrade",
                    Some("Remote mobile access is a Pro feature."),
                    div()
                        .text_color(colors.accent)
                        .cursor_pointer()
                        .child("Go to billing".to_string())
                        .id("settings-mobile-go-billing")
                        .on_click(cx.listener(|view, _, _window, cx| {
                            view.set_active_section(SettingsSection::Billing, cx);
                        })),
                    false,
                )
                .into_any_element(),
            );
        }

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(
                colors,
                Some("Remote Mobile Access"),
                settings_rows(rows),
            ));

        if self.mobile_status_loading {
            content = content.child(settings_banner("Loading mobile access status…", false));
        }
        if let Some(err) = self.mobile_status_error.clone() {
            content = content.child(settings_banner(&err, true));
        }
        if let Some(err) = self.mobile_enable_error.clone() {
            content = content.child(settings_banner(&err, true));
        }

        if let Some(qr) = self.mobile_qr.clone() {
            let expires_at = qr.pairing_expires_at;
            content = content.child(settings_card(
                colors,
                Some("Pair a mobile device"),
                div()
                    .px(px(16.0))
                    .py(px(14.0))
                    .flex()
                    .gap(px(24.0))
                    .items_center()
                    .flex_wrap()
                    .child(render_qr_from_value(&qr.qr_payload))
                    .child(
                        div()
                            .min_w(px(240.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white(0.84))
                                    .mb(px(6.0))
                                    .child("Scan with ctx mobile".to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(white(0.42))
                                    .child(format!(
                                        "This QR code pairs a device using end-to-end encryption. It expires at {}.",
                                        expires_at
                                    )),
                            ),
                    ),
            ));
        }

        content.into_any_element()
    }

    fn render_resource_utilization(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        if self.selected_workspace.is_none() {
            return settings_empty("No workspace selected.", false).into_any_element();
        }

        let snapshot = self.resource_snapshot.clone();
        let system = snapshot.as_ref().map(|s| &s.system);
        let disk = snapshot
            .as_ref()
            .and_then(|s| s.workspace.disk.as_ref());
        let workspace_name = self
            .selected_workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id))
            .map(|ws| ws.name.clone())
            .unwrap_or_else(|| "Workspace".to_string());
        let disk_label = disk
            .map(|d| format!("{} · {}", d.mount_point, d.file_system))
            .unwrap_or_else(|| "Workspace volume".to_string());

        let memory_pct = system
            .and_then(|sys| {
                if sys.memory_total_bytes > 0 {
                    Some((sys.memory_used_bytes as f32 / sys.memory_total_bytes as f32) * 100.0)
                } else {
                    None
                }
            });
        let swap_pct = system
            .and_then(|sys| {
                if sys.swap_total_bytes > 0 {
                    Some((sys.swap_used_bytes as f32 / sys.swap_total_bytes as f32) * 100.0)
                } else {
                    None
                }
            });
        let disk_pct = disk.and_then(|disk| {
            if disk.total_bytes > 0 {
                Some(
                    ((disk.total_bytes - disk.available_bytes) as f32 / disk.total_bytes as f32)
                        * 100.0,
                )
            } else {
                None
            }
        });

        let overview_updated = snapshot
            .as_ref()
            .map(|snap| format!("Updated {} ago", format_age_ms(Some(snap.cache_age_ms))))
            .unwrap_or_else(|| "Awaiting resource data…".to_string());
        let disk_updated = snapshot
            .as_ref()
            .map(|snap| format!("Disk scan {} ago", format_age_ms(Some(snap.workspace.size_cache_age_ms))))
            .unwrap_or_else(|| "Disk scan pending…".to_string());

        let mut process_rows = vec![];
        if let Some(snapshot) = snapshot.as_ref() {
            if let Some(daemon) = snapshot.processes.daemon.clone() {
                process_rows.push(daemon);
            }
            let mut providers = snapshot.processes.providers.clone();
            providers.sort_by(|a, b| a.label.cmp(&b.label));
            process_rows.extend(providers);
        }

        let worktree_rows = snapshot
            .as_ref()
            .map(|snap| snap.workspace.worktrees.clone())
            .unwrap_or_default();

        let overview_card = settings_card(
            colors,
            Some("Overview"),
            div()
                .px(px(16.0))
                .py(px(14.0))
                .grid()
                .gap(px(12.0))
                .child(
                    div()
                        .grid()
                        .gap(px(12.0))
                        .grid_cols(4)
                        .child(settings_metric(
                            "CPU",
                            &format_pct(system.map(|s| s.cpu_pct)),
                            Some("System CPU usage"),
                            system.map(|s| s.cpu_pct),
                        ))
                        .child(settings_metric(
                            "Memory",
                            &system
                                .map(|s| {
                                    format!(
                                        "{} / {}",
                                        format_bytes(Some(s.memory_used_bytes)),
                                        format_bytes(Some(s.memory_total_bytes))
                                    )
                                })
                                .unwrap_or_else(|| "—".to_string()),
                            Some("Physical memory"),
                            memory_pct,
                        ))
                        .child(settings_metric(
                            "Swap",
                            &system
                                .map(|s| {
                                    format!(
                                        "{} / {}",
                                        format_bytes(Some(s.swap_used_bytes)),
                                        format_bytes(Some(s.swap_total_bytes))
                                    )
                                })
                                .unwrap_or_else(|| "—".to_string()),
                            Some("Swap usage"),
                            swap_pct,
                        ))
                        .child(settings_metric(
                            "Disk",
                            &disk
                                .map(|d| {
                                    format!(
                                        "{} free / {}",
                                        format_bytes(Some(d.available_bytes)),
                                        format_bytes(Some(d.total_bytes))
                                    )
                                })
                                .unwrap_or_else(|| "—".to_string()),
                            Some(disk_label.as_str()),
                            disk_pct,
                        )),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(white(0.4))
                        .child(if self.resource_loading {
                            "Refreshing…".to_string()
                        } else {
                            overview_updated
                        }),
                ),
        );

        let process_card = settings_card(
            colors,
            Some("Processes"),
            div()
                .px(px(16.0))
                .py(px(14.0))
                .grid()
                .gap(px(12.0))
                .child(if process_rows.is_empty() {
                    settings_empty("No process metrics yet.", false).into_any_element()
                } else {
                    let head = div()
                        .flex()
                        .gap(px(12.0))
                        .text_size(px(11.0))
                        .text_color(white(0.45))
                        .border_b_1()
                        .border_color(white(0.08))
                        .pb(px(6.0))
                        .child(div().w(relative(0.476)).child("PROCESS".to_string()))
                        .child(div().w(relative(0.167)).child("CPU".to_string()))
                        .child(div().w(relative(0.214)).child("MEMORY".to_string()))
                        .child(div().w(relative(0.143)).child("PID".to_string()));

                    let row_count = process_rows.len();
                    let rows = process_rows.iter().enumerate().flat_map(|(index, process)| {
                        let expanded = *self.expanded_process_pids.get(&process.pid).unwrap_or(&false);
                        let has_children = process.child_count > 0 || !process.children.is_empty();
                        let child_count_label = format!(
                            "{} child process{}",
                            process.child_count,
                            if process.child_count == 1 { "" } else { "es" }
                        );

                        let show_border = index + 1 != row_count || expanded;
                        let mut row = div()
                            .flex()
                            .gap(px(12.0))
                            .py(px(8.0));
                        if show_border {
                            row = row.border_b_1().border_color(white(0.06));
                        }

                        let mut elements = vec![
                            row.child(
                                    div()
                                        .w(relative(0.476))
                                        .flex()
                                        .gap(px(10.0))
                                        .items_start()
                                        .child(
                                            div()
                                                .w(px(24.0))
                                                .h(px(24.0))
                                                .rounded(px(8.0))
                                                .border_1()
                                                .border_color(white(0.12))
                                                .bg(white(0.06))
                                                .text_color(white(0.7))
                                                .text_size(px(12.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(if has_children {
                                                    if expanded { "▾" } else { "▸" }
                                                } else {
                                                    "·"
                                                })
                                                .id(ElementId::named_usize(
                                                    "settings-process-expand",
                                                    process.pid as usize,
                                                ))
                                                .when(has_children, |this| {
                                                    this.cursor_pointer().on_click(cx.listener({
                                                        let pid = process.pid;
                                                        move |view, _, _window, cx| {
                                                            view.toggle_process_expanded(pid, cx);
                                                        }
                                                    }))
                                                })
                                                .when(!has_children, |this| this.opacity(0.4)),
                                        )
                                        .child(
                                            div()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(white(0.86))
                                                        .child(process.label.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .text_color(white(0.45))
                                                        .child(if process.children_truncated {
                                                            format!("{} (truncated)", child_count_label)
                                                        } else {
                                                            child_count_label
                                                        }),
                                                ),
                                        ),
                                )
                                .child(div().w(relative(0.167)).child(format_pct(Some(process.cpu_pct))))
                                .child(div().w(relative(0.214)).child(format_bytes(Some(process.memory_bytes))))
                                .child(
                                    div()
                                        .w(relative(0.143))
                                        .font_family("ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace")
                                        .text_size(px(12.0))
                                        .text_color(white(0.75))
                                        .child(process.pid.to_string()),
                                )
                                .into_any_element(),
                        ];

                        if expanded {
                            let children_section = div()
                                .ml(px(34.0))
                                .pl(px(12.0))
                                .py(px(6.0))
                                .border_l_1()
                                .border_color(white(0.08))
                                .child(if process.child_count == 0 {
                                    settings_empty_compact("No child processes.").into_any_element()
                                } else {
                                    let meta = if process.children_truncated {
                                        format!(
                                            "Showing {} of {} descendants (sorted by memory)",
                                            process.children.len(),
                                            process.child_count
                                        )
                                    } else {
                                        format!("{} descendants", process.children.len())
                                    };

                                    div()
                                        .grid()
                                        .gap(px(8.0))
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(white(0.45))
                                                .child(meta),
                                        )
                                        .child(
                                            div()
                                                .grid()
                                                .gap(px(6.0))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .gap(px(12.0))
                                                        .text_size(px(11.0))
                                                        .text_color(white(0.45))
                                                        .border_b_1()
                                                        .border_color(white(0.08))
                                                        .pb(px(6.0))
                                                        .child(div().w(relative(0.476)).child("CHILD PROCESS".to_string()))
                                                        .child(div().w(relative(0.167)).child("CPU".to_string()))
                                                        .child(div().w(relative(0.214)).child("MEMORY".to_string()))
                                                        .child(div().w(relative(0.143)).child("PID".to_string())),
                                                )
                                                .children(process.children.iter().enumerate().map(|(child_idx, child)| {
                                                    let mut row = div()
                                                        .flex()
                                                        .gap(px(12.0))
                                                        .py(px(8.0));
                                                    if child_idx + 1 != process.children.len() {
                                                        row = row.border_b_1().border_color(white(0.06));
                                                    }

                                                    row.child(
                                                            div()
                                                                .w(relative(0.476))
                                                                .child(
                                                                    div()
                                                                        .font_weight(FontWeight::SEMIBOLD)
                                                                        .text_color(white(0.86))
                                                                        .child(child.name.clone()),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(px(11.0))
                                                                        .text_color(white(0.45))
                                                                        .child(if let Some(cmdline) = child.cmdline.as_ref() {
                                                                            format!(
                                                                                "ppid {} · {}",
                                                                                child
                                                                                    .parent_pid
                                                                                    .map(|pid| pid.to_string())
                                                                                    .unwrap_or_else(|| "—".to_string()),
                                                                                truncate_text(cmdline, 120)
                                                                            )
                                                                        } else {
                                                                            format!(
                                                                                "ppid {}",
                                                                                child
                                                                                    .parent_pid
                                                                                    .map(|pid| pid.to_string())
                                                                                    .unwrap_or_else(|| "—".to_string())
                                                                            )
                                                                        }),
                                                                ),
                                                        )
                                                        .child(div().w(relative(0.167)).child(format_pct(Some(child.cpu_pct))))
                                                        .child(div().w(relative(0.214)).child(format_bytes(Some(child.memory_bytes))))
                                                        .child(
                                                            div()
                                                                .w(relative(0.143))
                                                                .font_family("ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace")
                                                                .text_size(px(12.0))
                                                                .text_color(white(0.75))
                                                                .child(child.pid.to_string()),
                                                        )
                                                }))
                                        )
                                        .into_any_element()
                                });
                            elements.push(children_section.into_any_element());
                        }

                        elements
                    });

                    div().grid().gap(px(6.0)).child(head).children(rows).into_any_element()
                }),
        );

        let workspace_card = settings_card(
            colors,
            Some("Workspace Disk"),
            div()
                .px(px(16.0))
                .py(px(14.0))
                .grid()
                .gap(px(12.0))
                .child(
                    div()
                        .grid()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(white(0.88))
                                .child(workspace_name),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(white(0.55))
                                .child(snapshot.as_ref().map(|s| s.workspace.root_path.clone()).unwrap_or_else(|| "—".to_string())),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(white(0.4))
                                .child(if let Some(snapshot) = snapshot.as_ref() {
                                    format!(
                                        "{} total · {}",
                                        format_bytes(Some(snapshot.workspace.size_bytes)),
                                        disk_updated
                                    )
                                } else {
                                    "Sizing…".to_string()
                                }),
                        ),
                )
                .child(if worktree_rows.is_empty() {
                    settings_empty("No worktrees found.", false).into_any_element()
                } else {
                    let head = div()
                        .flex()
                        .gap(px(12.0))
                        .text_size(px(11.0))
                        .text_color(white(0.45))
                        .border_b_1()
                        .border_color(white(0.08))
                        .pb(px(6.0))
                        .child(div().w(relative(0.741)).child("WORKTREE".to_string()))
                        .child(div().w(relative(0.259)).child("SIZE".to_string()));

                    div()
                        .grid()
                        .gap(px(6.0))
                        .child(head)
                        .children(worktree_rows.iter().enumerate().map(|(idx, wt)| {
                            let mut row = div()
                                .flex()
                                .gap(px(12.0))
                                .py(px(8.0));
                            if idx + 1 != worktree_rows.len() {
                                row = row.border_b_1().border_color(white(0.06));
                            }

                            row.child(
                                    div()
                                        .w(relative(0.741))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(white(0.86))
                                                .child(wt.worktree_id.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(white(0.45))
                                                .child(wt.root_path.clone()),
                                        ),
                                )
                                .child(div().w(relative(0.259)).child(format_bytes(Some(wt.size_bytes))))
                        }))
                        .into_any_element()
                }),
        );

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(overview_card)
            .child(process_card)
            .child(workspace_card);

        if let Some(error) = self.resource_error.clone() {
            content = content.child(settings_banner(&error, true));
        }

        content.into_any_element()
    }

    fn render_dictation(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let model_options = vec![
            ("auto", "Default (Deepgram Nova-3)"),
            ("deepgram/flux-general", "Deepgram Flux"),
            ("deepgram/nova-3", "Deepgram Nova-3"),
            ("deepgram/nova-3-medical", "Deepgram Nova-3 Medical"),
            ("deepgram/nova-2", "Deepgram Nova-2"),
            ("deepgram/nova-2-medical", "Deepgram Nova-2 Medical"),
            ("deepgram/nova-2-conversationalai", "Deepgram Nova-2 Conversational AI"),
            ("deepgram/nova-2-phonecall", "Deepgram Nova-2 Phonecall"),
            ("assemblyai/universal-streaming", "AssemblyAI Universal-Streaming"),
            (
                "assemblyai/universal-streaming-multilingual",
                "AssemblyAI Universal-Streaming Multilingual",
            ),
            ("cartesia/ink-whisper", "Cartesia Ink Whisper"),
            ("elevenlabs/scribe_v2_realtime", "ElevenLabs Scribe V2 Realtime"),
        ]
        .into_iter()
        .map(|(value, label)| LabeledOption {
            value: value.to_string(),
            label: label.to_string(),
        })
        .collect::<Vec<_>>();

        let model_select = Self::ensure_select_state(
            &mut self.dictation_model_select,
            model_options,
            Some(self.dictation_model.clone()),
            super::super::state::SettingsSelectKind::DictationModel,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let language_input = Self::ensure_input_state(
            &mut self.dictation_language_input,
            "en",
            super::super::state::SettingsInputKind::DictationLanguage,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let base_url_input = Self::ensure_input_state(
            &mut self.dictation_base_url_input,
            "https://agent-gateway.livekit.cloud/v1",
            super::super::state::SettingsInputKind::DictationBaseUrl,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let api_key_input = Self::ensure_input_state(
            &mut self.dictation_api_key_input,
            "APIK…",
            super::super::state::SettingsInputKind::DictationApiKey,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let secret_placeholder = if self.dictation_secret_set { "(set)" } else { "MAB…" };
        let api_secret_input = Self::ensure_input_state(
            &mut self.dictation_api_secret_input,
            secret_placeholder,
            super::super::state::SettingsInputKind::DictationApiSecret,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        api_secret_input.update(cx, |state, cx| {
            state.set_masked(true, window, cx);
        });

        let dictation_disabled = self.settings.is_none();
        let toggle = settings_toggle(self.dictation_enabled, dictation_disabled)
            .id("settings-dictation-enable")
            .when(!dictation_disabled, |this| {
                this.on_click(cx.listener(|view, _, _window, cx| {
                    view.set_dictation_enabled(!view.dictation_enabled, cx);
                }))
            });

        let rows = vec![
            settings_row(
                "Enable dictation",
                Some("LiveKit Inference STT streams transcription while you speak."),
                toggle,
                true,
            )
            .into_any_element(),
            settings_row(
                "Model",
                Some("Transcription model used by the provider."),
                div()
                    .id("settings-dictation-model-select")
                    .child(
                        Select::new(&model_select)
                            .appearance(true)
                            .bg(white(0.06))
                            .border_color(white(0.08))
                            .rounded(px(8.0))
                            .h(px(30.0))
                            .px(px(10.0))
                            .text_size(px(13.0))
                            .disabled(!self.dictation_enabled),
                    ),
                false,
            )
            .into_any_element(),
            settings_row(
                "Language",
                Some("BCP-47 code (e.g. en, es, multi)."),
                Input::new(&language_input)
                    .appearance(true)
                    .bg(white(0.06))
                    .border_color(white(0.08))
                    .rounded(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .text_size(px(13.0))
                    .disabled(!self.dictation_enabled),
                false,
            )
            .into_any_element(),
            settings_row(
                "Inference base URL",
                Some("LiveKit Agent Gateway endpoint."),
                Input::new(&base_url_input)
                    .appearance(true)
                    .bg(white(0.06))
                    .border_color(white(0.08))
                    .rounded(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .text_size(px(13.0))
                    .w(relative(0.48))
                    .max_w(px(460.0))
                    .disabled(!self.dictation_enabled),
                false,
            )
            .into_any_element(),
            settings_row(
                "LiveKit API key",
                Some("Stored locally in your ctx data dir."),
                Input::new(&api_key_input)
                    .appearance(true)
                    .bg(white(0.06))
                    .border_color(white(0.08))
                    .rounded(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .text_size(px(13.0))
                    .w(relative(0.48))
                    .max_w(px(460.0))
                    .disabled(!self.dictation_enabled),
                false,
            )
            .into_any_element(),
            settings_row(
                "LiveKit API secret",
                Some(if self.dictation_secret_set {
                    "Secret is stored; enter a new value to rotate."
                } else {
                    "Required."
                }),
                Input::new(&api_secret_input)
                    .appearance(true)
                    .bg(white(0.06))
                    .border_color(white(0.08))
                    .rounded(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .text_size(px(13.0))
                    .w(relative(0.48))
                    .max_w(px(460.0))
                    .disabled(!self.dictation_enabled),
                false,
            )
            .into_any_element(),
        ];

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, settings_rows(rows)));
        if !self.dictation_can_save() && self.dictation_enabled {
            content = content.child(settings_banner(
                "Enter an API key and secret to enable dictation.",
                true,
            ));
        }
        content.into_any_element()
    }

    fn render_title_generation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let mode_options = vec![
            LabeledOption {
                value: "remote".to_string(),
                label: "Remote".to_string(),
            },
            LabeledOption {
                value: "local".to_string(),
                label: "Local".to_string(),
            },
        ];
        let mode_value = match self.title_mode {
            TitleGenerationMode::Local => "local".to_string(),
            TitleGenerationMode::Remote => "remote".to_string(),
        };
        let mode_select = Self::ensure_select_state(
            &mut self.title_mode_select,
            mode_options,
            Some(mode_value),
            super::super::state::SettingsSelectKind::TitleMode,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let base_url_input = Self::ensure_input_state(
            &mut self.title_base_url_input,
            "https://openrouter.ai/api/v1",
            super::super::state::SettingsInputKind::TitleBaseUrl,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let api_key_input = Self::ensure_input_state(
            &mut self.title_api_key_input,
            "sk-...",
            super::super::state::SettingsInputKind::TitleApiKey,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        api_key_input.update(cx, |state, cx| {
            state.set_masked(true, window, cx);
        });
        let model_input = Self::ensure_input_state(
            &mut self.title_model_input,
            "google/gemini-3-flash-preview",
            super::super::state::SettingsInputKind::TitleModel,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let local_model_input = Self::ensure_input_state(
            &mut self.title_local_model_input,
            "ggml-org/Qwen3-1.7B-GGUF",
            super::super::state::SettingsInputKind::TitleLocalModel,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let title_disabled = !self.settings.is_some();
        let remote_toggle = settings_toggle(self.title_use_json, title_disabled)
            .id("settings-title-json-toggle")
            .when(!title_disabled, |this| {
                this.on_click(cx.listener(|view, _, _window, cx| {
                    view.set_title_use_json(!view.title_use_json, cx);
                }))
            });
        let local_toggle = settings_toggle(self.title_local_use_json, title_disabled)
            .id("settings-title-json-local-toggle")
            .when(!title_disabled, |this| {
                this.on_click(cx.listener(|view, _, _window, cx| {
                    view.set_title_local_use_json(!view.title_local_use_json, cx);
                }))
            });

        let mode_control = div().child(
            Select::new(&mode_select)
                .appearance(true)
                .bg(white(0.06))
                .border_color(white(0.08))
                .rounded(px(8.0))
                .h(px(30.0))
                .px(px(10.0))
                .text_size(px(13.0))
                .disabled(title_disabled),
        );

        let mut rows = vec![settings_row(
            "Mode",
            Some("Choose between remote API or local model for session titles."),
            mode_control,
            true,
        )
        .into_any_element()];

        if matches!(self.title_mode, TitleGenerationMode::Remote) {
            rows.extend(vec![
                settings_row(
                    "Base URL",
                    Some(
                        "OpenAI-compatible endpoint for title generation (best-effort; falls back to truncating the prompt).",
                    ),
                    Input::new(&base_url_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .w(relative(0.48))
                        .max_w(px(460.0)),
                    false,
                )
                .into_any_element(),
                settings_row(
                    "API key",
                    Some("Stored locally in your ctx data dir."),
                    Input::new(&api_key_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .w(relative(0.48))
                        .max_w(px(460.0)),
                    false,
                )
                .into_any_element(),
                settings_row(
                    "Model",
                    Some("Model used for generating session titles."),
                    Input::new(&model_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .w(relative(0.48))
                        .max_w(px(460.0)),
                    false,
                )
                .into_any_element(),
                settings_row(
                    "Structured output (JSON)",
                    Some("Enable when the model supports JSON schema output."),
                    remote_toggle,
                    false,
                )
                .into_any_element(),
            ]);
        } else {
            let install_ui = self.installs.get(TITLE_GENERATION_LOCAL_INSTALL_KEY);
            let install_progress = install_ui.and_then(|session| session.pct);
            let install_running = install_ui
                .map(|session| matches!(session.state, InstallStateKind::Running))
                .unwrap_or(false)
                || self
                    .title_local_status
                    .as_ref()
                    .map(|s| s.install_running)
                    .unwrap_or(false);
            let installed = self
                .title_local_status
                .as_ref()
                .map(|s| s.ready)
                .unwrap_or(false);
            let status_label = if self.title_local_status_loading {
                "Loading…".to_string()
            } else if self.title_local_status_error.is_some() {
                "Status unavailable".to_string()
            } else if install_running {
                "Installing…".to_string()
            } else if installed {
                "Installed".to_string()
            } else {
                "Not installed".to_string()
            };
            let status_variant = if self.title_local_status_error.is_some() {
                PillVariant::Err
            } else if installed {
                PillVariant::Ok
            } else {
                PillVariant::Warn
            };
            let install_label = if install_running && install_progress.is_some() {
                format!("{}%", clamp_pct(install_progress.unwrap_or(0)))
            } else if install_running {
                "Installing…".to_string()
            } else if installed {
                "Installed".to_string()
            } else {
                "Install".to_string()
            };
            let install_disabled =
                title_disabled || install_running || installed || self.title_local_install_busy;

            let install_button = settings_button(
                &install_label,
                ButtonVariant::Secondary,
                install_disabled,
                install_progress,
            )
            .id("settings-title-generation-local-install")
            .when(!install_disabled, |this| {
                this.on_click(cx.listener(|view, _, _window, cx| {
                    view.install_title_generation_local(cx);
                }))
            });
            let install_control = div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(settings_pill(&status_label, status_variant, false))
                .child(install_button);

            rows.extend(vec![
                settings_row(
                    "Local model id",
                    Some("Model id for the local title generator."),
                    Input::new(&local_model_input)
                        .appearance(true)
                        .bg(white(0.06))
                        .border_color(white(0.08))
                        .rounded(px(8.0))
                        .h(px(30.0))
                        .px(px(10.0))
                        .text_size(px(13.0))
                        .w(relative(0.48))
                        .max_w(px(460.0)),
                    false,
                )
                .into_any_element(),
                settings_row(
                    "Structured output (JSON)",
                    Some("Enable when the model supports JSON schema output."),
                    local_toggle,
                    false,
                )
                .into_any_element(),
                settings_row(
                    "Local install",
                    Some("Downloads the llama.cpp runtime and Qwen3 1.7B model to the daemon host."),
                    install_control,
                    false,
                )
                .into_any_element(),
            ]);
        }

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, settings_rows(rows)));
        if let Some(err) = self.title_local_status_error.as_ref() {
            content = content.child(settings_banner(err, true));
        }

        content.into_any_element()
    }

    fn render_billing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let email_input = Self::ensure_input_state(
            &mut self.billing_email_input,
            "you@company.com",
            super::super::state::SettingsInputKind::BillingEmail,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        let password_input = Self::ensure_input_state(
            &mut self.billing_password_input,
            "••••••••",
            super::super::state::SettingsInputKind::BillingPassword,
            &mut self.input_subscriptions,
            window,
            cx,
        );
        password_input.update(cx, |state, cx| {
            state.set_masked(true, window, cx);
        });

        let account_rows = vec![
            settings_row(
                "Email",
                None,
                Input::new(&email_input)
                    .appearance(true)
                    .bg(white(0.06))
                    .border_color(white(0.08))
                    .rounded(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .text_size(px(13.0))
                    .w(relative(0.48))
                    .max_w(px(460.0)),
                true,
            )
            .into_any_element(),
            settings_row(
                "Password",
                None,
                Input::new(&password_input)
                    .appearance(true)
                    .bg(white(0.06))
                    .border_color(white(0.08))
                    .rounded(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .text_size(px(13.0))
                    .w(relative(0.48))
                    .max_w(px(460.0)),
                false,
            )
            .into_any_element(),
            settings_row(
                "Sign in / Create account",
                Some("Subscriptions are purchased via Stripe on desktop; mobile devices inherit access when connected."),
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(settings_button("Sign in", ButtonVariant::Secondary, false, None))
                    .child(settings_button("Create account", ButtonVariant::Primary, false, None)),
                false,
            )
            .into_any_element(),
        ];

        let subscription_rows = vec![
            settings_row(
                "Plan",
                Some("Free/Local"),
                settings_pill("free_local", PillVariant::Default, false),
                true,
            )
            .into_any_element(),
            settings_row(
                "Remote mobile access",
                Some("Stable remote access + push notifications are Pro features. Purchase on desktop."),
                settings_pill("Disabled", PillVariant::Default, false),
                false,
            )
            .into_any_element(),
            settings_row(
                "Subscribe",
                Some("USD only. CTX Pro is $20/month or $200/year."),
                div()
                    .flex()
                    .gap(px(8.0))
                    .flex_wrap()
                    .justify_end()
                    .child(settings_button("$20 / month", ButtonVariant::Secondary, true, None))
                    .child(settings_button("$200 / year", ButtonVariant::Secondary, true, None))
                    .child(settings_button("Manage", ButtonVariant::Primary, true, None)),
                false,
            )
            .into_any_element(),
        ];

        div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, Some("Account"), settings_rows(account_rows)))
            .child(settings_card(
                colors,
                Some("Subscription"),
                settings_rows(subscription_rows),
            ))
            .into_any_element()
    }

    fn render_agent_harnesses(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let any_workspace = !self.workspaces.is_empty();
        let visible_providers = self
            .providers
            .iter()
            .filter(|p| p.details.get("ui_hidden").map(|v| v != "true").unwrap_or(true))
            .collect::<Vec<_>>();

        let mut providers_by_id = std::collections::HashMap::new();
        for provider in visible_providers {
            providers_by_id.insert(provider.provider_id.clone(), provider.clone());
        }

        let mut order = std::collections::HashMap::new();
        for (idx, entry) in harness_catalog().iter().enumerate() {
            order.insert(entry.id, idx);
        }

        let mut harnesses = Vec::new();
        for entry in harness_catalog().iter() {
            if providers_by_id.contains_key(entry.id) {
                harnesses.push((entry.id.to_string(), entry.label.to_string(), true));
            }
        }

        let mut extras = providers_by_id
            .keys()
            .filter(|id| !order.contains_key(id.as_str()))
            .map(|id| (id.clone(), id.clone(), false))
            .collect::<Vec<_>>();
        extras.sort_by(|a, b| a.0.cmp(&b.0));
        harnesses.extend(extras);

        let workspace_options = self
            .workspaces
            .iter()
            .map(|ws| LabeledOption {
                value: ws.id.0.to_string(),
                label: ws.name.clone(),
            })
            .collect::<Vec<_>>();

        let workspace_select = Self::ensure_select_state(
            &mut self.workspace_select,
            workspace_options,
            self.selected_workspace.map(|id| id.0.to_string()),
            super::super::state::SettingsSelectKind::Workspace,
            &mut self.input_subscriptions,
            window,
            cx,
        );

        let header_rows = vec![
            settings_row(
                "Install all",
                Some("Installs supported harnesses to ~/.ctx/providers/agent-servers."),
                settings_button(
                    if self.install_busy.as_deref() == Some("all") {
                        "Installing…"
                    } else {
                        "Install all"
                    },
                    ButtonVariant::Primary,
                    self.install_busy.is_some(),
                    None,
                )
                .id("settings-harness-install-all")
                .when(self.install_busy.is_none(), |this| {
                    this.on_click(cx.listener(|view, _, _window, cx| {
                        view.install_all_providers(cx);
                    }))
                }),
                true,
            )
            .into_any_element(),
            settings_row(
                "Workspace",
                Some("Used for authenticate/verify checks."),
                div()
                    .id("settings-harness-workspace-select")
                    .child(
                        Select::new(&workspace_select)
                            .appearance(true)
                            .bg(white(0.06))
                            .border_color(white(0.08))
                            .rounded(px(8.0))
                            .h(px(30.0))
                            .px(px(10.0))
                            .text_size(px(13.0))
                            .disabled(!any_workspace),
                    ),
                false,
            )
            .into_any_element(),
        ];

        let harness_rows = harnesses
            .iter()
            .enumerate()
            .filter_map(|(index, (id, label, _is_catalog))| {
            let provider = providers_by_id.get(id)?;
            let installed = provider.installed && matches!(provider.health, ProviderHealth::Ok);
            let install_supported = provider
                .details
                .get("install_supported")
                .map(|v| v == "true")
                .unwrap_or(false);
            let install_running = provider
                .details
                .get("install_running")
                .map(|v| v == "true")
                .unwrap_or(false);
            let install_ui = self.installs.get(id);
            let install_busy_local = self.install_busy.is_some() || install_running;
            let progress = install_ui.and_then(|session| session.pct);
            let install_label = if install_busy_local && progress.is_some() {
                format!("{}%", clamp_pct(progress.unwrap_or(0)))
            } else if install_busy_local {
                "Installing…".to_string()
            } else if provider.installed {
                "Update".to_string()
            } else {
                "Install".to_string()
            };

            let opts = self.provider_options.get(id);
            let verify_status = opts.and_then(|o| o.verify.as_ref()).map(|v| v.status.clone()).unwrap_or_default();
            let needs_auth = opts.map(|o| o.auth_required).unwrap_or(false) || verify_status == "auth_required";
            let show_verify = !needs_auth && verify_status != "ok";

            let status_pill = if opts.is_none() {
                None
            } else if needs_auth {
                Some(settings_pill("Auth required", PillVariant::Warn, false))
            } else if verify_status == "network_error" {
                Some(settings_pill("Offline", PillVariant::Warn, false))
            } else if verify_status == "error" {
                Some(settings_pill("Error", PillVariant::Err, false))
            } else if opts.and_then(|o| o.probe_ok).unwrap_or(true) == false {
                Some(settings_pill("Unhealthy", PillVariant::Err, false))
            } else if verify_status == "ok" {
                Some(settings_pill("Verified", PillVariant::Ok, false))
            } else {
                None
            };

            let logo = self.harness_logos.get(id);

            let title_row = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(if let Some(image) = logo {
                    img(image.clone())
                        .w(px(16.0))
                        .h(px(16.0))
                        .object_fit(ObjectFit::Contain)
                        .into_any_element()
                } else {
                    div()
                        .w(px(16.0))
                        .h(px(16.0))
                        .bg(white(0.08))
                        .into_any_element()
                })
                .child(
                    div()
                        .text_color(white(0.84))
                        .truncate()
                        .child(label.clone()),
                )
                .when_some(status_pill, |this, pill| this.child(pill));

            let subtitle = format!(
                "{}{}",
                if installed { "Installed" } else { "Not installed" },
                provider
                    .version
                    .as_ref()
                    .map(|v| format!(" · {}", v))
                    .unwrap_or_default()
            );

            let actions = if !installed {
                let install_disabled = !install_supported || install_busy_local;
                settings_button(
                    &install_label,
                    ButtonVariant::Secondary,
                    install_disabled,
                    progress,
                )
                .id(format!("settings-harness-install-{}", id))
                .when(!install_disabled, |this| {
                    this.on_click(cx.listener({
                        let id = id.clone();
                        move |view, _, _window, cx| {
                            view.install_provider(id.clone(), cx);
                        }
                    }))
                })
                .into_any_element()
            } else {
                let mut action_row = div().flex().gap(px(8.0));
                if needs_auth {
                    let auth_disabled = !any_workspace || self.auth_busy.get(id).copied().unwrap_or(false);
                    action_row = action_row.child(
                        settings_button(
                            if self.auth_busy.get(id).copied().unwrap_or(false) {
                                "Auth…"
                            } else {
                                "Authenticate"
                            },
                            ButtonVariant::Secondary,
                            auth_disabled,
                            None,
                        )
                        .id(format!("settings-harness-auth-{}", id))
                        .when(!auth_disabled, |this| {
                            this.on_click(cx.listener({
                                let id = id.clone();
                                move |view, _, _window, cx| {
                                    view.authenticate_provider(id.clone(), cx);
                                }
                            }))
                        }),
                    );
                }
                if show_verify {
                    let verify_disabled = !any_workspace || self.verify_busy.get(id).copied().unwrap_or(false);
                    action_row = action_row.child(
                        settings_button(
                            if self.verify_busy.get(id).copied().unwrap_or(false) {
                                "Verifying…"
                            } else {
                                "Verify"
                            },
                            ButtonVariant::Secondary,
                            verify_disabled,
                            None,
                        )
                        .id(format!("settings-harness-verify-{}", id))
                        .when(!verify_disabled, |this| {
                            this.on_click(cx.listener({
                                let id = id.clone();
                                move |view, _, _window, cx| {
                                    view.verify_provider(id.clone(), cx);
                                }
                            }))
                        }),
                    );
                }
                let check_disabled = !any_workspace || self.opts_busy.get(id).copied().unwrap_or(false);
                action_row = action_row.child(
                    settings_button(
                        if self.opts_busy.get(id).copied().unwrap_or(false) {
                            "Checking…"
                        } else {
                            "Check"
                        },
                        ButtonVariant::Secondary,
                        check_disabled,
                        None,
                    )
                    .id(format!("settings-harness-check-{}", id))
                    .when(!check_disabled, |this| {
                        this.on_click(cx.listener({
                            let id = id.clone();
                            move |view, _, _window, cx| {
                                view.ensure_provider_options(id.clone(), true, cx);
                            }
                        }))
                    }),
                );
                action_row.into_any_element()
            };

            let mut row = div()
                .flex()
                .gap(px(18.0))
                .px(px(16.0))
                .py(px(12.0))
                .hover(|style| style.bg(white(0.02)));
            if index != 0 {
                row = row.border_t_1().border_color(white(0.08));
            }

            Some(
                row.child(
                        div()
                            .flex_1()
                            .grid()
                            .gap(px(2.0))
                            .child(title_row)
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(white(0.42))
                                    .child(subtitle),
                            )
                            .when(!installed, |this| this.opacity(0.7)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .justify_end()
                            .items_center()
                            .gap(px(8.0))
                            .child(actions),
                    )
                    .into_any_element(),
            )
        });

        let list_card = settings_card(
            colors,
            None,
            div()
                .grid()
                .children(harness_rows)
                .when(harnesses.is_empty(), |this| {
                    this.child(settings_empty("No harnesses.", false))
                }),
        );

        let mut content = div()
            .grid()
            .gap(px(14.0))
            .child(settings_card(colors, None, settings_rows(header_rows)))
            .child(list_card);

        if let Some(error) = self.provider_error.clone() {
            content = content.child(settings_banner(&error, true));
        }

        content.into_any_element()
    }
}
