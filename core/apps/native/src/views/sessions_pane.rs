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
        maybe_refresh_web_sessions(cx);
        let (web_sessions, selected_web_id, web_loading, web_error) = web_sessions_snapshot();
        let selected_web_session = selected_web_id
            .as_ref()
            .and_then(|id| web_sessions.iter().find(|session| session.id == *id))
            .or_else(|| web_sessions.first());
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
                    let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_session(index, cx);
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

        let web_list = if web_sessions.is_empty() {
            let message = if web_loading {
                "Loading web sessions..."
            } else {
                "No web sessions available."
            };
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child(message)
        } else {
            web_sessions
                .iter()
                .enumerate()
                .fold(div().flex().flex_col().gap_2(), |list, (index, session)| {
                let is_selected = selected_web_session
                    .map(|selected| selected.id == session.id)
                    .unwrap_or(false);
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
                let session_id = session.id.clone();
                let label = web_session_label(session);
                let on_click = cx.listener(move |_, _: &ClickEvent, _window, cx| {
                    if let Ok(mut state) = web_sessions_state().lock() {
                        state.selected_id = Some(session_id.clone());
                    }
                    cx.notify();
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
                        .child(label)
                        .child(
                            div()
                                .px(px(metrics.spacing.md))
                                .py(px(0.0))
                                .text_sm()
                                .text_color(self.colors.muted)
                                .child(session.status.clone()),
                        )
                        .cursor_pointer()
                        .id(ElementId::named_usize("web-session", index))
                        .on_click(on_click),
                )
            })
        };

        let web_stream_section = if let Some(session) = selected_web_session {
            let viewer_message = if selected_web_stream.is_some() {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Embedded web stream viewer is not available in GPUI yet.")
                    .child("Next steps: add a WebView component or render stream frames as images.")
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
                .h(px(220.0))
                .child(viewer_message);

            let mut details = div().flex().flex_col().gap_1();
            details = details.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(format!("Status: {}", session.status)),
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
                .child(viewer)
                .child(details)
        } else {
            div()
        };

        let web_count = div()
            .px(px(metrics.spacing.md))
            .py(px(0.0))
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_full()
            .bg(self.colors.panel)
            .text_color(self.colors.text)
            .child(format!("{}", web_sessions.len()));

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
                    .text_color(self.colors.muted)
                    .child("Web Sessions")
                    .child(web_count),
            )
            .child(web_list)
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

#[derive(Default)]
struct WebSessionsCache {
    sessions: Vec<WebSessionInfo>,
    loading: bool,
    last_error: Option<String>,
    last_fetch: Option<Instant>,
    selected_id: Option<String>,
}

fn web_sessions_state() -> &'static Mutex<WebSessionsCache> {
    static STATE: OnceLock<Mutex<WebSessionsCache>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(WebSessionsCache::default()))
}

fn web_sessions_snapshot() -> (Vec<WebSessionInfo>, Option<String>, bool, Option<String>) {
    if let Ok(state) = web_sessions_state().lock() {
        (
            state.sessions.clone(),
            state.selected_id.clone(),
            state.loading,
            state.last_error.clone(),
        )
    } else {
        (Vec::new(), None, false, Some("Web sessions unavailable.".to_string()))
    }
}

fn maybe_refresh_web_sessions(cx: &mut Context<ShellView>) {
    let should_refresh = if let Ok(mut state) = web_sessions_state().lock() {
        let stale = state
            .last_fetch
            .map(|last| last.elapsed() >= WEB_SESSIONS_REFRESH)
            .unwrap_or(true);
        if !state.loading && stale {
            state.loading = true;
            true
        } else {
            false
        }
    } else {
        false
    };

    if !should_refresh {
        return;
    }

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
                state.loading = false;
                state.last_fetch = Some(Instant::now());
                match result {
                    Ok(sessions) => {
                        state.sessions = sessions;
                        state.last_error = None;
                        if let Some(selected) = state.selected_id.as_ref() {
                            if !state.sessions.iter().any(|session| &session.id == selected) {
                                state.selected_id =
                                    state.sessions.first().map(|session| session.id.clone());
                            }
                        } else {
                            state.selected_id =
                                state.sessions.first().map(|session| session.id.clone());
                        }
                    }
                    Err(err) => {
                        state.last_error = Some(err.to_string());
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
