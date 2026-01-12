use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};

use ctx_core::models::{MessageRole, SessionTurnStatus};
use gpui::{
    div, img, linear_color_stop, linear_gradient, list, prelude::*, px, AnyElement, App,
    ClickEvent, Context, Edges, ElementId, Element, Image, ImageFormat, InteractiveElement,
    ListState, ObjectFit, Pixels, Rgba, SharedString, Stateful, StatefulInteractiveElement,
    StyleRefinement, WeakEntity, Window,
};
use gpui_component::{Icon, IconName, StyledExt};
use gpui_component::text::{InlineCodeStyle, TextView, TextViewStyle};

use crate::theme::{ThemeColors, ThemeMetrics};

use super::super::models::{
    attachment_cache_key, MessageAttachment, ThreadItem, ThreadListItem, ThreadToolItem,
};
use super::super::state::ShellView;

fn markdown_view(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    colors: ThemeColors,
    is_dark: bool,
    copy_prefix: impl Into<String>,
    weak: WeakEntity<ShellView>,
) -> TextView {
    let style = text_style(colors, is_dark);
    let colors_copy = colors;
    let prefix = copy_prefix.into();
    let weak_for_actions = weak.clone();
    TextView::markdown(id, text)
        .style(style)
        .code_block_actions(move |block, _window, cx| {
            let code = block.code();
            let span_key = block
                .span
                .as_ref()
                .map(|span| format!("{}:{}-{}", prefix, span.start, span.end))
                .unwrap_or_else(|| format!("{}:{}", prefix, code.len()));
            let copied = weak_for_actions
                .update(cx, |view, _| view.copied_flag(&span_key))
                .unwrap_or(false);
            let weak_click = weak_for_actions.clone();
            let code_text = code.to_string();
            let key = span_key.clone();
            let icon = if copied {
                IconName::Check
            } else {
                IconName::Copy
            };
            let button = with_click(
                div()
                    .ml_auto()
                    .p(px(4.0))
                    .rounded(px(4.0))
                    .bg(Rgba {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.06,
                    })
                    .border_1()
                    .border_color(colors_copy.border)
                    .cursor_pointer()
                    .opacity(if copied { 0.95 } else { 0.6 })
                    .hover(|style| {
                        style.opacity(1.0).bg(Rgba {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.10,
                        })
                    })
                    .id(format!("code-copy-{key}")),
            click_handler(
                weak_click.clone(),
                move |view, _ev, _window, cx| {
                    view.copy_text_with_key(key.clone(), code_text.clone(), cx);
                },
                ),
            )
            .child(
                Icon::new(icon)
                    .size(px(14.0))
                    .text_color(colors_copy.text),
            );

            div()
                .absolute()
                .top(px(8.0))
                .left(px(12.0))
                .right(px(12.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(button)
                .into_any_element()
        })
}

fn click_handler(
    weak: WeakEntity<ShellView>,
    f: impl Fn(&mut ShellView, &ClickEvent, &mut Window, &mut Context<ShellView>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |ev, window, cx_app| {
        weak.update(cx_app, |view, cx| f(view, ev, window, cx)).ok();
    }
}

fn with_click<T>(el: Stateful<T>, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Stateful<T>
where
    T: InteractiveElement + Element,
{
    el.on_click(handler)
}

fn text_style(colors: ThemeColors, is_dark: bool) -> TextViewStyle {
    let inline_bg = colors.panel;
    let border = colors.border;
    let code_bg = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.22,
    };
    let inline = InlineCodeStyle {
        background_color: Some(inline_bg.into()),
        border_color: Some(border.into()),
        border_width: px(1.0),
        border_radius: px(4.0),
        padding_x: px(4.0),
        padding_y: px(2.0),
        ..Default::default()
    };
    let code_block = StyleRefinement::default()
        .paddings(Edges {
            top: px(28.0),
            right: px(12.0),
            bottom: px(12.0),
            left: px(12.0),
        })
        .rounded(px(10.0))
        .bg(code_bg)
        .border(px(1.0))
        .border_color(Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.08,
        });
    let mut style = TextViewStyle::default()
        .inline_code(inline)
        .code_block(code_block);
    style.is_dark = is_dark;
    style
}

pub(super) struct ThreadListView<'a> {
    pub(super) shell: &'a ShellView,
    pub(super) items: &'a [ThreadListItem],
    pub(super) list_state: &'a ListState,
    pub(super) new_item_count: usize,
}

impl<'a> ThreadListView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let colors = self.shell.colors;
        let is_dark = self.shell.is_dark;
        let weak_view = cx.entity().downgrade();
        let list_weak = weak_view.clone();
        let copied_flags = self.shell.copied_flags.clone();
        let items = self.items.to_vec();
        let expanded_turn_headers = self.shell.expanded_turn_headers.clone();
        let expanded_messages = self.shell.expanded_messages.clone();
        let expanded_turn_details = self.shell.expanded_turn_details.clone();
        let expanded_tools = self.shell.expanded_tools.clone();
        let attachment_images = self.shell.composer_attachment_images.clone();
        let attachment_loading = self.shell.composer_attachment_loading.clone();
        let attachment_failed = self.shell.attachment_fetch_failed.clone();
        let list_attachment_images = attachment_images.clone();
        let list_attachment_loading = attachment_loading.clone();
        let list_attachment_failed = attachment_failed.clone();
        let list = list(self.list_state.clone(), move |index, _window, cx| {
            let Some(item) = items.get(index) else {
                return div().into_any_element();
            };
            render_thread_item(
                item.clone(),
                colors,
                &expanded_turn_headers,
                &expanded_messages,
                &expanded_turn_details,
                &expanded_tools,
                is_dark,
                copied_flags.clone(),
                list_attachment_images.clone(),
                list_attachment_loading.clone(),
                list_attachment_failed.clone(),
                list_weak.clone(),
                cx,
            )
        });

        let sticky = if let Some(header) = &self.shell.sticky_turn_header {
            if self.shell.sticky_turn_header_at_top {
                div().into_any_element()
            } else {
                div()
                    .absolute()
                    .top(px(metrics.spacing.sm))
                    .left(px(metrics.spacing.sm))
                    .right(px(metrics.spacing.sm))
                    .child(
                        div()
                            .px(px(metrics.spacing.md))
                            .py(px(metrics.spacing.sm))
                            .bg(colors.panel)
                            .border_1()
                            .border_color(colors.border)
                            .rounded_md()
                            .child(
                                markdown_view(
                                    "sticky-turn-header",
                                    header.content.clone(),
                                    colors,
                                    is_dark,
                                    "sticky-turn-header",
                                    weak_view.clone(),
                                )
                                .selectable(true),
                            )
                            .child(render_attachments(
                                &header.attachments,
                                colors,
                                &attachment_images,
                                &attachment_loading,
                                &attachment_failed,
                                AttachmentVariant::Header,
                            )),
                    )
                    .into_any_element()
            }
        } else {
            div().into_any_element()
        };

        let overlay = if self.new_item_count > 0 {
            let label = if self.new_item_count == 1 {
                "1 new item".to_string()
            } else {
                format!("{} new items", self.new_item_count)
            };
            with_click(
                div()
                    .absolute()
                    .bottom(px(metrics.spacing.md))
                    .right(px(metrics.spacing.md))
                    .px(px(metrics.spacing.lg))
                    .py(px(metrics.spacing.sm))
                    .rounded_full()
                    .bg(colors.accent)
                    .text_color(colors.panel)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.9))
                    .id("jump-to-latest"),
                click_handler(
                    weak_view.clone(),
                    |view, ev, window, cx| view.on_jump_to_latest_click(ev, window, cx),
                ),
            )
            .child(label)
            .into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .id("thread-list")
            .flex()
            .flex_col()
            .relative()
            .overflow_hidden()
            .child(sticky)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .child(list.w_full().h_full()),
            )
            .child(overlay)
    }
}

fn render_thread_item(
    item: ThreadListItem,
    colors: ThemeColors,
    expanded_turn_headers: &std::collections::HashMap<String, bool>,
    expanded_messages: &std::collections::HashMap<String, bool>,
    expanded_turn_details: &std::collections::HashMap<String, bool>,
    expanded_tools: &std::collections::HashMap<String, bool>,
    is_dark: bool,
    copied_flags: std::collections::HashMap<String, Instant>,
    attachment_images: HashMap<String, Arc<Image>>,
    attachment_loading: HashSet<String>,
    attachment_failed: HashSet<String>,
    weak_view: WeakEntity<ShellView>,
    cx: &mut App,
) -> AnyElement {
    let metrics = ThemeMetrics::default();
    match item {
        ThreadListItem::TurnHeader { id, header } => {
            let is_long =
                header.plain_text.split('\n').count() > 4 || header.plain_text.len() > 280;
            let expanded = expanded_turn_headers.get(&id).copied().unwrap_or(!is_long);
            let copy_key = format!("turn-header:{id}");
            let copied = copied_flags.contains_key(&copy_key);
            let content = markdown_view(
                format!("turn-header-{id}"),
                header.content.clone(),
                colors,
                is_dark,
                copy_key.clone(),
                weak_view.clone(),
            )
            .selectable(true);
            let content = if expanded {
                content.into_any_element()
            } else {
                div()
                    .relative()
                    .max_h(px(1.45 * 3.5 * 16.0))
                    .overflow_hidden()
                    .child(content)
                    .child(fade_overlay(px(36.0), colors.bg))
                    .into_any_element()
            };
            let attachments = render_attachments(
                &header.attachments,
                colors,
                &attachment_images,
                &attachment_loading,
                &attachment_failed,
                AttachmentVariant::Header,
            );
            let copy_button = if header.content.trim().is_empty() {
                div().into_any_element()
            } else {
                let copy_text = header.content.clone();
                let button = div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .p(px(4.0))
                    .rounded(px(4.0))
                    .bg(Rgba {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.06,
                    })
                    .border_1()
                    .border_color(colors.border)
                    .opacity(if copied { 0.95 } else { 0.4 })
                    .hover(|style| {
                        style.opacity(1.0).bg(Rgba {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.10,
                        })
                    })
                    .cursor_pointer()
                    .id(format!("turn-header-copy-{id}"))
                    .child(
                        Icon::new(if copied { IconName::Check } else { IconName::Copy })
                            .size(px(12.0))
                            .text_color(colors.text),
                    );
                with_click(
                    button,
                    click_handler(weak_view.clone(), move |view, _ev, _window, cx| {
                        cx.stop_propagation();
                        view.copy_text_with_key(copy_key.clone(), copy_text.clone(), cx);
                    }),
                )
                .into_any_element()
            };
            let toggle_id = id.clone();
            let container = with_click(
                div()
                    .relative()
                    .border_1()
                    .border_color(Rgba {
                        a: 0.08,
                        ..colors.border
                    })
                    .bg(Rgba {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.02,
                    })
                    .rounded(px(10.0))
                    .p(px(8.0))
                    .cursor_pointer()
                    .id(format!("turn-header-container-{id}"))
                    .child(copy_button)
                    .child(content)
                    .child(attachments),
                click_handler(
                    weak_view.clone(),
                    move |view, _ev, _window, cx| {
                        view.on_toggle_turn_header(toggle_id.clone(), cx);
                    },
                ),
            );
            div()
                .id(id)
                .px(px(metrics.spacing.sm))
                .pt(px(metrics.spacing.xs))
                .pb(px(metrics.spacing.md))
                .child(container)
                .into_any_element()
        }
        ThreadListItem::Item(ThreadItem::Message {
            id,
            role,
            content,
            attachments,
            ..
        }) => {
            let is_long = content.split('\n').count() > 20 || content.len() > 1500;
            let expanded = expanded_messages.get(&id).copied().unwrap_or(!is_long);
            let (bubble_bg, bubble_border) = match role {
                MessageRole::User => (
                    Rgba {
                        r: colors.panel.r,
                        g: colors.panel.g,
                        b: colors.panel.b,
                        a: (colors.panel.a * 0.84) + 0.16,
                    },
                    colors.border,
                ),
                MessageRole::Assistant => (
                    Rgba {
                        r: colors.panel.r,
                        g: colors.panel.g,
                        b: colors.panel.b,
                        a: (colors.panel.a * 0.88) + 0.12,
                    },
                    colors.border,
                ),
                MessageRole::System => (
                    Rgba {
                        r: colors.panel.r,
                        g: colors.panel.g,
                        b: colors.panel.b,
                        a: (colors.panel.a * 0.88) + 0.12,
                    },
                    Rgba {
                        r: colors.border.r,
                        g: colors.border.g,
                        b: colors.border.b,
                        a: (colors.border.a * 0.7) + 0.3,
                    },
                ),
            };
            let text = markdown_view(
                format!("msg-{id}"),
                content.clone(),
                colors,
                is_dark,
                format!("msg:{id}"),
                weak_view.clone(),
            )
            .selectable(true);
            let body = if expanded {
                text.into_any_element()
            } else {
                div()
                    .relative()
                    .max_h(px(1.45 * 8.0 * 16.0))
                    .overflow_hidden()
                    .child(text)
                    .child(fade_overlay(px(48.0), bubble_bg))
                    .into_any_element()
            };
            let toggle_id = id.clone();
            let toggle = if is_long {
                let label = if expanded { "Show less" } else { "Show more" };
                with_click(
                    div()
                        .text_sm()
                        .text_color(colors.accent)
                        .cursor_pointer()
                        .id(format!("msg-toggle-{id}"))
                        .child(label),
                    click_handler(
                        weak_view.clone(),
                        move |view, _ev, _window, cx| {
                            cx.stop_propagation();
                            view.on_toggle_message(toggle_id.clone(), cx)
                        },
                    ),
                )
                .into_any_element()
            } else {
                div().into_any_element()
            };
            let attachments = render_attachments(
                &attachments,
                colors,
                &attachment_images,
                &attachment_loading,
                &attachment_failed,
                AttachmentVariant::Message,
            );
            let role_label = format!("{:?}", role).to_lowercase();
            let mut bubble = div()
                .bg(bubble_bg)
                .border_1()
                .border_color(bubble_border)
                .rounded(px(6.0))
                .p(px(10.0))
                .max_w(px(1200.0))
                .flex()
                .flex_col()
                .gap(px(metrics.spacing.xs))
                .child(div().text_sm().text_color(colors.muted).child(role_label))
                .child(body)
                .child(attachments)
                .child(toggle);
            bubble = match role {
                MessageRole::User => bubble.ml_auto(),
                MessageRole::Assistant => bubble.mr_auto(),
                MessageRole::System => bubble.mx_auto(),
            };
            div()
                .id(id.clone())
                .px(px(metrics.spacing.md))
                .py(px(metrics.spacing.sm))
                .child(bubble)
                .into_any_element()
        }
        ThreadListItem::Item(ThreadItem::Assistant { id, content, .. }) => div()
            .id(id.clone())
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .child(
                markdown_view(
                    format!("assistant-{id}"),
                    content,
                    colors,
                    is_dark,
                    format!("assistant:{id}"),
                    weak_view.clone(),
                )
                .selectable(true),
            )
            .into_any_element(),
        ThreadListItem::Item(ThreadItem::Thought { id, content, .. }) => div()
            .id(id.clone())
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .child(
                div()
                    .px(px(metrics.spacing.xs))
                    .pb(px(metrics.spacing.sm))
                    .text_sm()
                    .text_color(colors.muted)
                    .italic()
                    .child("Thought")
                    .child(
                        div()
                            .mt(px(metrics.spacing.xs))
                            .p(px(8.0))
                            .border_1()
                            .border_color(colors.border)
                            .rounded(px(10.0))
                            .bg(Rgba {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.22,
                            })
                            .child(
                                markdown_view(
                                    format!("thought-{id}"),
                                    content,
                                    colors,
                                    is_dark,
                                    format!("thought:{id}"),
                                    weak_view.clone(),
                                )
                                .selectable(true),
                            ),
                    ),
            )
            .into_any_element(),
        ThreadListItem::Item(ThreadItem::TurnStatus {
            id,
            status,
            custom_status,
            started_at,
            updated_at,
            assistant_messages_content,
            ..
        }) => {
            let label = custom_status.unwrap_or_else(|| format!("{:?}", status));
            let is_completed = matches!(status, SessionTurnStatus::Completed);
            let elapsed_ms = (updated_at - started_at).num_milliseconds();
            let elapsed_label = format_elapsed(elapsed_ms);
            let has_content = assistant_messages_content
                .as_ref()
                .map(|c| !c.trim().is_empty())
                .unwrap_or(false);
            let copy_key = format!("turn-status:{id}");
            let copied = copied_flags.contains_key(&copy_key);
            let copy_button = if is_completed && has_content {
                let text = assistant_messages_content.unwrap_or_default();
                let button = div()
                    .p(px(2.0))
                    .rounded(px(4.0))
                    .opacity(if copied { 0.95 } else { 0.6 })
                    .hover(|style| style.opacity(1.0))
                    .cursor_pointer()
                    .id(format!("turn-status-copy-{id}"))
                    .child(
                        Icon::new(if copied { IconName::Check } else { IconName::Copy })
                            .size(px(12.0))
                            .text_color(colors.muted),
                    );
                with_click(
                    button,
                    click_handler(weak_view.clone(), move |view, _ev, _window, cx| {
                        cx.stop_propagation();
                        view.copy_text_with_key(copy_key.clone(), text.clone(), cx);
                    }),
                )
                .into_any_element()
            } else {
                div().into_any_element()
            };
            div()
                .id(id)
                .px(px(metrics.spacing.md))
                .py(px(metrics.spacing.xs))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(metrics.spacing.xs))
                        .px(px(metrics.spacing.xs))
                        .pb(px(metrics.spacing.sm))
                        .pt(px(metrics.spacing.xxs))
                        .text_sm()
                        .text_color(colors.muted)
                        .child(div().text_color(colors.text).child(label))
                        .child(div().text_color(colors.muted).child("·"))
                        .child(div().child(elapsed_label))
                        .child(copy_button),
                )
                .into_any_element()
        }
        ThreadListItem::Item(ThreadItem::Tool(tool)) => {
            let expanded = expanded_tools.get(&tool.id).copied().unwrap_or(false);
            render_tool_item(
                tool,
                colors,
                is_dark,
                expanded,
                &copied_flags,
                weak_view.clone(),
                cx,
            )
        }
        ThreadListItem::Item(ThreadItem::ToolGroup { id, tools, .. }) => {
            let expanded = expanded_turn_details.get(&id).copied().unwrap_or(false);
            let label = if tools.is_empty() {
                "Activity".to_string()
            } else {
                let total = tools.len();
                format!("{total} tool{}", if total == 1 { "" } else { "s" })
            };
            let tools_list = if expanded {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(metrics.spacing.sm))
                    .children(tools.into_iter().map(|tool| {
                        let expanded_tool = expanded_tools.get(&tool.id).copied().unwrap_or(false);
                        render_tool_item(
                            tool,
                            colors,
                            is_dark,
                            expanded_tool,
                            &copied_flags,
                            weak_view.clone(),
                            cx,
                        )
                    }))
                    .into_any_element()
            } else {
                div().into_any_element()
            };
            div()
                .id(id.clone())
                .px(px(metrics.spacing.md))
                .py(px(metrics.spacing.xs))
                .flex()
                .flex_col()
                .gap(px(metrics.spacing.xs))
                .child(
                    with_click(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(metrics.spacing.xs))
                            .px(px(metrics.spacing.xxs))
                            .py(px(metrics.spacing.xxs))
                            .text_sm()
                            .text_color(colors.muted)
                            .italic()
                            .cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(Rgba {
                                        r: 1.0,
                                        g: 1.0,
                                        b: 1.0,
                                        a: 0.02,
                                    })
                                    .rounded(px(8.0))
                            })
                            .id(format!("turn-tools-{id}"))
                            .child(div().flex().min_w(px(0.0)).child(label))
                            .child(div().ml_auto().child(if expanded { "▴" } else { "▾" })),
                        click_handler(
                            weak_view.clone(),
                            move |view, _ev, _window, cx| {
                                view.on_toggle_turn_details(id.clone(), cx);
                            },
                        ),
                    ),
                )
                .child(
                    div()
                        .when(expanded, |this| {
                            this.border_1()
                                .border_color(colors.border)
                                .rounded(px(10.0))
                                .bg(Rgba {
                                    r: 1.0,
                                    g: 1.0,
                                    b: 1.0,
                                    a: 0.02,
                                })
                                .p(px(8.0))
                        })
                        .child(tools_list),
                )
                .into_any_element()
        }
        ThreadListItem::Item(ThreadItem::Spacer { id, .. }) => div().id(id).into_any_element(),
    }
}

fn fade_overlay(height: Pixels, bg: Rgba) -> AnyElement {
    div()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .h(height)
        .bg(linear_gradient(
            180.0,
            linear_color_stop(Rgba { a: 0.0, ..bg }, 0.0),
            linear_color_stop(Rgba { a: bg.a, ..bg }, 1.0),
        ))
        .into_any_element()
}

fn format_elapsed(ms: i64) -> String {
    let total_ms = ms.max(0);
    let total_secs = total_ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins > 0 {
        format!("{mins}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

fn render_tool_item(
    tool: ThreadToolItem,
    colors: ThemeColors,
    is_dark: bool,
    expanded: bool,
    copied_flags: &std::collections::HashMap<String, Instant>,
    weak_view: WeakEntity<ShellView>,
    _cx: &mut App,
) -> AnyElement {
    let metrics = ThemeMetrics::default();
    let tool_id = tool.id.clone();
    let title = tool.title.trim().to_string();
    let mut title_iter = title.splitn(2, ' ');
    let verb = title_iter.next().unwrap_or_default().to_string();
    let rest = title_iter.next().unwrap_or("").trim().to_string();
    let header = div()
        .flex()
        .items_center()
        .gap(px(metrics.spacing.xs))
        .text_sm()
        .text_color(colors.muted)
        .italic()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(metrics.spacing.xxs))
                .min_w(px(0.0))
                .child(
                    div()
                        .text_color(colors.text)
                        .child(verb)
                        .overflow_hidden()
                        .text_ellipsis(),
                )
                .child(
                    div()
                        .text_color(colors.muted)
                        .child(if rest.is_empty() { "".to_string() } else { format!(" · {}", rest) }),
                )
                .child(
                    div()
                        .text_color(colors.muted)
                        .child(if tool.status.is_empty() {
                            "".to_string()
                        } else {
                            format!(" · {}", tool.status)
                        }),
                ),
        )
        .child(div().ml_auto().child(if expanded { "▴" } else { "▾" }));
    let body = if expanded {
        let output = if tool.output_text.is_empty() {
            div().into_any_element()
        } else {
            markdown_view(
                format!("tool-{}", tool.id),
                tool.output_text.clone(),
                colors,
                is_dark,
                format!("tool-output:{}", tool.id),
                weak_view.clone(),
            )
            .selectable(true)
            .into_any_element()
        };
        let input = tool
            .input
            .as_ref()
            .map(|value| format!("{}", value))
            .unwrap_or_default();
        let input_row = if input.is_empty() {
            div().into_any_element()
        } else {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child(format!("Input: {}", input))
                .into_any_element()
        };
        let copy_key = format!("tool:{}", tool.id);
        let copied = copied_flags.contains_key(&copy_key);
        let copy_button = if tool.output_text.trim().is_empty() {
            div().into_any_element()
        } else {
            let text_to_copy = tool.output_text.clone();
            let button = div()
                .absolute()
                .top(px(metrics.spacing.xs))
                .right(px(metrics.spacing.xs))
                .p(px(4.0))
                .rounded(px(4.0))
                .bg(Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.06,
                })
                .border_1()
                .border_color(colors.border)
                .opacity(if copied { 0.95 } else { 0.6 })
                .hover(|style| style.opacity(1.0))
                .cursor_pointer()
                .id(format!("tool-copy-{}", tool.id))
                .child(
                    Icon::new(if copied { IconName::Check } else { IconName::Copy })
                        .size(px(12.0))
                        .text_color(colors.text),
                );
            with_click(
                button,
                click_handler(weak_view.clone(), move |view, _ev, _window, cx| {
                    view.copy_text_with_key(copy_key.clone(), text_to_copy.clone(), cx);
                }),
            )
            .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.xs))
            .relative()
            .child(copy_button)
            .child(input_row)
            .child(output)
            .into_any_element()
    } else {
        div().into_any_element()
    };
    let header_row = with_click(
        div()
            .flex()
            .items_center()
            .gap(px(metrics.spacing.xs))
            .cursor_pointer()
            .id(format!("tool-header-{}", tool.id))
            .text_sm()
            .text_color(colors.muted)
            .hover(|style| {
                style
                    .bg(Rgba {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.02,
                    })
                    .rounded(px(8.0))
            })
            .child(header),
        click_handler(
            weak_view.clone(),
            move |view, _ev, _window, cx| {
                view.on_toggle_tool(tool_id.clone(), cx);
            },
        ),
    );
    div()
        .id(tool.id.clone())
        .px(px(metrics.spacing.sm))
        .py(px(metrics.spacing.sm))
        .flex()
        .flex_col()
        .gap(px(metrics.spacing.xxs))
        .child(header_row)
        .child(
            div()
                .when(expanded, |this| {
                    this.border_1()
                        .border_color(colors.border)
                        .rounded(px(10.0))
                        .bg(Rgba {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.02,
                        })
                        .p(px(8.0))
                })
                .child(body),
        )
        .into_any_element()
}

enum AttachmentVariant {
    Header,
    Message,
}

fn render_attachments(
    attachments: &[MessageAttachment],
    colors: ThemeColors,
    images: &HashMap<String, Arc<Image>>,
    loading: &HashSet<String>,
    failed: &HashSet<String>,
    variant: AttachmentVariant,
) -> AnyElement {
    if attachments.is_empty() {
        return div().into_any_element();
    }
    let (gap, margin_top) = match variant {
        AttachmentVariant::Header => (6.0, 8.0),
        AttachmentVariant::Message => (8.0, 8.0),
    };
    let row = attachments.iter().enumerate().fold(
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(gap)),
        |row, (i, attachment)| {
            row.child(render_attachment(
                i,
                attachment,
                colors,
                images,
                loading,
                failed,
                &variant,
            ))
        },
    );
    div().flex().flex_col().mt(px(margin_top)).child(row).into_any_element()
}

fn render_attachment(
    _index: usize,
    attachment: &MessageAttachment,
    colors: ThemeColors,
    images: &HashMap<String, Arc<Image>>,
    loading: &HashSet<String>,
    failed: &HashSet<String>,
    variant: &AttachmentVariant,
) -> AnyElement {
    let cache_key = attachment_cache_key(attachment);
    let image = images
        .get(&cache_key)
        .cloned()
        .or_else(|| match attachment {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                ..
            } => inline_attachment_image(mime_type, data_base64),
            _ => None,
        });
    let is_loading = loading.contains(&cache_key);
    let is_failed = failed.contains(&cache_key);
    let (width, height, radius, fit) = match variant {
        AttachmentVariant::Header => (44.0, 44.0, 8.0, ObjectFit::Cover),
        AttachmentVariant::Message => (240.0, 180.0, 6.0, ObjectFit::Contain),
    };

    let content: AnyElement = if let Some(image) = image {
        img(image)
            .w(px(width))
            .h(px(height))
            .rounded(px(radius))
            .border_1()
            .border_color(colors.border)
            .object_fit(fit)
            .into_any_element()
    } else {
        let icon = if is_failed {
            IconName::TriangleAlert
        } else {
            IconName::Frame
        };
        div()
            .w(px(width))
            .h(px(height))
            .rounded(px(radius))
            .border_1()
            .border_color(colors.border)
            .bg(Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.02,
            })
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .child(
                Icon::new(icon)
                    .size(px(if matches!(variant, AttachmentVariant::Header) {
                        14.0
                    } else {
                        16.0
                    }))
                    .text_color(colors.muted),
            )
            .into_any_element()
    };

    let mut container = div()
        .relative()
        .overflow_hidden()
        .rounded(px(radius))
        .child(content);

    container = match variant {
        AttachmentVariant::Header => container.w(px(width)).h(px(height)),
        AttachmentVariant::Message => container.w(px(width)).h(px(height)),
    };

    if is_loading {
        container = container.opacity(0.8);
    }
    container.into_any_element()
}

fn inline_attachment_image(mime_type: &str, data_base64: &str) -> Option<Arc<Image>> {
    let format = ImageFormat::from_mime_type(mime_type).unwrap_or(ImageFormat::Png);
    let bytes = STANDARD.decode(data_base64).ok()?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
