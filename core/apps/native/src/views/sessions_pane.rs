use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::{ClickEvent, Context, ElementId, div, prelude::*, px};
use gpui_tokio::Tokio;

use crate::{
    automation_tree,
    theme::{ThemeColors, ThemeMetrics},
};

use ctx_client::WebSessionInfo;

use super::super::icons::{Icon, IconName};
use super::super::state::{ShellView, StreamStatus};
use super::super::workspace_summary::SessionSummaryItem;

const WEB_SESSIONS_REFRESH: Duration = Duration::from_secs(10);

pub(crate) struct SessionsPaneView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) sessions: &'a [SessionSummaryItem],
    pub(super) selected_session: Option<usize>,
    pub(super) stream_status: &'a StreamStatus,
    pub(super) resyncing: bool,
}

impl<'a> SessionsPaneView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let selected_session_id = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id.0.to_string());
        maybe_refresh_web_sessions(cx, selected_session_id);
        let (web_sessions, selected_web_id, active_web_kind, web_loading, web_error) =
            web_sessions_snapshot();
        let web_sections = web_session_sections(&web_sessions);
        let visible_web_sections = web_sections
            .iter()
            .filter(|section| !section.sessions.is_empty())
            .collect::<Vec<_>>();
        let active_section = active_web_kind
            .as_ref()
            .and_then(|key| web_sections.iter().find(|section| section.key == *key))
            .or_else(|| visible_web_sections.first().copied());
        let active_sessions = active_section
            .map(|section| section.sessions.as_slice())
            .unwrap_or(&[]);
        let selected_web_session = selected_web_id
            .as_ref()
            .and_then(|id| active_sessions.iter().find(|session| session.id == *id))
            .or_else(|| active_sessions.first());
        let selected_web_stream = selected_web_session.and_then(build_stream_url);

        let list = if self.sessions.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No sessions available for this run.")
        } else {
            self.sessions
                .iter()
                .enumerate()
                .fold(div().flex().flex_col().gap_2(), |list, (index, session)| {
                    let is_selected = self.selected_session == Some(index);
                    let item_bg = if is_selected {
                        self.colors.panel
                    } else {
                        self.colors.panel_2
                    };
                    let item_border = if is_selected {
                        self.colors.border_strong
                    } else {
                        self.colors.border
                    };
                    let on_click = cx.listener(move |view, _: &ClickEvent, window, cx| {
                        view.select_session(index, window, cx);
                    });
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(metrics.spacing.xl))
                            .py(px(metrics.spacing.md))
                            .border_1()
                            .border_color(item_border)
                            .rounded_sm()
                            .bg(item_bg)
                            .text_sm()
                            .child(session.title.clone())
                            .child(
                                div()
                                    .px(px(metrics.spacing.md))
                                    .py(px(0.0))
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child(session.status.clone()),
                            )
                            .cursor_pointer()
                            .id(ElementId::named_usize("session-item", index))
                            .on_click(on_click),
                    )
                })
        }
        .on_children_prepainted(automation_tree::track_children_bounds(
            "sessions-list",
            "list",
            Some("Sessions"),
            Some("app-shell"),
        ));

        let mut stream_block = div()
            .flex()
            .flex_col()
            .gap_1()
            .text_sm()
            .text_color(self.colors.muted)
            .child(format!("Stream: {}", self.stream_status.label()));
        if let Some(detail) = self.stream_status.detail() {
            stream_block = stream_block.child(detail.to_string());
        }
        if self.resyncing {
            stream_block = stream_block.child("Resyncing session data...");
        }

        let count_pill = div()
            .px(px(metrics.spacing.md))
            .py(px(0.0))
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_full()
            .bg(self.colors.panel)
            .text_color(self.colors.text)
            .child(format!("{}", self.sessions.len()));

        let web_kind_tabs = if visible_web_sections.len() > 1 {
            visible_web_sections
                .iter()
                .enumerate()
                .fold(div().flex().items_center().gap_1(), |tabs, (index, section)| {
                    let is_active = active_section
                        .map(|active| active.key == section.key)
                        .unwrap_or(false);
                    let tab_bg = if is_active {
                        self.colors.panel
                    } else {
                        self.colors.panel_2
                    };
                    let tab_border = if is_active {
                        self.colors.border_strong
                    } else {
                        self.colors.border
                    };
                    let tab_color = if is_active {
                        self.colors.text
                    } else {
                        self.colors.muted
                    };
                    let section_key = section.key.clone();
                    let selected_id = section.sessions.first().map(|session| session.id.clone());
                    let on_click = cx.listener(move |_, _: &ClickEvent, _window, cx| {
                        if let Ok(mut state) = web_sessions_state().lock() {
                            state.active_kind = Some(section_key.clone());
                            state.selected_id = selected_id.clone();
                        }
                        cx.notify();
                    });
                    tabs.child(
                        div()
                            .px(px(metrics.spacing.md))
                            .py(px(metrics.spacing.xs))
                            .border_1()
                            .border_color(tab_border)
                            .rounded_full()
                            .bg(tab_bg)
                            .text_sm()
                            .text_color(tab_color)
                            .child(section.label.clone())
                            .cursor_pointer()
                            .id(ElementId::named_usize("web-session-kind", index))
                            .on_click(on_click),
                    )
                })
        } else {
            div()
        };

        let web_session_tabs = if active_sessions.len() > 1 {
            active_sessions
                .iter()
                .enumerate()
                .fold(div().flex().items_center().gap_1(), |tabs, (index, session)| {
                    let is_selected = selected_web_session
                        .map(|selected| selected.id == session.id)
                        .unwrap_or(false);
                    let tab_bg = if is_selected {
                        self.colors.panel
                    } else {
                        self.colors.panel_2
                    };
                    let tab_border = if is_selected {
                        self.colors.border_strong
                    } else {
                        self.colors.border
                    };
                    let tab_color = if is_selected {
                        self.colors.text
                    } else {
                        self.colors.muted
                    };
                    let session_id = session.id.clone();
                    let label = web_session_label(session);
                    let on_click = cx.listener(move |_, _: &ClickEvent, _window, cx| {
                        if let Ok(mut state) = web_sessions_state().lock() {
                            state.selected_id = Some(session_id.clone());
                        }
                        cx.notify();
                    });
                    tabs.child(
                        div()
                            .px(px(metrics.spacing.md))
                            .py(px(metrics.spacing.xs))
                            .border_1()
                            .border_color(tab_border)
                            .rounded_full()
                            .bg(tab_bg)
                            .text_sm()
                            .text_color(tab_color)
                            .child(label)
                            .cursor_pointer()
                            .id(ElementId::named_usize("web-session-tab", index))
                            .on_click(on_click),
                    )
                })
        } else {
            div()
        };

        let web_stream_section = if active_sessions.is_empty() {
            let message = if web_loading {
                "Loading sessions..."
            } else {
                "No sessions available for this run."
            };
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child(message)
        } else if let Some(session) = selected_web_session {
            let viewer_message = if selected_web_stream.is_some() {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .items_center()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Embedded web stream viewer isn't available in GPUI yet.")
                    .child("Open the stream in a browser to interact.")
            } else {
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Stream unavailable.")
            };

            let viewer = div()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .bg(self.colors.panel_2)
                .p(px(metrics.spacing.xl))
                .min_h(px(220.0))
                .flex()
                .items_center()
                .justify_center()
                .child(viewer_message);

            let mut details = div().flex().flex_col().gap_1();
            details = details.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(format!("Status: {}", session.status)),
            );
            details = details.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(format!(
                        "Viewers: {} · {}x{} @ {}fps",
                        session.viewers, session.viewport.width, session.viewport.height, session.fps
                    )),
            );
            if let Some(stream_url) = selected_web_stream.as_ref() {
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child(format!("Stream URL: {stream_url}")),
                );
            } else {
                details = details.child(
                    div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Stream unavailable."),
                );
            }

            let mut open_button = div()
                .px(px(metrics.spacing.xl))
                .py(px(metrics.spacing.sm))
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_full()
                .child("Open in browser")
                .id("web-stream-open");
            if let Some(stream_url) = selected_web_stream.as_ref() {
                let open_url = stream_url.clone();
                let on_open = cx.listener(move |_, _: &ClickEvent, _window, cx| {
                    cx.open_url(&open_url);
                });
                open_button = open_button
                    .bg(self.colors.panel)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.85))
                    .on_click(on_open);
            } else {
                open_button = open_button
                    .bg(self.colors.panel_2)
                    .text_color(self.colors.muted);
            }

            details = details.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(open_button),
            );

            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(web_session_tabs)
                .child(viewer)
                .child(details)
        } else {
            div()
        };

        div()
            .id("sessions-pane-body")
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(metrics.spacing.xl))
                    .py(px(metrics.spacing.lg))
                    .border_b_1()
                    .border_color(self.colors.border)
                    .bg(self.colors.panel)
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(IconName::Sessions, 12.0, self.colors.muted))
                            .child(div().text_color(self.colors.text).child("Sessions")),
                    )
                    .child(count_pill),
            )
            .child(list)
            .child(stream_block)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(metrics.spacing.xl))
                    .py(px(metrics.spacing.md))
                    .border_b_1()
                    .border_color(self.colors.border)
                    .bg(self.colors.panel)
                    .text_sm()
                    .child(div().text_color(self.colors.text).child("Web Sessions"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(web_kind_tabs)
                            .child(if let Some(active) = active_section {
                                div()
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child(format!("{} · {}", active.label, active.sessions.len()))
                            } else {
                                div()
                            }),
                    ),
            )
            .child(web_stream_section)
            .child(if let Some(error) = web_error {
                div()
                    .text_sm()
                    .text_color(self.colors.error)
                    .child(error)
            } else {
                div()
            })
    }
}

#[derive(Clone)]
struct WebSessionSection {
    key: String,
    label: String,
    sessions: Vec<WebSessionInfo>,
}

#[derive(Default)]
struct WebSessionsCache {
    sessions: Vec<WebSessionInfo>,
    loading: bool,
    last_error: Option<String>,
    last_fetch: Option<Instant>,
    selected_id: Option<String>,
    active_kind: Option<String>,
    session_scope: Option<String>,
}

fn web_sessions_state() -> &'static Mutex<WebSessionsCache> {
    static STATE: OnceLock<Mutex<WebSessionsCache>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(WebSessionsCache::default()))
}

fn web_sessions_snapshot() -> (Vec<WebSessionInfo>, Option<String>, Option<String>, bool, Option<String>) {
    if let Ok(state) = web_sessions_state().lock() {
        (
            state.sessions.clone(),
            state.selected_id.clone(),
            state.active_kind.clone(),
            state.loading,
            state.last_error.clone(),
        )
    } else {
        (
            Vec::new(),
            None,
            None,
            false,
            Some("Web sessions unavailable.".to_string()),
        )
    }
}

fn maybe_refresh_web_sessions(cx: &mut Context<ShellView>, active_session_id: Option<String>) {
    let mut session_scope = None;
    let should_refresh = if let Ok(mut state) = web_sessions_state().lock() {
        if state.session_scope != active_session_id {
            state.session_scope = active_session_id.clone();
            state.sessions.clear();
            state.selected_id = None;
            state.active_kind = None;
            state.last_error = None;
            state.last_fetch = None;
        }
        if state.session_scope.is_none() {
            state.loading = false;
            return;
        }
        let stale = state
            .last_fetch
            .map(|last| last.elapsed() >= WEB_SESSIONS_REFRESH)
            .unwrap_or(true);
        if !state.loading && stale {
            state.loading = true;
            session_scope = state.session_scope.clone();
            true
        } else {
            session_scope = state.session_scope.clone();
            false
        }
    } else {
        false
    };

    if !should_refresh {
        return;
    }

    let Some(session_scope) = session_scope else {
        return;
    };

    let task = Tokio::spawn_result(cx, async move {
        let config = ctx_client::resolve_daemon_config()?;
        let client = ctx_client::Client::new(config)?;
        let sessions = client.list_web_sessions().await?;
        Ok(sessions)
    });

    cx.spawn(|this: gpui::WeakEntity<ShellView>, cx: &mut gpui::AsyncApp| {
        let mut cx = cx.clone();
        async move {
            let result = task.await;
            if let Ok(mut state) = web_sessions_state().lock() {
                if state.session_scope.as_deref() != Some(session_scope.as_str()) {
                    state.loading = false;
                } else {
                    state.loading = false;
                    state.last_fetch = Some(Instant::now());
                    match result {
                        Ok(sessions) => {
                            let filtered = sessions
                                .into_iter()
                                .filter(|session| {
                                    session.session_id.as_deref() == Some(session_scope.as_str())
                                })
                                .filter(web_session_is_running)
                                .collect::<Vec<_>>();
                            state.sessions = filtered;
                            state.last_error = None;
                            let sections = web_session_sections(&state.sessions);
                            if sections.is_empty() {
                                state.active_kind = None;
                                state.selected_id = None;
                            } else {
                                let next_kind = state
                                    .active_kind
                                    .clone()
                                    .filter(|key| sections.iter().any(|section| &section.key == key))
                                    .or_else(|| sections.first().map(|section| section.key.clone()));
                                state.active_kind = next_kind.clone();
                                let active_sessions = next_kind
                                    .as_ref()
                                    .and_then(|key| {
                                        sections
                                            .iter()
                                            .find(|section| &section.key == key)
                                            .map(|section| section.sessions.as_slice())
                                    })
                                    .unwrap_or(&[]);
                                let has_selected = state.selected_id.as_ref().and_then(|id| {
                                    active_sessions
                                        .iter()
                                        .find(|session| &session.id == id)
                                });
                                if has_selected.is_none() {
                                    state.selected_id =
                                        active_sessions.first().map(|session| session.id.clone());
                                }
                            }
                        }
                        Err(err) => {
                            state.last_error = Some(err.to_string());
                        }
                    }
                }
            }
            let _ = this.update(&mut cx, |_, cx| {
                cx.notify();
            });
        }
    })
    .detach();
}

fn web_session_sections(sessions: &[WebSessionInfo]) -> Vec<WebSessionSection> {
    let mut grouped: BTreeMap<String, Vec<WebSessionInfo>> = BTreeMap::new();
    for session in sessions {
        grouped
            .entry(session.kind.clone())
            .or_default()
            .push(session.clone());
    }
    let mut sections = Vec::new();
    for kind in ["web", "ios", "android"] {
        if let Some(sessions) = grouped.remove(kind) {
            sections.push(WebSessionSection {
                key: kind.to_string(),
                label: web_session_kind_label(kind),
                sessions,
            });
        }
    }
    for (kind, sessions) in grouped {
        sections.push(WebSessionSection {
            key: kind.clone(),
            label: web_session_kind_label(&kind),
            sessions,
        });
    }
    sections
}

fn web_session_kind_label(kind: &str) -> String {
    match kind {
        "web" => "Web Sessions".to_string(),
        "ios" => "iOS Sessions".to_string(),
        "android" => "Android Sessions".to_string(),
        _ => {
            let mut chars = kind.chars();
            let Some(first) = chars.next() else {
                return "Sessions".to_string();
            };
            let first = first.to_uppercase().collect::<String>();
            let rest = chars.collect::<String>();
            format!("{}{} Sessions", first, rest)
        }
    }
}

fn web_session_is_running(session: &WebSessionInfo) -> bool {
    session.status.eq_ignore_ascii_case("running")
}

fn web_session_label(session: &WebSessionInfo) -> String {
    let label = web_session_label_from_url(&session.url)
        .unwrap_or_else(|| session.id.chars().take(8).collect());
    let label_len = label.chars().count();
    if label_len > 32 {
        let truncated: String = label.chars().take(28).collect();
        format!("{truncated}...")
    } else {
        label
    }
}

fn web_session_label_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))?;
    if without_scheme.is_empty() {
        return None;
    }
    let without_fragment = without_scheme.split('#').next().unwrap_or(without_scheme);
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
    let (host, path) = if let Some((host, rest)) = without_query.split_once('/') {
        let mut path = String::from("/");
        path.push_str(rest);
        (host, path)
    } else {
        (without_query, String::new())
    };
    if host.is_empty() {
        return None;
    }
    let path = if path == "/" { String::new() } else { path };
    Some(format!("{host}{path}"))
}

fn build_stream_url(session: &WebSessionInfo) -> Option<String> {
    if let Some(url) = session.stream_url.as_ref() {
        return Some(url.clone());
    }
    if session.stream_path.is_empty() {
        return None;
    }
    let base = ctx_client::resolve_daemon_config().ok()?.base_url;
    let base = base.trim_end_matches('/');
    let path = if session.stream_path.starts_with('/') {
        session.stream_path.clone()
    } else {
        format!("/{}", session.stream_path)
    };
    Some(format!("{base}{path}"))
}
