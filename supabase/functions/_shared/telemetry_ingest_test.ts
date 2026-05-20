import {
  assertEquals,
  assertRejects,
} from "https://deno.land/std@0.224.0/assert/mod.ts";
import {
  buildTelemetryIngestPlan,
  selectPostHogCapturesForInsertedRows,
  TelemetryIngestError,
} from "./telemetry_ingest.ts";
import { postHogCaptureTargetNames } from "./posthog.ts";

const basePayload = () => ({
  broker_install_id: "broker-install-1",
  broker_runtime: "daemon",
  broker_app_version: "1.2.3",
  broker_os: "macos",
  broker_arch: "arm64",
  events: [
    {
      event_id: "event-1",
      event_name: "app_opened",
      event_version: 1,
      occurred_at: "2026-05-06T12:00:00.000Z",
      plane: "product",
      delivery: "remote",
      origin_runtime: "desktop",
      origin_install_id: "origin-install-1",
      app_version: "1.2.3",
      os: "macos",
      arch: "arm64",
      surface: "desktop",
      env_target: "remote",
      source: "app_bridge",
      properties: {
        launch_surface: "desktop",
        analytics_environment: "production",
        traffic_class: "user",
        workspace_id: "raw-workspace-id",
      },
    },
  ],
});

Deno.test("buildTelemetryIngestPlan stores canonical hashed ids and mirrors by origin install", async () => {
  const plan = await buildTelemetryIngestPlan(basePayload(), {
    idSalt: "test-salt",
    now: () => new Date("2026-05-06T12:00:01.000Z"),
  });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].event_name, "app_opened");
  assertEquals(plan.rows[0].plane, "product");
  assertEquals(plan.rows[0].origin_runtime, "desktop");
  assertEquals(Boolean(plan.rows[0].origin_install_id_hash), true);
  assertEquals(Boolean(plan.rows[0].broker_install_id_hash), true);
  assertEquals(plan.rows[0].properties.launch_surface, "desktop");
  assertEquals(plan.rows[0].analytics_environment, "production");
  assertEquals(plan.rows[0].traffic_class, "user");
  assertEquals(plan.rows[0].properties.traffic_class, "user");
  assertEquals("workspace_id" in plan.rows[0].properties, false);
  assertEquals(plan.posthogCaptures[0].eventId, plan.rows[0].event_id);
  assertEquals(
    plan.posthogCaptures[0].distinctId,
    `install:${plan.rows[0].origin_install_id_hash}`,
  );
  assertEquals(
    plan.posthogCaptures[0].properties.origin_install_id_hash,
    plan.rows[0].origin_install_id_hash,
  );
  assertEquals(
    plan.posthogCaptures[0].properties.broker_install_id_hash,
    plan.rows[0].broker_install_id_hash,
  );
});

Deno.test("buildTelemetryIngestPlan rejects non-daemon events without origin install ids", async () => {
  const payload = basePayload();
  delete (payload.events[0] as Record<string, unknown>).origin_install_id;

  const error = await assertRejects(
    () => buildTelemetryIngestPlan(payload, { idSalt: "test-salt" }),
    TelemetryIngestError,
  );
  assertEquals(error.status, 400);
  assertEquals(error.code, "missing_origin_install_id");
});

Deno.test("buildTelemetryIngestPlan rejects legacy aliases and unknown event fields", async () => {
  const payload = basePayload();
  const event = payload.events[0] as Record<string, unknown>;
  event.name = event.event_name;
  delete event.event_name;

  const error = await assertRejects(
    () => buildTelemetryIngestPlan(payload, { idSalt: "test-salt" }),
    TelemetryIngestError,
  );
  assertEquals(error.status, 400);
  assertEquals(error.code, "unknown_event_field");
});

Deno.test("buildTelemetryIngestPlan rejects client-origin events without surface", async () => {
  const payload = basePayload();
  delete (payload.events[0] as Record<string, unknown>).surface;

  const error = await assertRejects(
    () => buildTelemetryIngestPlan(payload, { idSalt: "test-salt" }),
    TelemetryIngestError,
  );
  assertEquals(error.status, 400);
  assertEquals(error.code, "missing_surface");
});

Deno.test("buildTelemetryIngestPlan accepts daemon-origin events by using broker install identity", async () => {
  const payload = basePayload();
  delete (payload.events[0] as Record<string, unknown>).origin_install_id;
  payload.events[0].origin_runtime = "daemon";
  delete (payload.events[0] as Record<string, unknown>).surface;

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(Boolean(plan.rows[0].origin_install_id_hash), true);
  assertEquals(
    plan.rows[0].origin_install_id_hash,
    plan.rows[0].broker_install_id_hash,
  );
  assertEquals(plan.posthogCaptures.length, 1);
  assertEquals(plan.posthogCaptures[0].eventId, plan.rows[0].event_id);
});

Deno.test("buildTelemetryIngestPlan queues staging client events for canary routing", async () => {
  const payload = basePayload();
  const properties = payload.events[0].properties as Record<string, unknown>;
  properties.analytics_environment = "staging";

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].properties.analytics_environment, "staging");
  assertEquals(plan.rows[0].traffic_class, "internal");
  assertEquals(plan.posthogCaptures.length, 1);
  assertEquals(plan.posthogCaptures[0].properties.traffic_class, "internal");
});

Deno.test("buildTelemetryIngestPlan queues fake provider events for canary routing", async () => {
  const payload = basePayload();
  const event = payload.events[0] as Record<string, unknown>;
  event.event_name = "turn_started";
  event.provider_id = "fake";
  const properties = payload.events[0].properties as Record<string, unknown>;
  properties.provider_id = "fake";

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].provider_id, "fake");
  assertEquals(plan.rows[0].traffic_class, "synthetic");
  assertEquals(plan.posthogCaptures.length, 1);
  assertEquals(plan.posthogCaptures[0].properties.provider_id, "fake");
});

Deno.test("buildTelemetryIngestPlan copies top-level fake model into routing properties", async () => {
  const payload = basePayload();
  const event = payload.events[0] as Record<string, unknown>;
  event.event_name = "provider_call";
  event.model_id = "fake-model";

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].model_id, "fake-model");
  assertEquals(plan.rows[0].traffic_class, "synthetic");
  assertEquals(plan.posthogCaptures.length, 1);
  assertEquals(plan.posthogCaptures[0].properties.model_id, "fake-model");
  assertEquals(plan.posthogCaptures[0].properties.traffic_class, "synthetic");
});

Deno.test("buildTelemetryIngestPlan queues local build events for canary routing", async () => {
  const payload = basePayload();
  payload.events[0].app_version = "0.0.0-dev";

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].app_version, "0.0.0-dev");
  assertEquals(plan.rows[0].traffic_class, "synthetic");
  assertEquals(plan.posthogCaptures.length, 1);
  assertEquals(plan.posthogCaptures[0].properties.app_version, "0.0.0-dev");
});

Deno.test("buildTelemetryIngestPlan lets canonical fields override client properties before routing", async () => {
  const payload = basePayload();
  const event = payload.events[0] as Record<string, unknown>;
  event.event_name = "incident_reported";
  event.plane = "incident";
  const properties = event.properties as Record<string, unknown>;
  properties.plane = "product";
  properties.app_version = "9.9.9";

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].plane, "incident");
  assertEquals(plan.rows[0].properties.plane, "incident");
  assertEquals(plan.rows[0].properties.app_version, "1.2.3");
  assertEquals(plan.posthogCaptures[0].properties.plane, "incident");
  assertEquals(plan.posthogCaptures[0].properties.app_version, "1.2.3");
  assertEquals(
    postHogCaptureTargetNames(
      plan.posthogCaptures[0].event,
      plan.posthogCaptures[0].properties,
    ),
    [],
  );
});

Deno.test("buildTelemetryIngestPlan preserves canonical routing fields with max-size client properties", async () => {
  const payload = basePayload();
  const event = payload.events[0] as Record<string, unknown>;
  event.event_name = "provider_call";
  event.provider_id = "fake";
  const properties = event.properties as Record<string, unknown>;
  for (let idx = 0; idx < 48; idx += 1) {
    properties[`custom_${idx}`] = `value-${idx}`;
  }

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].traffic_class, "synthetic");
  assertEquals(plan.rows[0].properties.provider_id, "fake");
  assertEquals(plan.posthogCaptures[0].properties.provider_id, "fake");
  assertEquals(plan.posthogCaptures[0].properties.traffic_class, "synthetic");
  assertEquals(
    postHogCaptureTargetNames(
      plan.posthogCaptures[0].event,
      plan.posthogCaptures[0].properties,
    ),
    ["canary"],
  );
});

Deno.test("buildTelemetryIngestPlan still mirrors pipeline smoke health events", async () => {
  const payload = basePayload();
  const event = payload.events[0] as Record<string, unknown>;
  event.event_name = "analytics_pipeline_smoke";
  event.plane = "incident";
  event.app_version = "0.0.0-smoke";
  event.properties = {
    smoke_run_id: "smoke-1",
    analytics_environment: "pipeline_smoke",
  };

  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  assertEquals(plan.rows.length, 1);
  assertEquals(plan.rows[0].traffic_class, "synthetic");
  assertEquals(plan.posthogCaptures.length, 1);
  assertEquals(plan.posthogCaptures[0].properties.traffic_class, "synthetic");
});

Deno.test("selectPostHogCapturesForInsertedRows matches captures by event id after filtering", async () => {
  const payload = basePayload() as unknown as Record<string, unknown> & {
    events: Array<Record<string, unknown>>;
  };
  const baseEvent = payload.events[0];
  const baseProperties = baseEvent.properties as Record<string, unknown>;
  payload.events = [
    {
      ...baseEvent,
      event_id: "event-fake",
      event_name: "turn_started",
      provider_id: "fake",
      properties: {
        ...baseProperties,
        provider_id: "fake",
      },
    },
    {
      ...baseEvent,
      event_id: "event-real",
      event_name: "app_opened",
    },
  ];
  const plan = await buildTelemetryIngestPlan(payload, { idSalt: "test-salt" });

  const captures = selectPostHogCapturesForInsertedRows(
    plan.posthogCaptures,
    [plan.rows[1]],
  );

  assertEquals(plan.posthogCaptures.length, 2);
  assertEquals(captures.length, 1);
  assertEquals(captures[0].eventId, "event-real");
});
