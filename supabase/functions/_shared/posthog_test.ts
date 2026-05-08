import { assertEquals } from "https://deno.land/std@0.224.0/assert/mod.ts";
import { capturePostHogEvent } from "./posthog.ts";

Deno.test("capturePostHogEvent keeps supabase edge source authoritative", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_test");
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
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 1);
  const body = JSON.parse(String(calls[0].init?.body)) as {
    properties: Record<string, unknown>;
  };
  assertEquals(body.properties.source, "supabase_edge");
  assertEquals(body.properties.smoke_run_id, "smoke-1");
});

Deno.test("capturePostHogEvent drops internal production-noise events", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_test");
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
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 0);
});

Deno.test("capturePostHogEvent keeps analytics pipeline smoke visible", async () => {
  const previousKey = Deno.env.get("POSTHOG_PROJECT_API_KEY");
  const previousHost = Deno.env.get("POSTHOG_HOST");
  const previousFetch = globalThis.fetch;
  const calls: Array<{ input: string; init?: RequestInit }> = [];

  Deno.env.set("POSTHOG_PROJECT_API_KEY", "phc_test");
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
    if (previousHost === undefined) {
      Deno.env.delete("POSTHOG_HOST");
    } else {
      Deno.env.set("POSTHOG_HOST", previousHost);
    }
  }

  assertEquals(calls.length, 1);
});
