const assert = require("node:assert/strict");
const test = require("node:test");

const {
  parseBoolish,
  resolveBoolishFlag,
  stringMapFlag,
} = require("./boolish.cjs");

test("parseBoolish accepts the shared vocabulary", () => {
  assert.equal(parseBoolish("1"), true);
  assert.equal(parseBoolish(" yes "), true);
  assert.equal(parseBoolish("OFF"), false);
  assert.equal(parseBoolish("0"), false);
});

test("resolveBoolishFlag applies defaults and rejects invalid values", () => {
  assert.equal(resolveBoolishFlag("", true, "TEST_FLAG"), true);
  assert.equal(resolveBoolishFlag("false", true, "TEST_FLAG"), false);
  assert.throws(
    () => resolveBoolishFlag("maybe", false, "TEST_FLAG"),
    /Invalid TEST_FLAG/,
  );
});

test("stringMapFlag only enables parsed true values", () => {
  assert.equal(stringMapFlag({ install_running: "true" }, "install_running"), true);
  assert.equal(stringMapFlag({ install_running: "1" }, "install_running"), true);
  assert.equal(stringMapFlag({ install_running: "false" }, "install_running"), false);
  assert.equal(stringMapFlag({ install_running: "maybe" }, "install_running"), false);
});
