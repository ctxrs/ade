use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderUsageQuery {
    pub(in crate::api::providers) refresh: Option<bool>,
}
