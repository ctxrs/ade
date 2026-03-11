use serde_json::json;

use super::copilot_models_value_for_version;

#[derive(Clone, Copy)]
struct PinnedReasoningModel {
    id: &'static str,
    display_name: &'static str,
    default_effort: &'static str,
    efforts: &'static [&'static str],
}

const CODEX_PINNED_SUBSCRIPTION_MODELS: [PinnedReasoningModel; 6] = [
    PinnedReasoningModel {
        id: "gpt-5.4",
        display_name: "gpt-5.4",
        default_effort: "medium",
        efforts: &["low", "medium", "high", "xhigh"],
    },
    PinnedReasoningModel {
        id: "gpt-5.3-codex",
        display_name: "gpt-5.3-codex",
        default_effort: "medium",
        efforts: &["low", "medium", "high", "xhigh"],
    },
    PinnedReasoningModel {
        id: "gpt-5.2-codex",
        display_name: "gpt-5.2-codex",
        default_effort: "medium",
        efforts: &["low", "medium", "high", "xhigh"],
    },
    PinnedReasoningModel {
        id: "gpt-5.1-codex-max",
        display_name: "gpt-5.1-codex-max",
        default_effort: "medium",
        efforts: &["low", "medium", "high", "xhigh"],
    },
    PinnedReasoningModel {
        id: "gpt-5.2",
        display_name: "gpt-5.2",
        default_effort: "medium",
        efforts: &["low", "medium", "high", "xhigh"],
    },
    PinnedReasoningModel {
        id: "gpt-5.1-codex-mini",
        display_name: "gpt-5.1-codex-mini",
        default_effort: "medium",
        efforts: &["medium", "high"],
    },
];

const CLAUDE_PINNED_SUBSCRIPTION_MODELS: [PinnedReasoningModel; 3] = [
    PinnedReasoningModel {
        id: "default",
        display_name: "Default",
        default_effort: "medium",
        efforts: &["low", "medium", "high"],
    },
    PinnedReasoningModel {
        id: "sonnet",
        display_name: "Sonnet",
        default_effort: "medium",
        efforts: &["low", "medium", "high"],
    },
    PinnedReasoningModel {
        id: "opus",
        display_name: "Opus",
        default_effort: "medium",
        efforts: &["low", "medium", "high"],
    },
];

fn effort_label(effort: &str) -> &'static str {
    match effort {
        "xhigh" => "Extra High",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        "minimal" => "Minimal",
        "none" => "None",
        _ => "Unknown",
    }
}

fn pinned_reasoning_models_value(
    catalog_source: &'static str,
    current_model_id: String,
    models: &[PinnedReasoningModel],
) -> serde_json::Value {
    json!({
        "catalog_source": catalog_source,
        "current_model_id": current_model_id,
        "models": models.iter().flat_map(|model| {
            model.efforts.iter().map(|effort| json!({
                "id": format!("{}/{}", model.id, effort),
                "name": format!("{} ({})", model.display_name, effort_label(effort)),
            })).collect::<Vec<_>>()
        }).collect::<Vec<_>>(),
        "meta": {
            "source_kind": "subscription",
            "catalog_source": catalog_source,
            "refresh_pending": true,
        },
    })
}

pub(crate) fn pinned_subscription_models_value(
    provider_id: &str,
    provider_version: Option<&str>,
) -> Option<serde_json::Value> {
    match provider_id {
        "codex" => Some(pinned_reasoning_models_value(
            "codex_bundle_pinned",
            format!(
                "{}/{}",
                CODEX_PINNED_SUBSCRIPTION_MODELS[0].id,
                CODEX_PINNED_SUBSCRIPTION_MODELS[0].default_effort
            ),
            &CODEX_PINNED_SUBSCRIPTION_MODELS,
        )),
        "claude-crp" => Some(pinned_reasoning_models_value(
            "claude_subscription_pinned",
            format!(
                "{}/{}",
                CLAUDE_PINNED_SUBSCRIPTION_MODELS[0].id,
                CLAUDE_PINNED_SUBSCRIPTION_MODELS[0].default_effort
            ),
            &CLAUDE_PINNED_SUBSCRIPTION_MODELS,
        )),
        "copilot" => provider_version.and_then(copilot_models_value_for_version),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::pinned_subscription_models_value;

    #[test]
    fn codex_pinned_subscription_models_include_reasoning_variants() {
        let payload =
            pinned_subscription_models_value("codex", None).expect("codex pinned payload");
        assert_eq!(
            payload
                .get("current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("gpt-5.4/medium")
        );
        assert_eq!(
            payload
                .pointer("/meta/refresh_pending")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            payload
                .pointer("/models/0/id")
                .and_then(serde_json::Value::as_str),
            Some("gpt-5.4/low")
        );
    }

    #[test]
    fn claude_pinned_subscription_models_include_effort_variants() {
        let payload =
            pinned_subscription_models_value("claude-crp", None).expect("claude pinned payload");
        assert_eq!(
            payload
                .get("current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("default/medium")
        );
        assert_eq!(
            payload
                .pointer("/models/5/id")
                .and_then(serde_json::Value::as_str),
            Some("sonnet/high")
        );
    }
}
