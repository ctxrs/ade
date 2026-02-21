export type EndpointHarnessMatrixEntry = {
  providerId: string;
  menuLabel: string;
  searchTerm: string;
};

// First-pass endpoint harness cohort for OpenRouter endpoint + API key validation.
export const OPENROUTER_ENDPOINT_FIRST_PASS_HARNESSES: EndpointHarnessMatrixEntry[] = [
  { providerId: "codex", menuLabel: "Codex", searchTerm: "codex" },
  { providerId: "qwen", menuLabel: "Qwen Code", searchTerm: "qwen" },
  { providerId: "opencode", menuLabel: "OpenCode", searchTerm: "opencode" },
  { providerId: "mistral", menuLabel: "Mistral Vibe", searchTerm: "mistral" },
  { providerId: "goose", menuLabel: "Goose", searchTerm: "goose" },
  { providerId: "kimi", menuLabel: "Kimi", searchTerm: "kimi" },
  { providerId: "cagent", menuLabel: "cagent", searchTerm: "cagent" },
  { providerId: "pi", menuLabel: "Pi", searchTerm: "pi" },
  { providerId: "cline", menuLabel: "Cline", searchTerm: "cline" },
  { providerId: "openhands", menuLabel: "OpenHands", searchTerm: "openhands" },
];

// Explicitly excluded from this suite (handled by separate provider-token-only coverage).
export const OPENROUTER_ENDPOINT_FIRST_PASS_EXCLUDED_PROVIDER_TOKEN_ONLY = [
  "gemini",
  "copilot",
  "kiro",
  "cursor",
  "auggie",
  "amp",
  "droid",
  "continue",
] as const;

// Tracked but unsupported harnesses intentionally excluded from first-pass endpoint e2e.
export const OPENROUTER_ENDPOINT_FIRST_PASS_EXCLUDED_UNSUPPORTED = [
  "junie",
  "codebuff",
  "charm",
  "aider",
  "kilo",
] as const;
