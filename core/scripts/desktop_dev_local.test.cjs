const assert = require("node:assert/strict");
const test = require("node:test");

const {
  envFlagEnabled,
} = require("./desktop_dev_local.cjs");

test("envFlagEnabled respects common false-y values", () => {
  assert.equal(envFlagEnabled("", true), true);
  assert.equal(envFlagEnabled("0", true), false);
  assert.equal(envFlagEnabled("false", true), false);
  assert.equal(envFlagEnabled("no", true), false);
});

test("envFlagEnabled treats non-false-y values as enabled", () => {
  assert.equal(envFlagEnabled("1", false), true);
  assert.equal(envFlagEnabled("true", false), true);
  assert.equal(envFlagEnabled("yes", false), true);
});

test("envFlagEnabled rejects invalid values", () => {
  assert.throws(() => envFlagEnabled("maybe", false), /Invalid desktop dev flag/);
});
