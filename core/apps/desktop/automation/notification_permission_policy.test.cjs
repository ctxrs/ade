const test = require("node:test");
const assert = require("node:assert/strict");

const {
  resolveNotificationPermissionAction,
} = require("./notification_permission_policy.cjs");

test("notification permission policy requests the prompt only for interactive default state", () => {
  assert.deepEqual(
    resolveNotificationPermissionAction({ permission: "default", ci: false }),
    { action: "request", normalizedPermission: "default" },
  );
  assert.deepEqual(
    resolveNotificationPermissionAction({ permission: "default", ci: true }),
    { action: "proceed", normalizedPermission: "default" },
  );
});

test("notification permission policy fails denied and unsupported states", () => {
  assert.deepEqual(
    resolveNotificationPermissionAction({ permission: "denied", ci: true }),
    { action: "fail", normalizedPermission: "denied" },
  );
  assert.deepEqual(
    resolveNotificationPermissionAction({ permission: "unsupported", ci: true }),
    { action: "fail", normalizedPermission: "unsupported" },
  );
});

test("notification permission policy preserves granted state", () => {
  assert.deepEqual(
    resolveNotificationPermissionAction({ permission: "granted", ci: true }),
    { action: "proceed", normalizedPermission: "granted" },
  );
});
