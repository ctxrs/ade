#!/usr/bin/env node
import { randomUUID } from "node:crypto";

import { api, readDaemonAuth, resolvePathOrNull } from "./demo_lib.mjs";

function parseArgs(argv) {
  const out = {
    providerId: null,
    endpointName: "demo-relay",
    endpointId: "",
    baseUrl: null,
    modelId: null,
    apiShape: "openai_responses",
    authType: "bearer",
    apiKey: "demo-key",
    daemonUrl: process.env.CTX_DAEMON_URL || "",
    authToken: process.env.CTX_AUTH_TOKEN || "",
    dataDir: resolvePathOrNull(process.env.CTX_DATA_ROOT || ""),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = argv[i + 1];
    if (arg === "--provider") {
      out.providerId = next;
      i += 1;
    } else if (arg === "--endpoint-id") {
      out.endpointId = next;
      i += 1;
    } else if (arg === "--name") {
      out.endpointName = next;
      i += 1;
    } else if (arg === "--base-url") {
      out.baseUrl = next;
      i += 1;
    } else if (arg === "--model-id") {
      out.modelId = next;
      i += 1;
    } else if (arg === "--api-shape") {
      out.apiShape = next;
      i += 1;
    } else if (arg === "--auth-type") {
      out.authType = next;
      i += 1;
    } else if (arg === "--api-key") {
      out.apiKey = next;
      i += 1;
    } else if (arg === "--daemon-url") {
      out.daemonUrl = next;
      i += 1;
    } else if (arg === "--auth-token") {
      out.authToken = next;
      i += 1;
    } else if (arg === "--data-dir") {
      out.dataDir = resolvePathOrNull(next);
      i += 1;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  if (!out.providerId) {
    throw new Error("--provider is required");
  }
  if (!out.baseUrl) {
    throw new Error("--base-url is required");
  }
  if (!out.modelId) {
    throw new Error("--model-id is required");
  }
  return out;
}

function printHelp() {
  process.stdout.write(`demo_configure_provider_endpoint

Usage:
  node core/apps/desktop/scripts/demo_configure_provider_endpoint.mjs \\
    --provider codex \\
    --base-url http://127.0.0.1:4010/v1 \\
    --model-id gpt-5.4 \\
    --data-dir /tmp/ctx-demo-daemon
`);
}

export async function configureProviderEndpoint(options) {
  const { daemonUrl, authToken } = readDaemonAuth(options);
  const endpointId = String(options.endpointId || randomUUID()).trim();
  const config = await api(daemonUrl, authToken, "POST", `/api/providers/${options.providerId}/harness_config/endpoints`, {
    endpoint_id: endpointId,
    name: options.endpointName,
    base_url: options.baseUrl,
    api_shape: options.apiShape,
    auth_type: options.authType,
    api_key: options.apiKey,
    model_override: options.modelId,
    manual_model_ids: [options.modelId],
  });
  const endpoint = Array.isArray(config.endpoints)
    ? config.endpoints.find((candidate) => candidate.id === endpointId)
    : null;
  if (!endpoint?.id) {
    throw new Error(`Failed to resolve endpoint id ${endpointId} for ${options.endpointName}`);
  }
  const selected = await api(daemonUrl, authToken, "POST", `/api/providers/${options.providerId}/harness_config/select`, {
    source_kind: "endpoint",
    endpoint_id: endpoint.id,
  });
  return {
    provider_id: options.providerId,
    endpoint_id: endpointId,
    endpoint_name: options.endpointName,
    selected_source_kind: selected.selected_source_kind,
    model_id: options.modelId,
    base_url: options.baseUrl,
  };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = await configureProviderEndpoint(options);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
