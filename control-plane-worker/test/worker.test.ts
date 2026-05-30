import { describe, expect, test } from "vitest";

import { createControlPlaneWorker } from "../src/worker";

const worker = createControlPlaneWorker();

describe("control plane worker", () => {
  test("returns a free local entitlement snapshot", async () => {
    const response = await worker.fetch(new Request("https://api.ctx.rs/v1/entitlements"), {});
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toMatchObject({
      plan_type: "free_local",
      features: {
        mobile_relay: "disabled",
        org_admin: "disabled",
      },
    });
  });

  test("keeps team state explicitly empty while prelaunch", async () => {
    const response = await worker.fetch(new Request("https://api.ctx.rs/v1/team/state"), {});

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      organizations: [],
      active_org_id: null,
      active_invites: [],
      feature_grants: [],
      active_admin_state: null,
    });
  });

  test("rejects team mutations as prelaunch disabled", async () => {
    const response = await worker.fetch(
      new Request("https://api.ctx.rs/v1/team/admin", { method: "POST" }),
      {},
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toMatchObject({ error: "feature_unavailable" });
  });

  test("keeps managed mobile tunnel grant minting explicit while prelaunch", async () => {
    const response = await worker.fetch(
      new Request("https://api.ctx.rs/v1/mobile/tunnel-grant", { method: "POST" }),
      {},
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toMatchObject({ error: "feature_unavailable" });
  });

  test("handles CORS preflight", async () => {
    const response = await worker.fetch(
      new Request("https://api.ctx.rs/v1/entitlements", {
        method: "OPTIONS",
        headers: { origin: "https://ctx.rs" },
      }),
      {},
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("access-control-allow-origin")).toBe("https://ctx.rs");
    expect(response.headers.get("access-control-allow-credentials")).toBe("true");
    expect(response.headers.get("access-control-allow-headers")).toContain("x-ctx-active-org-id");
  });

  test("allows packaged desktop CORS origins", async () => {
    for (const origin of ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"]) {
      const response = await worker.fetch(
        new Request("https://api.ctx.rs/v1/entitlements", {
          method: "OPTIONS",
          headers: { origin },
        }),
        {},
      );

      expect(response.status).toBe(200);
      expect(response.headers.get("access-control-allow-origin")).toBe(origin);
      expect(response.headers.get("access-control-allow-credentials")).toBe("true");
    }
  });

  test("rejects credentialed CORS for untrusted origins", async () => {
    const response = await worker.fetch(
      new Request("https://api.ctx.rs/v1/entitlements", {
        method: "OPTIONS",
        headers: { origin: "https://evil.example" },
      }),
      {},
    );

    expect(response.status).toBe(403);
    expect(response.headers.get("access-control-allow-origin")).toBeNull();
    expect(response.headers.get("access-control-allow-credentials")).toBeNull();
  });
});
