type ChatMessage = {
  role: "user";
  content: string;
};

type ChatCompletionsRequest = {
  model: string;
  messages: ChatMessage[];
  temperature: number;
};

export type RuntimeConfig = {
  apiKey: string;
  baseUrl: string;
  model: string;
};

const DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_MODEL = "openai/gpt-5.2-codex";

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
};

const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

const trimToNonEmpty = (value: string | undefined | null): string | null => {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
};

const firstNonEmpty = (...values: Array<string | undefined | null>): string | null => {
  for (const value of values) {
    const candidate = trimToNonEmpty(value);
    if (candidate) return candidate;
  }
  return null;
};

const normalizeBaseUrl = (url: string): string => url.replace(/\/+$/, "");

export function resolveRuntimeConfig(modelHint?: string): RuntimeConfig {
  const apiKey = firstNonEmpty(
    process.env.OPENAI_API_KEY,
    process.env.LLM_API_KEY,
    process.env.OPENROUTER_API_KEY,
  );
  if (!apiKey) {
    throw new Error(
      "missing API key for OpenHands ACP adapter (set OPENAI_API_KEY or LLM_API_KEY or OPENROUTER_API_KEY)",
    );
  }

  const baseUrl = normalizeBaseUrl(
    firstNonEmpty(
      process.env.OPENAI_BASE_URL,
      process.env.LLM_BASE_URL,
      process.env.OPENROUTER_BASE_URL,
      DEFAULT_BASE_URL,
    )!,
  );

  const model = firstNonEmpty(
    modelHint,
    process.env.OPENAI_MODEL,
    process.env.LLM_MODEL,
    process.env.OPENROUTER_MODEL,
    DEFAULT_MODEL,
  )!;

  return {
    apiKey,
    baseUrl,
    model,
  };
}

export function extractAssistantText(payload: unknown): string {
  const root = asRecord(payload);
  const choices = asArray(root?.choices);
  const firstChoice = asRecord(choices[0]);
  const message = asRecord(firstChoice?.message);
  const content = message?.content;

  if (typeof content === "string") {
    const trimmed = content.trim();
    if (trimmed.length > 0) return trimmed;
  }

  if (Array.isArray(content)) {
    const textParts = content
      .map((entry) => asRecord(entry))
      .filter((entry): entry is Record<string, unknown> => entry !== null)
      .map((entry) => {
        const directText = trimToNonEmpty(entry.text as string | undefined);
        if (directText) return directText;
        const nested = asRecord(entry.content);
        return trimToNonEmpty(nested?.text as string | undefined);
      })
      .filter((entry): entry is string => entry !== null);

    const joined = textParts.join("\n").trim();
    if (joined.length > 0) return joined;
  }

  throw new Error("chat completions response did not include assistant message content");
}

export async function requestAssistantCompletion(opts: {
  promptText: string;
  modelHint?: string;
  signal?: AbortSignal;
}): Promise<string> {
  const { promptText, modelHint, signal } = opts;
  const prompt = promptText.trim();
  if (!prompt) {
    throw new Error("prompt is empty");
  }

  const runtime = resolveRuntimeConfig(modelHint);
  const url = `${runtime.baseUrl}/chat/completions`;
  const body: ChatCompletionsRequest = {
    model: runtime.model,
    messages: [
      {
        role: "user",
        content: prompt,
      },
    ],
    temperature: 0,
  };

  const response = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${runtime.apiKey}`,
    },
    body: JSON.stringify(body),
    signal,
  });

  if (!response.ok) {
    const errorBody = (await response.text()).trim();
    throw new Error(
      `chat completions request failed (${response.status} ${response.statusText}): ${errorBody}`,
    );
  }

  const payload: unknown = await response.json();
  return extractAssistantText(payload);
}
