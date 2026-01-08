use gpui::{ClickEvent, Context, ElementId, Render, Window, div, prelude::*};

use ctx_client::InstallStateKind;
use ctx_providers::adapters::ProviderHealth;

use crate::theme::ThemeColors;
use super::super::state::SettingsState;

enum StatusTone {
    Good,
    Warn,
    Bad,
    Neutral,
}

fn status_pill(colors: ThemeColors, label: &str, tone: StatusTone) -> impl IntoElement {
    let text_color = match tone {
        StatusTone::Good => colors.success,
        StatusTone::Warn => colors.warning,
        StatusTone::Bad => colors.error,
        StatusTone::Neutral => colors.accent,
    };

    div()
        .px_1()
        .py_0()
        .text_sm()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel)
        .text_color(text_color)
        .child(label.to_string())
}

fn button_base<E: IntoElement>(colors: ThemeColors, label: E) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .text_sm()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .child(label)
}

fn section_card<E: IntoElement>(colors: ThemeColors, title: &str, body: E) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .border_1()
        .border_color(colors.border)
        .rounded_sm()
        .bg(colors.panel_2)
        .p_2()
        .child(
            div()
                .text_sm()
                .text_color(colors.muted)
                .child(title.to_string()),
        )
        .child(body)
}

fn error_banner(colors: ThemeColors, title: &str, message: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .border_1()
        .border_color(colors.error)
        .rounded_sm()
        .bg(colors.panel)
        .p_2()
        .child(
            div()
                .text_sm()
                .text_color(colors.error)
                .child(title.to_string()),
        )
        .child(div().text_sm().text_color(colors.text).child(message.to_string()))
}

impl Render for SettingsState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let refresh_enabled =
            !(self.workspaces_loading || self.providers_loading || self.settings_loading);
        let refresh_label = if refresh_enabled {
            "Refresh"
        } else {
            "Refreshing..."
        };

        let refresh_button = if refresh_enabled {
            button_base(colors, refresh_label)
                .bg(colors.panel)
                .cursor_pointer()
                .id("settings-refresh")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.start_load(cx);
                }))
                .into_any_element()
        } else {
            button_base(colors, refresh_label)
                .bg(colors.panel_2)
                .text_color(colors.muted)
                .into_any_element()
        };

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_sm().child("Settings"))
            .child(refresh_button);

        let summary_rows = self.summary_rows();
        let summary_list = if summary_rows.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No settings loaded.")
        } else {
            summary_rows.iter().fold(
                div().flex().flex_col().gap_1(),
                |list, row| {
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_sm()
                            .child(
                                div()
                                    .text_color(colors.muted)
                                    .child(row.label.clone()),
                            )
                            .child(row.value.clone()),
                    )
                },
            )
        };

        let summary_card = section_card(colors, "Settings summary", summary_list);

        let workspace_list = if self.workspaces_loading {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("Loading workspaces...")
        } else if self.workspaces.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No workspaces yet.")
        } else {
            self.workspaces
                .iter()
                .enumerate()
                .fold(div().flex().flex_col().gap_1(), |list, (index, workspace)| {
                    let is_selected = self.selected_workspace == Some(workspace.id);
                    let item_bg = if is_selected {
                        colors.panel
                    } else {
                        colors.panel_2
                    };
                    let item_border = if is_selected {
                        colors.border_strong
                    } else {
                        colors.border
                    };
                    let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_workspace(index, cx);
                    });
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(item_border)
                            .rounded_sm()
                            .bg(item_bg)
                            .text_sm()
                            .child(workspace.name.clone())
                            .cursor_pointer()
                            .id(ElementId::named_usize("settings-workspace", index))
                            .on_click(on_click),
                    )
                })
        };

        let install_all_label = if self.install_busy.as_deref() == Some("all") {
            "Installing..."
        } else {
            "Install all"
        };
        let install_all_enabled = self.install_busy.is_none();
        let install_all_button = if install_all_enabled {
            button_base(colors, install_all_label)
                .bg(colors.panel)
                .cursor_pointer()
                .id("settings-install-all")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.install_all_providers(cx);
                }))
                .into_any_element()
        } else {
            button_base(colors, install_all_label)
                .bg(colors.panel_2)
                .text_color(colors.muted)
                .into_any_element()
        };

        let provider_controls = section_card(
            colors,
            "Agent harnesses",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0()
                                .child(div().text_sm().child("Install all"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted)
                                        .child("Installs supported harnesses to ~/.ctx/providers/agent-servers."),
                                ),
                        )
                        .child(install_all_button),
                )
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0()
                                .child(div().text_sm().child("Workspace"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted)
                                        .child("Used for authenticate/verify checks."),
                                ),
                        )
                        .child(workspace_list),
                )
        );

        let any_workspace = self.selected_workspace.is_some();
        let mut visible_providers: Vec<_> = self
            .providers
            .iter()
            .filter(|provider| {
                !provider
                    .details
                    .get("ui_hidden")
                    .map(|value| value == "true")
                    .unwrap_or(false)
            })
            .collect();
        visible_providers.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));

        let provider_list = if self.providers_loading {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("Loading harnesses...")
        } else if visible_providers.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No harnesses.")
        } else {
            visible_providers.iter().fold(
                div().flex().flex_col().gap_2(),
                |list, provider| {
                    let provider_id = provider.provider_id.clone();
                    let installed_ok = provider.installed
                        && matches!(provider.health, ProviderHealth::Ok);
                    let install_supported = provider
                        .details
                        .get("install_supported")
                        .map(|value| value == "true")
                        .unwrap_or(false);
                    let mut install_running = provider
                        .details
                        .get("install_running")
                        .map(|value| value == "true")
                        .unwrap_or(false);
                    let install_session = self.installs.get(&provider_id);
                    if let Some(session) = install_session.as_ref() {
                        if matches!(session.state, InstallStateKind::Running) {
                            install_running = true;
                        }
                    }
                    let install_busy = self.install_busy.is_some() || install_running;
                    let install_label = if install_busy {
                        if let Some(pct) = install_session.and_then(|session| session.pct) {
                            format!("{pct}%")
                        } else {
                            "Installing...".to_string()
                        }
                    } else if provider.installed {
                        "Update".to_string()
                    } else {
                        "Install".to_string()
                    };

                    let mut detail_line = if installed_ok {
                        "Installed".to_string()
                    } else {
                        "Not installed".to_string()
                    };
                    if let Some(version) = provider.version.as_ref() {
                        detail_line = format!("{detail_line} - {version}");
                    }
                    if !matches!(provider.health, ProviderHealth::Ok) {
                        detail_line =
                            format!("{detail_line} - health: {:?}", provider.health);
                    }

                    let opts = self.provider_options.get(&provider_id);
                    let verify_status = opts
                        .and_then(|options| options.verify.as_ref())
                        .map(|verify| verify.status.as_str())
                        .unwrap_or("");
                    let needs_auth = opts.map(|options| options.auth_required).unwrap_or(false)
                        || verify_status == "auth_required";
                    let show_verify = !needs_auth && verify_status != "ok";
                    let status_badge = if opts.is_none()
                        && self.opts_busy.get(&provider_id).copied().unwrap_or(false)
                    {
                        Some(status_pill(colors, "Checking", StatusTone::Neutral))
                    } else if needs_auth {
                        Some(status_pill(colors, "Auth required", StatusTone::Warn))
                    } else if verify_status == "network_error" {
                        Some(status_pill(colors, "Offline", StatusTone::Warn))
                    } else if verify_status == "error" {
                        Some(status_pill(colors, "Error", StatusTone::Bad))
                    } else if opts.and_then(|options| options.probe_ok) == Some(false) {
                        Some(status_pill(colors, "Unhealthy", StatusTone::Bad))
                    } else if verify_status == "ok" {
                        Some(status_pill(colors, "Verified", StatusTone::Good))
                    } else {
                        None
                    };

                    let mut action_row =
                        div().flex().items_center().justify_end().gap_1();
                    if !installed_ok {
                        let install_button = if install_supported && !install_busy {
                            let install_provider_id = provider_id.clone();
                            let on_click =
                                cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.install_provider(install_provider_id.clone(), cx);
                                });
                            button_base(colors, install_label.clone())
                                .bg(colors.panel)
                                .cursor_pointer()
                                .id(format!("settings-install-{provider_id}"))
                                .active(|style| style.opacity(0.85))
                                .on_click(on_click)
                                .into_any_element()
                        } else {
                            button_base(colors, install_label.clone())
                                .bg(colors.panel_2)
                                .text_color(colors.muted)
                                .into_any_element()
                        };
                        action_row = action_row.child(install_button);
                    } else {
                        if needs_auth {
                            let auth_busy = self
                                .auth_busy
                                .get(&provider_id)
                                .copied()
                                .unwrap_or(false);
                            let auth_label = if auth_busy {
                                "Auth..."
                            } else {
                                "Authenticate"
                            };
                            let auth_button = if any_workspace && !auth_busy {
                                let auth_provider_id = provider_id.clone();
                                let on_click =
                                    cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                        view.authenticate_provider(auth_provider_id.clone(), cx);
                                    });
                                button_base(colors, auth_label)
                                    .bg(colors.panel)
                                    .cursor_pointer()
                                    .id(format!("settings-auth-{provider_id}"))
                                    .active(|style| style.opacity(0.85))
                                    .on_click(on_click)
                                    .into_any_element()
                            } else {
                                button_base(colors, auth_label)
                                    .bg(colors.panel_2)
                                    .text_color(colors.muted)
                                    .into_any_element()
                            };
                            action_row = action_row.child(auth_button);
                        }
                        if show_verify {
                            let verify_busy = self
                                .verify_busy
                                .get(&provider_id)
                                .copied()
                                .unwrap_or(false);
                            let verify_label = if verify_busy {
                                "Verifying..."
                            } else {
                                "Verify"
                            };
                            let verify_button = if any_workspace && !verify_busy {
                                let verify_provider_id = provider_id.clone();
                                let on_click =
                                    cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                        view.verify_provider(verify_provider_id.clone(), cx);
                                    });
                                button_base(colors, verify_label)
                                    .bg(colors.panel)
                                    .cursor_pointer()
                                    .id(format!("settings-verify-{provider_id}"))
                                    .active(|style| style.opacity(0.85))
                                    .on_click(on_click)
                                    .into_any_element()
                            } else {
                                button_base(colors, verify_label)
                                    .bg(colors.panel_2)
                                    .text_color(colors.muted)
                                    .into_any_element()
                            };
                            action_row = action_row.child(verify_button);
                        }

                        let opts_busy = self
                            .opts_busy
                            .get(&provider_id)
                            .copied()
                            .unwrap_or(false);
                        let check_label = if opts_busy {
                            "Checking..."
                        } else {
                            "Check"
                        };
                        let check_button = if any_workspace && !opts_busy {
                            let check_provider_id = provider_id.clone();
                            let on_click =
                                cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.ensure_provider_options(check_provider_id.clone(), true, cx);
                                });
                            button_base(colors, check_label)
                                .bg(colors.panel)
                                .cursor_pointer()
                                .id(format!("settings-check-{provider_id}"))
                                .active(|style| style.opacity(0.85))
                                .on_click(on_click)
                                .into_any_element()
                        } else {
                            button_base(colors, check_label)
                                .bg(colors.panel_2)
                                .text_color(colors.muted)
                                .into_any_element()
                        };
                        action_row = action_row.child(check_button);
                    }

                    let mut detail_lines = vec![detail_line];
                    if !install_supported {
                        detail_lines.push("Install not supported yet".to_string());
                    }
                    if let Some(session) = install_session {
                        match session.state {
                            InstallStateKind::Running => {
                                let mut install_detail = "Install running".to_string();
                                if let Some(stage) = session.last_stage.as_ref() {
                                    install_detail = format!("Install: {stage}");
                                }
                                if let Some(message) = session.last_message.as_ref() {
                                    install_detail =
                                        format!("{install_detail} - {message}");
                                }
                                detail_lines.push(install_detail);
                            }
                            InstallStateKind::Failed => {
                                let message = session
                                    .error
                                    .as_deref()
                                    .or(session.last_message.as_deref())
                                    .unwrap_or("Install failed");
                                detail_lines.push(format!("Install failed: {message}"));
                            }
                            InstallStateKind::Succeeded => {}
                        }
                    }
                    if let Some(options) = opts {
                        if let Some(error) = options.probe_error.as_ref() {
                            detail_lines.push(format!("Probe error: {error}"));
                        }
                    }

                    let detail_block = detail_lines.iter().fold(
                        div().flex().flex_col().gap_0(),
                        |detail, line| {
                            detail.child(
                                div()
                                    .text_sm()
                                    .text_color(colors.muted)
                                    .child(line.clone()),
                            )
                        },
                    );

                    let mut title_row = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(provider.provider_id.clone());
                    if let Some(pill) = status_badge {
                        title_row = title_row.child(pill);
                    }

                    list.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_2()
                            .py_2()
                            .border_1()
                            .border_color(colors.border)
                            .rounded_sm()
                            .bg(colors.panel)
                            .text_sm()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(title_row)
                                    .child(action_row),
                            )
                            .child(detail_block),
                    )
                },
            )
        };

        let provider_card = section_card(colors, "Harnesses", provider_list);

        let mut errors = div().flex().flex_col().gap_1();
        if let Some(message) = self.workspace_error.as_ref() {
            errors = errors.child(error_banner(colors, "Workspace error", message));
        }
        if let Some(message) = self.provider_error.as_ref() {
            errors = errors.child(error_banner(colors, "Provider error", message));
        }
        if let Some(message) = self.settings_error.as_ref() {
            errors = errors.child(error_banner(colors, "Settings error", message));
        }

        div()
            .id("settings-root")
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(header)
            .child(summary_card)
            .child(provider_controls)
            .child(provider_card)
            .child(errors)
    }
}
