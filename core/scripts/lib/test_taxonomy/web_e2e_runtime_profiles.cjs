const path = require("node:path");

const SUITE_LABELS = {
  premerge_required: "//core/apps/web/e2e:premerge_required",
  quarantine: "//core/apps/web/e2e:quarantine",
  release_required: "//core/apps/web/e2e:release_required",
  cross_platform: "//core/apps/web/e2e:cross_platform",
  visual: "//core/apps/web/e2e:visual",
  soak: "//core/apps/web/e2e:soak",
  load: "//core/apps/web/e2e:load",
};

const SUITE_RUNTIME_PROFILES = {
  premerge_required: "agent-full",
  quarantine: "agent-full",
  release_required: "web-artifact",
  cross_platform: "agent-full",
  visual: "agent-full",
  soak: "agent-full",
  load: "agent-full",
};

const SPEC_LABELS = {
  "e2e/workbench-pretext-parity-corpus.spec.ts": "//core/apps/web/e2e:pretext_parity_corpus",
  "e2e/workbench-pretext-parity-fuzz.spec.ts": "//core/apps/web/e2e:pretext_parity_fuzz",
  "e2e/workbench-pretext-wrap-rule-fuzz.spec.ts": "//core/apps/web/e2e:pretext_wrap_rule_fuzz",
  "e2e/workbench-pretext-wrap-rules.spec.ts": "//core/apps/web/e2e:pretext_wrap_rules",
  "e2e/workbench-unarchive-visible.spec.ts": "//core/apps/web/e2e:workbench_unarchive_visible",
};

function classifyRuntimeProfile(spec, suite) {
  const base = path.basename(spec);
  if (suite === "release_required" || /update|updater/u.test(base)) {
    return "web-artifact";
  }
  if (/provider|runtime-harness|second-message|terminal|golden|launcher/u.test(base)) {
    return "agent-full";
  }
  return "workbench-lite";
}

function classifyImpactGroups(spec, family, suite) {
  const groups = new Set();
  if (family === "web-workbench" || spec.includes("workbench-")) {
    groups.add("web-workbench-state");
    groups.add("web-workbench-api");
  }
  if (family === "workspace-stream" || /ws-|stream|snapshot|session-gap/u.test(spec)) {
    groups.add("web-workbench-state");
  }
  if (family === "updates-release" || /update|updater/u.test(spec)) {
    groups.add("web-updater");
  }
  if (suite === "release_required") {
    groups.add("web-release-artifact");
  }
  groups.add("web-e2e-utils");
  return [...groups].sort();
}

function buildWebE2EExecutionTarget({ spec, family, suite }) {
  const suiteLabel = SUITE_LABELS[suite];
  if (!suiteLabel) {
    throw new Error(`missing web E2E suite label for suite '${suite}'`);
  }
  const specLabel = SPEC_LABELS[spec];
  const runtimeProfile = specLabel
    ? classifyRuntimeProfile(spec, suite)
    : SUITE_RUNTIME_PROFILES[suite];
  if (!runtimeProfile) {
    throw new Error(`missing web E2E runtime profile for suite '${suite}'`);
  }
  return {
    bazelLabels: [specLabel || suiteLabel],
    impactGroups: classifyImpactGroups(spec, family, suite),
    runtimeProfile,
  };
}

module.exports = {
  SPEC_LABELS,
  SUITE_LABELS,
  SUITE_RUNTIME_PROFILES,
  buildWebE2EExecutionTarget,
  classifyImpactGroups,
  classifyRuntimeProfile,
};
