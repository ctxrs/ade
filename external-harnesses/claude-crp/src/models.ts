import type { ModelInfo } from "@anthropic-ai/claude-agent-sdk";

const CLAUDE_MODEL_VERSION_RE = /\b(Opus|Sonnet|Haiku)\s+(\d+(?:\.\d+)?)\b/i;
const KNOWN_EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "max"] as const;
const DEFAULT_SUPPORTED_EFFORT = "high";

type CrpModelEntry = {
  id: string;
  name?: string;
};

type ClaudeModelsListEnvelope = {
  models: CrpModelEntry[];
  currentModelId?: string;
};

function asNonEmptyTrimmedString(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}

function asRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
}

function appendUniqueModel(
  models: CrpModelEntry[],
  modelId: string,
  modelName?: string,
): void {
  const nextId = modelId.trim();
  if (!nextId) return;
  if (models.some((entry) => entry.id === nextId)) return;
  models.push({ id: nextId, name: modelName?.trim() || undefined });
}

function capitalizeWord(value: string): string {
  const trimmed = value.trim().toLowerCase();
  if (!trimmed) return trimmed;
  return trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
}

function stripTrailingParenthetical(value: string): string {
  return value.replace(/\s*\([^()]+\)\s*$/, "").trim();
}

export function normalizeClaudeEffortId(value: unknown): string | undefined {
  const normalized = asNonEmptyTrimmedString(value)?.toLowerCase();
  if (!normalized) return undefined;
  return (KNOWN_EFFORT_LEVELS as readonly string[]).includes(normalized) ? normalized : undefined;
}

function formatClaudeEffortLabel(effort: string): string {
  const normalized = normalizeClaudeEffortId(effort) ?? effort.trim().toLowerCase();
  if (normalized === "xhigh") return "XHigh";
  return capitalizeWord(normalized);
}

function splitModelIdAndEffort(fullModelId: string): { base: string; effort?: string } {
  const trimmed = fullModelId.trim();
  if (!trimmed) return { base: "" };
  const idx = trimmed.lastIndexOf("/");
  if (idx <= 0) return { base: trimmed };
  const base = trimmed.slice(0, idx).trim();
  const effort = normalizeClaudeEffortId(trimmed.slice(idx + 1));
  return effort ? { base, effort } : { base: trimmed };
}

function readModelId(model: ModelInfo): string | undefined {
  const record = asRecord(model);
  return asNonEmptyTrimmedString(record.value) ?? asNonEmptyTrimmedString(record.id);
}

function readModelDisplayName(model: ModelInfo): string | undefined {
  const record = asRecord(model);
  return asNonEmptyTrimmedString(record.displayName) ?? asNonEmptyTrimmedString(record.name);
}

function readModelDescription(model: ModelInfo): string | undefined {
  const record = asRecord(model);
  return asNonEmptyTrimmedString(record.description);
}

function readSupportedEffortLevels(model: ModelInfo): string[] {
  const record = asRecord(model);
  const rawLevels = Array.isArray(record.supportedEffortLevels)
    ? record.supportedEffortLevels
    : [];
  const levels = rawLevels
    .map((level) => normalizeClaudeEffortId(level))
    .filter((level): level is string => Boolean(level));
  return [...new Set(levels)];
}

export function humanizeClaudeModelId(modelId: string): string | undefined {
  const trimmed = modelId.trim();
  if (!trimmed) return undefined;
  if (trimmed === "default") return "Default";
  if (trimmed === "opus" || trimmed === "sonnet" || trimmed === "haiku") {
    return capitalizeWord(trimmed);
  }
  const match = trimmed.match(/(?:^|\/)claude-(opus|sonnet|haiku)-(\d+)-(\d+)(?:[-@]\d+)?$/i);
  if (!match) return undefined;
  return `${capitalizeWord(match[1])} ${match[2]}.${match[3]}`;
}

export function deriveClaudeModelDisplayName(model: ModelInfo): string | undefined {
  const id = readModelId(model);
  const displayName = readModelDisplayName(model);
  const description = readModelDescription(model);

  if (id === "default") {
    if (description) {
      const versionMatch = description.match(CLAUDE_MODEL_VERSION_RE);
      if (versionMatch) {
        return `Default (${capitalizeWord(versionMatch[1])} ${versionMatch[2]})`;
      }
    }
    return displayName ? stripTrailingParenthetical(displayName) : "Default";
  }

  if (description) {
    const versionMatch = description.match(CLAUDE_MODEL_VERSION_RE);
    if (versionMatch) {
      return `${capitalizeWord(versionMatch[1])} ${versionMatch[2]}`;
    }
  }

  if (displayName) return stripTrailingParenthetical(displayName);
  if (id) return humanizeClaudeModelId(id) ?? id;
  return undefined;
}

function inferDefaultEffortForModel(model: ModelInfo | undefined): string | undefined {
  const supported = model ? readSupportedEffortLevels(model) : [];
  if (supported.length < 2) return undefined;
  if (supported.includes(DEFAULT_SUPPORTED_EFFORT)) return DEFAULT_SUPPORTED_EFFORT;
  return supported[0];
}

function buildSelectedModelEntry(
  currentModelId: string,
  supportedModels: ModelInfo[],
): CrpModelEntry {
  const { base, effort } = splitModelIdAndEffort(currentModelId);
  const supported = supportedModels.find((model) => readModelId(model) === base);
  const baseName =
    (supported ? deriveClaudeModelDisplayName(supported) : undefined) ??
    humanizeClaudeModelId(base) ??
    base;
  return {
    id: currentModelId,
    name: effort ? `${baseName} (${formatClaudeEffortLabel(effort)})` : baseName,
  };
}

export function mapClaudeModelInfo(model: ModelInfo): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const id = readModelId(model);
  const name = deriveClaudeModelDisplayName(model);
  const description = readModelDescription(model);
  if (id) out.id = id;
  if (name) out.name = name;
  if (description) out.description = description;
  return out;
}

export function buildClaudeModelsListEnvelope(params: {
  supportedModels: ModelInfo[];
  requestedModel?: string;
  requestedReasoningEffort?: string;
}): ClaudeModelsListEnvelope {
  const supportedModels = Array.isArray(params.supportedModels) ? params.supportedModels : [];
  const models: CrpModelEntry[] = [];

  for (const model of supportedModels) {
    const modelId = readModelId(model);
    if (!modelId) continue;
    const displayName = deriveClaudeModelDisplayName(model) ?? humanizeClaudeModelId(modelId) ?? modelId;
    const effortLevels = readSupportedEffortLevels(model);
    if (effortLevels.length >= 2) {
      for (const effort of effortLevels) {
        appendUniqueModel(models, `${modelId}/${effort}`, `${displayName} (${formatClaudeEffortLabel(effort)})`);
      }
      continue;
    }
    appendUniqueModel(models, modelId, displayName);
  }

  const requestedModel = asNonEmptyTrimmedString(params.requestedModel);
  const requestedReasoningEffort = normalizeClaudeEffortId(params.requestedReasoningEffort);
  const resolvedBaseModelId =
    requestedModel ??
    supportedModels.map((model) => readModelId(model)).find((id) => id === "default") ??
    supportedModels.map((model) => readModelId(model)).find((id): id is string => Boolean(id));

  const resolvedModel = resolvedBaseModelId
    ? supportedModels.find((model) => readModelId(model) === resolvedBaseModelId)
    : undefined;
  const resolvedReasoningEffort =
    requestedReasoningEffort ?? inferDefaultEffortForModel(resolvedModel);
  const currentModelId = resolvedBaseModelId
    ? resolvedReasoningEffort
      ? `${resolvedBaseModelId}/${resolvedReasoningEffort}`
      : resolvedBaseModelId
    : undefined;

  if (currentModelId) {
    const selectedEntry = buildSelectedModelEntry(currentModelId, supportedModels);
    appendUniqueModel(models, selectedEntry.id, selectedEntry.name);
  }

  return {
    models,
    currentModelId,
  };
}
