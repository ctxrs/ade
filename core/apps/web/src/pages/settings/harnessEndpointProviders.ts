import type { HarnessApiShape } from "../../api/client";

export type HarnessEndpointProviderPreset = {
  id: string;
  label: string;
  base_url: string | null;
  recommended_api_shape: HarnessApiShape;
};

const escapeRegex = (value: string): string => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";

// Presets are intentionally launch-focused: known direct endpoints where stable,
// otherwise OpenRouter-compatible fallback so users can still route via one key.
export const HARNESS_ENDPOINT_PROVIDER_PRESETS: HarnessEndpointProviderPreset[] = [
  { id: "ai21", label: "AI21", base_url: "https://api.ai21.com/studio/v1", recommended_api_shape: "openai_responses" },
  { id: "aionlabs", label: "AionLabs", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  {
    id: "alibaba_cloud_intl",
    label: "Alibaba Cloud Int.",
    base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    recommended_api_shape: "openai_responses",
  },
  {
    id: "amazon_bedrock",
    label: "Amazon Bedrock",
    base_url: OPENROUTER_BASE_URL,
    recommended_api_shape: "openai_responses",
  },
  { id: "anthropic", label: "Anthropic", base_url: "https://api.anthropic.com/v1", recommended_api_shape: "anthropic_messages" },
  { id: "arcee_ai", label: "Arcee AI", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "atlascloud", label: "AtlasCloud", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  {
    id: "azure",
    label: "Azure",
    base_url: "https://YOUR_RESOURCE_NAME.openai.azure.com/openai/v1",
    recommended_api_shape: "openai_responses",
  },
  { id: "baseten", label: "Baseten", base_url: "https://inference.baseten.co/v1", recommended_api_shape: "openai_responses" },
  { id: "cerebras", label: "Cerebras", base_url: "https://api.cerebras.ai/v1", recommended_api_shape: "openai_responses" },
  { id: "chutes", label: "Chutes", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "cirrascale", label: "Cirrascale", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "clarifai", label: "Clarifai", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  {
    id: "cloudflare",
    label: "Cloudflare",
    base_url: "https://api.cloudflare.com/client/v4/accounts/YOUR_ACCOUNT_ID/ai/v1",
    recommended_api_shape: "openai_responses",
  },
  { id: "cohere", label: "Cohere", base_url: "https://api.cohere.com/compatibility/v1", recommended_api_shape: "openai_responses" },
  { id: "crusoe", label: "Crusoe", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "deepinfra", label: "DeepInfra", base_url: "https://api.deepinfra.com/v1/openai", recommended_api_shape: "openai_responses" },
  { id: "deepseek", label: "DeepSeek", base_url: "https://api.deepseek.com/v1", recommended_api_shape: "openai_responses" },
  { id: "featherless", label: "Featherless", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "fireworks", label: "Fireworks", base_url: "https://api.fireworks.ai/inference/v1", recommended_api_shape: "openai_responses" },
  { id: "friendli", label: "Friendli", base_url: "https://api.friendli.ai/serverless/v1", recommended_api_shape: "openai_responses" },
  { id: "gmicloud", label: "GMICloud", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  {
    id: "google_ai_studio",
    label: "Google AI Studio",
    base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
    recommended_api_shape: "openai_responses",
  },
  {
    id: "google_vertex",
    label: "Google Vertex",
    base_url: "https://REGION-aiplatform.googleapis.com/v1/projects/PROJECT/locations/REGION/endpoints/openapi",
    recommended_api_shape: "openai_responses",
  },
  { id: "groq", label: "Groq", base_url: "https://api.groq.com/openai/v1", recommended_api_shape: "openai_responses" },
  { id: "hyperbolic", label: "Hyperbolic", base_url: "https://api.hyperbolic.xyz/v1", recommended_api_shape: "openai_responses" },
  { id: "inception", label: "Inception", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "inceptron", label: "Inceptron", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "infermatic", label: "Infermatic", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "inflection", label: "Inflection", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "liquid", label: "Liquid", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "mancer", label: "Mancer", base_url: "https://api.mancer.tech/v1", recommended_api_shape: "openai_responses" },
  { id: "minimax", label: "MiniMax", base_url: "https://api.minimax.chat/v1", recommended_api_shape: "openai_responses" },
  { id: "mistral", label: "Mistral", base_url: "https://api.mistral.ai/v1", recommended_api_shape: "openai_responses" },
  { id: "moonshot_ai", label: "Moonshot AI", base_url: "https://api.moonshot.ai/v1", recommended_api_shape: "openai_responses" },
  { id: "morph", label: "Morph", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "nebius_token_factory", label: "Nebius Token Factory", base_url: "https://api.studio.nebius.com/v1", recommended_api_shape: "openai_responses" },
  { id: "nextbit", label: "NextBit", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "novita_ai", label: "NovitaAI", base_url: "https://api.novita.ai/v3/openai", recommended_api_shape: "openai_responses" },
  { id: "openai", label: "OpenAI", base_url: "https://api.openai.com/v1", recommended_api_shape: "openai_responses" },
  { id: "openinference", label: "OpenInference", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "parasail", label: "Parasail", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "perplexity", label: "Perplexity", base_url: "https://api.perplexity.ai", recommended_api_shape: "openai_responses" },
  { id: "phala", label: "Phala", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "relace", label: "Relace", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "sambanova", label: "SambaNova", base_url: "https://api.sambanova.ai/v1", recommended_api_shape: "openai_responses" },
  { id: "switchpoint", label: "Switchpoint", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "together", label: "Together", base_url: "https://api.together.xyz/v1", recommended_api_shape: "openai_responses" },
  { id: "venice", label: "Venice", base_url: "https://api.venice.ai/api/v1", recommended_api_shape: "openai_responses" },
  { id: "weights_biases", label: "Weights & Biases", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "xai", label: "xAI", base_url: "https://api.x.ai/v1", recommended_api_shape: "openai_responses" },
  { id: "xiaomi", label: "Xiaomi", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "z_ai", label: "Z.ai", base_url: "https://api.z.ai/v1", recommended_api_shape: "openai_responses" },
  { id: "openrouter", label: "OpenRouter", base_url: OPENROUTER_BASE_URL, recommended_api_shape: "openai_responses" },
  { id: "other", label: "Other", base_url: null, recommended_api_shape: "openai_responses" },
];

const PRESET_BY_ID = new Map(HARNESS_ENDPOINT_PROVIDER_PRESETS.map((preset) => [preset.id, preset]));
const OTHER_PRESET: HarnessEndpointProviderPreset = {
  id: "other",
  label: "Other",
  base_url: null,
  recommended_api_shape: "openai_responses",
};

export const getHarnessEndpointProviderPreset = (id: string): HarnessEndpointProviderPreset =>
  PRESET_BY_ID.get(id) ?? OTHER_PRESET;

export const defaultEndpointProviderPresetForHarness = (harnessProviderId: string): string => {
  if (harnessProviderId === "codex") return "openai";
  if (harnessProviderId === "claude-crp") return "anthropic";
  if (harnessProviderId === "gemini") return "google_ai_studio";
  if (harnessProviderId === "kimi") return "moonshot_ai";
  if (harnessProviderId === "cursor") return "other";
  return "openrouter";
};

export const defaultShapeForHarnessProvider = (harnessProviderId: string): HarnessApiShape =>
  harnessProviderId === "claude-crp" ? "anthropic_messages" : "openai_responses";

export const supportsOptionalBaseUrlForHarness = (harnessProviderId: string): boolean =>
  harnessProviderId === "cody" || harnessProviderId === "pi" || harnessProviderId === "cursor";

export const normalizeOptionalBaseUrl = (rawBaseUrl: string): string | null => {
  const trimmed = rawBaseUrl.trim();
  return trimmed.length > 0 ? trimmed : null;
};

export const nextDefaultEndpointName = (
  endpointProviderId: string,
  existingNames: string[],
): string => {
  const preset = getHarnessEndpointProviderPreset(endpointProviderId);
  const base = preset.label.trim() || "Endpoint";
  const matcher = new RegExp(`^${escapeRegex(base)}\\s+(\\d+)$`, "i");
  const usedNumbers = new Set<number>();

  for (const existingName of existingNames) {
    const match = existingName.trim().match(matcher);
    if (!match) continue;
    const parsed = Number.parseInt(match[1] ?? "", 10);
    if (Number.isFinite(parsed) && parsed > 0) {
      usedNumbers.add(parsed);
    }
  }

  let next = 1;
  while (usedNumbers.has(next)) next += 1;
  return `${base} ${next}`;
};

export const nextTokenEndpointName = (
  providerId: string,
  existingNames: string[],
): string => {
  const base = `${providerId} token`;
  const used = new Set(
    existingNames
      .map((value) => value.trim().toLowerCase())
      .filter((value) => value.length > 0),
  );
  if (!used.has(base.toLowerCase())) {
    return base;
  }
  let next = 2;
  while (used.has(`${base} ${next}`.toLowerCase())) {
    next += 1;
  }
  return `${base} ${next}`;
};
