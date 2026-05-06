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
