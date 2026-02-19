import { describe, expect, it } from "vitest";
import type { ProviderOptions, ProviderStatus } from "../../api/client";
import {
  collectSelectableHarnessProviderIds,
  getHarnessMruStorageKey,
  resolveInitialHarnessSelection,
  shouldFinalizeInitialHarnessSelection,
} from "./harnessSelection";

const provider = (
  provider_id: string,
  opts?: Partial<ProviderStatus>,
): ProviderStatus => ({
  provider_id,
  installed: true,
  health: "ok",
  diagnostics: [],
  ...opts,
});

const baseOptions = (providerId: string): ProviderOptions => ({
  provider_id: providerId,
  workspace_id: "ws-test",
  supports_load: false,
  auth_required: false,
  probed_at: new Date().toISOString(),
});

describe("harnessSelection", () => {
  it("builds workspace-scoped MRU keys", () => {
    expect(getHarnessMruStorageKey("ws-123")).toBe("wb.harnessMru.ws-123");
  });

  it("collects only installed, healthy, visible providers", () => {
    const providersById: Record<string, ProviderStatus> = {
      codex: provider("codex"),
      hidden: provider("hidden", { details: { ui_hidden: "true" } }),
      missing: provider("missing", { installed: false }),
      unhealthy: provider("unhealthy", { health: "error" }),
    };
    expect(collectSelectableHarnessProviderIds(providersById)).toEqual(["codex"]);
  });

  it("prefers MRU when that provider has active auth", () => {
    const providerOptions: Record<string, ProviderOptions | undefined> = {
      codex: { ...baseOptions("codex"), has_active_auth: true },
      cursor: { ...baseOptions("cursor"), has_active_auth: true },
    };
    const selected = resolveInitialHarnessSelection({
      providerIds: ["codex", "cursor"],
      providerOptions,
      mruProviderId: "cursor",
    });
    expect(selected).toBe("cursor");
  });

  it("falls back to single authed provider when MRU is invalid", () => {
    const providerOptions: Record<string, ProviderOptions | undefined> = {
      codex: { ...baseOptions("codex"), has_active_auth: true },
      cursor: baseOptions("cursor"),
    };
    const selected = resolveInitialHarnessSelection({
      providerIds: ["codex", "cursor"],
      providerOptions,
      mruProviderId: "cursor",
    });
    expect(selected).toBe("codex");
  });

  it("returns null when none or multiple candidates are authed", () => {
    const noneAuthed = resolveInitialHarnessSelection({
      providerIds: ["codex", "cursor"],
      providerOptions: {
        codex: baseOptions("codex"),
        cursor: baseOptions("cursor"),
      },
      mruProviderId: null,
    });
    expect(noneAuthed).toBeNull();

    const multipleAuthed = resolveInitialHarnessSelection({
      providerIds: ["codex", "cursor"],
      providerOptions: {
        codex: { ...baseOptions("codex"), has_active_auth: true },
        cursor: { ...baseOptions("cursor"), has_active_auth: true },
      },
      mruProviderId: null,
    });
    expect(multipleAuthed).toBeNull();
  });

  it("finalizes initial resolver only after a provider is actually selected", () => {
    expect(shouldFinalizeInitialHarnessSelection(null)).toBe(false);
    expect(shouldFinalizeInitialHarnessSelection("codex")).toBe(true);
  });
});
