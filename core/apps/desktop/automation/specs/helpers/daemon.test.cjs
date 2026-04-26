const test = require("node:test");
const assert = require("node:assert/strict");

const { shouldRefreshLocalDesktopConnection } = require("./daemon.cjs");

test("daemon helper refreshes local desktop connections", () => {
  assert.equal(shouldRefreshLocalDesktopConnection({ kind: "local" }), true);
  assert.equal(shouldRefreshLocalDesktopConnection({ kind: "ssh" }), false);
  assert.equal(shouldRefreshLocalDesktopConnection({ kind: "none" }), false);
  assert.equal(shouldRefreshLocalDesktopConnection(null), false);
});
