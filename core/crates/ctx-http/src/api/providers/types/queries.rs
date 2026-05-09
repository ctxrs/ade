use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct InstallTargetQuery {
    pub(in crate::api::providers) target: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderUsageQuery {
    pub(in crate::api::providers) refresh: Option<bool>,
}
