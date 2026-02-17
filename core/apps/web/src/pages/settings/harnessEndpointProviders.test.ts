import { describe, expect, it } from "vitest";
import {
  defaultEndpointProviderPresetForHarness,
  defaultShapeForHarnessProvider,
  getHarnessEndpointProviderPreset,
  HARNESS_ENDPOINT_PROVIDER_PRESETS,
  nextDefaultEndpointName,
} from "./harnessEndpointProviders";

describe("harnessEndpointProviders", () => {
  it("returns harness-specific default provider presets", () => {
    expect(defaultEndpointProviderPresetForHarness("codex")).toBe("openai");
    expect(defaultEndpointProviderPresetForHarness("claude-crp")).toBe("anthropic");
    expect(defaultEndpointProviderPresetForHarness("gemini")).toBe("openrouter");
  });

  it("keeps openrouter and other as final options", () => {
    const len = HARNESS_ENDPOINT_PROVIDER_PRESETS.length;
    expect(HARNESS_ENDPOINT_PROVIDER_PRESETS[len - 2]?.id).toBe("openrouter");
    expect(HARNESS_ENDPOINT_PROVIDER_PRESETS[len - 1]?.id).toBe("other");
  });

  it("maps default harness shapes", () => {
    expect(defaultShapeForHarnessProvider("codex")).toBe("openai_responses");
    expect(defaultShapeForHarnessProvider("claude-crp")).toBe("anthropic_messages");
  });

  it("falls back to other for unknown provider ids", () => {
    const other = getHarnessEndpointProviderPreset("does-not-exist");
    expect(other.id).toBe("other");
    expect(other.base_url).toBeNull();
  });

  it("builds incremental default endpoint names by provider", () => {
    expect(nextDefaultEndpointName("openai", [])).toBe("OpenAI 1");
    expect(nextDefaultEndpointName("openai", ["OpenAI 1", "OpenAI 3"])).toBe("OpenAI 2");
    expect(nextDefaultEndpointName("anthropic", ["Anthropic 1", "Anthropic 2"])).toBe("Anthropic 3");
  });

  it("uses endpoint fallback naming for unknown provider ids", () => {
    expect(nextDefaultEndpointName("not-real", [])).toBe("Other 1");
  });
});
