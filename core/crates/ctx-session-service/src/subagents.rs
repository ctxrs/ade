use std::collections::{HashMap, HashSet};

use ctx_session_tools::model_resolution::normalize_effort_id;

pub const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;
pub const DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT: usize = 12;
pub const DEFAULT_MAX_SUBAGENT_DEPTH: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubagentWorktreeSelection {
    Inherit,
    New,
}

pub fn resolve_max_subagents_per_call(configured: Option<u32>) -> usize {
    configured
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_SUBAGENTS_PER_CALL)
}

pub fn parse_subagent_worktree(value: Option<&str>) -> Result<SubagentWorktreeSelection, String> {
    let trimmed = value.map(str::trim).filter(|raw| !raw.is_empty());
    match trimmed {
        Some("inherit") => Ok(SubagentWorktreeSelection::Inherit),
        Some("new") => Ok(SubagentWorktreeSelection::New),
        Some(_) => Err("worktree must be 'inherit' or 'new'".to_string()),
        None => Err("worktree is required".to_string()),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SubagentRequestAgent<'a> {
    pub prompt: &'a str,
    pub label: Option<&'a str>,
    pub harness: Option<&'a str>,
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
}

fn normalize_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalize_reasoning_effort(value: Option<&str>) -> Option<String> {
    normalize_optional(value)
        .map(normalize_effort_id)
        .filter(|value| !value.is_empty())
}

pub fn build_subagent_request_json(agents: &[SubagentRequestAgent<'_>]) -> serde_json::Value {
    let mut items = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.iter().enumerate() {
        let prompt = agent.prompt.trim();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "position".to_string(),
            serde_json::Value::Number(serde_json::Number::from(idx as u64)),
        );
        obj.insert(
            "prompt_length".to_string(),
            serde_json::Value::Number(serde_json::Number::from(prompt.chars().count() as u64)),
        );
        if let Some(label) = normalize_optional(agent.label) {
            obj.insert(
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
        if let Some(harness) = normalize_optional(agent.harness) {
            obj.insert(
                "harness".to_string(),
                serde_json::Value::String(harness.to_string()),
            );
        }
        if let Some(model) = normalize_optional(agent.model) {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(reasoning_effort) = normalize_reasoning_effort(agent.reasoning_effort) {
            obj.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(reasoning_effort),
            );
        }
        items.push(serde_json::Value::Object(obj));
    }

    serde_json::json!({
        "agents_total": agents.len(),
        "agents": items,
    })
}

pub fn collect_provider_ids(
    agents: &[SubagentRequestAgent<'_>],
    parent_provider_id: &str,
) -> Result<HashSet<String>, String> {
    let mut provider_ids = HashSet::new();
    for agent in agents {
        let provider_id = agent.harness.unwrap_or(parent_provider_id).trim();
        if provider_id.is_empty() {
            return Err("harness is required".to_string());
        }
        provider_ids.insert(provider_id.to_string());
    }
    Ok(provider_ids)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentWaitMode {
    Any,
    All,
}

impl AgentWaitMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentWaitUntil {
    Terminal,
    Update,
}

impl AgentWaitUntil {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Update => "update",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AgentWaitDetail<'a> {
    pub agent_id: &'a str,
    pub has_current_run: bool,
    pub has_latest_result: bool,
    pub last_event_seq: i64,
}

pub fn parse_wait_mode(mode: Option<&str>) -> Result<AgentWaitMode, String> {
    match mode.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("any") => Ok(AgentWaitMode::Any),
        Some("all") => Ok(AgentWaitMode::All),
        Some(other) => Err(format!("unsupported wait mode '{other}'")),
    }
}

pub fn parse_wait_until(until: Option<&str>) -> Result<AgentWaitUntil, String> {
    match until.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("terminal") => Ok(AgentWaitUntil::Terminal),
        Some("update") => Ok(AgentWaitUntil::Update),
        Some(other) => Err(format!("unsupported wait until '{other}'")),
    }
}

pub fn normalize_wait_agent_ids(
    agent_id: Option<&str>,
    agent_ids: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let raw_ids = match (agent_id, agent_ids) {
        (Some(agent_id), None) => vec![agent_id.to_string()],
        (None, Some(agent_ids)) => agent_ids.to_vec(),
        (Some(_), Some(_)) => {
            return Err("provide either agent_id or agent_ids".to_string());
        }
        (None, None) => {
            return Err("agent_id or agent_ids is required".to_string());
        }
    };
    if raw_ids.is_empty() {
        return Err("agent_ids is required".to_string());
    }
    let mut seen = HashSet::new();
    let mut normalized_ids = Vec::with_capacity(raw_ids.len());
    for raw in raw_ids {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return Err("agent_id cannot be empty".to_string());
        }
        if !seen.insert(trimmed.clone()) {
            return Err(format!("duplicate agent_id '{trimmed}'"));
        }
        normalized_ids.push(trimmed);
    }
    Ok(normalized_ids)
}

fn detail_satisfies_terminal(detail: &AgentWaitDetail<'_>) -> bool {
    !detail.has_current_run && detail.has_latest_result
}

fn detail_satisfies_update(detail: &AgentWaitDetail<'_>, threshold: i64) -> bool {
    detail.last_event_seq > threshold
}

pub fn wait_predicate_satisfied(
    details: &[AgentWaitDetail<'_>],
    mode: AgentWaitMode,
    until: AgentWaitUntil,
    thresholds: &HashMap<String, i64>,
) -> bool {
    let per_agent = |detail: &AgentWaitDetail<'_>| match until {
        AgentWaitUntil::Terminal => detail_satisfies_terminal(detail),
        AgentWaitUntil::Update => detail_satisfies_update(
            detail,
            thresholds.get(detail.agent_id).copied().unwrap_or_default(),
        ),
    };

    match mode {
        AgentWaitMode::Any => details.iter().any(per_agent),
        AgentWaitMode::All => details.iter().all(per_agent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_max_subagents_per_call_from_positive_config() {
        assert_eq!(resolve_max_subagents_per_call(Some(3)), 3);
        assert_eq!(
            resolve_max_subagents_per_call(Some(0)),
            DEFAULT_MAX_SUBAGENTS_PER_CALL
        );
        assert_eq!(
            resolve_max_subagents_per_call(None),
            DEFAULT_MAX_SUBAGENTS_PER_CALL
        );
    }

    #[test]
    fn parses_subagent_worktree_selection_strictly() {
        assert_eq!(
            parse_subagent_worktree(Some(" inherit ")),
            Ok(SubagentWorktreeSelection::Inherit)
        );
        assert_eq!(
            parse_subagent_worktree(Some("new")),
            Ok(SubagentWorktreeSelection::New)
        );
        assert_eq!(
            parse_subagent_worktree(Some("reuse"))
                .as_ref()
                .map_err(String::as_str),
            Err("worktree must be 'inherit' or 'new'")
        );
        assert_eq!(
            parse_subagent_worktree(None)
                .as_ref()
                .map_err(String::as_str),
            Err("worktree is required")
        );
    }

    #[test]
    fn builds_subagent_request_json_with_normalized_optional_fields() {
        let request = build_subagent_request_json(&[
            SubagentRequestAgent {
                prompt: " hello ",
                label: Some(" alpha "),
                harness: Some(" codex "),
                model: Some(" gpt-5.4 "),
                reasoning_effort: Some("extra high"),
            },
            SubagentRequestAgent {
                prompt: "second",
                label: Some(""),
                harness: None,
                model: None,
                reasoning_effort: None,
            },
        ]);

        assert_eq!(
            request,
            serde_json::json!({
                "agents_total": 2,
                "agents": [
                    {
                        "position": 0,
                        "prompt_length": 5,
                        "label": "alpha",
                        "harness": "codex",
                        "model": "gpt-5.4",
                        "reasoning_effort": "xhigh",
                    },
                    {
                        "position": 1,
                        "prompt_length": 6,
                    }
                ],
            })
        );
    }

    #[test]
    fn collects_provider_ids_with_parent_default_and_empty_rejection() {
        let agents = [
            SubagentRequestAgent {
                prompt: "one",
                label: None,
                harness: None,
                model: None,
                reasoning_effort: None,
            },
            SubagentRequestAgent {
                prompt: "two",
                label: None,
                harness: Some(" gemini "),
                model: None,
                reasoning_effort: None,
            },
        ];
        let ids = collect_provider_ids(&agents, "codex").expect("provider ids should resolve");
        assert!(ids.contains("codex"));
        assert!(ids.contains("gemini"));

        let agents = [SubagentRequestAgent {
            prompt: "one",
            label: None,
            harness: Some(" "),
            model: None,
            reasoning_effort: None,
        }];
        assert_eq!(
            collect_provider_ids(&agents, "codex")
                .as_ref()
                .map_err(String::as_str),
            Err("harness is required")
        );
    }

    #[test]
    fn parses_wait_mode_and_until_strictly() {
        assert_eq!(parse_wait_mode(None), Ok(AgentWaitMode::Any));
        assert_eq!(parse_wait_mode(Some(" all ")), Ok(AgentWaitMode::All));
        assert_eq!(
            parse_wait_mode(Some("first"))
                .as_ref()
                .map_err(String::as_str),
            Err("unsupported wait mode 'first'")
        );

        assert_eq!(parse_wait_until(None), Ok(AgentWaitUntil::Terminal));
        assert_eq!(
            parse_wait_until(Some(" update ")),
            Ok(AgentWaitUntil::Update)
        );
        assert_eq!(
            parse_wait_until(Some("done"))
                .as_ref()
                .map_err(String::as_str),
            Err("unsupported wait until 'done'")
        );
    }

    #[test]
    fn normalizes_wait_agent_ids_strictly() {
        assert_eq!(
            normalize_wait_agent_ids(Some(" agent_1 "), None),
            Ok(vec!["agent_1".to_string()])
        );
        assert_eq!(
            normalize_wait_agent_ids(
                None,
                Some(&[" agent_1 ".to_string(), "agent_2".to_string()])
            ),
            Ok(vec!["agent_1".to_string(), "agent_2".to_string()])
        );
        assert_eq!(
            normalize_wait_agent_ids(Some("agent_1"), Some(&["agent_2".to_string()]))
                .as_ref()
                .map_err(String::as_str),
            Err("provide either agent_id or agent_ids")
        );
        assert_eq!(
            normalize_wait_agent_ids(
                None,
                Some(&["agent_1".to_string(), " agent_1 ".to_string()])
            )
            .as_ref()
            .map_err(String::as_str),
            Err("duplicate agent_id 'agent_1'")
        );
    }

    #[test]
    fn evaluates_wait_predicates() {
        let details = [
            AgentWaitDetail {
                agent_id: "agent_1",
                has_current_run: false,
                has_latest_result: true,
                last_event_seq: 12,
            },
            AgentWaitDetail {
                agent_id: "agent_2",
                has_current_run: true,
                has_latest_result: false,
                last_event_seq: 9,
            },
        ];
        let thresholds = HashMap::from([("agent_1".to_string(), 10), ("agent_2".to_string(), 10)]);

        assert!(wait_predicate_satisfied(
            &details,
            AgentWaitMode::Any,
            AgentWaitUntil::Terminal,
            &thresholds
        ));
        assert!(!wait_predicate_satisfied(
            &details,
            AgentWaitMode::All,
            AgentWaitUntil::Terminal,
            &thresholds
        ));
        assert!(wait_predicate_satisfied(
            &details,
            AgentWaitMode::Any,
            AgentWaitUntil::Update,
            &thresholds
        ));
        assert!(!wait_predicate_satisfied(
            &details,
            AgentWaitMode::All,
            AgentWaitUntil::Update,
            &thresholds
        ));
    }
}
