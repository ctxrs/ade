use ctx_harness_sources::{HarnessApiShape, HarnessEndpointRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOptionsProbePlan<'a> {
    EnvOnly,
    RuntimeModels,
    SelectedEndpointRuntimeLaunch(&'a str),
}

pub fn provider_options_probe_plan<'a>(
    use_crp_probe: bool,
    selected_endpoint_id: Option<&'a str>,
) -> ProviderOptionsProbePlan<'a> {
    if let Some(endpoint_id) = selected_endpoint_id.filter(|value| !value.trim().is_empty()) {
        return ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch(endpoint_id);
    }
    if use_crp_probe {
        ProviderOptionsProbePlan::RuntimeModels
    } else {
        ProviderOptionsProbePlan::EnvOnly
    }
}

pub fn endpoint_supports_model_catalog_verify(endpoint: &HarnessEndpointRecord) -> bool {
    endpoint.api_shape == HarnessApiShape::OpenaiResponses
        && endpoint
            .base_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ctx_harness_sources::{
        EndpointModelCatalogStatus, HarnessApiShape, HarnessEndpointRecord,
        HarnessEndpointVerificationStatus,
    };

    use super::{
        endpoint_supports_model_catalog_verify, provider_options_probe_plan,
        ProviderOptionsProbePlan,
    };

    fn test_endpoint() -> HarnessEndpointRecord {
        HarnessEndpointRecord {
            id: "endpoint-1".to_string(),
            provider_id: "codex".to_string(),
            name: "Test endpoint".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_shape: HarnessApiShape::OpenaiResponses,
            auth_type: "bearer".to_string(),
            model_override: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_verification_status: HarnessEndpointVerificationStatus::Unknown,
            last_verification_at: None,
            last_error: None,
            has_api_key: true,
            model_catalog_status: EndpointModelCatalogStatus::Unknown,
            model_catalog_fetched_at: None,
            model_catalog_error: None,
            model_catalog_models: Vec::new(),
            manual_model_ids: Vec::new(),
            model_catalog_source: None,
        }
    }

    #[test]
    fn provider_options_probe_plan_prefers_selected_endpoint_runtime_launch() {
        assert_eq!(
            provider_options_probe_plan(false, Some("endpoint-1")),
            ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch("endpoint-1")
        );
        assert_eq!(
            provider_options_probe_plan(true, Some("endpoint-1")),
            ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch("endpoint-1")
        );
    }

    #[test]
    fn provider_options_probe_plan_uses_env_only_without_selected_endpoint_or_catalog_probe() {
        assert_eq!(
            provider_options_probe_plan(false, None),
            ProviderOptionsProbePlan::EnvOnly
        );
    }

    #[test]
    fn provider_options_probe_plan_uses_runtime_models_without_selected_endpoint() {
        assert_eq!(
            provider_options_probe_plan(true, None),
            ProviderOptionsProbePlan::RuntimeModels
        );
    }

    #[test]
    fn provider_options_probe_plan_ignores_blank_selected_endpoint() {
        assert_eq!(
            provider_options_probe_plan(false, Some("   ")),
            ProviderOptionsProbePlan::EnvOnly
        );
    }

    #[test]
    fn endpoint_supports_model_catalog_verify_requires_openai_shape_and_base_url() {
        let mut endpoint = test_endpoint();
        assert!(endpoint_supports_model_catalog_verify(&endpoint));

        endpoint.api_shape = HarnessApiShape::AnthropicMessages;
        assert!(!endpoint_supports_model_catalog_verify(&endpoint));

        endpoint.api_shape = HarnessApiShape::OpenaiResponses;
        endpoint.base_url = Some("   ".to_string());
        assert!(!endpoint_supports_model_catalog_verify(&endpoint));
    }
}
