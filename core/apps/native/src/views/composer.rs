use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use gpui::{
    anchored, AnchoredPositionMode, AnyElement, BorderStyle, BoxShadow, ClickEvent, Context,
    Corner, CursorStyle, ElementId, Entity, ExternalPaths, FontWeight, Hsla, Image, ImageFormat,
    InteractiveElement, MouseButton, ObjectFit, Rgba, Transformation, Window, div, img, prelude::*, px, point, radians,
};
use gpui_component::{ElementExt, input::Input};
use gpui_component::scroll::ScrollableElement;

use crate::automation_tree;

use ctx_core::models::{MessageAttachment, SessionEventType};
use ctx_providers::adapters::ProviderHealth;

use super::super::harness_catalog::{harness_catalog, harness_entry, HarnessCatalogEntry};
use super::super::model_effort::{
    build_model_catalog, compose_model_id, derive_full_model_id_for_base, format_effort_label,
    parse_model_id, pick_default_effort, ModelCatalog, ModelOption,
};
use super::super::state::composer::{ComposerAutocompleteItemKind, MAX_TRACKS_PER_PROVIDER};
use ctx_client::InstallStateKind;
use ctx_client::ProviderOptions;

use super::super::icons::{Icon, IconName};
use super::super::state::{ComposerMenuId, ComposerVerbosity, ShellView, WorkbenchModeId};

pub(super) enum ComposerVariant {
    NewTask,
    ActiveSession,
}

pub(super) struct ComposerView<'a> {
    pub(super) shell: &'a ShellView,
    pub(super) variant: ComposerVariant,
}

const MENU_DESC_HARNESS: &str = "Agent harnesses are the low-level wrappers around models that provide the basic plumbing to allow the model to interact with the workspace. This normally includes features like filesystem access, shell access, configurations to set up MCP servers, and more. Despite similiarities between them, different harnesses will have varying tools, capabilities, and performance - even if used with the same underlying models. From here, you can install agent harnesses you haven't used before, switch between them for new tasks, and even run multiple agent harnesses in parallel on the same task. This can be useful to compare performance or to survey multiple different approaches.";
const MENU_DESC_MODEL: &str = "You can switch between different models here. Model selection offers a tradeoff between cost, latency, and intelligence - but it also offers an opportunity to leverage the differences in their weights for collaboration. Even if two different models score similarly on popular coding benchmarks, they might have different \"habits\" - or biases. This means that if you are working on a pernicious bug fix, you might want multiple different models to both look at the problem from a different angle.";
const MENU_DESC_EFFORT: &str = "Some models have a \"thinking effort\" or \"reasoning effort\" setting, while others do not. The effort level simply corresponds to how many tokens a model spends on thinking while solving a problem. Models that offer high or extra high can sometimes be very powerful, at the expense of latency and cost. However, you can also experience an unintended negative consequence from extra high thinking: if the model is emitting lots of thinking tokens that don't add much value, this will cause the context window to fill up faster (not just from thinking tokens alone, but also from more excessive tool calls like reading files). Performance on coding tasks declines as context increases beyond the minimum context needed to solve the problem, so effort level is a key lever in tuning your agent for optimal performance.";
const MENU_DESC_MODE: &str = "Modes are basically just prompts, sometimes combined with access limitations. For example, the review mode is nothing more than prompting the agent to tell it to review the code and putting it in a read-only access level. That sounds fairly simple, but there is a hidden benefit: developers who build agent harnesses and models in conjunction will often train their custom model to use their bespoke harness, including its different modes. So in a way, this prompt can be more than just a regular prompt. It is a special prompt than has been trained on via reinforcement learning to achieve certain outcomes. For example, OpenAI trained their codex model to use their codex harness in review mode, so as to output only high value review comments with priority details. If you give the exact same prompt to a model that has not undergone the same RL, it will emit much less useful review comments. We recommend using RPIR (Research, Plan, Implement, Review) pattern for most changes except for small and easy ones.";
const MENU_DESC_ISOLATION: &str = "If you are new to using an ADE, you likely have your agents running in Local isolation mode, which basically means no isolation. In local mode, your agents work on the locally checked-out branch and could collide with other agents or your own changes. This results in dirty working branches, possible collisions, and risks of lost changes. An improvement is using git worktrees. They create a totally separate workspace that is disk-efficient. You can spin up many agents to all work in different worktrees and they won't collide with eachother. When they are done, you can approve and merge their changes back into the local working branch. This is a very powerful and resource-efficient isolation pattern. Finally there is container-level isolation. This is the most isolated environment, but it consumes many more resources: you have to run all of your processes again inside the container, and you have to copy all of the disk space. Despite the additional overhead, container-based isolation is most powerful when your agents need to test your application on the same ports. A simple example: if you have a key part of your application that always runs on port 3000 and you want your agent to be able to test it, worktrees won't save you: only one process can serve requests on that port. Containers solve that problem because you could have many agents working in different containers, and they can all claim their own port 3000 as theirs without worrying about collisions. Depending on your application, you may or may not need this. Containers of course also improved security isolation properties which worktrees cannot.";
const MENU_DESC_VERBOSITY: &str = "Verbosity controls how much activity is shown during a turn. Terse hides tools and thoughts, default shows summaries and thoughts, and verbose will eventually expand full tool details.";

fn attachment_label(att: &MessageAttachment) -> String {
    match att {
        MessageAttachment::Image { name, mime_type, .. }
        | MessageAttachment::ImageRef { name, mime_type, .. } => name
            .as_deref()
            .map(|value| value.to_string())
            .unwrap_or_else(|| mime_type.clone()),
    }
}

fn image_format_for_mime(mime: &str) -> Option<ImageFormat> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        "image/webp" => Some(ImageFormat::Webp),
        "image/gif" => Some(ImageFormat::Gif),
        "image/svg+xml" => Some(ImageFormat::Svg),
        "image/bmp" => Some(ImageFormat::Bmp),
        "image/tiff" => Some(ImageFormat::Tiff),
        _ => None,
    }
}

fn split_path(path: &str) -> (String, String) {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let file_name = parts
        .last()
        .map(|value| value.to_string())
        .unwrap_or_else(|| normalized.clone());
    let dir_name = if parts.len() > 1 {
        parts[..parts.len() - 1].join("/")
    } else {
        String::new()
    };
    (file_name, dir_name)
}

fn image_from_attachment(att: &MessageAttachment, shell: &ShellView) -> Option<Arc<Image>> {
    match att {
        MessageAttachment::Image {
            mime_type,
            data_base64,
            ..
        } => {
            let format = image_format_for_mime(mime_type)?;
            let bytes = STANDARD.decode(data_base64).ok()?;
            Some(Arc::new(Image::from_bytes(format, bytes)))
        }
        MessageAttachment::ImageRef { blob_id, .. } => {
            shell.composer_attachment_images.get(blob_id).cloned()
        }
    }
}

fn harness_logo(entry: &HarnessCatalogEntry, is_dark: bool) -> Arc<Image> {
    if is_dark && entry.invert_in_dark {
        entry.inverted_image.clone()
    } else {
        entry.image.clone()
    }
}

fn tint(color: Rgba, alpha: f32) -> Rgba {
    Rgba {
        r: color.r,
        g: color.g,
        b: color.b,
        a: alpha,
    }
}

fn model_options_from_models_value(raw: Option<&serde_json::Value>) -> Vec<ModelOption> {
    let raw = match raw {
        Some(raw) => raw,
        None => return Vec::new(),
    };
    let list = raw
        .get("availableModels")
        .or_else(|| raw.get("available_models"))
        .or_else(|| raw.get("models"))
        .or_else(|| Some(raw));
    let Some(list) = list.and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|value| {
            let id = value
                .get("modelId")
                .or_else(|| value.get("model_id"))
                .or_else(|| value.get("id"))
                .or_else(|| value.get("name"))
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())?;
            let name = value
                .get("name")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            Some(ModelOption { id, name })
        })
        .collect()
}

fn model_options_from_provider_options(opts: Option<&ProviderOptions>) -> Vec<ModelOption> {
    model_options_from_models_value(opts.and_then(|opts| opts.models.as_ref()))
}

fn current_model_id_from_models_value(raw: Option<&serde_json::Value>) -> Option<String> {
    let raw = raw?;
    let current = raw
        .get("currentModelId")
        .or_else(|| raw.get("current_model_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if current.is_some() {
        return current;
    }
    let list = raw
        .get("availableModels")
        .or_else(|| raw.get("available_models"))
        .or_else(|| raw.get("models"))
        .or_else(|| Some(raw))?;
    let list = list.as_array()?;
    let first = list.first()?;
    first
        .get("modelId")
        .or_else(|| first.get("model_id"))
        .or_else(|| first.get("id"))
        .or_else(|| first.get("name"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn models_value_from_session_events(shell: &ShellView) -> Option<&serde_json::Value> {
    shell.session_events.iter().rev().find_map(|event| {
        if !matches!(&event.event_type, SessionEventType::Init) {
            return None;
        }
        event.payload_json.get("models")
    })
}

fn fallback_model_options(provider_id: &str) -> Vec<ModelOption> {
    match provider_id {
        "gemini" => vec![
            ModelOption {
                id: "gemini-2.0-flash".to_string(),
                name: Some("Gemini 2.0 Flash".to_string()),
            },
            ModelOption {
                id: "gemini-2.0-flash-lite".to_string(),
                name: Some("Gemini 2.0 Flash Lite".to_string()),
            },
            ModelOption {
                id: "gemini-2.5-pro".to_string(),
                name: Some("Gemini 2.5 Pro".to_string()),
            },
            ModelOption {
                id: "gemini-1.5-pro".to_string(),
                name: Some("Gemini 1.5 Pro".to_string()),
            },
            ModelOption {
                id: "gemini-1.5-flash".to_string(),
                name: Some("Gemini 1.5 Flash".to_string()),
            },
        ],
        _ => Vec::new(),
    }
}

fn active_provider_id(shell: &ShellView, is_new: bool) -> Option<String> {
    if is_new {
        if let Some(track) = shell.composer_draft_tracks.first() {
            return Some(track.provider_id.clone());
        }
        return shell.composer_provider_id.clone();
    }
    shell
        .selected_session
        .and_then(|index| shell.sessions.get(index))
        .and_then(|summary| shell.session_summary_map.get(&summary.session_id))
        .map(|summary| summary.session.provider_id.clone())
        .or_else(|| shell.composer_provider_id.clone())
}

fn active_model_id(shell: &ShellView, is_new: bool) -> Option<String> {
    if is_new {
        if let Some(track) = shell.composer_draft_tracks.first() {
            if !track.model_id.trim().is_empty() {
                return Some(track.model_id.clone());
            }
        }
    }
    shell
        .composer_model_id
        .clone()
        .or_else(|| {
            shell.selected_session
                .and_then(|index| shell.sessions.get(index))
                .and_then(|summary| shell.session_summary_map.get(&summary.session_id))
                .map(|summary| summary.session.model_id.clone())
        })
}

fn active_session_model_id(
    shell: &ShellView,
    model_options: &[ModelOption],
    current_from_events: Option<String>,
) -> String {
    let session_model_id = shell
        .composer_model_id
        .clone()
        .or_else(|| {
            shell.selected_session
                .and_then(|index| shell.sessions.get(index))
                .and_then(|summary| shell.session_summary_map.get(&summary.session_id))
                .map(|summary| summary.session.model_id.clone())
        })
        .unwrap_or_default();
    let session_trimmed = session_model_id.trim().to_string();
    let current_from_events = current_from_events.unwrap_or_default();
    let option_ids = model_options
        .iter()
        .map(|opt| opt.id.as_str())
        .collect::<std::collections::HashSet<_>>();

    if session_trimmed.is_empty() || session_trimmed.eq_ignore_ascii_case("default") {
        return if current_from_events.is_empty() {
            session_trimmed
        } else {
            current_from_events
        };
    }
    if !option_ids.is_empty() && !option_ids.contains(session_trimmed.as_str()) {
        return if current_from_events.is_empty() {
            session_trimmed
        } else {
            current_from_events
        };
    }
    if !session_trimmed.is_empty() {
        return session_trimmed;
    }
    current_from_events
}

impl<'a> ComposerView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let shell = self.shell;
        let colors = shell.colors;
        let is_new = matches!(self.variant, ComposerVariant::NewTask);
        let show_model_effort = !is_new || shell.composer_draft_tracks.len() <= 1;

        let (min_height, _max_height) = if is_new {
            (px(88.0), px(380.0))
        } else {
            (px(28.0), px(220.0))
        };
        let font_size = px(13.0);
        let line_height = px(18.85);
        let pad = px(2.0);

        let input_disabled = is_new && shell.composer_start_busy;

        let mut input = Input::new(shell.active_composer_input())
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .text_size(font_size)
            .line_height(line_height)
            .px(pad)
            .py(pad)
            .w_full()
            .items_start()
            .disabled(input_disabled);
        if is_new {
            input = input.text_color(colors.text);
        }

        let mut input_wrap = div()
            .w_full()
            .relative()
            .on_children_prepainted(automation_tree::track_children_bounds(
                "composer-input",
                "input",
                Some("Composer"),
                Some("app-shell"),
            ))
            .id("composer-input-wrap")
            .on_key_down(cx.listener(ShellView::on_composer_key_down))
            .on_key_up(cx.listener(ShellView::on_composer_key_up))
            .on_mouse_up(MouseButton::Left, cx.listener(ShellView::on_composer_mouse_up))
            .on_click(cx.listener(ShellView::focus_composer))
            .on_prepaint({
                let view = cx.entity();
                move |bounds, window, cx| {
                    view.update(cx, |view, cx| {
                        view.update_composer_input_bounds(bounds, window, cx);
                    });
                }
            })
            .child(input);
        if is_new {
            input_wrap = input_wrap.min_h(min_height);
        }

        let attachments = if shell.composer_attachments.is_empty() {
            div()
        } else {
            shell.composer_attachments.iter().enumerate().fold(
                div().flex().flex_row().gap(px(6.0)).flex_wrap(),
                |row, (idx, att)| {
                    let _name = attachment_label(att);
                    let thumb = div()
                        .relative()
                        .w(px(44.0))
                        .h(px(44.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(colors.border)
                        .overflow_hidden()
                        .bg(tint(colors.text, 0.02))
                        .child(
                            image_from_attachment(att, shell)
                                .map(|image| {
                                    img(image)
                                        .w_full()
                                        .h_full()
                                        .object_fit(ObjectFit::Cover)
                                        .into_any_element()
                                })
                                .unwrap_or_else(|| {
                                    div()
                                        .w_full()
                                        .h_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(Icon::new(IconName::Image, 14.0, colors.muted))
                                        .into_any_element()
                                }),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(3.0))
                                .right(px(3.0))
                                .w(px(18.0))
                                .h(px(18.0))
                                .rounded_full()
                                .border_1()
                                .border_color(tint(colors.text, 0.18))
                                .bg(Rgba {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 0.5,
                                })
                                .text_color(tint(colors.text, 0.92))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(14.0))
                                .line_height(px(16.0))
                                .cursor_pointer()
                                .hover(|style| {
                                    style.bg(Rgba {
                                        r: 0.0,
                                        g: 0.0,
                                        b: 0.0,
                                        a: 0.65,
                                    })
                                })
                                .child("×")
                                .id(("composer-attachment-remove", idx))
                                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.remove_composer_attachment(idx, cx);
                                })),
                        );
                    row.child(thumb)
                },
            )
        };

        let autocomplete_menu = if shell.composer_autocomplete.open {
            let items = shell.composer_autocomplete.items.clone();
            let active = shell.composer_autocomplete.active_index;
            let view = cx.entity();
            if items.is_empty() && shell.composer_autocomplete.loading {
                div().into_any_element()
            } else {
                let menu = if items.is_empty() {
                    div()
                        .h(px(30.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .text_size(px(13.0))
                        .text_color(tint(colors.text, 0.55))
                        .child("No matches")
                        .into_any_element()
                } else {
                    items
                        .iter()
                        .enumerate()
                        .fold(div().flex().flex_col().gap(px(2.0)), |list, (idx, item)| {
                            let is_active = idx == active;
                            let (left_text, right_text) = match item.kind {
                                ComposerAutocompleteItemKind::File => {
                                    let path = item.path.clone().unwrap_or_default();
                                    let (file, dir) = split_path(&path);
                                    (
                                        file,
                                        if dir.is_empty() {
                                            String::new()
                                        } else {
                                            format!("…/{}", dir)
                                        },
                                    )
                                }
                                ComposerAutocompleteItemKind::Slash => (
                                    item.label.clone(),
                                    item.description.clone().unwrap_or_default(),
                                ),
                            };
                            let label_left = div()
                                .text_sm()
                                .font_weight(FontWeight(600.0))
                                .text_color(tint(colors.text, 0.92))
                                .child(left_text);
                            let right = if right_text.is_empty() {
                                div().into_any_element()
                            } else {
                                div()
                                    .text_sm()
                                    .text_color(tint(colors.text, 0.38))
                                    .child(right_text)
                                    .into_any_element()
                            };
                            let mut row = div()
                                .h(px(30.0))
                                .px(px(8.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(if is_active {
                                    tint(colors.text, 0.08)
                                } else {
                                    tint(colors.text, 0.0)
                                })
                                .bg(if is_active {
                                    tint(colors.text, 0.08)
                                } else {
                                    Rgba {
                                        r: 0.0,
                                        g: 0.0,
                                        b: 0.0,
                                        a: 0.0,
                                    }
                                })
                                .id(ElementId::named_usize("composer-autocomplete-item", idx))
                                .cursor_pointer()
                                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                                    view.pick_autocomplete(idx, window, cx);
                                }))
                                .on_hover({
                                    let view = view.clone();
                                    move |hovered: &bool, _window, cx| {
                                        if *hovered {
                                            view.update(cx, |view, cx| {
                                                view.set_autocomplete_active_index(idx, cx);
                                            });
                                        }
                                    }
                                })
                                .child(
                                    div()
                                        .w(px(16.0))
                                        .h(px(16.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(Icon::new(
                                            if matches!(
                                                item.kind,
                                                ComposerAutocompleteItemKind::File
                                            ) {
                                                IconName::Artifact
                                            } else {
                                                IconName::Slash
                                            },
                                            12.0,
                                            tint(colors.accent, 0.95),
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w(px(0.0))
                                        .child(label_left),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .flex_1()
                                        .justify_end()
                                        .min_w(px(0.0))
                                        .child(right),
                                );
                            if is_active
                                && matches!(item.kind, ComposerAutocompleteItemKind::File)
                            {
                                let view = view.clone();
                                row = row.on_prepaint(move |bounds, window, cx| {
                                    view.update(cx, |view, _| {
                                        view.update_autocomplete_preview_position(bounds, window);
                                    });
                                });
                            }
                            list.child(row)
                        })
                        .into_any_element()
                };

                let mut menu_wrap = div()
                    .border_1()
                    .border_color(colors.border)
                    .rounded(px(12.0))
                    .bg(colors.panel_2)
                    .p(px(6.0))
                    .overflow_hidden()
                    .child(menu);
                let placement = shell.composer_autocomplete_menu_placement;
                if let Some(width) = shell.composer_autocomplete_menu_width {
                    menu_wrap = menu_wrap.w(width);
                } else {
                    menu_wrap = menu_wrap.max_w(px(520.0)).min_w(px(340.0));
                }
                if placement.is_none() {
                    menu_wrap = menu_wrap.mt(px(6.0));
                }
                menu_wrap.style().box_shadow = Some(vec![BoxShadow {
                    color: Hsla::from(Rgba {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.55,
                    }),
                    offset: point(px(0.0), px(16.0)),
                    blur_radius: px(40.0),
                    spread_radius: px(0.0),
                }]);
                let menu_wrap = if let Some(max_height) = shell
                    .composer_autocomplete_menu_placement
                    .and_then(|placement| placement.max_height)
                {
                    menu_wrap
                        .max_h(max_height)
                        .overflow_y_scrollbar()
                        .into_any_element()
                } else {
                    menu_wrap.into_any_element()
                };

                if let Some(placement) = placement {
                    let menu_layer = anchored()
                        .position_mode(AnchoredPositionMode::Window)
                        .anchor(placement.anchor)
                        .position(placement.position)
                        .child(menu_wrap)
                        .into_any_element();
                    let preview_layer = if shell
                        .composer_autocomplete_preview_placement
                        .is_some()
                    {
                        self.render_autocomplete_preview(&items, active)
                    } else {
                        div().into_any_element()
                    };
                    div().child(menu_layer).child(preview_layer).into_any_element()
                } else {
                    menu_wrap
                }
            }
        } else {
            div().into_any_element()
        };
        let count_menu = self.render_harness_count_menu(cx);

        let provider_id = active_provider_id(shell, is_new).unwrap_or_else(|| "".to_string());
        let provider_entry = harness_entry(&provider_id);
        let provider_label = provider_entry
            .map(|entry| entry.label.to_string())
            .unwrap_or_else(|| if provider_id.is_empty() { "Harness".to_string() } else { provider_id.clone() });
        let provider_logo = provider_entry.map(|entry| harness_logo(entry, shell.is_dark));

        let session_models_value = if is_new {
            None
        } else {
            models_value_from_session_events(shell)
        };
        let session_model_options = if is_new {
            Vec::new()
        } else {
            model_options_from_models_value(session_models_value)
        };
        let session_current_model_id = if is_new {
            None
        } else {
            current_model_id_from_models_value(session_models_value)
        };

        let current_model_id = if is_new {
            active_model_id(shell, is_new).unwrap_or_default()
        } else {
            active_session_model_id(shell, &session_model_options, session_current_model_id)
        };
        let model_loading = if is_new {
            provider_id.is_empty()
                || !shell
                    .composer_provider_options
                    .contains_key(&provider_id)
        } else {
            false
        };
        let mut model_options = if is_new {
            if provider_id.is_empty() {
                Vec::new()
            } else {
                model_options_from_provider_options(
                    shell.composer_provider_options.get(&provider_id),
                )
            }
        } else {
            session_model_options
        };
        if is_new && model_options.is_empty() {
            model_options = fallback_model_options(&provider_id);
        }
        let catalog = build_model_catalog(&model_options);
        let parsed = parse_model_id(&current_model_id, Some(&catalog));
        let current_base = if !parsed.base.is_empty() {
            parsed.base.clone()
        } else {
            catalog.base_ids.first().cloned().unwrap_or_default()
        };
        let current_effort = parsed.effort.clone();
        let effort_options = catalog
            .efforts_by_base
            .get(&current_base)
            .cloned()
            .unwrap_or_default();

        let switcher_button = |label: String,
                               icon: Option<IconName>,
                               disabled: bool,
                               id: ElementId,
                               on_click: Box<
                                    dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
                                >| {
            let mut button = div()
                .h(px(22.0))
                .rounded(px(8.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_sm()
                .text_color(tint(colors.text, 0.62))
                .id(id);

            if let Some(icon) = icon {
                button = button.child(
                    div()
                        .w(px(16.0))
                        .h(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::current(icon, 12.0)),
                );
            }

            button = button.child(
                div()
                    .max_w(px(360.0))
                    .text_sm()
                    .truncate()
                    .child(label),
            );

            if !disabled {
                button = button
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(tint(colors.text, 0.06))
                            .text_color(tint(colors.text, 0.90))
                    })
                    .active(|style| {
                        style.bg(tint(colors.text, 0.04))
                    })
                    .on_click(on_click);
            } else {
                button = button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
            }
            button
        };

        fn register_menu_trigger<E: ElementExt>(
            view: Entity<ShellView>,
            menu_id: ComposerMenuId,
            button: E,
        ) -> E {
            button.on_prepaint(move |bounds, window, cx| {
                view.update(cx, |view, cx| {
                    view.update_composer_menu_trigger_bounds(menu_id, bounds, window, cx);
                });
            })
        }

        let view = cx.entity();
        let harness_button = {
            let mut button = div()
                .h(px(22.0))
                .rounded(px(8.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_sm()
                .text_color(tint(colors.text, 0.62))
                .id("composer-harness-button");

            if let Some(logo) = provider_logo {
                button = button.child(img(logo).w(px(16.0)).h(px(16.0)).object_fit(ObjectFit::Contain));
            } else {
                button = button.child(
                    div()
                        .w(px(16.0))
                        .h(px(16.0))
                        .bg(tint(colors.text, 0.08)),
                );
            }

            button = button.child(
                div()
                    .max_w(px(360.0))
                    .text_sm()
                    .truncate()
                    .child(provider_label),
            );

            if is_new {
                button = button
                    .child(Icon::current(IconName::ChevronDown, 14.0))
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(tint(colors.text, 0.06))
                            .text_color(tint(colors.text, 0.90))
                    })
                    .active(|style| {
                        style.bg(tint(colors.text, 0.04))
                    })
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                        view.toggle_menu(ComposerMenuId::Harness, window, cx);
                    }));
            } else {
                button = button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
            }
            button
        };
        let harness_button =
            register_menu_trigger(view.clone(), ComposerMenuId::Harness, harness_button);
        let harness_button = div()
            .on_children_prepainted(automation_tree::track_children_bounds(
                "composer-harness-button",
                "button",
                Some("Harness"),
                Some("app-shell"),
            ))
            .child(harness_button);

        let mode_button = switcher_button(
            shell.composer_mode_id.label().to_string(),
            None,
            false,
            ElementId::from("composer-mode-button"),
            Box::new(cx.listener(|view, _: &ClickEvent, window, cx| {
                view.toggle_menu(ComposerMenuId::Mode, window, cx);
            })),
        )
        .child(Icon::current(IconName::ChevronDown, 14.0));
        let mode_button = register_menu_trigger(view.clone(), ComposerMenuId::Mode, mode_button);

        let local_disabled =
            is_new && shell.composer_use_multiple_agents && shell.composer_draft_tracks.len() > 1;
        let env_target = if local_disabled {
            ctx_client::EnvTarget::Worktree
        } else {
            shell.composer_env_target.clone()
        };
        let env_label = if is_new {
            match env_target {
                ctx_client::EnvTarget::Worktree => "Worktree",
                ctx_client::EnvTarget::Local => "Local",
                ctx_client::EnvTarget::Unknown => "Worktree",
            }
        } else {
            "Worktree"
        };
        let env_icon = match env_target {
            ctx_client::EnvTarget::Worktree => IconName::Diff,
            ctx_client::EnvTarget::Local => IconName::Laptop,
            ctx_client::EnvTarget::Unknown => IconName::Diff,
        };
        let env_locked = !is_new;
        let mut env_button = switcher_button(
            env_label.to_string(),
            Some(env_icon),
            env_locked,
            ElementId::from("composer-env-button"),
            Box::new(cx.listener(|view, _: &ClickEvent, window, cx| {
                view.toggle_menu(ComposerMenuId::Isolation, window, cx);
            })),
        );
        if !env_locked {
            env_button = env_button.child(Icon::current(IconName::ChevronDown, 14.0));
        }
        let env_button =
            register_menu_trigger(view.clone(), ComposerMenuId::Isolation, env_button);

        let provider_for_models = provider_id.clone();
        let model_button = switcher_button(
            if current_base.is_empty() {
                "Model".to_string()
            } else {
                catalog
                    .display_name_by_base
                    .get(&current_base)
                    .cloned()
                    .unwrap_or(current_base.clone())
            },
            None,
            false,
            ElementId::from("composer-model-button"),
            Box::new(cx.listener(move |view, _: &ClickEvent, window, cx| {
                if !provider_for_models.is_empty() {
                    view.ensure_provider_options(provider_for_models.clone(), false, cx);
                }
                view.toggle_menu(ComposerMenuId::Model, window, cx);
            })),
        )
        .child(Icon::current(IconName::ChevronDown, 14.0));
        let model_button = register_menu_trigger(view.clone(), ComposerMenuId::Model, model_button);

        let effort_label = if effort_options.is_empty() {
            "Effort".to_string()
        } else {
            let eff = current_effort
                .as_deref()
                .or_else(|| pick_default_effort(&effort_options));
            if let Some(eff) = eff {
                format_effort_label(Some(eff))
            } else {
                "Effort".to_string()
            }
        };
        let provider_for_effort = provider_id.clone();
        let effort_button = switcher_button(
            effort_label,
            None,
            false,
            ElementId::from("composer-effort-button"),
            Box::new(cx.listener(move |view, _: &ClickEvent, window, cx| {
                if !provider_for_effort.is_empty() {
                    view.ensure_provider_options(provider_for_effort.clone(), false, cx);
                }
                view.toggle_menu(ComposerMenuId::Effort, window, cx);
            })),
        )
        .child(Icon::current(IconName::ChevronDown, 14.0));
        let effort_button =
            register_menu_trigger(view.clone(), ComposerMenuId::Effort, effort_button);

        let switcher_row = {
            let mut row = div()
                .flex()
                .gap(px(8.0))
                .items_center()
                .flex_wrap()
                .min_w(px(0.0));

            row = row.child(div().child(harness_button));

            if show_model_effort {
                row = row.child(
                    div()
                        .child(model_button),
                );

                if !effort_options.is_empty() {
                    row = row.child(
                        div()
                            .child(effort_button),
                    );
                }
            }

            row = row.child(
                div()
                    .child(mode_button),
            );

            row = row.child(
                div()
                    .child(env_button),
            );

            row
        };

        let action_icon = |icon: IconName,
                           active: bool,
                           disabled: bool,
                           id: ElementId,
                           on_click: Box<
                                dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
                            >| {
            let mut button = div()
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(8.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(tint(colors.text, 0.62))
                .id(id)
                .child(Icon::current(icon, 14.0));

            if active {
                button = button.bg(tint(colors.accent, 0.22));
            }

            if disabled {
                button = button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
            } else {
                button = button
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(tint(colors.text, 0.06))
                            .text_color(tint(colors.text, 0.92))
                    })
                    .on_click(on_click);
            }
            button
        };

        let action_row = {
            let mut row = div().flex().flex_row().items_center().gap(px(6.0));

            if matches!(self.variant, ComposerVariant::ActiveSession)
                && shell.selected_session.is_some()
            {
                row = row.child(action_icon(
                    IconName::Square,
                    false,
                    false,
                    ElementId::from("composer-interrupt"),
                    Box::new(cx.listener(ShellView::on_interrupt_click)),
                ));
            }

            if !is_new {
                let verbosity_button = register_menu_trigger(
                    view.clone(),
                    ComposerMenuId::Verbosity,
                    action_icon(
                        IconName::Ellipsis,
                        false,
                        false,
                        ElementId::from("composer-verbosity"),
                        Box::new(cx.listener(|view, _: &ClickEvent, window, cx| {
                            view.toggle_menu(ComposerMenuId::Verbosity, window, cx);
                        })),
                    ),
                );
                row = row.child(div().child(verbosity_button));
            }

            row = row
                .child(action_icon(
                    IconName::AtSign,
                    false,
                    false,
                    ElementId::from("composer-insert-at"),
                    Box::new(cx.listener(ShellView::on_insert_at)),
                ))
                .child(action_icon(
                    IconName::Slash,
                    false,
                    false,
                    ElementId::from("composer-insert-slash"),
                    Box::new(cx.listener(ShellView::on_insert_slash)),
                ))
                .child(action_icon(
                    IconName::Image,
                    false,
                    false,
                    ElementId::from("composer-attach-image"),
                    Box::new(cx.listener(ShellView::on_attach_click)),
                ))
                .child(action_icon(
                    if shell.composer_recording {
                        IconName::Square
                    } else {
                        IconName::Mic
                    },
                    shell.composer_recording,
                    false,
                    ElementId::from("composer-record"),
                    Box::new(cx.listener(ShellView::toggle_recording)),
                ));

            let send_disabled = !shell.can_send_message(cx)
                || (matches!(self.variant, ComposerVariant::NewTask)
                    && shell.composer_start_busy);
            let send_foreground = if shell.is_dark {
                tint(colors.text, 0.98)
            } else {
                tint(colors.bg, 0.98)
            };
            let mut send_button = div()
                .w(px(24.0))
                .h(px(24.0))
                .rounded_full()
                .border_1()
                .border_color(tint(colors.accent, 0.65))
                .bg(tint(colors.accent, 0.20))
                .flex()
                .items_center()
                .justify_center()
                .text_color(send_foreground)
                .child(Icon::current(IconName::ArrowUp, 14.0))
                .id("composer-send");

            if send_disabled {
                send_button = send_button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
            } else {
                send_button = send_button
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(tint(colors.accent, 0.26))
                            .border_color(tint(colors.accent, 0.70))
                    })
                    .on_click(cx.listener(ShellView::on_send_click));
            }

            row.child(
                div()
                    .on_children_prepainted(automation_tree::track_children_bounds(
                        "composer-send",
                        "button",
                        Some("Send"),
                        Some("app-shell"),
                    ))
                    .child(send_button),
            )
        };

        let menu_overlay = match shell.composer_open_menu {
            Some(ComposerMenuId::Harness) => {
                if matches!(self.variant, ComposerVariant::NewTask) {
                    self.render_menu_layer(
                        ComposerMenuId::Harness,
                        self.render_harness_menu(cx),
                        cx,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                }
            }
            Some(ComposerMenuId::Model) => self
                .render_menu_layer(
                    ComposerMenuId::Model,
                    self.render_model_menu(
                        &catalog,
                        &current_base,
                        current_effort.as_deref(),
                        model_loading,
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            Some(ComposerMenuId::Effort) => {
                if effort_options.is_empty() {
                    div().into_any_element()
                } else {
                    self.render_menu_layer(
                        ComposerMenuId::Effort,
                        self.render_effort_menu(
                            &catalog,
                            &current_base,
                            current_effort.as_deref(),
                            &effort_options,
                            cx,
                        ),
                        cx,
                    )
                    .into_any_element()
                }
            }
            Some(ComposerMenuId::Mode) => self
                .render_menu_layer(ComposerMenuId::Mode, self.render_mode_menu(cx), cx)
                .into_any_element(),
            Some(ComposerMenuId::Isolation) => {
                if matches!(self.variant, ComposerVariant::NewTask) {
                    self.render_menu_layer(
                        ComposerMenuId::Isolation,
                        self.render_env_menu(cx),
                        cx,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                }
            }
            Some(ComposerMenuId::Verbosity) => {
                if matches!(self.variant, ComposerVariant::ActiveSession) {
                    self.render_menu_layer(
                        ComposerMenuId::Verbosity,
                        self.render_verbosity_menu(cx),
                        cx,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                }
            }
            None => div().into_any_element(),
        };

        let mut container = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .border_1()
            .border_color(if is_new {
                tint(colors.text, 0.10)
            } else {
                colors.border
            })
            .rounded(if is_new { px(18.0) } else { px(12.0) })
            .bg(tint(colors.text, 0.03))
            .child(attachments)
            .child(input_wrap)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(6.0))
                    .child(switcher_row)
                    .child(action_row),
            )
            .group("composer-drop")
            .on_drop({
                let view = cx.entity();
                move |paths: &ExternalPaths, window, cx| {
                    view.update(cx, |view, cx| {
                        view.on_composer_drop(paths, window, cx);
                    });
                }
            })
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style
                    .border_color(tint(colors.accent, 0.55))
                    .bg(tint(colors.accent, 0.08))
            });

        if is_new {
            container = container.p(px(12.0)).min_h(px(150.0));
            container.style().box_shadow = Some(vec![BoxShadow {
                color: Hsla::from(Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.25,
                }),
                offset: point(px(0.0), px(12.0)),
                blur_radius: px(40.0),
                spread_radius: px(0.0),
            }]);
            container = container.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .h(px(1.0))
                    .bg(tint(colors.text, 0.04)),
            );
        } else {
            container = container.px(px(12.0)).py(px(10.0));
        }

        if let Some(info) = shell.composer_context_window.as_ref() {
            let summary = format_context_window(info);
            if !summary.is_empty() {
                container = container.child(
                    div()
                        .absolute()
                        .top(px(2.0))
                        .right(px(8.0))
                        .text_size(px(9.0))
                        .text_color(tint(colors.text, 0.42))
                        .child(summary),
                );
            }
        }

        let mut drop_overlay = div()
            .absolute()
            .top(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .left(px(0.0))
            .rounded(if is_new { px(18.0) } else { px(12.0) })
            .border_1()
            .border_color(tint(colors.accent, 0.55))
            .bg(tint(colors.accent, 0.08))
            .flex()
            .items_center()
            .justify_center()
            .opacity(0.0)
            .group_drag_over::<ExternalPaths>("composer-drop", |style: gpui::StyleRefinement| {
                style.opacity(1.0)
            })
            .child(
                div()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(tint(colors.text, 0.14))
                    .bg(colors.panel_2)
                    .px(px(12.0))
                    .py(px(10.0))
                    .text_size(px(13.0))
                    .text_color(tint(colors.text, 0.92))
                    .child("Drop image to attach"),
            );
        drop_overlay.style().border_style = Some(BorderStyle::Dashed);
        container = container.child(drop_overlay);

        let body = if is_new {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .w_full()
                .max_w(px(825.0))
                .child(container)
        } else {
            container
        };

        div()
            .child(body)
            .child(autocomplete_menu)
            .child(menu_overlay)
            .child(count_menu)
    }

    fn render_menu_title_row(
        &self,
        menu_id: ComposerMenuId,
        title: &str,
        description: &str,
        cx: &mut Context<ShellView>,
    ) -> gpui::Div {
        let colors = self.shell.colors;
        let view = cx.entity();
        let info_button = div()
            .w(px(18.0))
            .h(px(18.0))
            .rounded_full()
            .border_1()
            .border_color(tint(colors.text, 0.14))
            .bg(tint(colors.text, 0.04))
            .text_color(tint(colors.text, 0.80))
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::current(IconName::Info, 12.0))
            .id(ElementId::from(format!(
                "composer-menu-info-{menu_id:?}"
            )))
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(tint(colors.text, 0.06))
                    .border_color(tint(colors.text, 0.22))
            })
            .on_hover({
                let view = view.clone();
                move |hovered: &bool, _window, cx| {
                    if *hovered {
                        view.update(cx, |view, cx| {
                            view.open_menu_tooltip(menu_id, cx);
                        });
                    } else {
                        view.update(cx, |view, cx| {
                            view.schedule_close_menu_tooltip(menu_id, cx);
                        });
                    }
                }
            })
            .on_prepaint({
                let view = view.clone();
                move |bounds, window, cx| {
                    view.update(cx, |view, cx| {
                        view.update_composer_tooltip_trigger_bounds(menu_id, bounds, window, cx);
                    });
                }
            });

        let row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight(600.0))
                    .text_color(tint(colors.text, 0.92))
                    .child(title.to_string()),
            )
            .child(info_button);

        div()
            .flex()
            .flex_col()
            .child(row)
            .child(self.render_menu_tooltip(menu_id, description, cx))
    }

    fn render_menu_tooltip(
        &self,
        menu_id: ComposerMenuId,
        description: &str,
        cx: &mut Context<ShellView>,
    ) -> AnyElement {
        let colors = self.shell.colors;
        if self.shell.composer_tooltip_open != Some(menu_id) {
            return div().into_any_element();
        }
        let placement = self.shell.composer_tooltip_placements.get(&menu_id).copied();
        let position = placement
            .map(|placement| placement.position)
            .unwrap_or_else(|| point(px(0.0), px(0.0)));
        let anchor = placement
            .map(|placement| placement.anchor)
            .unwrap_or(Corner::TopLeft);
        let mut tooltip = div()
            .border_1()
            .border_color(colors.border)
            .rounded(px(12.0))
            .bg(colors.panel)
            .p(px(10.0))
            .text_size(px(12.0))
            .line_height(px(16.2))
            .text_color(tint(colors.text, 0.92))
            .child(description.to_string())
            .id(ElementId::from(format!(
                "composer-menu-tooltip-{menu_id:?}"
            )))
            .on_hover({
                let view = cx.entity();
                move |hovered: &bool, _window, cx| {
                    if *hovered {
                        view.update(cx, |view, cx| {
                            view.open_menu_tooltip(menu_id, cx);
                        });
                    } else {
                        view.update(cx, |view, cx| {
                            view.schedule_close_menu_tooltip(menu_id, cx);
                        });
                    }
                }
            })
            .on_prepaint({
                let view = cx.entity();
                move |bounds, window, cx| {
                    view.update(cx, |view, cx| {
                        view.update_composer_tooltip_bounds(menu_id, bounds, window, cx);
                    });
                }
            });
        if let Some(max_width) = placement.and_then(|placement| placement.max_width) {
            tooltip = tooltip.max_w(max_width);
        }
        if placement.is_none() {
            tooltip = tooltip.opacity(0.0);
        }
        let tooltip = if let Some(max_height) = placement.and_then(|placement| placement.max_height)
        {
            tooltip
                .max_h(max_height)
                .overflow_y_scrollbar()
                .into_any_element()
        } else {
            tooltip.into_any_element()
        };

        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .anchor(anchor)
            .position(position)
            .child(tooltip)
            .into_any_element()
    }

    fn render_menu_layer(
        &self,
        menu_id: ComposerMenuId,
        menu: gpui::Div,
        cx: &mut Context<ShellView>,
    ) -> AnyElement {
        let placement = self.shell.composer_menu_placements.get(&menu_id).copied();
        let position = placement
            .map(|placement| placement.position)
            .unwrap_or_else(|| point(px(0.0), px(0.0)));
        let anchor = placement
            .map(|placement| placement.anchor)
            .unwrap_or(Corner::TopLeft);
        let view = cx.entity();
        let mut menu = menu
            .on_prepaint({
                let view = view.clone();
                move |bounds, window, cx| {
                    view.update(cx, |view, cx| {
                        view.update_composer_menu_bounds(menu_id, bounds, window, cx);
                    });
                }
            })
            .on_mouse_up_out(MouseButton::Left, cx.listener(|view, _, _window, cx| {
                view.close_menu(cx);
            }));
        if placement.is_none() {
            menu = menu.opacity(0.0);
        }

        let (automation_id, automation_name) = Self::menu_automation_target(menu_id);
        let menu = div()
            .relative()
            .child(self.render_menu_callout(menu_id))
            .child(menu);
        let menu = div()
            .on_children_prepainted(automation_tree::track_children_bounds(
                automation_id,
                "menu",
                Some(automation_name),
                Some("app-shell"),
            ))
            .id(automation_id)
            .child(menu);

        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .anchor(anchor)
            .position(position)
            .child(menu)
            .into_any_element()
    }

    fn render_menu_callout(&self, menu_id: ComposerMenuId) -> AnyElement {
        let colors = self.shell.colors;
        let Some(placement) = self.shell.composer_menu_placements.get(&menu_id).copied() else {
            return div().into_any_element();
        };
        let Some(trigger) = self
            .shell
            .composer_menu_trigger_bounds
            .get(&menu_id)
            .copied()
        else {
            return div().into_any_element();
        };
        let Some(menu_bounds) = self.shell.composer_menu_bounds.get(&menu_id).copied() else {
            return div().into_any_element();
        };

        let menu_left = placement.position.x;
        let menu_top = placement.position.y;
        let menu_width = menu_bounds.size.width.max(px(0.0));
        let trigger_center = trigger.left() + trigger.size.width / 2.0;
        let callout_size = px(12.0);
        let padding = px(10.0);
        let mut left = trigger_center - menu_left - callout_size / 2.0;
        let max_left = if menu_width > callout_size + padding * 2.0 {
            menu_width - callout_size - padding
        } else {
            padding
        };
        if left < padding {
            left = padding;
        } else if left > max_left {
            left = max_left;
        }

        let open_above = menu_top + menu_bounds.size.height <= trigger.top();
        let mut icon =
            Icon::new(IconName::ChevronDown, 12.0, tint(colors.text, 0.35));
        if !open_above {
            icon = icon.transform(Transformation::rotate(radians(std::f32::consts::PI)));
        }

        let mut callout = div()
            .absolute()
            .left(left)
            .w(callout_size)
            .h(callout_size)
            .flex()
            .items_center()
            .justify_center()
            .child(icon);
        if open_above {
            callout = callout.bottom(px(-6.0));
        } else {
            callout = callout.top(px(-6.0));
        }

        callout.into_any_element()
    }

    fn render_autocomplete_preview(
        &self,
        items: &[super::super::state::composer::ComposerAutocompleteItem],
        active_index: usize,
    ) -> AnyElement {
        let Some(item) = items.get(active_index) else {
            return div().into_any_element();
        };
        let ComposerAutocompleteItemKind::File = item.kind else {
            return div().into_any_element();
        };
        let colors = self.shell.colors;
        let path = item.path.clone().unwrap_or_default();
        let placement = match self.shell.composer_autocomplete_preview_placement {
            Some(placement) => placement,
            None => return div().into_any_element(),
        };

        let normalized = path.replace('\\', "/");
        let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return div().into_any_element();
        }
        let file = parts.last().unwrap().to_string();
        let dirs = &parts[..parts.len() - 1];
        let max_dirs = 6;
        let start = dirs.len().saturating_sub(max_dirs);
        let trimmed = &dirs[start..];
        let has_more = dirs.len() > trimmed.len();

        let mut tree = div().flex().flex_col().gap(px(2.0));
        if has_more {
            tree = tree.child(
                div()
                    .text_sm()
                    .text_color(tint(colors.text, 0.45))
                    .child("…"),
            );
        }
        for (idx, seg) in trimmed.iter().enumerate() {
            tree = tree.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(14.0 * idx as f32))
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(14.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(tint(colors.text, 0.70))
                            .child(Icon::current(IconName::Folder, 12.0)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(tint(colors.text, 0.82))
                            .child(seg.to_string()),
                    ),
            );
        }
        let file_indent = 14.0 * trimmed.len() as f32;
        tree = tree.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .pl(px(file_indent))
                .child(
                    div()
                        .w(px(14.0))
                        .h(px(14.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(tint(colors.text, 0.80))
                        .child(Icon::current(IconName::Artifact, 12.0)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(tint(colors.text, 0.92))
                        .child(file),
                ),
        );

        let mut preview = div()
            .border_1()
            .border_color(colors.border)
            .rounded(px(12.0))
            .bg(colors.panel)
            .p(px(10.0))
            .text_size(px(12.0))
            .line_height(px(16.2))
            .child(tree);
        if let Some(max_width) = placement.max_width {
            preview = preview.w(max_width);
        }
        preview.style().box_shadow = Some(vec![BoxShadow {
            color: Hsla::from(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.55,
            }),
            offset: point(px(0.0), px(16.0)),
            blur_radius: px(40.0),
            spread_radius: px(0.0),
        }]);
        let preview = if let Some(max_height) = placement.max_height {
            preview
                .max_h(max_height)
                .overflow_y_scrollbar()
                .into_any_element()
        } else {
            preview.into_any_element()
        };

        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .anchor(placement.anchor)
            .position(placement.position)
            .child(preview)
            .into_any_element()
    }

    fn menu_automation_target(menu_id: ComposerMenuId) -> (&'static str, &'static str) {
        match menu_id {
            ComposerMenuId::Harness => ("composer-menu-harness", "Harness"),
            ComposerMenuId::Model => ("composer-menu-model", "Model"),
            ComposerMenuId::Effort => ("composer-menu-effort", "Effort"),
            ComposerMenuId::Mode => ("composer-menu-mode", "Mode"),
            ComposerMenuId::Isolation => ("composer-menu-isolation", "Isolation"),
            ComposerMenuId::Verbosity => ("composer-menu-verbosity", "Verbosity"),
        }
    }

    fn render_menu_shell(&self, menu_id: ComposerMenuId, body: gpui::Div) -> gpui::Div {
        let colors = self.shell.colors;
        let placement = self.shell.composer_menu_placements.get(&menu_id).copied();
        let mut shell = div()
            .border_1()
            .border_color(colors.border)
            .rounded(px(12.0))
            .bg(colors.panel_2)
            .p(px(6.0))
            .min_w(px(220.0))
            .max_w(px(440.0));
        if let Some(max_width) = placement.and_then(|placement| placement.max_width) {
            shell = shell.max_w(max_width);
        }
        if menu_id == ComposerMenuId::Harness {
            shell = shell.overflow_hidden();
        }
        shell.style().box_shadow = Some(vec![BoxShadow {
            color: Hsla::from(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.55,
            }),
            offset: point(px(0.0), px(16.0)),
            blur_radius: px(40.0),
            spread_radius: px(0.0),
        }]);
        let body = if let Some(max_height) = placement.and_then(|placement| placement.max_height) {
            if menu_id == ComposerMenuId::Harness {
                body.max_h(max_height).overflow_hidden().into_any_element()
            } else {
                body.max_h(max_height)
                    .overflow_y_scrollbar()
                    .into_any_element()
            }
        } else {
            body.into_any_element()
        };
        shell.child(body)
    }

    fn render_mode_menu(&self, cx: &mut Context<ShellView>) -> gpui::Div {
        let shell = self.shell;
        let colors = shell.colors;
        let modes = [
            WorkbenchModeId::Default,
            WorkbenchModeId::Research,
            WorkbenchModeId::Plan,
            WorkbenchModeId::Review,
        ];
        let top = div()
            .px(px(4.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(tint(colors.text, 0.08))
            .mb(px(6.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.render_menu_title_row(
                ComposerMenuId::Mode,
                "Mode",
                MENU_DESC_MODE,
                cx,
            ));
        let list = modes
            .iter()
            .enumerate()
            .fold(div().flex().flex_col(), |list, (index, mode)| {
                let mode_value = *mode;
                let active = shell.composer_mode_id == mode_value;
                let label = mode_value.label();
                list.child(self.menu_item(
                    ElementId::named_usize("composer-mode-item", index),
                    label,
                    active,
                    false,
                    cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.set_mode_id(mode_value, cx);
                        view.close_menu(cx);
                    }),
                ))
            });
        self.render_menu_shell(
            ComposerMenuId::Mode,
            div().flex().flex_col().child(top).child(list),
        )
    }

    fn render_effort_menu(
        &self,
        catalog: &ModelCatalog,
        base: &str,
        current_effort: Option<&str>,
        efforts: &[String],
        cx: &mut Context<ShellView>,
    ) -> gpui::Div {
        let colors = self.shell.colors;
        let top = div()
            .px(px(4.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(tint(colors.text, 0.08))
            .mb(px(6.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.render_menu_title_row(
                ComposerMenuId::Effort,
                "Effort",
                MENU_DESC_EFFORT,
                cx,
            ));
        let list = efforts
            .iter()
            .enumerate()
            .fold(div().flex().flex_col(), |list, (index, eff)| {
                let active = current_effort == Some(eff.as_str());
                let label = format_effort_label(Some(eff));
                let base_value = base.to_string();
                let eff_value = eff.clone();
                let next = catalog
                    .full_id_by_base_effort
                    .get(&base_value)
                    .and_then(|map| map.get(&eff_value))
                    .cloned()
                    .unwrap_or_else(|| compose_model_id(&base_value, Some(&eff_value)));
                let next_value = next.clone();
                list.child(self.menu_item(
                    ElementId::named_usize("composer-effort-item", index),
                    label,
                    active,
                    false,
                    cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_composer_model(next_value.clone(), cx);
                        view.close_menu(cx);
                    }),
                ))
            });
        self.render_menu_shell(
            ComposerMenuId::Effort,
            div().flex().flex_col().child(top).child(list),
        )
    }

    fn render_model_menu(
        &self,
        catalog: &ModelCatalog,
        current_base: &str,
        current_effort: Option<&str>,
        model_loading: bool,
        cx: &mut Context<ShellView>,
    ) -> gpui::Div {
        let shell = self.shell;
        let colors = shell.colors;
        let list_empty = catalog.base_ids.is_empty();
        let top = div()
            .px(px(4.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(tint(colors.text, 0.08))
            .mb(px(6.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.render_menu_title_row(
                ComposerMenuId::Model,
                "Model",
                MENU_DESC_MODEL,
                cx,
            ))
            .child(
                div()
                    .on_children_prepainted(automation_tree::track_children_bounds(
                        "composer-model-search-input",
                        "input",
                        Some("Model search"),
                        Some("composer-menu-model"),
                    ))
                    .id("composer-model-search-input")
                    .child(
                        Input::new(&shell.composer_model_search)
                            .appearance(false)
                            .text_color(shell.colors.text)
                            .text_size(px(12.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .w_full()
                            .border_1()
                            .border_color(tint(colors.text, 0.10))
                            .bg(tint(colors.text, 0.04))
                            .rounded(px(10.0))
                            .disabled(true),
                    ),
            );

        let body = if !list_empty {
            catalog
                .base_ids
                .iter()
                .enumerate()
                .fold(div().flex().flex_col(), |list, (index, base)| {
                    let active = base == current_base;
                    let label = catalog
                        .display_name_by_base
                        .get(base)
                        .cloned()
                        .unwrap_or_else(|| base.clone());
                    let base_value = base.clone();
                    let next = derive_full_model_id_for_base(
                        catalog,
                        &base_value,
                        current_effort,
                    );
                    let next_value = next.clone();
                    list.child(self.menu_item(
                        ElementId::named_usize("composer-model-item", index),
                        label,
                        active,
                        false,
                        cx.listener(move |view, _: &ClickEvent, _window, cx| {
                            view.select_composer_model(next_value.clone(), cx);
                            view.close_menu(cx);
                        }),
                    ))
                })
        } else {
            div()
                .text_sm()
                .text_color(tint(colors.text, 0.75))
                .child(
                    div()
                        .mb(px(6.0))
                        .child(if model_loading {
                            "Loading models…"
                        } else {
                            "Enter model id"
                        }),
                )
                .child(
                    div()
                        .on_children_prepainted(automation_tree::track_children_bounds(
                            "composer-model-manual-input",
                            "input",
                            Some("Model id"),
                            Some("composer-menu-model"),
                        ))
                        .id("composer-model-manual-input")
                        .child(
                            Input::new(&shell.composer_model_manual)
                                .appearance(false)
                                .text_color(shell.colors.text)
                                .text_size(px(12.0))
                                .px(px(10.0))
                                .py(px(6.0))
                                .w_full()
                                .border_1()
                                .border_color(tint(colors.text, 0.10))
                                .bg(tint(colors.text, 0.04))
                                .rounded(px(10.0)),
                        ),
                )
        };

        self.render_menu_shell(
            ComposerMenuId::Model,
            div().flex().flex_col().child(top).child(body),
        )
        .max_w(px(420.0))
        .min_w(px(320.0))
    }

    fn render_env_menu(&self, cx: &mut Context<ShellView>) -> gpui::Div {
        let shell = self.shell;
        let colors = shell.colors;
        if !matches!(self.variant, ComposerVariant::NewTask) {
            return div();
        }
        let local_disabled = shell.composer_use_multiple_agents && shell.composer_draft_tracks.len() > 1;
        let active = if local_disabled {
            ctx_client::EnvTarget::Worktree
        } else {
            shell.composer_env_target.clone()
        };
        let top = div()
            .px(px(4.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(tint(colors.text, 0.08))
            .mb(px(6.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.render_menu_title_row(
                ComposerMenuId::Isolation,
                "Isolation",
                MENU_DESC_ISOLATION,
                cx,
            ));
        let list = div()
            .flex()
            .flex_col()
            .child(self.menu_item(
                ElementId::from("composer-env-worktree"),
                "Worktree",
                matches!(active, ctx_client::EnvTarget::Worktree),
                false,
                cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.set_env_target(ctx_client::EnvTarget::Worktree, cx);
                    view.close_menu(cx);
                }),
            ))
            .child(self.menu_item(
                ElementId::from("composer-env-local"),
                "Local",
                matches!(active, ctx_client::EnvTarget::Local),
                local_disabled,
                cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.set_env_target(ctx_client::EnvTarget::Local, cx);
                    view.close_menu(cx);
                }),
            ))
            .child(self.menu_item(
                ElementId::from("composer-env-container"),
                "Container (soon)",
                false,
                true,
                cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.close_menu(cx);
                }),
            ));
        self.render_menu_shell(
            ComposerMenuId::Isolation,
            div().flex().flex_col().child(top).child(list),
        )
    }

    fn render_verbosity_menu(&self, cx: &mut Context<ShellView>) -> gpui::Div {
        let shell = self.shell;
        let colors = shell.colors;
        let levels = [
            ComposerVerbosity::Terse,
            ComposerVerbosity::Default,
            ComposerVerbosity::Verbose,
        ];
        let top = div()
            .px(px(4.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(tint(colors.text, 0.08))
            .mb(px(6.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.render_menu_title_row(
                ComposerMenuId::Verbosity,
                "Verbosity",
                MENU_DESC_VERBOSITY,
                cx,
            ));
        let list = levels
            .iter()
            .enumerate()
            .fold(div().flex().flex_col(), |list, (index, level)| {
                let level_value = *level;
                let active = shell.composer_verbosity == level_value;
                let label = level_value.label();
                list.child(self.menu_item(
                    ElementId::named_usize("composer-verbosity-item", index),
                    label,
                    active,
                    false,
                    cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.set_verbosity(level_value, cx);
                        view.close_menu(cx);
                    }),
                ))
            });
        self.render_menu_shell(
            ComposerMenuId::Verbosity,
            div().flex().flex_col().child(top).child(list),
        )
    }

    fn render_harness_menu(&self, cx: &mut Context<ShellView>) -> gpui::Div {
        if !matches!(self.variant, ComposerVariant::NewTask) {
            return div();
        }
        let shell = self.shell;
        let colors = shell.colors;
        let query = shell
            .composer_harness_search
            .read(cx)
            .value()
            .to_string()
            .to_ascii_lowercase();

        let mut items: Vec<String> = harness_catalog()
            .iter()
            .map(|entry| entry.id.to_string())
            .collect();
        let mut extras: Vec<String> = shell
            .providers
            .iter()
            .filter(|provider| {
                let hidden = provider
                    .details
                    .get("ui_hidden")
                    .map(|value| value == "true")
                    .unwrap_or(false);
                !hidden && !items.contains(&provider.provider_id)
            })
            .map(|provider| provider.provider_id.clone())
            .collect();
        extras.sort();
        items.extend(extras);

        let mut counts: HashMap<String, usize> = HashMap::new();
        for track in &shell.composer_draft_tracks {
            *counts.entry(track.provider_id.clone()).or_insert(0) += 1;
        }

        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .min_h(px(0.0))
            .flex_1()
            .overflow_y_scrollbar()
            .px(px(2.0));
        let mut has_installable = false;
        let mut found_any = false;

        for id in items {
            let entry = harness_entry(&id);
            let label = entry
                .map(|entry| entry.label.to_string())
                .unwrap_or_else(|| id.clone());
            if !query.is_empty()
                && !label.to_ascii_lowercase().contains(&query)
                && !id.to_ascii_lowercase().contains(&query)
            {
                continue;
            }
            let provider_status = shell.providers.iter().find(|p| p.provider_id == id);
            if provider_status
                .and_then(|p| p.details.get("ui_hidden"))
                .map(|value| value == "true")
                .unwrap_or(false)
            {
                continue;
            }
            found_any = true;

            let installed = provider_status
                .map(|p| p.installed && matches!(p.health, ProviderHealth::Ok))
                .unwrap_or(false);
            let install_supported = provider_status
                .and_then(|p| p.details.get("install_supported"))
                .map(|value| value == "true")
                .unwrap_or(false);
            let install_running = provider_status
                .and_then(|p| p.details.get("install_running"))
                .map(|value| value == "true")
                .unwrap_or(false);
            if install_supported && !installed {
                has_installable = true;
            }

            let count = counts.get(&id).copied().unwrap_or(0);
            let checked = count > 0;
            let count_display = count.clamp(1, MAX_TRACKS_PER_PROVIDER);
            let opts = shell.composer_provider_options.get(&id);
            let verify_status = opts
                .and_then(|opts| opts.verify.as_ref())
                .map(|verify| verify.status.as_str());
            let warning = tint(colors.warning, 0.92);
            let error = tint(colors.error, 0.92);
            let status = if opts.map(|opts| opts.auth_required).unwrap_or(false)
                || verify_status == Some("auth_required")
            {
                Some(("Auth required", warning))
            } else if verify_status == Some("network_error") {
                Some(("Offline", warning))
            } else if verify_status == Some("error") {
                Some(("Error", error))
            } else if opts.map(|opts| opts.probe_ok == Some(false)).unwrap_or(false) {
                Some(("Unhealthy", error))
            } else {
                None
            };

            let id_for_toggle = id.clone();
            let row_id = ElementId::from(format!("composer-harness-row-{id}"));
            let row_main = {
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(6.0))
                    .py(px(6.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(tint(colors.text, 0.0))
                    .text_color(tint(colors.text, 0.92))
                    .id(row_id.clone());

                row = row.child(
                    div()
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(tint(colors.text, 0.35))
                        .bg(if checked {
                            tint(colors.text, 0.16)
                        } else {
                            Rgba {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .child(if checked { "✓" } else { "" }),
                );

                if let Some(entry) = entry {
                    let logo = harness_logo(entry, shell.is_dark);
                    row = row.child(
                        img(logo)
                            .w(px(16.0))
                            .h(px(16.0))
                            .object_fit(ObjectFit::Contain),
                    );
                } else {
                    row = row.child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .bg(tint(colors.text, 0.08)),
                    );
                }

                row = row.child(div().text_sm().truncate().child(label.clone()));

                if let Some((status_label, status_color)) = status {
                    row = row.child(
                        div()
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded_full()
                            .border_1()
                            .border_color(Rgba {
                                r: status_color.r,
                                g: status_color.g,
                                b: status_color.b,
                                a: 0.28,
                            })
                            .bg(Rgba {
                                r: status_color.r,
                                g: status_color.g,
                                b: status_color.b,
                                a: 0.14,
                            })
                            .text_xs()
                            .text_color(status_color)
                            .child(status_label),
                    );
                }

                if installed {
                    row = row
                        .cursor_pointer()
                        .hover(|style| {
                            style
                                .bg(tint(colors.text, 0.06))
                                .border_color(tint(colors.text, 0.08))
                        })
                        .active(|style| style.opacity(0.85))
                        .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                            view.toggle_harness_provider(&id_for_toggle, cx);
                            view.ensure_provider_options(id_for_toggle.clone(), false, cx);
                            if !view.composer_use_multiple_agents {
                                view.close_menu(cx);
                            }
                        }));
                } else {
                    row = row.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
                }

                row
            };

            let id_for_action = id.clone();
            let actions = {
                let mut actions = div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .justify_end()
                    .min_w(px(78.0));
                if !installed {
                    let install = shell.composer_provider_installs.get(&id);
                    let install_running_ui = install
                        .map(|st| matches!(st.state, InstallStateKind::Running))
                        .unwrap_or(false);
                    let install_finishing = install
                        .map(|st| matches!(st.state, InstallStateKind::Succeeded))
                        .unwrap_or(false)
                        && !installed;
                    let install_busy = install_running || install_running_ui || install_finishing;
                    let install_pct = if install_finishing {
                        Some(100.0)
                    } else {
                        install.and_then(|st| st.pct)
                    };
                    let label = if install_busy {
                        if install_finishing {
                            "Finalizing…".to_string()
                        } else if let Some(pct) = install_pct {
                            format!("{:.0}%", pct.clamp(0.0, 100.0))
                        } else {
                            "Installing…".to_string()
                        }
                    } else if install
                        .map(|st| matches!(st.state, InstallStateKind::Failed))
                        .unwrap_or(false)
                    {
                        "Retry".to_string()
                    } else if provider_status.map(|st| st.installed).unwrap_or(false) {
                        "Update".to_string()
                    } else {
                        "Install".to_string()
                    };

                    let mut button = div()
                        .h(px(22.0))
                        .min_w(px(78.0))
                        .w(px(78.0))
                        .rounded_full()
                        .border_1()
                        .border_color(tint(colors.text, 0.12))
                        .bg(tint(colors.text, 0.03))
                        .text_size(px(12.0))
                        .text_color(tint(colors.text, 0.92))
                        .flex()
                        .items_center()
                        .justify_center()
                        .relative()
                        .id(ElementId::from(format!(
                            "composer-harness-install-{id_for_action}"
                        )));
                    if let Some(pct) = install_pct {
                        let width = px(78.0 * (pct.clamp(0.0, 100.0) / 100.0));
                        button = button.child(
                            div()
                                .absolute()
                                .top(px(0.0))
                                .left(px(0.0))
                                .bottom(px(0.0))
                                .w(width)
                                .bg(Rgba {
                                    r: 78.0 / 255.0,
                                    g: 163.0 / 255.0,
                                    b: 255.0 / 255.0,
                                    a: 0.22,
                                }),
                        );
                    }
                    button = button.child(label.clone());

                    if !install_supported || install_busy {
                        button =
                            button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
                    } else {
                        let id_for_install = id_for_action.clone();
                        button = button
                            .cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(tint(colors.text, 0.06))
                                    .border_color(tint(colors.text, 0.18))
                            })
                            .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                view.install_provider(id_for_install.clone(), cx);
                            }));
                    }
                    actions = actions.child(button);
                } else {
                    if opts.map(|opts| opts.auth_required).unwrap_or(false) {
                        let mut button = div()
                            .h(px(22.0))
                            .px(px(10.0))
                            .rounded_full()
                            .border_1()
                            .border_color(tint(colors.text, 0.12))
                            .bg(tint(colors.text, 0.03))
                            .text_size(px(12.0))
                            .text_color(tint(colors.text, 0.88))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Authenticate")
                            .id(ElementId::from(format!(
                                "composer-harness-auth-{id_for_action}"
                            )));
                        if shell
                            .composer_provider_auth_busy
                            .get(&id)
                            .copied()
                            .unwrap_or(false)
                        {
                            button =
                                button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
                        } else {
                            let id_for_auth = id_for_action.clone();
                            button = button
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(tint(colors.text, 0.06))
                                        .border_color(tint(colors.text, 0.18))
                                })
                                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.authenticate_provider(id_for_auth.clone(), cx);
                                }));
                        }
                        actions = actions.child(button);
                    }

                    if verify_status.is_some()
                        && verify_status != Some("ok")
                        && !opts.map(|opts| opts.auth_required).unwrap_or(false)
                    {
                        let mut button = div()
                            .h(px(22.0))
                            .px(px(10.0))
                            .rounded_full()
                            .border_1()
                            .border_color(tint(colors.text, 0.12))
                            .bg(tint(colors.text, 0.03))
                            .text_size(px(12.0))
                            .text_color(tint(colors.text, 0.88))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Verify")
                            .id(ElementId::from(format!(
                                "composer-harness-verify-{id_for_action}"
                            )));
                        if shell
                            .composer_provider_verify_busy
                            .get(&id)
                            .copied()
                            .unwrap_or(false)
                        {
                            button =
                                button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
                        } else {
                            let id_for_verify = id_for_action.clone();
                            button = button
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(tint(colors.text, 0.06))
                                        .border_color(tint(colors.text, 0.18))
                                })
                                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.verify_provider(id_for_verify.clone(), cx);
                                }));
                        }
                        actions = actions.child(button);
                    }

                    let can_configure = shell.composer_use_multiple_agents
                        && shell.composer_draft_tracks.len() > 1
                        && checked;
                    if checked {
                        let mut expand_button = div()
                            .w(px(22.0))
                            .h(px(22.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(tint(colors.text, 0.85))
                            .child(Icon::current(IconName::ChevronDown, 14.0))
                            .id(ElementId::from(format!(
                                "composer-harness-expand-{id_for_action}"
                            )));
                        if can_configure {
                            let id_for_expand = id_for_action.clone();
                            expand_button = expand_button
                                .cursor_pointer()
                                .hover(|style| {
                                    style.bg(tint(colors.text, 0.06))
                                })
                                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.ensure_provider_options(id_for_expand.clone(), false, cx);
                                    view.toggle_harness_expanded_provider(
                                        id_for_expand.clone(),
                                        cx,
                                    );
                                }));
                        } else {
                            expand_button = expand_button.opacity(0.4);
                        }
                        actions = actions.child(expand_button);
                    }

                    if checked && shell.composer_use_multiple_agents {
                        let id_for_count = id.clone();
                        let view = cx.entity();
                        let mut count_button = div()
                            .h(px(22.0))
                            .px(px(10.0))
                            .rounded_full()
                            .border_1()
                            .border_color(tint(colors.text, 0.12))
                            .bg(tint(colors.text, 0.03))
                            .text_size(px(12.0))
                            .text_color(tint(colors.text, 0.88))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(format!("{count_display}x"))
                            .child(Icon::current(IconName::ChevronDown, 12.0))
                            .id(ElementId::from(format!(
                                "composer-harness-count-{id_for_count}"
                            )))
                            .on_prepaint({
                                let id_for_count = id_for_count.clone();
                                let view = view.clone();
                                move |bounds, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.update_harness_count_trigger_bounds(
                                            id_for_count.clone(),
                                            bounds,
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            });
                        count_button = count_button
                            .cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(tint(colors.text, 0.06))
                                    .border_color(tint(colors.text, 0.18))
                            })
                            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                                view.toggle_harness_count_menu(id_for_count.clone(), window, cx);
                            }));
                        actions = actions.child(count_button);
                    }
                }
                actions
            };

            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .py(px(4.0))
                    .child(row_main)
                    .child(actions),
            );

            let expanded = shell.composer_harness_expanded_provider.as_deref() == Some(&id);
            let can_configure = shell.composer_use_multiple_agents
                && shell.composer_draft_tracks.len() > 1
                && checked;
            if expanded && can_configure {
                let rows: Vec<_> = shell
                    .composer_draft_tracks
                    .iter()
                    .filter(|track| track.provider_id == id)
                    .collect();
                let config = rows.iter().fold(
                    div()
                        .ml(px(30.0))
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(tint(colors.text, 0.10))
                        .bg(tint(colors.text, 0.03))
                        .p(px(8.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0)),
                    |config, track| {
                        let key = track.key.clone();
                        let input = shell
                            .composer_track_model_inputs
                            .get(&track.key)
                            .cloned();
                        let input = input.map(|state| {
                            Input::new(&state)
                                .appearance(false)
                                .bordered(true)
                                .text_color(colors.text)
                                .text_size(px(12.0))
                                .px(px(10.0))
                                .py(px(8.0))
                                .w_full()
                                .max_w(px(440.0))
                        });

                        let add_disabled = rows.len() >= MAX_TRACKS_PER_PROVIDER;
                        let remove_disabled = rows.len() <= 1;

                        let mut add_button = div()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(10.0))
                            .border_1()
                            .border_color(tint(colors.text, 0.12))
                            .bg(tint(colors.text, 0.04))
                            .text_size(px(16.0))
                            .text_color(tint(colors.text, 0.90))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("+")
                            .id(ElementId::from(format!(
                                "composer-track-add-{}-{}",
                                id, key
                            )));
                        if add_disabled {
                            add_button = add_button
                                .opacity(0.55)
                                .cursor(CursorStyle::OperationNotAllowed);
                        } else {
                            let id_for_add = id.clone();
                            add_button = add_button
                                .cursor_pointer()
                                .hover(|style| {
                                    style.bg(tint(colors.text, 0.08))
                                })
                                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.add_track_for_provider(id_for_add.clone(), cx);
                                }));
                        }

                        let mut remove_button = div()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(10.0))
                            .border_1()
                            .border_color(tint(colors.text, 0.12))
                            .bg(tint(colors.text, 0.04))
                            .text_size(px(16.0))
                            .text_color(tint(colors.text, 0.90))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("−")
                            .id(ElementId::from(format!(
                                "composer-track-remove-{}-{}",
                                id, key
                            )));
                        if remove_disabled {
                            remove_button = remove_button
                                .opacity(0.55)
                                .cursor(CursorStyle::OperationNotAllowed);
                        } else {
                            let key_for_remove = key.clone();
                            remove_button = remove_button
                                .cursor_pointer()
                                .hover(|style| {
                                    style.bg(tint(colors.text, 0.08))
                                })
                                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                    view.remove_track_by_key(&key_for_remove, cx);
                                }));
                        }

                        config.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(10.0))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.0))
                                        .min_w(px(0.0))
                                        .flex_1()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(tint(colors.text, 0.65))
                                                .child("Agent"),
                                        )
                                        .child(
                                            input
                                                .map(|input: Input| input.into_any_element())
                                                .unwrap_or_else(|| div().into_any_element()),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(add_button)
                                        .child(remove_button),
                                ),
                        )
                    },
                );
                list = list.child(config);
            }
        }

        if !found_any {
            list = list.child(
                div()
                    .px(px(8.0))
                    .py(px(10.0))
                    .text_sm()
                    .text_color(tint(colors.text, 0.55))
                    .child("No matching agents."),
            );
        }

        let mut install_all_button = div()
            .h(px(22.0))
            .px(px(10.0))
            .rounded_full()
            .border_1()
            .border_color(tint(colors.text, 0.12))
            .bg(tint(colors.text, 0.03))
            .text_size(px(12.0))
            .text_color(tint(colors.text, 0.90))
            .flex()
            .items_center()
            .justify_center()
            .child(if shell.composer_install_all_busy {
                "Installing…"
            } else {
                "Install all"
            })
            .id(ElementId::from("composer-harness-install-all"));
        if !has_installable || shell.composer_install_all_busy {
            install_all_button =
                install_all_button.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
        } else {
            install_all_button = install_all_button
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(tint(colors.text, 0.06))
                        .border_color(tint(colors.text, 0.18))
                })
                .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.install_all_providers(cx);
                }));
        }

        let toggle = {
            let mut toggle = div()
                .w(px(30.0))
                .h(px(16.0))
                .rounded_full()
                .border_1()
                .border_color(tint(colors.text, 0.18))
                .bg(tint(colors.text, 0.10))
                .relative();
            if shell.composer_use_multiple_agents {
                toggle = toggle
                    .bg(tint(colors.success, 0.30))
                    .border_color(tint(colors.success, 0.55));
            }
            let knob_left =
                if shell.composer_use_multiple_agents { px(16.0) } else { px(2.0) };
            toggle.child(
                div()
                    .absolute()
                    .top(px(1.0))
                    .left(knob_left)
                    .w(px(12.0))
                    .h(px(12.0))
                    .rounded_full()
                    .bg(tint(colors.text, 0.85)),
            )
        };

        let banner = match (
            shell.composer_provider_action_error.as_ref(),
            shell.composer_provider_action_notice.as_ref(),
        ) {
            (Some(msg), _) => div()
                .mt(px(6.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(tint(colors.error, 0.28))
                .bg(tint(colors.error, 0.10))
                .text_sm()
                .text_color(tint(colors.error, 0.92))
                .child(msg.clone())
                .into_any_element(),
            (None, Some(msg)) => div()
                .mt(px(6.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(tint(colors.text, 0.10))
                .bg(tint(colors.text, 0.04))
                .text_sm()
                .text_color(tint(colors.text, 0.85))
                .child(msg.clone())
                .into_any_element(),
            _ => div().into_any_element(),
        };

        self.render_menu_shell(
            ComposerMenuId::Harness,
            div()
                .flex()
                .flex_col()
                .min_h(px(0.0))
                .gap(px(8.0))
                .child(
                    div()
                        .px(px(4.0))
                        .pt(px(4.0))
                        .pb(px(8.0))
                        .border_b_1()
                        .border_color(tint(colors.text, 0.08))
                        .mb(px(6.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(self.render_menu_title_row(
                            ComposerMenuId::Harness,
                            "Harness",
                            MENU_DESC_HARNESS,
                            cx,
                        ))
                        .child(
                            div()
                                .on_children_prepainted(automation_tree::track_children_bounds(
                                    "composer-harness-search-input",
                                    "input",
                                    Some("Harness search"),
                                    Some("composer-menu-harness"),
                                ))
                                .id("composer-harness-search-input")
                                .child(
                                    Input::new(&shell.composer_harness_search)
                                        .appearance(false)
                                        .text_color(shell.colors.text)
                                        .text_size(px(12.0))
                                        .px(px(10.0))
                                        .py(px(6.0))
                                        .w_full()
                                        .border_1()
                                        .border_color(tint(colors.text, 0.10))
                                        .bg(tint(colors.text, 0.04))
                                        .rounded(px(10.0)),
                                ),
                        )
                       .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(12.0))
                                .text_sm()
                                .text_color(tint(colors.text, 0.85))
                                .child("Use Multiple Agents")
                                .child(
                                    div()
                                        .id("composer-multi-agent-toggle")
                                        .cursor_pointer()
                                        .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                                            view.set_use_multiple_agents(!view.composer_use_multiple_agents, cx);
                                        }))
                                        .child(toggle),
                                ),
                        )
                        .child(div().flex().justify_end().child(install_all_button)),
                )
                .child(list)
                .child(banner),
        )
        .min_w(px(320.0))
        .max_w(px(320.0))
    }

    fn render_harness_count_menu(&self, cx: &mut Context<ShellView>) -> AnyElement {
        let shell = self.shell;
        let colors = shell.colors;
        if !shell.composer_use_multiple_agents {
            return div().into_any_element();
        }
        let Some(provider_id) = shell.composer_harness_count_menu_provider.clone() else {
            return div().into_any_element();
        };
        let Some(placement) = shell.composer_harness_count_menu_placement else {
            return div().into_any_element();
        };

        let count = shell
            .composer_draft_tracks
            .iter()
            .filter(|track| track.provider_id == provider_id)
            .count()
            .clamp(1, MAX_TRACKS_PER_PROVIDER);

        let view = cx.entity();
        let mut menu = div()
            .border_1()
            .border_color(colors.border)
            .rounded(px(12.0))
            .bg(colors.panel_2)
            .p(px(6.0))
            .min_w(px(140.0))
            .max_w(px(220.0))
            .on_prepaint({
                let view = view.clone();
                move |bounds, window, cx| {
                    view.update(cx, |view, cx| {
                        view.update_harness_count_menu_bounds(bounds, window, cx);
                    });
                }
            })
            .on_mouse_up_out(MouseButton::Left, cx.listener(|view, _, _window, cx| {
                view.close_harness_count_menu(cx);
            }));
        menu.style().box_shadow = Some(vec![BoxShadow {
            color: Hsla::from(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.55,
            }),
            offset: point(px(0.0), px(16.0)),
            blur_radius: px(40.0),
            spread_radius: px(0.0),
        }]);

        let mut list = div().flex().flex_col();
        for n in 1..=MAX_TRACKS_PER_PROVIDER {
            let active = count == n;
            let provider_id_for_action = provider_id.clone();
            let mut item = div()
                .w_full()
                .text_sm()
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(tint(colors.text, if active { 0.08 } else { 0.0 }))
                .bg(if active {
                    tint(colors.text, 0.08)
                } else {
                    Rgba {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }
                })
                .text_color(tint(colors.text, 0.92))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(format!("{n}x"))
                        .child(if active { "✓" } else { "" }),
                )
                .id(ElementId::from(format!(
                    "composer-track-count-{}-{}",
                    provider_id, n
                )));
            item = item
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(tint(colors.text, 0.06))
                        .border_color(tint(colors.text, 0.08))
                })
                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.set_track_count_for_provider(provider_id_for_action.clone(), n, cx);
                    view.close_harness_count_menu(cx);
                }));
            list = list.child(item);
        }

        menu = menu.child(list);

        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .anchor(placement.anchor)
            .position(placement.position)
            .child(menu)
            .into_any_element()
    }

    fn menu_item(
        &self,
        id: ElementId,
        label: impl Into<String>,
        active: bool,
        disabled: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> AnyElement {
        let colors = self.shell.colors;
        let label = label.into();
        let indicator = if active {
            div()
                .w(px(12.0))
                .h(px(12.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(colors.accent)
                .child(Icon::current(IconName::ChevronRight, 12.0))
        } else {
            div().w(px(12.0)).h(px(12.0))
        };
        let content = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(indicator)
            .child(div().flex_1().min_w(px(0.0)).child(label));
        let mut item = div()
            .w_full()
            .text_sm()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(tint(colors.text, 0.0))
            .bg(if active {
                tint(colors.text, 0.08)
            } else {
                Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }
            })
            .text_color(tint(colors.text, 0.92))
            .child(content)
            .id(id);

        if disabled {
            item = item.opacity(0.55).cursor(CursorStyle::OperationNotAllowed);
        } else {
            item = item
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(tint(colors.text, 0.06))
                        .border_color(tint(colors.text, 0.08))
                })
                .on_click(on_click);
        }

        item.into_any_element()
    }

}

fn format_context_window(info: &super::super::state::ContextWindowInfo) -> String {
    let Some(window_tokens) = info.window_tokens else {
        return String::new();
    };
    let used = if let Some(used) = info.used_tokens {
        used
    } else if let Some(remaining) = info.remaining_tokens {
        window_tokens - remaining
    } else if let Some(fraction) = info.remaining_fraction {
        window_tokens * (1.0 - fraction)
    } else {
        return String::new();
    };
    let window_tokens = window_tokens.max(1.0).round();
    let used = used.max(0.0).min(window_tokens).round();
    let percent = ((used / window_tokens) * 100.0).round();
    format!("{}% · {}/{}", percent, used as i64, window_tokens as i64)
}
