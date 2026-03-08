const test = require("node:test");
const assert = require("node:assert/strict");

const { assertNoLocationRegression } = require("./specs/helpers/workspace_wizard_flow.cjs");

test("assertNoLocationRegression accepts monotonic wizard progress", () => {
  assert.doesNotThrow(() => {
    assertNoLocationRegression([
      { step: "location", pathname: "/workspace-setup" },
      { step: "container", pathname: "/workspace-setup" },
      { step: "source", pathname: "/workspace-setup" },
      { step: "confirm", pathname: "/workspace-setup" },
      { step: "", pathname: "/workspaces/ws_123" },
    ]);
  });
});

test("assertNoLocationRegression rejects returning to location after advancing", () => {
  assert.throws(
    () => {
      assertNoLocationRegression([
        { step: "location", pathname: "/workspace-setup" },
        { step: "container", pathname: "/workspace-setup" },
        { step: "location", pathname: "/workspace-setup" },
      ]);
    },
    /wizard regressed to location after advancing/,
  );
});
