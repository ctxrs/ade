const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  ACTIVE_BUDGETS_ENV,
  HOST_HEAVY_BUDGET_KEY,
  LEASE_ID_ENV,
  computeDefaultHeavyBudgetSlots,
  resolveBudgetEnvKey,
  withHostJobBudget,
} = require("./host_job_budget.cjs");

test("computeDefaultHeavyBudgetSlots keeps the checked-in host-heavy fanout bounded", () => {
  assert.equal(computeDefaultHeavyBudgetSlots(4), 1);
  assert.equal(computeDefaultHeavyBudgetSlots(8), 2);
  assert.equal(computeDefaultHeavyBudgetSlots(16), 3);
  assert.equal(computeDefaultHeavyBudgetSlots(32), 4);
});

test("withHostJobBudget acquires a slot and restores the env afterwards", () => {
  const budgetRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-host-budget-"));
  const env = {};
  let lockSeen = false;

  withHostJobBudget({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    budgetRoot,
    command: "budget-test",
    env,
    slots: 1,
    timeoutMs: 5,
  }, () => {
    lockSeen = fs.existsSync(path.join(budgetRoot, HOST_HEAVY_BUDGET_KEY, "slot-0.lock"));
    assert.equal(env[ACTIVE_BUDGETS_ENV], HOST_HEAVY_BUDGET_KEY);
    assert.match(String(env[LEASE_ID_ENV] || ""), /\S/);
  });

  assert.equal(lockSeen, true);
  assert.equal(env[ACTIVE_BUDGETS_ENV], undefined);
  assert.equal(env[LEASE_ID_ENV], undefined);
});

test("withHostJobBudget is reentrant for the same active budget", () => {
  const budgetRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-host-budget-reentrant-"));
  const env = {
    [ACTIVE_BUDGETS_ENV]: HOST_HEAVY_BUDGET_KEY,
    [LEASE_ID_ENV]: "lease-existing",
  };

  withHostJobBudget({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    budgetRoot,
    command: "nested-budget",
    env,
    slots: 1,
    timeoutMs: 5,
  }, () => {});

  assert.equal(fs.existsSync(path.join(budgetRoot, HOST_HEAVY_BUDGET_KEY)), false);
});

test("withHostJobBudget reports the current holder on timeout", () => {
  const budgetRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-host-budget-timeout-"));
  const slotDir = path.join(budgetRoot, HOST_HEAVY_BUDGET_KEY);
  fs.mkdirSync(slotDir, { recursive: true });
  fs.writeFileSync(
    path.join(slotDir, "slot-0.lock"),
    `${JSON.stringify({ pid: process.pid, command: "other-build", leaseId: "lease-789" })}\n`,
    "utf8",
  );

  assert.throws(
    () => withHostJobBudget({
      budgetKey: HOST_HEAVY_BUDGET_KEY,
      budgetRoot,
      command: "blocked-build",
      env: {},
      slots: 1,
      staleMs: 60_000,
      timeoutMs: 0,
    }, () => {}),
    /other-build/,
  );
});

test("withHostJobBudget honors explicit slot env overrides", () => {
  const budgetRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-host-budget-env-"));
  const env = {
    [resolveBudgetEnvKey(HOST_HEAVY_BUDGET_KEY)]: "1",
  };

  withHostJobBudget({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    budgetRoot,
    command: "env-override-budget",
    env,
    timeoutMs: 5,
  }, () => {
    assert.equal(fs.existsSync(path.join(budgetRoot, HOST_HEAVY_BUDGET_KEY, "slot-0.lock")), true);
  });
});
