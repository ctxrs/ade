import { describe, expect, it } from "vitest";
import { takeNextClaudeAuthUrlToOpen } from "./useHarnessAuthenticationController";

describe("takeNextClaudeAuthUrlToOpen", () => {
  it("normalizes and deduplicates urls", () => {
    const opened = new Set<string>();

    const first = takeNextClaudeAuthUrlToOpen(" https://claude.ai/oauth/authorize?code=abc ", opened);
    const duplicate = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=abc", opened);

    expect(first).toBe("https://claude.ai/oauth/authorize?code=abc");
    expect(duplicate).toBeNull();
  });

  it("ignores empty values", () => {
    const opened = new Set<string>();

    expect(takeNextClaudeAuthUrlToOpen("", opened)).toBeNull();
    expect(takeNextClaudeAuthUrlToOpen("   ", opened)).toBeNull();
    expect(takeNextClaudeAuthUrlToOpen(null, opened)).toBeNull();
    expect(takeNextClaudeAuthUrlToOpen(undefined, opened)).toBeNull();
  });

  it("allows distinct urls once each", () => {
    const opened = new Set<string>();

    const first = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=abc", opened);
    const second = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=def", opened);
    const secondDuplicate = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=def", opened);

    expect(first).toBe("https://claude.ai/oauth/authorize?code=abc");
    expect(second).toBe("https://claude.ai/oauth/authorize?code=def");
    expect(secondDuplicate).toBeNull();
  });
});
