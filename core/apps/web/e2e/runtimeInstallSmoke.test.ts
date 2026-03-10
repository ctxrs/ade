import { describe, expect, it } from "vitest";

import {
  bundledOnlyModeAppliesToProvider,
  shouldSkipBundledOnlyInstall,
} from "./runtimeInstallSmoke";

describe("runtime install smoke helpers", () => {
  it("requires bundled-only mode before treating a provider as preseeded", () => {
    expect(
      bundledOnlyModeAppliesToProvider("goose", {
        CTX_E2E_BUNDLED_ONLY: "0",
        CTX_E2E_BUNDLED_ONLY_PROVIDERS: "goose",
      }),
    ).toBe(false);
  });

  it("treats an empty bundled-only provider list as all providers", () => {
    expect(
      bundledOnlyModeAppliesToProvider("goose", {
        CTX_E2E_BUNDLED_ONLY: "1",
        CTX_E2E_BUNDLED_ONLY_PROVIDERS: " , ",
      }),
    ).toBe(true);
  });

  it("skips explicit installs only for listed bundled-only providers when enabled", () => {
    const env = {
      CTX_E2E_BUNDLED_ONLY: "1",
      CTX_E2E_BUNDLED_ONLY_PROVIDERS: "goose,openhands",
      CTX_E2E_INSTALL_SMOKE_SKIP_BUNDLED_ONLY_INSTALLS: "1",
    };

    expect(shouldSkipBundledOnlyInstall("goose", env)).toBe(true);
    expect(shouldSkipBundledOnlyInstall("codex", env)).toBe(false);
  });

  it("does not skip installs unless the lane explicitly opts in", () => {
    expect(
      shouldSkipBundledOnlyInstall("goose", {
        CTX_E2E_BUNDLED_ONLY: "1",
        CTX_E2E_BUNDLED_ONLY_PROVIDERS: "goose",
        CTX_E2E_INSTALL_SMOKE_SKIP_BUNDLED_ONLY_INSTALLS: "0",
      }),
    ).toBe(false);
  });
});
