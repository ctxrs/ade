const WEB_SMOKE_BAZEL_TARGETS = Object.freeze([
  "//core/packages/session-supervisor-core:unit_tests",
  "//core/packages/session-thread-layout:unit_smoke",
  "//core/apps/web:unit_smoke",
]);

const WEB_LINUX_RBE_SAFE_BAZEL_TEST_TARGETS = Object.freeze([
  ...WEB_SMOKE_BAZEL_TARGETS,
  "//core/apps/web:lint",
  "//core/apps/web:typecheck",
  "//core/apps/web:pretext_measurement_unit_tests",
  "//core/apps/web:unit_tests",
  "//core/apps/web:unit_tests_non_pretext",
  "//core/apps/web:unit_tests_non_pretext_foundation",
  "//core/apps/web:unit_tests_non_pretext_foundation_shared_api",
  "//core/apps/web:unit_tests_non_pretext_foundation_shared_support",
  "//core/apps/web:unit_tests_non_pretext_foundation_shared_utils",
  "//core/apps/web:unit_tests_non_pretext_foundation_shared_utils_analytics",
  "//core/apps/web:unit_tests_non_pretext_foundation_state",
  "//core/apps/web:unit_tests_non_pretext_misc",
  "//core/apps/web:unit_tests_non_pretext_settings_setup",
  "//core/apps/web:unit_tests_non_pretext_workbench",
  "//core/apps/web:unit_tests_non_pretext_workbench_shell",
  "//core/apps/web:unit_tests_non_pretext_workbench_surface",
  "//core/apps/web:unit_tests_non_pretext_workbench_surface_app",
  "//core/apps/web:unit_tests_non_pretext_workbench_surface_session",
  "//core/packages/session-thread-layout:unit_tests",
]);

function formatWebSmokeBazelPilotCommand() {
  return ["node", "scripts/run_bazel_pilot.cjs", "test", ...WEB_SMOKE_BAZEL_TARGETS].join(" ");
}

module.exports = {
  formatWebSmokeBazelPilotCommand,
  WEB_LINUX_RBE_SAFE_BAZEL_TEST_TARGETS,
  WEB_SMOKE_BAZEL_TARGETS,
};
