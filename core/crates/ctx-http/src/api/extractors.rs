use lsp_types;

pub(super) fn extract_workspace_edit_from_command(
    cmd: &lsp_types::Command,
) -> Option<lsp_types::WorkspaceEdit> {
    let args = cmd.arguments.as_ref()?;
    for arg in args {
        if let Ok(edit) = serde_json::from_value::<lsp_types::WorkspaceEdit>(arg.clone()) {
            let has_edits = edit
                .changes
                .as_ref()
                .map(|m| !m.is_empty())
                .unwrap_or(false)
                || edit.document_changes.is_some();
            if has_edits {
                return Some(edit);
            }
        }
        if let Some(obj) = arg.as_object() {
            for key in ["edit", "workspaceEdit"] {
                if let Some(val) = obj.get(key) {
                    if let Ok(edit) =
                        serde_json::from_value::<lsp_types::WorkspaceEdit>(val.clone())
                    {
                        let has_edits = edit
                            .changes
                            .as_ref()
                            .map(|m| !m.is_empty())
                            .unwrap_or(false)
                            || edit.document_changes.is_some();
                        if !has_edits {
                            continue;
                        }
                        return Some(edit);
                    }
                }
            }
        }
    }
    None
}

pub(super) fn extract_model_entries(models: &serde_json::Value) -> Vec<(String, Option<String>)> {
    let list = if let Some(list) = models.as_array() {
        list
    } else if let Some(obj) = models.as_object() {
        let Some(list) = obj
            .get("availableModels")
            .or_else(|| obj.get("available_models"))
            .or_else(|| obj.get("models"))
            .and_then(|value| value.as_array())
        else {
            return Vec::new();
        };
        list
    } else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
        if let Some(id) = item.as_str() {
            let id = id.trim();
            if !id.is_empty() {
                out.push((id.to_string(), None));
            }
            continue;
        }
        let Some(obj) = item.as_object() else {
            continue;
        };
        let id = obj
            .get("modelId")
            .or_else(|| obj.get("id"))
            .or_else(|| obj.get("model_id"))
            .or_else(|| obj.get("model"))
            .or_else(|| obj.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        out.push((id, name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::extract_model_entries;
    use serde_json::json;

    #[test]
    fn extracts_model_id_from_model_id_field() {
        let models = json!({
            "availableModels": [
                {
                    "modelId": "gpt-5.2-codex/high",
                    "name": "gpt-5.2-codex (high)"
                }
            ]
        });
        let entries = extract_model_entries(&models);
        assert_eq!(
            entries,
            vec![(
                "gpt-5.2-codex/high".to_string(),
                Some("gpt-5.2-codex (high)".to_string())
            )]
        );
    }
}
