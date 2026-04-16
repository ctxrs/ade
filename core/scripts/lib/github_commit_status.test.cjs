const test = require("node:test");
const assert = require("node:assert/strict");

const {
  DEFAULT_REQUIRED_GITHUB_STATUS_CONTEXTS,
  classifyGitHubCommitStatus,
  ensureGitHubCommitStatusesPassed,
  getLatestGitHubCommitStatusForContext,
  normalizeGitHubRepoUrl,
} = require("./github_commit_status.cjs");

test("GitHub commit status helper keeps the checked-in default Buildkite context explicit", () => {
  assert.deepEqual(DEFAULT_REQUIRED_GITHUB_STATUS_CONTEXTS, ["buildkite/ctx-main"]);
});

test("GitHub commit status helper normalizes GitHub repo URLs", () => {
  assert.deepEqual(normalizeGitHubRepoUrl("https://github.com/ctxrs/ctx-monorepo"), {
    owner: "ctxrs",
    repo: "ctx-monorepo",
  });
  assert.throws(() => normalizeGitHubRepoUrl("https://example.com/ctxrs/ctx-monorepo"), /unsupported GitHub repo URL/);
});

test("GitHub commit status helper selects the newest matching context", () => {
  const latest = getLatestGitHubCommitStatusForContext({
    context: "buildkite/ctx-main",
    statuses: [
      { context: "buildkite/ctx-main", state: "pending", updated_at: "2026-04-16T23:00:00Z" },
      { context: "other", state: "success", updated_at: "2026-04-16T23:05:00Z" },
      { context: "buildkite/ctx-main", state: "success", updated_at: "2026-04-16T23:10:00Z" },
    ],
  });

  assert.equal(latest.state, "success");
});

test("GitHub commit status helper classifies statuses deterministically", () => {
  assert.equal(classifyGitHubCommitStatus(null), "missing");
  assert.equal(classifyGitHubCommitStatus({ state: "pending" }), "pending");
  assert.equal(classifyGitHubCommitStatus({ state: "success" }), "passed");
  assert.equal(classifyGitHubCommitStatus({ state: "failure" }), "failed");
});

test("GitHub commit status helper reuses already-satisfied Buildkite main", async () => {
  const result = await ensureGitHubCommitStatusesPassed({
    commitSha: "abc123",
    contexts: ["buildkite/ctx-main"],
    fetchImpl: async () => ({
      ok: true,
      json: async () => ({
        statuses: [
          {
            context: "buildkite/ctx-main",
            state: "success",
            target_url: "https://buildkite.example/builds/16",
            updated_at: "2026-04-16T23:23:55Z",
          },
        ],
      }),
    }),
    repoUrl: "https://github.com/ctxrs/ctx-monorepo",
  });

  assert.deepEqual(result, {
    required_actions: ["buildkite/ctx-main"],
    checks: [
      {
        action_name: "buildkite/ctx-main",
        initial_state: "passed",
        resolution: "already_satisfied",
        invocation_id: "",
        invocation_url: "https://buildkite.example/builds/16",
        source: "github_commit_status",
      },
    ],
  });
});

test("GitHub commit status helper waits for pending Buildkite main to pass", async () => {
  let callCount = 0;

  const result = await ensureGitHubCommitStatusesPassed({
    commitSha: "abc123",
    contexts: ["buildkite/ctx-main"],
    fetchImpl: async () => {
      callCount += 1;
      const state = callCount < 2 ? "pending" : "success";
      return {
        ok: true,
        json: async () => ({
          statuses: [
            {
              context: "buildkite/ctx-main",
              state,
              target_url: "https://buildkite.example/builds/16",
              updated_at: "2026-04-16T23:23:55Z",
            },
          ],
        }),
      };
    },
    pollMs: 0,
    repoUrl: "https://github.com/ctxrs/ctx-monorepo",
    sleepImpl: async () => {},
  });

  assert.equal(callCount, 2);
  assert.equal(result.checks[0].initial_state, "pending");
  assert.equal(result.checks[0].resolution, "waited_for_github_status");
});

test("GitHub commit status helper fails fast on red Buildkite main", async () => {
  await assert.rejects(
    ensureGitHubCommitStatusesPassed({
      commitSha: "abc123",
      contexts: ["buildkite/ctx-main"],
      fetchImpl: async () => ({
        ok: true,
        json: async () => ({
          statuses: [
            {
              context: "buildkite/ctx-main",
              state: "failure",
              description: "Build #17 failed",
              target_url: "https://buildkite.example/builds/17",
              updated_at: "2026-04-16T23:24:55Z",
            },
          ],
        }),
      }),
      repoUrl: "https://github.com/ctxrs/ctx-monorepo",
    }),
    /required GitHub commit status 'buildkite\/ctx-main' failed for commit abc123/,
  );
});
