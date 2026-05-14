use std::sync::Arc;

use ctx_core::models::Workspace;
use ctx_core::redaction::redact_json_value;
use ctx_harness_sources as harness_sources;
use ctx_managed_installs as installer;
use ctx_observability::logs;
use ctx_provider_accounts as provider_accounts;
use ctx_providers::crp::probe_crp_models;

use crate::daemon::DaemonState;

mod loader;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub use loader::load_provider_model_catalog;
pub use loader::load_provider_model_catalog_for_execution_environment;
