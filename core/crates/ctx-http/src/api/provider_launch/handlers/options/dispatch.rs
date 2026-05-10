use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_runtime::provider_launch::options::{
    provider_options_probe_plan, ProviderOptionsProbePlan,
};

use super::branches::{
    env_probe_provider_options, runtime_models_provider_options,
    selected_endpoint_runtime_launch_provider_options, ProviderOptionsProbeContext,
};
use super::*;

pub(super) async fn dispatch_provider_options_probe(
    use_crp_probe: bool,
    selected_endpoint: Option<&HarnessEndpointRecord>,
    probe_context: ProviderOptionsProbeContext<'_>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match provider_options_probe_plan(
        use_crp_probe,
        selected_endpoint.map(|endpoint| endpoint.id.as_str()),
    ) {
        ProviderOptionsProbePlan::EnvOnly => env_probe_provider_options(probe_context).await,
        ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch(endpoint_id) => {
            selected_endpoint_runtime_launch_provider_options(
                probe_context,
                endpoint_id.to_string(),
            )
            .await
        }
        ProviderOptionsProbePlan::RuntimeModels => {
            runtime_models_provider_options(probe_context).await
        }
    }
}
