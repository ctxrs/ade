import type { ProviderOptions } from "../api/client";

export function hasConfiguredHarnessAuth(
  providerId: string,
  providerOptions: ProviderOptions | undefined,
): boolean {
  if (providerId === "fake") return true;
  if (!providerOptions) return false;
  if (providerOptions.has_active_auth === true) return true;

  const endpointSelected =
    providerOptions.source?.selected_source_kind === "endpoint"
    && Boolean(providerOptions.source?.selected_endpoint_id);
  if (endpointSelected) return true;

  const unmanagedSubscriptionSelected =
    providerOptions.source?.selected_source_kind === "subscription"
    && providerId === "codex";
  if (unmanagedSubscriptionSelected) return true;

  const unmanagedSubscriptionAuthMode =
    providerOptions.auth_mode === "subscription"
    && providerId === "codex";
  if (unmanagedSubscriptionAuthMode) return true;

  return false;
}
