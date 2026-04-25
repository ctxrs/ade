use super::*;
use ctx_core::provider_ids::canonical_provider_id;
use ctx_harness_sources as harness_sources;
use ctx_provider_accounts as provider_accounts;

mod catalog;
mod loader;
mod resolution;

#[cfg(test)]
mod tests;

const DEFAULT_REASONING_EFFORT: &str = "medium";
const KNOWN_EFFORT_IDS: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Clone)]
struct ModelInfo {
    base: String,
    effort: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelCatalog {
    pub(crate) full_ids: Vec<String>,
    pub(crate) current_model_id: Option<String>,
    base_ids: Vec<String>,
    efforts_by_base: HashMap<String, Vec<String>>,
    full_id_by_base_effort: HashMap<String, HashMap<String, String>>,
    info_by_full_id: HashMap<String, ModelInfo>,
}

impl ModelCatalog {
    pub(crate) fn default_model_id(&self) -> Option<&str> {
        self.current_model_id
            .as_deref()
            .or_else(|| self.full_ids.first().map(String::as_str))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedModel {
    pub(crate) model_id: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) full_model_id: String,
}

pub(crate) use catalog::{deserialize_optional_reasoning_effort, normalize_effort_id};
#[cfg(test)]
pub(crate) use loader::load_provider_model_catalog;
pub(crate) use loader::load_provider_model_catalog_for_execution_environment;
pub(crate) use resolution::{compose_model_id, resolve_model_id};
