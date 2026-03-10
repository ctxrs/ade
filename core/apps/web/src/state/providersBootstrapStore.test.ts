import { beforeEach, describe, expect, it, vi } from "vitest";

const clientMocks = vi.hoisted(() => ({
  getProviderHarnessConfig: vi.fn(),
  getProvidersBootstrap: vi.fn(),
  listAmpAccounts: vi.fn(),
  listClaudeAccounts: vi.fn(),
  listCodexAccounts: vi.fn(),
  listCopilotAccounts: vi.fn(),
  listCursorAccounts: vi.fn(),
  listGeminiAccounts: vi.fn(),
  listKimiAccounts: vi.fn(),
  listMistralAccounts: vi.fn(),
  listProviders: vi.fn(),
  listQwenAccounts: vi.fn(),
}));

vi.mock("../api/client", () => clientMocks);

const emptyAccounts = {
  active_account_id: null,
  accounts: [],
};

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  clientMocks.listProviders.mockResolvedValue([
    {
      provider_id: "codex",
      display_name: "Codex",
      installed: true,
      health: "ok",
      diagnostics: [],
      details: {},
    },
    {
      provider_id: "claude-crp",
      display_name: "Claude",
      installed: true,
      health: "ok",
      diagnostics: [],
      details: {},
    },
  ]);
  clientMocks.getProviderHarnessConfig.mockImplementation(async (providerId: string) => ({
    provider_id: providerId,
    selected_source_kind: "subscription",
    selected_endpoint_id: null,
    endpoints: [],
  }));
  clientMocks.listCodexAccounts.mockResolvedValue({
    active_account_id: "acct-codex",
    accounts: [{ id: "acct-codex" }],
    logins: [],
  });
  clientMocks.listClaudeAccounts.mockResolvedValue({
    active_account_id: "acct-claude",
    accounts: [{ id: "acct-claude" }],
  });
  clientMocks.listGeminiAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.listQwenAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.listKimiAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.listMistralAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.listCopilotAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.listCursorAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.listAmpAccounts.mockResolvedValue(emptyAccounts);
  clientMocks.getProvidersBootstrap.mockResolvedValue(undefined);
});

describe("providersBootstrapStore", () => {
  it("loads host bootstrap with host auth and harness-config slices", async () => {
    const store = await import("./providersBootstrapStore");

    const bootstrap = await store.loadHostProvidersBootstrap();

    expect(clientMocks.listProviders).toHaveBeenCalledWith("host");
    expect(clientMocks.getProviderHarnessConfig).toHaveBeenCalledTimes(2);
    expect(bootstrap.codex_accounts.active_account_id).toBe("acct-codex");
    expect(bootstrap.claude_accounts.active_account_id).toBe("acct-claude");
    expect(bootstrap.provider_harness_config.codex).toMatchObject({
      provider_id: "codex",
      selected_source_kind: "subscription",
    });
    expect(store.getHostProvidersBootstrapSnapshot().provider_harness_config["claude-crp"]).toMatchObject({
      provider_id: "claude-crp",
    });
  });

  it("refreshes host bootstrap slices instead of leaving stale host auth in place", async () => {
    const store = await import("./providersBootstrapStore");

    await store.loadHostProvidersBootstrap();
    clientMocks.listCodexAccounts.mockResolvedValueOnce({
      active_account_id: "acct-codex-next",
      accounts: [{ id: "acct-codex-next" }],
      logins: [],
    });

    const refreshed = await store.refreshHostProvidersBootstrap();

    expect(refreshed.codex_accounts.active_account_id).toBe("acct-codex-next");
    expect(store.getHostProvidersBootstrapSnapshot().codex_accounts.active_account_id).toBe("acct-codex-next");
  });
});
