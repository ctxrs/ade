import { afterEach, describe, expect, test, vi } from "vitest";

import { TELEMETRY_INSERT_TABLE, type TelemetryDatabase } from "../src/database";
import { capturePostHogEvent, postHogCaptureTargetNames } from "../src/posthog";
import type { TelemetryPostHogCapture, TelemetryRow } from "../src/telemetry-ingest";
import { createTelemetryWorker, type Env } from "../src/worker";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("telemetry worker", () => {
  test("writes to the canonical ctx telemetry schema", () => {
    expect(TELEMETRY_INSERT_TABLE).toBe("ctx.telemetry_event");
  });

  test("handles telemetry CORS preflight", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);

    const response = await worker.fetch(
      new Request("https://https://telemetry.example.invalid/functions/v1/telemetry", {
        method: "OPTIONS",
        headers: { origin: "https://https://telemetry.example.invalid" },
      }),
      testEnv(),
    );

    expect(response.status).toBe(200);
    expect(await response.text()).toBe("ok");
    expect(response.headers.get("access-control-allow-origin")).toBe("https://https://telemetry.example.invalid");
    expect(response.headers.get("access-control-allow-methods")).toBe("POST, OPTIONS");
    expect(database.rows).toEqual([]);
  });

  test("rejects non-POST telemetry methods", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);

    const response = await worker.fetch(
      new Request("https://https://telemetry.example.invalid/functions/v1/telemetry", { method: "GET" }),
      testEnv(),
    );

    expect(response.status).toBe(405);
    expect(response.headers.get("allow")).toBe("POST, OPTIONS");
    expect(await response.json()).toEqual({ error: "method_not_allowed" });
    expect(database.rows).toEqual([]);
  });

  test("rejects invalid payloads before database insert", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);

    const response = await worker.fetch(
      jsonRequest({ broker_install_id: "broker_1", events: [] }),
      testEnv(),
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "empty_events" });
    expect(database.rows).toEqual([]);
  });

  test("quarantines recoverable per-event validation errors and inserts valid rows", async () => {
    const database = new FakeTelemetryDatabase();
    const posthogCaptures: TelemetryPostHogCapture[] = [];
    const worker = createTestWorker(database, posthogCaptures);
    vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const response = await worker.fetch(
      jsonRequest(validPayload([
        { event_id: "event_bad", origin_runtime: "desktop" },
        { event_id: "event_good" },
      ])),
      testEnv(),
    );

    expect(response.status).toBe(202);
    expect(await response.json()).toEqual({
      accepted_events: 1,
      quarantined_events: 1,
      diagnostics: [{ index: 0, code: "missing_origin_install_id" }],
    });
    expect(database.rows.map((row) => row.event_id)).toEqual(["event_good"]);
    expect(posthogCaptures.map((capture) => capture.eventId)).toEqual(["event_good"]);
    expect(console.warn).toHaveBeenCalledWith("telemetry_events_quarantined", {
      quarantined_events: 1,
      diagnostics: [{ index: 0, code: "missing_origin_install_id" }],
    });
  });

  test("derives legacy desktop surface from origin runtime", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);

    const response = await worker.fetch(
      jsonRequest(validPayload([
        {
          event_id: "event_1",
          origin_install_id: "desktop_install_1",
          origin_runtime: "desktop",
        },
      ])),
      testEnv(),
    );

    expect(response.status).toBe(204);
    expect(database.rows).toHaveLength(1);
    expect(database.rows[0]?.surface).toBe("desktop");
    expect(database.rows[0]?.properties.surface).toBe("desktop");
  });

  test("stores coarse Cloudflare provenance without raw IP data", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);
    const request = jsonRequest(validPayload([{ event_id: "event_1" }]));
    Object.defineProperty(request, "cf", {
      value: {
        asOrganization: "Example Network",
        asn: 64512,
        city: "Austin",
        colo: "dfw",
        country: "us",
        latitude: "30.2672",
        longitude: "-97.7431",
        region: "Texas",
      },
    });

    const response = await worker.fetch(request, testEnv());

    expect(response.status).toBe(204);
    expect(database.rows).toHaveLength(1);
    expect(database.rows[0]?.properties).toMatchObject({
      cf_as_organization: "Example Network",
      cf_asn: 64512,
      cf_colo: "DFW",
      cf_country: "US",
      cf_region: "Texas",
    });
    expect(database.rows[0]?.properties).not.toHaveProperty("city");
    expect(database.rows[0]?.properties).not.toHaveProperty("latitude");
    expect(database.rows[0]?.properties).not.toHaveProperty("longitude");
    expect(database.rows[0]?.properties).not.toHaveProperty("ip");
  });

  test("uses Worker default analytics environment for daemon product events", async () => {
    const database = new FakeTelemetryDatabase();
    const posthogCaptures: TelemetryPostHogCapture[] = [];
    const worker = createTestWorker(database, posthogCaptures);

    const response = await worker.fetch(
      jsonRequest(validPayload([
        {
          event_id: "event_1",
          event_name: "provider_call",
          properties: {
            model_id: "gpt-5",
            provider_id: "openai",
            success: true,
          },
        },
      ])),
      testEnv(),
    );

    expect(response.status).toBe(204);
    expect(database.rows).toHaveLength(1);
    expect(database.rows[0]?.analytics_environment).toBe("production");
    expect(database.rows[0]?.traffic_class).toBe("user");
    expect(database.rows[0]?.properties.analytics_environment).toBe("production");
    expect(posthogCaptures[0]?.properties).toMatchObject({
      analytics_environment: "production",
      broker_runtime: "daemon",
      origin_runtime: "daemon",
      traffic_class: "user",
    });
  });

  test("fails closed for invalid delivery before database insert", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);

    const response = await worker.fetch(
      jsonRequest(validPayload([
        { event_id: "event_bad", delivery: "local_only" },
        { event_id: "event_good" },
      ])),
      testEnv(),
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "invalid_remote_delivery" });
    expect(database.rows).toEqual([]);
  });

  test("fails closed when TELEMETRY_DATABASE_URL is missing", async () => {
    const database = new FakeTelemetryDatabase();
    const worker = createTestWorker(database);

    const response = await worker.fetch(
      jsonRequest(validPayload([{ event_id: "event_1" }])),
      { INSTALL_ID_HASH_SALT: "salt" },
    );

    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({ error: "telemetry_env_not_configured" });
    expect(database.rows).toEqual([]);
  });

  test("uses Neon idempotent insert results to mirror only inserted events", async () => {
    const database = new FakeTelemetryDatabase(new Set(["event_2"]));
    const posthogCaptures: TelemetryPostHogCapture[] = [];
    const worker = createTestWorker(database, posthogCaptures);

    const response = await worker.fetch(
      jsonRequest(validPayload([{ event_id: "event_1" }, { event_id: "event_2" }])),
      testEnv(),
    );

    expect(response.status).toBe(204);
    expect(await response.text()).toBe("");
    expect(database.rows.map((row) => row.event_id)).toEqual(["event_1", "event_2"]);
    expect(posthogCaptures.map((capture) => capture.eventId)).toEqual(["event_2"]);
  });

  test("mirrors to PostHog only after Neon insert succeeds", async () => {
    const order: string[] = [];
    const database = new FakeTelemetryDatabase(new Set(["event_1"]), order);
    const posthogCaptures: TelemetryPostHogCapture[] = [];
    const worker = createTestWorker(database, posthogCaptures, order);

    const response = await worker.fetch(
      jsonRequest(validPayload([{ event_id: "event_1" }])),
      testEnv(),
    );

    expect(response.status).toBe(204);
    expect(order).toEqual(["create_database", "insert", "posthog:event_1"]);
    expect(posthogCaptures).toHaveLength(1);
  });

  test("does not mirror to PostHog when Neon insert fails", async () => {
    const database = new FakeTelemetryDatabase(new Set(), undefined, new Error("db down"));
    const posthogCaptures: TelemetryPostHogCapture[] = [];
    const worker = createTestWorker(database, posthogCaptures);
    vi.spyOn(console, "error").mockImplementation(() => undefined);

    const response = await worker.fetch(
      jsonRequest(validPayload([{ event_id: "event_1" }])),
      testEnv(),
    );

    expect(response.status).toBe(502);
    expect(await response.json()).toEqual({ error: "telemetry_insert_failed" });
    expect(posthogCaptures).toEqual([]);
  });

  test("sends Cloudflare Worker as the PostHog source", async () => {
    const calls: Array<{ url: string; body: unknown }> = [];
    const fetchMock = (async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({
        url: String(input),
        body: init?.body == null ? undefined : JSON.parse(String(init.body)),
      });
      return new Response("ok");
    }) as typeof fetch;

    await capturePostHogEvent(
      {
        eventId: "event_1",
        event: "analytics_pipeline_smoke",
        distinctId: "install:hash",
        properties: {
          origin_runtime: "daemon",
          broker_runtime: "daemon",
          traffic_class: "synthetic",
        },
      },
      {
        POSTHOG_CANARY_PROJECT_API_KEY: "canary-key",
        POSTHOG_HOST: "https://posthog.test/",
      },
      fetchMock,
    );

    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ url: "https://posthog.test/capture/" });
    expect(calls[0]?.body).toMatchObject({
      api_key: "canary-key",
      event: "analytics_pipeline_smoke",
      distinct_id: "install:hash",
      properties: {
        posthog_target: "canary",
        source: "cloudflare_worker",
      },
    });
  });

  test("routes daemon-origin production agent traffic to PostHog production", () => {
    expect(postHogCaptureTargetNames(
      "provider_call",
      {
        analytics_environment: "production",
        app_version: "1.2.3",
        broker_runtime: "daemon",
        model_id: "gpt-5",
        origin_runtime: "daemon",
        plane: "product",
        provider_id: "openai",
        traffic_class: "user",
      },
      {},
    )).toEqual(["production"]);
  });
});

function createTestWorker(
  database: FakeTelemetryDatabase,
  posthogCaptures: TelemetryPostHogCapture[] = [],
  order?: string[],
) {
  return createTelemetryWorker({
    createDatabaseClient(databaseUrl) {
      expect(databaseUrl).toBe("postgres://telemetry.test/db");
      order?.push("create_database");
      return database;
    },
    async capturePostHogEvent(capture) {
      order?.push(`posthog:${capture.eventId}`);
      posthogCaptures.push(capture);
    },
  });
}

function testEnv(): Env {
  return {
    TELEMETRY_DATABASE_URL: "postgres://telemetry.test/db",
    INSTALL_ID_HASH_SALT: "salt",
    TELEMETRY_DEFAULT_ANALYTICS_ENVIRONMENT: "production",
  };
}

function jsonRequest(body: unknown): Request {
  return new Request("https://https://telemetry.example.invalid/functions/v1/telemetry", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

type TestEvent = { event_id: string; properties?: unknown } & Record<string, unknown>;

function isTestRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function validPayload(events: TestEvent[]): unknown {
  return {
    broker_install_id: "broker_install_1",
    broker_runtime: "daemon",
    broker_app_version: "1.2.3",
    broker_os: "darwin",
    broker_arch: "arm64",
    events: events.map(({ event_id, properties, ...overrides }) => ({
      event_id,
      event_name: "analytics_pipeline_smoke",
      event_version: 1,
      occurred_at: "2026-05-30T12:00:00.000Z",
      plane: "product",
      delivery: "remote",
      origin_runtime: "daemon",
      ...overrides,
      properties: {
        provider_id: "fake",
        prompt: "should be dropped",
        safe_count: 1,
        ...(isTestRecord(properties) ? properties : {}),
      },
    })),
  };
}

class FakeTelemetryDatabase implements TelemetryDatabase {
  readonly rows: TelemetryRow[] = [];

  constructor(
    private readonly insertedEventIds: Set<string> = new Set(),
    private readonly order?: string[],
    private readonly failure?: Error,
  ) {}

  async insertTelemetryRows(rows: readonly TelemetryRow[]): Promise<Set<string>> {
    this.order?.push("insert");
    if (this.failure) {
      throw this.failure;
    }
    this.rows.push(...rows);
    return this.insertedEventIds.size > 0
      ? new Set(this.insertedEventIds)
      : new Set(rows.map((row) => row.event_id));
  }
}
