const WEB_SMOKE_BAZEL_TARGETS = Object.freeze([
  "//core/packages/session-supervisor-core:unit_tests",
  "//core/packages/session-thread-layout:unit_smoke",
  "//core/apps/web:unit_smoke",
]);

function formatWebSmokeBazelPilotCommand() {
  return ["node", "scripts/run_bazel_pilot.cjs", "test", ...WEB_SMOKE_BAZEL_TARGETS].join(" ");
}

module.exports = {
  formatWebSmokeBazelPilotCommand,
  WEB_SMOKE_BAZEL_TARGETS,
};
