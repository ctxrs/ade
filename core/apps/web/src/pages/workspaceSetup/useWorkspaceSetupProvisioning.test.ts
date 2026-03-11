import { act, render } from "@testing-library/react";
import { createElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getSettings,
  listProviderAuthImportCandidates,
  listProviders,
} from "../../api/client";
import { useWorkspaceSetupProvisioning } from "./useWorkspaceSetupProvisioning";
import { deriveWorkspaceSetupEffectiveTarget } from "./workflowTypes";
import type { WizardRoutePlan } from "./wizardFlow";

vi.mock("../../api/client", () => ({
  cancelInstall: vi.fn(),
  getSettings: vi.fn(),
  getTitleGenerationLocalStatus: vi.fn(),
  importProviderAuthCandidates: vi.fn(),
  installProvider: vi.fn(),
  installTitleGenerationLocal: vi.fn(),
  listProviderAuthImportCandidates: vi.fn(),
  listProviders: vi.fn(),
  updateSettings: vi.fn(),
}));

vi.mock("../../state/installProgressMonitor", () => ({
  observeInstall: vi.fn(() => () => {}),
  subscribeInstallProgress: vi.fn(() => () => {}),
}));

vi.mock("../../state/providerInstallProgressStore", () => ({
  resolveProviderInstallProgressSession: vi.fn(() => null),
  subscribeProviderInstallProgress: vi.fn(() => () => {}),
  upsertProviderInstallProgress: vi.fn(),
}));

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
};

const deferred = <T,>(): Deferred<T> => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolveValue, rejectValue) => {
    resolve = resolveValue;
    reject = rejectValue;
  });
  return { promise, resolve, reject };
};

const configuredTitlingSettings = {
  title_generation: {
    mode: "remote",
    remote: {
      base_url: "https://openrouter.ai/api/v1",
      api_key: "sk-ready",
      model: "google/gemini-3-flash-preview",
      use_json: true,
    },
    local: {
      model_id: "ggml-org/Qwen3-1.7B-GGUF",
      use_json: true,
    },
  },
};

describe("useWorkspaceSetupProvisioning", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("ignores old refresh completions after the provisioning scope switches", async () => {
    const localAuth = deferred<{ candidates: Array<Record<string, string>> }>();
    const localHarness = deferred<Array<Record<string, unknown>>>();
    const localSettings = deferred<typeof configuredTitlingSettings>();

    vi.mocked(listProviderAuthImportCandidates)
      .mockImplementationOnce(() => localAuth.promise as never)
      .mockResolvedValueOnce({ candidates: [] } as never);
    vi.mocked(listProviders)
      .mockImplementationOnce(() => localHarness.promise as never)
      .mockResolvedValueOnce([] as never);
    vi.mocked(getSettings)
      .mockImplementationOnce(() => localSettings.promise as never)
      .mockResolvedValue(configuredTitlingSettings as never);

    const currentStepKeyRef = { current: "container" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = ({
      location,
      container,
      effectiveTarget,
      routePlan,
    }: {
      location: "local" | "remote";
      container: string;
      effectiveTarget: ReturnType<typeof deriveWorkspaceSetupEffectiveTarget>;
      routePlan: null;
    }) => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location,
          container,
        },
        routePlan,
        setRoutePlan,
        setRoutePlanningBusy,
        invalidateRoutePlan,
        desktopApp: true,
        effectiveTarget,
        remoteStatus: "connected",
        remoteStatusRef: { current: "connected" },
        connectDaemonForImport,
      });
      return null;
    };

    const { rerender } = render(
      createElement(Harness, {
        location: "local",
        container: "disk-isolated",
        effectiveTarget: deriveWorkspaceSetupEffectiveTarget("local", {
          remoteHostInput: "",
          remotePortInput: "4399",
          remoteDataDirInput: "",
        }),
        routePlan: null,
      }),
    );

    let localRoutePlanPromise: Promise<unknown> | null = null;
    await act(async () => {
      localRoutePlanPromise = latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    rerender(
      createElement(Harness, {
        location: "remote",
        container: "no-container",
        effectiveTarget: deriveWorkspaceSetupEffectiveTarget("remote", {
          remoteHostInput: "alice@builder.internal",
          remotePortInput: "4400",
          remoteDataDirInput: "/srv/ctx-b",
        }),
        routePlan: null,
      }),
    );

    let remoteRoutePlan: WizardRoutePlan | null = null;
    await act(async () => {
      remoteRoutePlan = await latest!.ensureRoutePlanForSelection("no-container");
    });

    await act(async () => {
      localAuth.resolve({
        candidates: [
          {
            id: "stale-auth",
            provider_id: "codex",
            provider_label: "Codex",
            kind: "file",
            path: "/tmp/codex.json",
            signal_strength: "high",
            confidence: "high",
            parse_status: "parsed",
          },
        ],
      });
      localHarness.resolve([
        {
          provider_id: "codex",
          installed: false,
          health: "error",
          diagnostics: [],
          details: {
            install_supported: "true",
          },
        },
      ]);
      localSettings.resolve(configuredTitlingSettings);
      await localRoutePlanPromise;
    });

    expect(remoteRoutePlan).toEqual({
      targetKey: expect.stringContaining("\"desktop_ssh\""),
      containerSelection: "no-container",
      includeHarnessDownloads: false,
      includeAuthImport: false,
      includeTitling: false,
    });
    expect(setRoutePlan).toHaveBeenLastCalledWith(remoteRoutePlan);
    expect(connectDaemonForImport).toHaveBeenCalledTimes(6);
  });
});
