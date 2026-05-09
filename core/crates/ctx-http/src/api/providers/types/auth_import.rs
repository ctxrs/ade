use serde::{Deserialize, Serialize};

use ctx_provider_auth_import as provider_auth_import;

#[derive(Debug, Serialize)]
pub(crate) struct ProviderAuthImportCandidatesResponse {
    pub(in crate::api::providers) candidates:
        Vec<provider_auth_import::ProviderAuthImportCandidate>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderAuthImportProfilesResponse {
    pub(in crate::api::providers) profiles: Vec<provider_auth_import::ProviderImportedAuthProfile>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderAuthImportReq {
    pub(in crate::api::providers) candidate_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderAuthImportResponse {
    pub(in crate::api::providers) results: Vec<provider_auth_import::ProviderAuthImportResult>,
}
