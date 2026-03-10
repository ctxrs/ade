import { describe, expect, it } from "vitest";

import { extractClaudeCallbackCodeFromUrl } from "../../e2e/utils/providerBrowserAuth";

describe("extractClaudeCallbackCodeFromUrl", () => {
  it("ignores Claude authorize URLs that only carry code=true", () => {
    expect(
      extractClaudeCallbackCodeFromUrl(
        "https://claude.ai/oauth/authorize?code=true&client_id=test-client&state=test-state",
      ),
    ).toBe("");
  });

  it("reads the real OAuth code from Anthropic callback URLs", () => {
    expect(
      extractClaudeCallbackCodeFromUrl(
        "https://platform.claude.com/oauth/code/callback?code=ePBMdWetJlSbZ0aR9fA2BcDeFgHiJkLmNoPqRsTuVwXyZa12",
      ),
    ).toBe("ePBMdWetJlSbZ0aR9fA2BcDeFgHiJkLmNoPqRsTuVwXyZa12");
  });

  it("reads the real OAuth code from callback URL fragments", () => {
    expect(
      extractClaudeCallbackCodeFromUrl(
        "https://console.anthropic.com/oauth/code/callback#code=ePBMdWetJlSbZ0aR9fA2BcDeFgHiJkLmNoPqRsTuVwXyZa12&state=test-state",
      ),
    ).toBe("ePBMdWetJlSbZ0aR9fA2BcDeFgHiJkLmNoPqRsTuVwXyZa12");
  });
});
