"use strict";

const MAX_SOURCE_TEXT_CHARS = 256 * 1024;
const MAX_EVIDENCE_SNIPPET_CHARS = 360;
const MAX_EVIDENCE_PER_CLASS = 4;

const ENVIRONMENT_FAILURE_CLASSES = Object.freeze([
  Object.freeze({
    id: "posthog_gateway_timeout",
    title: "PostHog 503/504 gateway failure",
    category: "external_service",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 2,
      backoff: "exponential_seconds_30_120",
    }),
    required_evidence: Object.freeze([
      "raw log snippet naming PostHog or a PostHog host",
      "raw HTTP 503/504, service-unavailable, or gateway-timeout evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "posthog_5xx_forward",
        regex: /\b(?:posthog|app\.posthog\.com|us\.i\.posthog\.com|eu\.i\.posthog\.com)\b[\s\S]{0,300}\b(?:503|504|gateway timeout|service unavailable)\b/iu,
      }),
      Object.freeze({
        id: "posthog_5xx_reverse",
        regex: /\b(?:503|504|gateway timeout|service unavailable)\b[\s\S]{0,300}\b(?:posthog|app\.posthog\.com|us\.i\.posthog\.com|eu\.i\.posthog\.com)\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "private_handoff_artifact_download_reset",
    title: "Private handoff artifact download connection reset",
    category: "artifact_transport",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 2,
      backoff: "linear_seconds_30",
    }),
    required_evidence: Object.freeze([
      "raw log snippet naming private handoff or Buildkite artifact download",
      "raw ECONNRESET/socket hang up/connection reset/curl 56 evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "private_handoff_reset",
        regex: /\b(?:private handoff|private[-_ ]handoff|download_artifacts_with_retry|buildkite-agent artifact download|artifact download)\b[\s\S]{0,400}\b(?:ECONNRESET|connection reset|reset by peer|socket hang up|curl:\s*\(56\)|recv failure)\b/iu,
      }),
      Object.freeze({
        id: "private_handoff_reset_reverse",
        regex: /\b(?:ECONNRESET|connection reset|reset by peer|socket hang up|curl:\s*\(56\)|recv failure)\b[\s\S]{0,400}\b(?:private handoff|private[-_ ]handoff|download_artifacts_with_retry|buildkite-agent artifact download|artifact download)\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "cloud_capacity",
    title: "Cloud or Buildkite capacity wait",
    category: "capacity",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 3,
      backoff: "queue_recheck_seconds_300",
    }),
    required_evidence: Object.freeze([
      "raw queue/capacity/no-agent evidence from Buildkite, Hetzner, or cloud provisioning",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "buildkite_no_agents",
        regex: /\b(?:waiting for agent|no agents? available|agent queue.+(?:saturated|at capacity|full)|limited by concurrency|waiting on concurrency|capacity wait)\b/iu,
      }),
      Object.freeze({
        id: "cloud_capacity",
        regex: /\b(?:hetzner|ec2|cloud|autoscal(?:e|er|ing))\b[\s\S]{0,300}\b(?:capacity|quota|rate limit|server limit|insufficient resources|resource unavailable)\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "infisical_auth_expired",
    title: "Infisical auth/session expired",
    category: "secret_manager",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 1,
      backoff: "after_reauth",
    }),
    required_evidence: Object.freeze([
      "raw log snippet naming Infisical",
      "raw expired-token, unauthenticated, 401, or login-required evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "infisical_auth_forward",
        regex: /\binfisical\b[\s\S]{0,300}\b(?:session expired|token expired|jwt expired|unauthenticated|not logged in|login required|401|403)\b/iu,
      }),
      Object.freeze({
        id: "infisical_auth_reverse",
        regex: /\b(?:session expired|token expired|jwt expired|unauthenticated|not logged in|login required|401|403)\b[\s\S]{0,300}\binfisical\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "buildkite_agent_lost",
    title: "Buildkite agent lost or disconnected",
    category: "ci_agent",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 2,
      backoff: "linear_seconds_60",
    }),
    required_evidence: Object.freeze([
      "raw Buildkite agent lost/disconnected/canceled-by-agent-loss evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "buildkite_agent_lost",
        regex: /\b(?:buildkite agent|agent)\b[\s\S]{0,240}\b(?:lost communication|disconnected|was lost|has been lost|agent disappeared|agent stopped|agent was terminated)\b/iu,
      }),
      Object.freeze({
        id: "buildkite_agent_lost_reverse",
        regex: /\b(?:lost communication|agent disappeared|agent was lost|agent has been lost|agent stopped)\b[\s\S]{0,240}\b(?:buildkite|job|agent)\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "disk_pressure",
    title: "Disk pressure or no free space",
    category: "host_resource",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 1,
      backoff: "after_disk_recovery",
    }),
    required_evidence: Object.freeze([
      "raw ENOSPC/no-space/insufficient-free-disk evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "enospc",
        regex: /\b(?:ENOSPC|no space left on device|disk quota exceeded|insufficient free disk|not enough free disk|No usable sandbox|free-space minimum(?: not met| failed| violated)?|minimum free space (?:not met|failed|violated))\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "webkit_process_killed_by_host",
    title: "WebKit process killed by host",
    category: "browser_host",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 2,
      backoff: "fresh_webdriver_attempt",
    }),
    required_evidence: Object.freeze([
      "raw WebKit/WebKitWebDriver/WebKitWebProcess evidence",
      "raw SIGKILL, signal 9, killed-by-host, or process-killed evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "webkit_killed_forward",
        regex: /\b(?:webkit|webkitgtk|webkitwebdriver|webkitwebprocess|web process)\b[\s\S]{0,400}\b(?:SIGKILL|signal\s*9|killed by host|process killed|was killed|exited with signal 9|terminated by signal 9)\b/iu,
      }),
      Object.freeze({
        id: "webkit_killed_reverse",
        regex: /\b(?:SIGKILL|signal\s*9|killed by host|process killed|was killed|exited with signal 9|terminated by signal 9)\b[\s\S]{0,400}\b(?:webkit|webkitgtk|webkitwebdriver|webkitwebprocess|web process)\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "buildbuddy_remote_cache_input_error",
    title: "BuildBuddy remote cache/input error",
    category: "remote_execution",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 2,
      backoff: "linear_seconds_60",
    }),
    required_evidence: Object.freeze([
      "raw BuildBuddy/remote-cache/remote-execution evidence",
      "raw missing input, CAS, bytestream, unavailable, or digest evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "buildbuddy_remote_input",
        regex: /\b(?:buildbuddy|remote cache|remote execution|bytestream|cas)\b[\s\S]{0,500}\b(?:missing input|input root|digest not found|blob not found|cache lookup failed|UNAVAILABLE|failed to fetch inputs?|no such blob|resource exhausted)\b/iu,
      }),
      Object.freeze({
        id: "buildbuddy_remote_input_reverse",
        regex: /\b(?:missing input|input root|digest not found|blob not found|cache lookup failed|UNAVAILABLE|failed to fetch inputs?|no such blob|resource exhausted)\b[\s\S]{0,500}\b(?:buildbuddy|remote cache|remote execution|bytestream|cas)\b/iu,
      }),
    ]),
  }),
  Object.freeze({
    id: "bazel_external_repository_fetch_5xx",
    title: "Bazel external repository or registry fetch 5xx",
    category: "external_dependency",
    retry_policy: Object.freeze({
      retryable: true,
      max_retries: 2,
      backoff: "linear_seconds_60",
    }),
    required_evidence: Object.freeze([
      "raw Bazel external repository, registry, BCR, or rules repository evidence",
      "raw HTTP 5xx, bad-gateway, proxy, or service-unavailable evidence",
    ]),
    patterns: Object.freeze([
      Object.freeze({
        id: "bazel_registry_5xx_forward",
        regex: /\b(?:bazel central registry|bcr|registry\.bazel\.build|bcr\.bazel\.build|rules_[a-z0-9_-]+|external repository)\b[\s\S]{0,600}\b(?:HTTP (?:5[0-9][0-9])|GET returned (?:5[0-9][0-9])|(?:^|[^\d])5(?:00|02|03|04)(?:[^\d]|$)|bad gateway|proxy error|service unavailable|gateway timeout)\b/iu,
      }),
      Object.freeze({
        id: "bazel_registry_5xx_reverse",
        regex: /\b(?:HTTP (?:5[0-9][0-9])|GET returned (?:5[0-9][0-9])|(?:^|[^\d])5(?:00|02|03|04)(?:[^\d]|$)|bad gateway|proxy error|service unavailable|gateway timeout)\b[\s\S]{0,600}\b(?:bazel central registry|bcr|registry\.bazel\.build|bcr\.bazel\.build|rules_[a-z0-9_-]+|external repository)\b/iu,
      }),
    ]),
  }),
]);

function trimValue(value) {
  return String(value || "").trim();
}

function normalizeRetryAttempt(value) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    return 0;
  }
  return parsed;
}

function normalizeSourceText(value) {
  return String(value || "").slice(0, MAX_SOURCE_TEXT_CHARS);
}

function pushSource(sources, source, value) {
  const text = normalizeSourceText(value);
  if (!text.trim()) {
    return;
  }
  sources.push({
    source: trimValue(source) || "log",
    text,
  });
}

function collectStringSources(value, source, sources, seen = new Set(), depth = 0) {
  if (sources.length >= 100 || depth > 5 || value == null) {
    return;
  }
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    pushSource(sources, source, value);
    return;
  }
  if (typeof value !== "object") {
    return;
  }
  if (seen.has(value)) {
    return;
  }
  seen.add(value);
  if (Array.isArray(value)) {
    value.slice(0, 50).forEach((entry, index) => {
      collectStringSources(entry, `${source}[${index}]`, sources, seen, depth + 1);
    });
    return;
  }
  for (const [key, entry] of Object.entries(value).slice(0, 80)) {
    collectStringSources(entry, source ? `${source}.${key}` : key, sources, seen, depth + 1);
  }
}

function normalizeEvidenceSources(input) {
  const sources = [];
  if (typeof input === "string") {
    pushSource(sources, "log", input);
    return sources;
  }
  if (Array.isArray(input)) {
    input.forEach((entry, index) => {
      if (entry && typeof entry === "object" && "text" in entry) {
        pushSource(sources, entry.source || `source[${index}]`, entry.text);
      } else {
        collectStringSources(entry, `source[${index}]`, sources);
      }
    });
    return sources;
  }
  collectStringSources(input, "build", sources);
  return sources;
}

function snippetForMatch(text, index, matchText) {
  const matchLength = Math.max(1, String(matchText || "").length);
  const budget = MAX_EVIDENCE_SNIPPET_CHARS;
  const before = Math.max(0, index - Math.floor((budget - matchLength) / 2));
  const after = Math.min(text.length, before + budget);
  return text
    .slice(before, after)
    .replace(/\s+/g, " ")
    .trim();
}

function matchEvidence(definition, sources) {
  const evidence = [];
  for (const source of sources) {
    for (const pattern of definition.patterns) {
      const regex = new RegExp(pattern.regex.source, pattern.regex.flags);
      const match = regex.exec(source.text);
      if (!match) {
        continue;
      }
      evidence.push({
        pattern_id: pattern.id,
        snippet: snippetForMatch(source.text, match.index, match[0]),
        source: source.source,
      });
      if (evidence.length >= MAX_EVIDENCE_PER_CLASS) {
        return evidence;
      }
    }
  }
  return evidence;
}

function buildProductFailureClassification({ reason = "no_environment_evidence" } = {}) {
  return {
    class_id: "product_failure",
    confidence: "default",
    evidence: [],
    failure_kind: "product",
    reason,
    release_blocking: true,
    required_evidence: [],
    requires_exact_sha_rerun: false,
    retry_policy: {
      backoff: "none",
      max_retries: 0,
      retry_allowed: false,
      retry_attempt: 0,
      retry_exhausted: false,
      retryable: false,
    },
    title: "Product or test failure",
    unblocks_release: false,
  };
}

function buildEnvironmentFailureClassification(definition, evidence, {
  commitSha = "",
  retryAttempt = 0,
} = {}) {
  const normalizedAttempt = normalizeRetryAttempt(retryAttempt);
  const retryExhausted = normalizedAttempt >= definition.retry_policy.max_retries;
  return {
    category: definition.category,
    class_id: definition.id,
    confidence: "matched_raw_evidence",
    evidence,
    failure_kind: "environment",
    release_blocking: true,
    required_evidence: [...definition.required_evidence],
    requires_exact_sha_rerun: true,
    rerun_requirement: {
      commit_sha: trimValue(commitSha),
      exact_sha_pass_required: true,
      required: true,
      reason: "Environment-classified failures remain release-blocking until the same commit SHA has a passing rerun for the failed evidence node.",
    },
    retry_policy: {
      ...definition.retry_policy,
      retry_allowed: definition.retry_policy.retryable && !retryExhausted,
      retry_attempt: normalizedAttempt,
      retry_exhausted: retryExhausted,
    },
    title: definition.title,
    unblocks_release: false,
  };
}

function classifyEnvironmentFailure(input, options = {}) {
  const sources = normalizeEvidenceSources(input);
  for (const definition of ENVIRONMENT_FAILURE_CLASSES) {
    const evidence = matchEvidence(definition, sources);
    if (evidence.length > 0) {
      return buildEnvironmentFailureClassification(definition, evidence, options);
    }
  }
  return buildProductFailureClassification();
}

module.exports = {
  ENVIRONMENT_FAILURE_CLASSES,
  buildProductFailureClassification,
  classifyEnvironmentFailure,
  normalizeEvidenceSources,
};
