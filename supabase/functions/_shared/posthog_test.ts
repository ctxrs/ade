import { assertEquals } from "https://deno.land/std@0.224.0/assert/mod.ts";
import { capturePostHogEvent, postHogCaptureTargetNames } from "./posthog.ts";

Deno.test("capturePostHogEvent keeps supabase edge source authoritative", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousCanaryKey = Deno.env.get("POSTHOG_CANARY_PROJECT_API_KEY");
  const previousProductionCanary = Deno.env.get(
    "POSTHOG_PRODUCTION_CANARY_ENABLED",
  );
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_test");
  Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
  Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
  Deno.env.set("POSTHOG_HOST", "https://posthog.example");
  globalThis.fetch = ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    calls.push({ input: String(input), init });
    return Promise.resolve(new Response(null, { status: 200 }));
  }) as typeof fetch;

  try {
    await capturePostHogEvent({
      event: "app_opened",
      distinctId: "install:abc",
      properties: {
        source: "product_event_source",
        analytics_environment: "production",
        traffic_class: "user",
        plane: "product",
        origin_runtime: "desktop",
        surface: "desktop",
        smoke_run_id: "smoke-1",
      },
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) {
      Deno.env.delete("POSTHOG_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_PROJECT_API_KEY", previousKey);
    }
    if (previousCanaryKey === undefined) {
      Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", previousCanaryKey);
    }
    if (previousProductionCanary === undefined) {
      Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
    } else {
      Deno.env.set(
        "POSTHOG_PRODUCTION_CANARY_ENABLED",
        previousProductionCanary,
      );
    }
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 1);
  const body = JSON.parse(String(calls[0].init?.body)) as {
    api_key: string;
    properties: Record<string, unknown>;
  };
  assertEquals(body.api_key, "phc_test");
  assertEquals(body.properties.source, "supabase_edge");
  assertEquals(body.properties.posthog_target, "production");
  assertEquals(body.properties.smoke_run_id, "smoke-1");
});

Deno.test("capturePostHogEvent only routes allowed user planes to production", () => {
  assertEquals(
    postHogCaptureTargetNames("incident_reported", {
      analytics_environment: "production",
      traffic_class: "user",
      plane: "incident",
      origin_runtime: "desktop",
      surface: "desktop",
    }),
    [],
  );
  assertEquals(
    postHogCaptureTargetNames("runtime_error_observed", {
      analytics_environment: "production",
      traffic_class: "user",
      plane: "incident",
      origin_runtime: "desktop",
      surface: "desktop",
    }),
    ["production"],
  );
  assertEquals(
    postHogCaptureTargetNames("app_opened", {
      analytics_environment: "production",
      traffic_class: "user",
      plane: "product",
      origin_runtime: "desktop",
      surface: "desktop",
    }),
    ["production"],
  );
});

Deno.test("capturePostHogEvent drops internal production-noise events", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousCanaryKey = Deno.env.get("POSTHOG_CANARY_PROJECT_API_KEY");
  const previousProductionCanary = Deno.env.get(
    "POSTHOG_PRODUCTION_CANARY_ENABLED",
  );
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_test");
  Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
  Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
  Deno.env.set("POSTHOG_HOST", "https://posthog.example");
  globalThis.fetch = ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    calls.push({ input: String(input), init });
    return Promise.resolve(new Response(null, { status: 200 }));
  }) as typeof fetch;

  try {
    await capturePostHogEvent({
      event: "provider_call",
      distinctId: "install:fake",
      properties: { provider_id: "fake" },
    });
    await capturePostHogEvent({
      event: "session_started",
      distinctId: "install:local",
      properties: { app_version: "0.0.0-dev" },
    });
    await capturePostHogEvent({
      event: "app_opened",
      distinctId: "install:staging",
      properties: { analytics_environment: "staging" },
    });
    await capturePostHogEvent({
      event: "app_opened",
      distinctId: "install:synthetic",
      properties: { traffic_class: "synthetic" },
    });
    await capturePostHogEvent({
      event: "session_started",
      distinctId: "install:daemon",
      properties: { origin_runtime: "daemon" },
    });
    await capturePostHogEvent({
      event: "release_download_redirected",
      distinctId: "download:abc",
      properties: { channel: "stable", version: "0.64.0" },
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) {
      Deno.env.delete("POSTHOG_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_PROJECT_API_KEY", previousKey);
    }
    if (previousCanaryKey === undefined) {
      Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", previousCanaryKey);
    }
    if (previousProductionCanary === undefined) {
      Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
    } else {
      Deno.env.set(
        "POSTHOG_PRODUCTION_CANARY_ENABLED",
        previousProductionCanary,
      );
    }
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 0);
});

Deno.test("capturePostHogEvent routes classified synthetic telemetry to canary only", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousCanaryKey = Deno.env.get("POSTHOG_CANARY_PROJECT_API_KEY");
  const previousProductionCanary = Deno.env.get(
    "POSTHOG_PRODUCTION_CANARY_ENABLED",
  );
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_product");
  Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", "phc_canary");
  Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
  Deno.env.set("POSTHOG_HOST", "https://posthog.example");
  globalThis.fetch = ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    calls.push({ input: String(input), init });
    return Promise.resolve(new Response(null, { status: 200 }));
  }) as typeof fetch;

  try {
    await capturePostHogEvent({
      event: "provider_call",
      distinctId: "install:fake",
      properties: {
        analytics_environment: "production",
        traffic_class: "synthetic",
        origin_runtime: "daemon",
        broker_runtime: "daemon",
        plane: "product",
        provider_id: "fake",
        model_id: "fake-model",
      },
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) {
      Deno.env.delete("POSTHOG_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_PROJECT_API_KEY", previousKey);
    }
    if (previousCanaryKey === undefined) {
      Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", previousCanaryKey);
    }
    if (previousProductionCanary === undefined) {
      Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
    } else {
      Deno.env.set(
        "POSTHOG_PRODUCTION_CANARY_ENABLED",
        previousProductionCanary,
      );
    }
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 1);
  const body = JSON.parse(String(calls[0].init?.body)) as {
    api_key: string;
    properties: Record<string, unknown>;
  };
  assertEquals(body.api_key, "phc_canary");
  assertEquals(body.properties.posthog_target, "canary");
});

Deno.test("capturePostHogEvent keeps unclassified release and download calls out of canary", async () => {
  assertEquals(
    postHogCaptureTargetNames("release_manifest_checked", {
      channel: "stable",
      current_version: "0.0.0-dev",
      platform: "linux-x64",
    }),
    [],
  );
  assertEquals(
    postHogCaptureTargetNames("release_download_redirected", {
      channel: "stable",
      version: "0.64.0",
      artifact: "ctx.AppImage",
    }),
    [],
  );
});

Deno.test("capturePostHogEvent routes analytics pipeline smoke to canary and optional production canary", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousCanaryKey = Deno.env.get("POSTHOG_CANARY_PROJECT_API_KEY");
  const previousProductionCanary = Deno.env.get(
    "POSTHOG_PRODUCTION_CANARY_ENABLED",
  );
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_product");
  Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", "phc_canary");
  Deno.env.set("POSTHOG_PRODUCTION_CANARY_ENABLED", "1");
  Deno.env.set("POSTHOG_HOST", "https://posthog.example");
  globalThis.fetch = ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    calls.push({ input: String(input), init });
    return Promise.resolve(new Response(null, { status: 200 }));
  }) as typeof fetch;

  try {
    await capturePostHogEvent({
      event: "analytics_pipeline_smoke",
      distinctId: "pipeline:smoke",
      properties: { app_version: "0.0.0-smoke" },
    });
  } finally {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) {
      Deno.env.delete("POSTHOG_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_PROJECT_API_KEY", previousKey);
    }
    if (previousCanaryKey === undefined) {
      Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", previousCanaryKey);
    }
    if (previousProductionCanary === undefined) {
      Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
    } else {
      Deno.env.set(
        "POSTHOG_PRODUCTION_CANARY_ENABLED",
        previousProductionCanary,
      );
    }
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 2);
  const bodies = calls.map((call) =>
    JSON.parse(String(call.init?.body)) as {
      api_key: string;
      properties: Record<string, unknown>;
    }
  );
  assertEquals(
    bodies.map((body) => body.api_key).sort(),
    ["phc_canary", "phc_product"],
  );
  assertEquals(
    bodies.map((body) => body.properties.posthog_target).sort(),
    ["canary", "production"],
  );
});

Deno.test("capturePostHogEvent attempts every selected target before reporting failure", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousCanaryKey = Deno.env.get("POSTHOG_CANARY_PROJECT_API_KEY");
  const previousProductionCanary = Deno.env.get(
    "POSTHOG_PRODUCTION_CANARY_ENABLED",
  );
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_product");
  Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", "phc_canary");
  Deno.env.set("POSTHOG_PRODUCTION_CANARY_ENABLED", "1");
  Deno.env.set("POSTHOG_HOST", "https://posthog.example");
  globalThis.fetch = ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    calls.push({ input: String(input), init });
    const body = JSON.parse(String(init?.body)) as {
      properties: Record<string, unknown>;
    };
    const status = body.properties.posthog_target === "canary" ? 503 : 200;
    return Promise.resolve(new Response(null, { status }));
  }) as typeof fetch;

  try {
    let caught: unknown;
    try {
      await capturePostHogEvent({
        event: "analytics_pipeline_smoke",
        distinctId: "pipeline:smoke",
        properties: { app_version: "0.0.0-smoke" },
      });
    } catch (err) {
      caught = err;
    }
    assertEquals(caught instanceof AggregateError, true);
  } finally {
    globalThis.fetch = previousFetch;
    if (previousKey === undefined) {
      Deno.env.delete("POSTHOG_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_PROJECT_API_KEY", previousKey);
    }
    if (previousCanaryKey === undefined) {
      Deno.env.delete("POSTHOG_CANARY_PROJECT_API_KEY");
    } else {
      Deno.env.set("POSTHOG_CANARY_PROJECT_API_KEY", previousCanaryKey);
    }
    if (previousProductionCanary === undefined) {
      Deno.env.delete("POSTHOG_PRODUCTION_CANARY_ENABLED");
    } else {
      Deno.env.set(
        "POSTHOG_PRODUCTION_CANARY_ENABLED",
        previousProductionCanary,
      );
    }
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 2);
  const targets = calls.map((call) => {
    const body = JSON.parse(String(call.init?.body)) as {
      properties: Record<string, unknown>;
    };
    return body.properties.posthog_target;
  });
  assertEquals(targets.sort(), ["canary", "production"]);
});
