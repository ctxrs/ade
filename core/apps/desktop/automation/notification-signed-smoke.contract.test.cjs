const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const specPath = path.join(__dirname, "specs", "notification-signed-smoke.spec.cjs");

test("signed notification smoke redacts delivered notification diagnostics", () => {
  const spec = fs.readFileSync(specPath, "utf8");
  assert.match(spec, /summarizeDeliveredSnapshot/);
  assert.match(spec, /summarizeAutomationSnapshot/);
  assert.match(spec, /notificationIdentifiers/);
  assert.doesNotMatch(spec, /JSON\.stringify\(automationSnapshot\)/);
  assert.doesNotMatch(spec, /JSON\.stringify\(lastSnapshot\)/);
  assert.doesNotMatch(spec, /JSON\.stringify\(snapshot\)/);
  assert.doesNotMatch(spec, /last=.*delivered/);
});
