const test = require("node:test");
const assert = require("node:assert/strict");

test("provider runtime helper module loads", () => {
  const runtime = require("./provider_runtime.cjs");
  assert.equal(typeof runtime.getProviderStatus, "function");
  assert.equal(typeof runtime.installProviderAndWait, "function");
  assert.equal(typeof runtime.configureOpenRouterEndpoint, "function");
});
