const test = require("node:test");
const assert = require("node:assert/strict");

const {
  computeSummary,
  markdownFromSummary,
} = require("./linux_arm_provider_report_summary.cjs");

test("summary computes pass/fail distributions", () => {
  const summary = computeSummary({
    generated_at: "2026-03-05T00:00:00Z",
    results: [
      { provider_id: "codex-crp", result: "pass" },
      { provider_id: "goose", result: "fail", category: "external_outage", stage: "first_turn", error_code: "timeout", reason: "upstream timeout" },
      { provider_id: "opencode", result: "fail", category: "product_regression", stage: "verify", error_code: "health_check_failed", reason: "verify failed" },
    ],
  });

  assert.equal(summary.provider_total, 3);
  assert.equal(summary.provider_passed, 1);
  assert.equal(summary.provider_failed, 2);
  assert.equal(summary.failure_categories.external_outage, 1);
  assert.equal(summary.failure_categories.product_regression, 1);
});

test("markdown summary includes failing provider table", () => {
  const markdown = markdownFromSummary({
    source_generated_at: "2026-03-05T00:00:00Z",
    provider_total: 1,
    provider_passed: 0,
    provider_failed: 1,
    pass_rate: 0,
    failure_categories: { environment: 1 },
    failing_providers: [
      {
        provider_id: "codex-crp",
        stage: "install",
        error_code: "download_failed",
        category: "environment",
        reason: "sandbox runtime unavailable",
      },
    ],
  });

  assert.match(markdown, /Linux ARM Provider Reliability Summary/);
  assert.match(markdown, /\| codex \| install \| download_failed \| environment \| sandbox runtime unavailable \|/);
});
