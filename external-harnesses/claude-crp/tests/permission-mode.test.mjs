import { test } from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = path.resolve(import.meta.dirname, "..");

async function loadRuntimeModule() {
  return import(pathToFileURL(path.join(rootDir, "dist", "runtime.js")).href);
}

test("resolvePermissionSettings honors explicit provider mode from env", async () => {
  const { resolvePermissionSettings } = await loadRuntimeModule();

  const resolved = resolvePermissionSettings({
    env: {
      CTX_PROVIDER_MODE: "bypassPermissions",
    },
  });

  assert.deepEqual(resolved, {
    permissionMode: "bypassPermissions",
    allowDangerouslySkipPermissions: true,
  });
});

test("resolvePermissionSettings derives bypassPermissions from full-access CRP config", async () => {
  const { resolvePermissionSettings } = await loadRuntimeModule();

  const resolved = resolvePermissionSettings({
    config: {
      approval_policy: "never",
      sandbox_mode: "danger-full-access",
    },
    env: {},
  });

  assert.deepEqual(resolved, {
    permissionMode: "bypassPermissions",
    allowDangerouslySkipPermissions: true,
  });
});

test("resolvePermissionSettings falls back to default when no override is present", async () => {
  const { resolvePermissionSettings } = await loadRuntimeModule();

  const resolved = resolvePermissionSettings({
    config: {},
    env: {},
  });

  assert.deepEqual(resolved, {
    permissionMode: "default",
    allowDangerouslySkipPermissions: false,
  });
});

test("buildPermissionControlOptions only auto-allows in default mode", async () => {
  const { buildPermissionControlOptions } = await loadRuntimeModule();

  const defaultOptions = buildPermissionControlOptions("default", false);
  assert.equal(defaultOptions.permissionMode, "default");
  assert.equal(typeof defaultOptions.canUseTool, "function");
  assert.equal(defaultOptions.allowDangerouslySkipPermissions, undefined);

  const bypassOptions = buildPermissionControlOptions("bypassPermissions", true);
  assert.equal(bypassOptions.permissionMode, "bypassPermissions");
  assert.equal(bypassOptions.allowDangerouslySkipPermissions, true);
  assert.equal("canUseTool" in bypassOptions, false);

  const acceptEditsOptions = buildPermissionControlOptions("acceptEdits", false);
  assert.equal(acceptEditsOptions.permissionMode, "acceptEdits");
  assert.equal("canUseTool" in acceptEditsOptions, false);
});
