import { describe, expect, test } from "vitest";

import {
  buildReadinessReport,
  parseArgs,
  parseCloudflareSecretNames,
  parseCloudflareWorkerNames,
  parsePsqlRows,
  parseWranglerToml,
  stableJson,
} from "../scripts/cloudflare-neon-readiness.mjs";

describe("cloudflare neon readiness script", () => {
  test("requires explicit mutation for Neon role creation", () => {
    expect(() => parseArgs(["--ensure-neon-roles"])).toThrow(/--mutate/u);
    expect(parseArgs(["--ensure-neon-roles", "--mutate"]).ensureNeonRoles).toBe(true);
  });

  test("env validation never serializes secret values", async () => {
    const env = {
      CLOUDFLARE_ACCOUNT_ID: "account-id",
      CLOUDFLARE_API_TOKEN: "cf-secret-token",
      NEON_API_KEY: "neon-secret-token",
      NEON_PROJECT_ID: "neon-project",
      RELAY_AUTHORITY_DATABASE_URL: "postgresql://ctx_relay_authority_app:db-secret@db.example.test/neondb?sslmode=require",
      RELAY_AUTHORITY_BEARER_TOKEN: "shared-secret",
      RELAY_AUTHORITY_CONTROL_PLANE_JWKS: "{\"keys\":[]}",
      AUTHORITY_BEARER_TOKEN: "shared-secret",
      CONTROL_PLANE_JWKS: "{\"keys\":[]}",
      OPENAI_API_KEY: "openai-secret",
    };

    const report = await buildReadinessReport({
      argv: ["--no-infisical", "--skip-cloudflare", "--skip-neon-api"],
      env,
      spawnSync: unavailableSpawn,
    });
    const serialized = stableJson(report);

    expect(report.env.missing).toEqual([]);
    expect(serialized).not.toContain("cf-secret-token");
    expect(serialized).not.toContain("neon-secret-token");
    expect(serialized).not.toContain("db-secret");
    expect(serialized).not.toContain("openai-secret");
    expect(report.env.equality).toContainEqual({
      comparable: true,
      names: ["AUTHORITY_BEARER_TOKEN", "RELAY_AUTHORITY_BEARER_TOKEN"],
      same_value: true,
    });
  });

  test("telemetry profile requires Worker secrets and Neon telemetry storage without serializing values", async () => {
    const env = {
      CLOUDFLARE_ACCOUNT_ID: "account-id",
      CLOUDFLARE_API_TOKEN: "cf-secret-token",
      NEON_API_KEY: "neon-secret-token",
      NEON_PROJECT_ID: "neon-project",
      TELEMETRY_DATABASE_URL: "postgresql://ctx_telemetry_ingest:db-secret@db.example.test/neondb?sslmode=require",
      INSTALL_ID_HASH_SALT: "install-salt",
      POSTHOG_CANARY_PROJECT_API_KEY: "posthog-canary-secret",
    };

    const report = await buildReadinessReport({
      argv: [
        "--profile",
        "telemetry",
        "--wrangler-config",
        "../telemetry-worker/wrangler.toml",
        "--worker-name",
        "ctx-telemetry",
        "--no-infisical",
        "--skip-cloudflare",
        "--skip-neon-api",
      ],
      env,
      spawnSync: unavailableSpawn,
    });
    const serialized = stableJson(report);

    expect(report.env.missing).toEqual([]);
    expect(report.mode.profile).toBe("telemetry");
    expect(report.neon.database).toMatchObject({
      checked: false,
      skip_reason: "disabled_by_profile",
    });
    expect(serialized).not.toContain("cf-secret-token");
    expect(serialized).not.toContain("neon-secret-token");
    expect(serialized).not.toContain("db-secret");
    expect(serialized).not.toContain("install-salt");
    expect(serialized).not.toContain("posthog-canary-secret");
    expect(report.failures).toEqual([]);
    expect(report.wrangler.environments).toContainEqual(
      expect.objectContaining({
        missing_vars: [],
        name: "prod",
        required_vars: ["POSTHOG_HOST"],
      }),
    );
  });

  test("wrangler parser reports env-specific vars instead of inheriting root vars", () => {
    const wrangler = parseWranglerToml(`
name = "ctx-llm-relay"

[vars]
ENVIRONMENT = "dev"
AUTHORITY_BASE_URL = "http://127.0.0.1:8792"

[env.staging.vars]
ENVIRONMENT = "staging"
GRANT_VERIFICATION_MODE = "signed_chain"
`);

    expect(wrangler.name).toBe("ctx-llm-relay");
    expect(Object.keys(wrangler.vars).sort()).toEqual(["AUTHORITY_BASE_URL", "ENVIRONMENT"]);
    expect(Object.keys(wrangler.env.staging.vars).sort()).toEqual([
      "ENVIRONMENT",
      "GRANT_VERIFICATION_MODE",
    ]);
  });

  test("parses Cloudflare list responses without secret text", () => {
    expect(parseCloudflareWorkerNames([{ id: "ctx-llm-relay" }, { name: "other" }])).toEqual([
      "ctx-llm-relay",
      "other",
    ]);
    expect(
      parseCloudflareSecretNames({
        items: [
          { name: "OPENAI_API_KEY", text: "must-not-serialize" },
          { name: "AUTHORITY_BEARER_TOKEN", text: "must-not-serialize-either" },
        ],
      }),
    ).toEqual(["AUTHORITY_BEARER_TOKEN", "OPENAI_API_KEY"]);
  });

  test("parses psql tuples for role and table checks", () => {
    expect(parsePsqlRows("ctx_relay_authority_app|t|f\n", ["name", "can_login", "superuser"])).toEqual([
      {
        can_login: "t",
        name: "ctx_relay_authority_app",
        superuser: "f",
      },
    ]);
  });
});

function unavailableSpawn() {
  return {
    error: new Error("unavailable"),
    status: 1,
    stderr: "",
    stdout: "",
  };
}
