const assert = require("node:assert/strict");
const test = require("node:test");

const {
  classifyEnvironmentFailure,
} = require("./lib/environment_failure_classifier.cjs");

const EXACT_SHA = "ebff39ba6fcb3bb6e36355080630c4bb805ae6da";

test("classifies known environment failures from raw evidence snippets", () => {
  const cases = [
    {
      classId: "posthog_gateway_timeout",
      text: "PostHog capture failed: HTTP 504 Gateway Timeout from us.i.posthog.com",
    },
    {
      classId: "private_handoff_artifact_download_reset",
      text: "private handoff artifact download failed: ECONNRESET connection reset by peer",
    },
    {
      classId: "cloud_capacity",
      text: "Buildkite waiting for agent: no agents available in queue, capacity wait",
    },
    {
      classId: "infisical_auth_expired",
      text: "infisical run failed: session expired, login required",
    },
    {
      classId: "buildkite_agent_lost",
      text: "Buildkite agent lost communication with the job and the agent disappeared",
    },
    {
      classId: "disk_pressure",
      text: "failed to write target artifact: ENOSPC: no space left on device",
    },
    {
      classId: "webkit_process_killed_by_host",
      text: "WebKitWebProcess exited with signal 9; WebKitWebDriver reports process killed",
    },
    {
      classId: "buildbuddy_remote_cache_input_error",
      text: "BuildBuddy remote cache failed to fetch inputs: CAS digest not found",
    },
    {
      classId: "bazel_external_repository_fetch_5xx",
      text: "Bazel external repository rules_python failed: Error downloading https://bcr.bazel.build/modules/rules_python: GET returned 502 Bad Gateway",
    },
  ];

  for (const entry of cases) {
    const classification = classifyEnvironmentFailure(entry.text, { commitSha: EXACT_SHA });
    assert.equal(classification.failure_kind, "environment", entry.classId);
    assert.equal(classification.class_id, entry.classId);
    assert.equal(classification.release_blocking, true);
    assert.equal(classification.requires_exact_sha_rerun, true);
    assert.equal(classification.rerun_requirement.commit_sha, EXACT_SHA);
    assert.equal(classification.rerun_requirement.exact_sha_pass_required, true);
    assert.ok(classification.evidence.length > 0);
    assert.ok(classification.evidence[0].snippet.length <= 360);
  }
});

test("treats ambiguous failures as product failures without retry policy", () => {
  const classification = classifyEnvironmentFailure("unit test failed after ECONNRESET was printed by app code");
  assert.equal(classification.failure_kind, "product");
  assert.equal(classification.class_id, "product_failure");
  assert.equal(classification.release_blocking, true);
  assert.equal(classification.requires_exact_sha_rerun, false);
  assert.equal(classification.retry_policy.retryable, false);
  assert.equal(classification.evidence.length, 0);
});

test("does not classify benign free-space mentions as disk pressure", () => {
  const classification = classifyEnvironmentFailure("free space check passed before product assertion failed");
  assert.equal(classification.failure_kind, "product");
  assert.equal(classification.class_id, "product_failure");
});

test("does not classify deterministic Bazel download failures as retryable 5xx", () => {
  const classification = classifyEnvironmentFailure(
    "Bazel external repository rules_python failed: Error downloading https://bcr.bazel.build/modules/rules_python: GET returned 404 Not Found",
  );
  assert.equal(classification.failure_kind, "product");
  assert.equal(classification.class_id, "product_failure");
  assert.equal(classification.retry_policy.retryable, false);
});

test("does not classify Bazel-run product HTTP failures as external dependency failures", () => {
  const classification = classifyEnvironmentFailure(
    "bazel test //core/apps/web:api_tests failed: ctx JSON route returned HTTP 503 for /api/tasks",
  );
  assert.equal(classification.failure_kind, "product");
  assert.equal(classification.class_id, "product_failure");
  assert.equal(classification.retry_policy.retryable, false);
});

test("marks environment retry policy exhausted at the class max retry count", () => {
  const classification = classifyEnvironmentFailure("infisical auth failed: token expired with 401", {
    commitSha: EXACT_SHA,
    retryAttempt: 1,
  });
  assert.equal(classification.class_id, "infisical_auth_expired");
  assert.equal(classification.retry_policy.max_retries, 1);
  assert.equal(classification.retry_policy.retry_attempt, 1);
  assert.equal(classification.retry_policy.retry_allowed, false);
  assert.equal(classification.retry_policy.retry_exhausted, true);
});

test("collects nested Buildkite build/job evidence for readiness summaries", () => {
  const classification = classifyEnvironmentFailure({
    number: 401,
    state: "failed",
    jobs: [
      {
        label: "release updater web e2e",
        log_excerpt: "WebKitWebDriver startup failed: WebKitWebProcess was killed by host with SIGKILL",
      },
    ],
  }, { commitSha: EXACT_SHA });
  assert.equal(classification.class_id, "webkit_process_killed_by_host");
  assert.equal(classification.evidence[0].source, "build.jobs[0].log_excerpt");
});
