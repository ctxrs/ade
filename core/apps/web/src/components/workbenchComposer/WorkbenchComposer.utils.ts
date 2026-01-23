import { blobUrl, type MessageAttachment, type ProviderOptions } from "../../api/client";
import type { SessionViewVerbosity } from "../../state/uiStateStore";
import type { WorkbenchModeId } from "./WorkbenchComposer.types";
import type { buildModelCatalog } from "../../utils/modelEffort";
import { composeModelId } from "../../utils/modelEffort";

export const MENU_DESCRIPTIONS = {
  harness: `Agent harnesses are the low-level wrappers around models that provide the basic plumbing to allow the model to interact with the workspace. This normally includes features like filesystem access, shell access, configurations to set up MCP servers, and more. Despite similiarities between them, different harnesses will have varying tools, capabilities, and performance - even if used with the same underlying models. From here, you can install agent harnesses you haven't used before, switch between them for new tasks, and even run multiple agent harnesses in parallel on the same task. This can be useful to compare performance or to survey multiple different approaches to the same problem.`,
  model: `You can switch between different models here. Model selection offers a tradeoff between cost, latency, and intelligence - but it also offers an opportunity to leverage the differences in their weights for collaboration. Even if two different models score similarly on popular coding benchmarks, they might have different "habits" - or biases. This means that if you are working on a pernicious bug fix, you might want multiple different models to both look at the problem from a different angle.`,
  effort: `Some models have a "thinking effort" or "reasoning effort" setting, while others do not. The effort level simply corresponds to how many tokens a model spends on thinking while solving a problem. Models that offer high or extra high can sometimes be very powerful, at the expense of latency and cost. However, you can also experience an unintended negative consequence from extra high thinking: if the model is emitting lots of thinking tokens that don't add much value, this will cause the context window to fill up faster (not just from thinking tokens alone, but also from more excessive tool calls like reading files). Performance on coding tasks declines as context increases beyond the minimum context needed to solve the problem, so effort level is a key lever in tuning your agent for optimal performance.`,
  mode: `Modes are basically just prompts, sometimes combined with access limitations. For example, the review mode is nothing more than prompting the agent to tell it to review the code and putting it in a read-only access level. That sounds fairly simple, but there is a hidden benefit: developers who build agent harnesses and models in conjunction will often train their custom model to use their bespoke harness, including its different modes. So in a way, this prompt can be more than just a regular prompt. It is a special prompt than has been trained on via reinforcement learning to achieve certain outcomes. For example, OpenAI trained their codex model to use their codex harness in review mode, so as to output only high value review comments with priority details. If you give the exact same prompt to a model that has not undergone the same RL, it will emit much less useful review comments. We recommend using RPIR (Research, Plan, Implement, Review) pattern for most changes except for small and easy ones.`,
  verbosity: `Verbosity controls how much activity is shown during a turn. Terse hides tools and thoughts, default shows summaries and thoughts, and verbose will eventually expand full tool details.`,
} as const;

export function clamp(n: number, min: number, max: number) {
  return Math.max(min, Math.min(max, n));
}

export function imageAttachmentSrc(a: MessageAttachment): string {
  return a.kind === "image_ref" ? blobUrl(a.blob_id) : `data:${a.mime_type};base64,${a.data_base64}`;
}

export function attachmentDisplayName(name?: string | null) {
  const n = String(name ?? "").trim();
  if (!n) return "image";
  return n.split(/[\\/]/).pop() || "image";
}

export function labelForMode(mode: WorkbenchModeId): string {
  if (mode === "default") return "Default";
  if (mode === "research") return "Research";
  if (mode === "plan") return "Plan";
  return "Review";
}

export function labelForVerbosity(level: SessionViewVerbosity): string {
  if (level === "terse") return "Terse";
  if (level === "verbose") return "Verbose";
  return "Default";
}

export function formatTokenCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) {
    const scaled = value / 1_000_000;
    const fixed = scaled >= 10 ? scaled.toFixed(0) : scaled.toFixed(1);
    return `${fixed.replace(/\.0$/, "")}m`;
  }
  if (value >= 1_000) {
    const scaled = value / 1_000;
    const fixed = scaled >= 100 ? scaled.toFixed(0) : scaled.toFixed(1);
    return `${fixed.replace(/\.0$/, "")}k`;
  }
  return `${Math.round(value)}`;
}

export function formatUsedTokenCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) {
    const scaled = value / 1_000_000;
    const fixed = scaled >= 10 ? scaled.toFixed(0) : scaled.toFixed(1);
    return `${fixed.replace(/\.0$/, "")}m`;
  }
  if (value >= 1_000) {
    const rounded = Math.round(value / 1_000);
    return `${rounded}k`;
  }
  return `${Math.round(value)}`;
}

export function buildModelsFromProviderOptions(opts?: ProviderOptions): Array<{ id: string; name?: string }> {
  const raw = opts?.models;
  if (!raw) return [];
  const list = (raw as any)?.availableModels ?? (raw as any)?.available_models ?? (raw as any)?.models ?? raw;
  if (!Array.isArray(list)) return [];
  return list
    .map((m: any) => ({
      id: String(m?.modelId ?? m?.model_id ?? m?.id ?? m?.name ?? "").trim(),
      name: typeof m?.name === "string" ? m.name : undefined,
    }))
    .filter((m) => m.id.length > 0);
}

const FALLBACK_MODELS_BY_PROVIDER: Record<string, Array<{ id: string; name?: string }>> = {
  gemini: [
    { id: "gemini-2.0-flash", name: "Gemini 2.0 Flash" },
    { id: "gemini-2.0-flash-lite", name: "Gemini 2.0 Flash Lite" },
    { id: "gemini-2.5-pro", name: "Gemini 2.5 Pro" },
    { id: "gemini-1.5-pro", name: "Gemini 1.5 Pro" },
    { id: "gemini-1.5-flash", name: "Gemini 1.5 Flash" },
  ],
};

export const MAX_TRACKS_PER_PROVIDER = 4;

export function buildModelsForProvider(providerId: string, opts?: ProviderOptions): Array<{ id: string; name?: string }> {
  const models = buildModelsFromProviderOptions(opts);
  if (models.length > 0) return models;
  return FALLBACK_MODELS_BY_PROVIDER[providerId] ?? [];
}

export function pickDefaultEffort(efforts: string[]): string | null {
  if (efforts.includes("medium")) return "medium";
  return efforts[0] ?? null;
}

export function deriveFullModelIdForBase(
  catalog: ReturnType<typeof buildModelCatalog>,
  base: string,
  preferredEffort: string | null,
): string {
  const efforts = catalog.effortsByBase[base] ?? [];
  if (efforts.length === 0) return base;
  const eff = preferredEffort && efforts.includes(preferredEffort) ? preferredEffort : pickDefaultEffort(efforts);
  if (!eff) return base;
  const mapped = catalog.fullIdByBaseEffort[base]?.[eff];
  return mapped ?? composeModelId(base, eff);
}

export function modelIdFromProviderOptions(opts?: ProviderOptions): string | null {
  const raw = opts?.models as any;
  if (!raw || typeof raw !== "object") return null;
  const current = raw.currentModelId ?? raw.current_model_id;
  if (typeof current === "string" && current.trim().length > 0) return current.trim();
  const list = raw.availableModels ?? raw.available_models ?? raw.models ?? [];
  if (!Array.isArray(list) || list.length === 0) return null;
  const first = list[0] as any;
  const id = first?.modelId ?? first?.model_id ?? first?.id ?? first?.name;
  return typeof id === "string" && id.trim().length > 0 ? id.trim() : null;
}
