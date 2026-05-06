import {
  assertEquals,
  assertRejects,
} from "https://deno.land/std@0.224.0/assert/mod.ts";
import {
  buildTelemetryIngestPlan,
  TelemetryIngestError,
} from "./telemetry_ingest.ts";

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
  assertEquals("workspace_id" in plan.rows[0].properties, false);
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
});
