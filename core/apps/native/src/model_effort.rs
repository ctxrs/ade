use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct ModelOption {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedModelId {
    #[allow(dead_code)]
    pub(crate) full: String,
    pub(crate) base: String,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ModelCatalog {
    pub(crate) base_ids: Vec<String>,
    pub(crate) display_name_by_base: HashMap<String, String>,
    pub(crate) efforts_by_base: HashMap<String, Vec<String>>,
    pub(crate) full_id_by_base_effort: HashMap<String, HashMap<String, String>>,
}

const PREFERRED_EFFORT_ORDER: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

fn split_on_last_slash(full_model_id: &str) -> (String, String, Option<String>) {
    let full = full_model_id.trim().to_string();
    if full.is_empty() {
        return (String::new(), String::new(), None);
    }
    if let Some(idx) = full.rfind('/') {
        if idx > 0 {
            let base = full[..idx].to_string();
            let suffix = full[idx + 1..].trim().to_string();
            return (full, base, if suffix.is_empty() { None } else { Some(suffix) });
        }
    }
    (full.clone(), full, None)
}

fn normalize_effort_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn is_preferred_effort_id(value: &str) -> bool {
    let norm = normalize_effort_id(value);
    PREFERRED_EFFORT_ORDER.iter().any(|e| *e == norm)
}

fn trailing_paren_suffix(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if !trimmed.ends_with(')') {
        return None;
    }
    let open = trimmed.rfind('(')?;
    if open + 1 >= trimmed.len() - 1 {
        return None;
    }
    Some(trimmed[open + 1..trimmed.len() - 1].trim().to_string())
}

fn has_trailing_paren_suffix(name: &str, suffix: &str) -> bool {
    trailing_paren_suffix(name)
        .map(|value| normalize_effort_id(&value) == normalize_effort_id(suffix))
        .unwrap_or(false)
}

fn strip_trailing_paren_if_effort(name: &str, effort_ids: &[String]) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some(suffix) = trailing_paren_suffix(trimmed) else {
        return trimmed.to_string();
    };
    let is_effort = effort_ids
        .iter()
        .any(|eff| normalize_effort_id(eff) == normalize_effort_id(&suffix));
    if !is_effort {
        return trimmed.to_string();
    }
    let open = trimmed.rfind('(').unwrap_or(trimmed.len());
    trimmed[..open].trim().to_string()
}

fn order_effort_ids(efforts: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut list: Vec<String> = efforts
        .into_iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    let mut seen = HashSet::new();
    list.retain(|eff| seen.insert(eff.clone()));

    let order_index = |value: &str| {
        let norm = normalize_effort_id(value);
        PREFERRED_EFFORT_ORDER
            .iter()
            .position(|e| *e == norm)
            .unwrap_or(usize::MAX)
    };

    list.sort_by(|a, b| {
        let ia = order_index(a);
        let ib = order_index(b);
        if ia != ib {
            return ia.cmp(&ib);
        }
        a.cmp(b)
    });

    list
}

pub(crate) fn parse_model_id(full_model_id: &str, catalog: Option<&ModelCatalog>) -> ParsedModelId {
    let (full, base, suffix) = split_on_last_slash(full_model_id);
    if full.is_empty() {
        return ParsedModelId {
            full,
            base,
            effort: None,
        };
    }

    let Some(suffix) = suffix else {
        return ParsedModelId {
            full: full.clone(),
            base: full,
            effort: None,
        };
    };

    if let Some(catalog) = catalog {
        if let Some(options) = catalog.efforts_by_base.get(&base) {
            if options.iter().any(|eff| eff == &suffix) {
                return ParsedModelId {
                    full,
                    base,
                    effort: Some(suffix),
                };
            }
        }
        if catalog.base_ids.iter().any(|id| id == &full) {
            return ParsedModelId {
                full: full.clone(),
                base: full,
                effort: None,
            };
        }
    }

    if is_preferred_effort_id(&suffix) {
        return ParsedModelId {
            full,
            base,
            effort: Some(suffix),
        };
    }

    ParsedModelId {
        full: full.clone(),
        base: full,
        effort: None,
    }
}

pub(crate) fn build_model_catalog(models: &[ModelOption]) -> ModelCatalog {
    let mut base_ids_set = HashSet::new();
    let mut raw_efforts_by_base: HashMap<String, HashSet<String>> = HashMap::new();
    let mut raw_names_by_base: HashMap<String, Vec<String>> = HashMap::new();
    let mut display_name_by_base: HashMap<String, String> = HashMap::new();
    let mut full_id_by_base_effort: HashMap<String, HashMap<String, String>> = HashMap::new();

    for model in models {
        let id = model.id.trim();
        if id.is_empty() {
            continue;
        }
        let name = model
            .name
            .as_deref()
            .unwrap_or(id)
            .trim()
            .to_string();
        let (_full, base_candidate, suffix) = split_on_last_slash(id);
        if base_candidate.is_empty() {
            continue;
        }

        let effort = match suffix {
            Some(ref suffix)
                if is_preferred_effort_id(suffix)
                    || has_trailing_paren_suffix(&name, suffix) =>
            {
                Some(suffix.clone())
            }
            _ => None,
        };

        let base = if effort.is_some() {
            base_candidate
        } else {
            id.to_string()
        };

        base_ids_set.insert(base.clone());
        raw_names_by_base.entry(base.clone()).or_default().push(name);

        if let Some(effort) = effort {
            raw_efforts_by_base
                .entry(base.clone())
                .or_default()
                .insert(effort.clone());
            full_id_by_base_effort
                .entry(base.clone())
                .or_default()
                .insert(effort, id.to_string());
        }
    }

    let mut base_ids: Vec<String> = base_ids_set.into_iter().collect();
    base_ids.sort();

    let mut efforts_by_base: HashMap<String, Vec<String>> = HashMap::new();
    for base in &base_ids {
        let efforts = raw_efforts_by_base
            .get(base)
            .map(|set| order_effort_ids(set.iter().cloned()))
            .unwrap_or_default();
        let efforts_for_base = if efforts.len() >= 2 { efforts } else { Vec::new() };
        let names = raw_names_by_base.get(base).cloned().unwrap_or_default();
        let stripped = names
            .iter()
            .map(|name| strip_trailing_paren_if_effort(name, &efforts_for_base))
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        let display_name = stripped.first().cloned().unwrap_or_else(|| base.clone());
        display_name_by_base.insert(base.clone(), display_name);
        efforts_by_base.insert(base.clone(), efforts_for_base);
    }

    ModelCatalog {
        base_ids,
        display_name_by_base,
        efforts_by_base,
        full_id_by_base_effort,
    }
}

pub(crate) fn compose_model_id(base: &str, effort: Option<&str>) -> String {
    let base = base.trim();
    if base.is_empty() {
        return String::new();
    }
    if let Some(effort) = effort {
        let eff = effort.trim();
        if !eff.is_empty() {
            return format!("{base}/{eff}");
        }
    }
    base.to_string()
}

pub(crate) fn pick_default_effort<'a>(efforts: &'a [String]) -> Option<&'a str> {
    if efforts.iter().any(|eff| eff == "medium") {
        return Some("medium");
    }
    efforts.first().map(|eff| eff.as_str())
}

pub(crate) fn derive_full_model_id_for_base(
    catalog: &ModelCatalog,
    base: &str,
    preferred_effort: Option<&str>,
) -> String {
    let Some(efforts) = catalog.efforts_by_base.get(base) else {
        return base.to_string();
    };
    if efforts.is_empty() {
        return base.to_string();
    }
    let eff = preferred_effort
        .and_then(|value| efforts.iter().find(|eff| *eff == value).map(|eff| eff.as_str()))
        .or_else(|| pick_default_effort(efforts));
    let Some(eff) = eff else {
        return base.to_string();
    };
    catalog
        .full_id_by_base_effort
        .get(base)
        .and_then(|map| map.get(eff))
        .cloned()
        .unwrap_or_else(|| compose_model_id(base, Some(eff)))
}

pub(crate) fn format_effort_label(effort: Option<&str>) -> String {
    let raw = effort.unwrap_or("").trim();
    if raw.is_empty() {
        return String::new();
    }
    let norm = raw.to_ascii_lowercase();
    if norm == "xhigh" || norm == "extra_high" || norm == "extra-high" {
        return "Extra High".to_string();
    }
    let mut chars = norm.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}
