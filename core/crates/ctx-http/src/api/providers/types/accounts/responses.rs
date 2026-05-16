use chrono::{DateTime, Utc};
use serde::Serialize;

use ctx_provider_runtime::provider_usage;

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountUsageEntry {
    pub(in crate::api::providers) account_id: Option<String>,
    pub(in crate::api::providers) label: String,
    pub(in crate::api::providers) email: Option<String>,
    pub(in crate::api::providers) plan_type: Option<String>,
    pub(in crate::api::providers) last_used_at: Option<DateTime<Utc>>,
    pub(in crate::api::providers) usage: provider_usage::ProviderUsageSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexAccountsUsageResponse {
    pub(in crate::api::providers) entries: Vec<CodexAccountUsageEntry>,
}
