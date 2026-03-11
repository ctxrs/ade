import { act, render } from "@testing-library/react";
import { createElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstallTarget } from "../../api/client";
import {
  getSettings,
  installProvider,
  listProviderAuthImportCandidates,
  listProviders,
} from "../../api/client";
import {
  type ProviderInstallProgressSession,
  type ProviderInstallProgressSnapshot,
  resolveProviderInstallProgressSession,
  subscribeProviderInstallProgressForScope,
  upsertProviderInstallProgressForScope,
} from "../../state/providerInstallProgressStore";
import { useWorkspaceSetupProvisioning } from "./useWorkspaceSetupProvisioning";
import {
  createWorkspaceSetupRouteScope,
  deriveWorkspaceSetupEffectiveTarget,
} from "./workflowTypes";
import type { WizardRoutePlan } from "./wizardFlow";
import { createHostOwnerScope } from "../../state/scopeIdentity";

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
  subscribeProviderInstallProgressForScope: vi.fn(() => () => {}),
  upsertProviderInstallProgressForScope: vi.fn(),
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
      api_key_set: true,
      model: "google/gemini-3-flash-preview",
      use_json: true,
    },
    local: {
      model_id: "ggml-org/Qwen3-1.7B-GGUF",
      use_json: true,
    },
  },
};

const authImportCandidateFixture = {
  id: "acct-1",
  provider_id: "codex",
  provider_label: "Codex",
  kind: "file",
  path: "/tmp/codex.json",
  signal_strength: "high",
  confidence: "high",
  parse_status: "parsed",
} as const;

type MockProviderInstallProgressSnapshot = Record<string, ProviderInstallProgressSession>;

describe("useWorkspaceSetupProvisioning", () => {
  let providerProgressSnapshot: ProviderInstallProgressSnapshot;
  let providerProgressListeners: Set<(snapshot: ProviderInstallProgressSnapshot) => void>;

  const emitProviderProgressSnapshot = (snapshot: MockProviderInstallProgressSnapshot) => {
    providerProgressSnapshot = snapshot as unknown as ProviderInstallProgressSnapshot;
    for (const listener of providerProgressListeners) {
      listener(providerProgressSnapshot);
    }
  };

  beforeEach(() => {
    vi.clearAllMocks();
    providerProgressSnapshot = {};
    providerProgressListeners = new Set();
    vi.mocked(subscribeProviderInstallProgressForScope).mockImplementation((_ownerScope, listener) => {
      const typedListener = listener as (snapshot: ProviderInstallProgressSnapshot) => void;
      providerProgressListeners.add(typedListener);
      typedListener(providerProgressSnapshot);
      return () => {
        providerProgressListeners.delete(typedListener);
      };
    });
    vi.mocked(resolveProviderInstallProgressSession).mockImplementation((snapshot, providerId, target) => {
      const typedSnapshot = snapshot as unknown as MockProviderInstallProgressSnapshot;
      const session = typedSnapshot[providerId];
      if (!session) return undefined;
      if (target && session.target && session.target !== target) {
        return undefined;
      }
      return session;
    });
  });

  it("retries same-target route planning after a transient refresh failure", async () => {
    vi.mocked(listProviderAuthImportCandidates)
      .mockResolvedValue({ candidates: [] } as never);
    vi.mocked(listProviders)
      .mockRejectedValueOnce(new Error("Harness scan failed."))
      .mockResolvedValueOnce([] as never);
    vi.mocked(getSettings)
      .mockResolvedValue(configuredTitlingSettings as never);

    const currentStepKeyRef = { current: "container" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});

    const localEffectiveTarget = deriveWorkspaceSetupEffectiveTarget("local", {
      remoteHostInput: "",
      remotePortInput: "4399",
      remoteDataDirInput: "",
    });

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = ({ routePlan }: { routePlan: WizardRoutePlan | null }) => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location: "local",
          container: "disk-isolated",
        },
        routePlan,
        setRoutePlan,
        setRoutePlanningBusy,
        invalidateRoutePlan,
        desktopApp: true,
        effectiveTarget: localEffectiveTarget,
        remoteStatus: "connected",
        remoteStatusRef: { current: "connected" },
        connectDaemonForImport,
      });
      return null;
    };

    const { rerender } = render(createElement(Harness, { routePlan: null }));

    let staleRoutePlan: WizardRoutePlan | null = null;
    await act(async () => {
      staleRoutePlan = await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(staleRoutePlan).toEqual({
      targetKey: expect.stringContaining("\"disk-isolated\""),
      containerSelection: "disk-isolated",
      includeHarnessDownloads: true,
      includeAuthImport: false,
      includeTitling: false,
    });
    expect(listProviderAuthImportCandidates).toHaveBeenCalledTimes(1);
    expect(listProviders).toHaveBeenCalledTimes(1);
    expect(getSettings).toHaveBeenCalledTimes(1);

    rerender(createElement(Harness, { routePlan: staleRoutePlan }));

    let recoveredRoutePlan: WizardRoutePlan | null = null;
    await act(async () => {
      recoveredRoutePlan = await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(recoveredRoutePlan).toEqual({
      targetKey: staleRoutePlan!.targetKey,
      containerSelection: "disk-isolated",
      includeHarnessDownloads: false,
      includeAuthImport: false,
      includeTitling: false,
    });
    expect(listProviderAuthImportCandidates).toHaveBeenCalledTimes(1);
    expect(listProviders).toHaveBeenCalledTimes(2);
    expect(getSettings).toHaveBeenCalledTimes(1);
    expect(setRoutePlan).toHaveBeenLastCalledWith(recoveredRoutePlan);
  });

  it("keeps ready same-target route planning on the fast path", async () => {
    vi.mocked(listProviderAuthImportCandidates)
      .mockResolvedValue({ candidates: [] } as never);
    vi.mocked(listProviders)
      .mockResolvedValue([] as never);
    vi.mocked(getSettings)
      .mockResolvedValue(configuredTitlingSettings as never);

    const currentStepKeyRef = { current: "container" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});

    const localEffectiveTarget = deriveWorkspaceSetupEffectiveTarget("local", {
      remoteHostInput: "",
      remotePortInput: "4399",
      remoteDataDirInput: "",
    });

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = ({ routePlan }: { routePlan: WizardRoutePlan | null }) => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location: "local",
          container: "disk-isolated",
        },
        routePlan,
        setRoutePlan,
        setRoutePlanningBusy,
        invalidateRoutePlan,
        desktopApp: true,
        effectiveTarget: localEffectiveTarget,
        remoteStatus: "connected",
        remoteStatusRef: { current: "connected" },
        connectDaemonForImport,
      });
      return null;
    };

    const { rerender } = render(createElement(Harness, { routePlan: null }));

    let readyRoutePlan: WizardRoutePlan | null = null;
    await act(async () => {
      readyRoutePlan = await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(readyRoutePlan).toEqual({
      targetKey: expect.stringContaining("\"disk-isolated\""),
      containerSelection: "disk-isolated",
      includeHarnessDownloads: false,
      includeAuthImport: false,
      includeTitling: false,
    });
    expect(listProviderAuthImportCandidates).toHaveBeenCalledTimes(1);
    expect(listProviders).toHaveBeenCalledTimes(1);
    expect(getSettings).toHaveBeenCalledTimes(1);
    expect(setRoutePlanningBusy).toHaveBeenCalledTimes(2);

    rerender(createElement(Harness, { routePlan: readyRoutePlan }));

    let secondRoutePlan: WizardRoutePlan | null = null;
    await act(async () => {
      secondRoutePlan = await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(secondRoutePlan).toEqual(readyRoutePlan);
    expect(listProviderAuthImportCandidates).toHaveBeenCalledTimes(1);
    expect(listProviders).toHaveBeenCalledTimes(1);
    expect(getSettings).toHaveBeenCalledTimes(1);
    expect(setRoutePlanningBusy).toHaveBeenCalledTimes(2);
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
      routePlan: WizardRoutePlan | null;
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

  it("clears stale auth-import candidates after a same-scope refresh failure", async () => {
    vi.mocked(listProviderAuthImportCandidates)
      .mockResolvedValueOnce({ candidates: [authImportCandidateFixture] } as never)
      .mockRejectedValueOnce(new Error("Auth refresh failed."));
    vi.mocked(listProviders).mockResolvedValue([] as never);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettings as never);

    const currentStepKeyRef = { current: "auth-import" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});
    const effectiveTarget = deriveWorkspaceSetupEffectiveTarget("local", {
      remoteHostInput: "",
      remotePortInput: "4399",
      remoteDataDirInput: "",
    });
    if (!effectiveTarget) {
      throw new Error("Expected a local workspace setup target.");
    }

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = () => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location: "local",
          container: "disk-isolated",
        },
        routePlan: null,
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

    render(createElement(Harness));

    await act(async () => {
      await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(latest!.authImportCandidates).toEqual([authImportCandidateFixture]);
    expect(latest!.authImportSelected).toEqual({ [authImportCandidateFixture.id]: true });

    await act(async () => {
      await latest!.refreshAuthImportForRouteScope(
        "local",
        createWorkspaceSetupRouteScope(effectiveTarget, "disk-isolated"),
        { force: true },
      );
    });

    expect(latest!.authImportCandidates).toEqual([]);
    expect(latest!.authImportSelected).toEqual({});
    expect(latest!.authImportError).toBe("Auth refresh failed.");
  });

  it("clears stale harness rows when a same-scope harness refresh fails", async () => {
    vi.mocked(listProviderAuthImportCandidates)
      .mockResolvedValueOnce({ candidates: [] } as never)
      .mockResolvedValueOnce({ candidates: [] } as never);
    vi.mocked(listProviders)
      .mockResolvedValueOnce([
        {
          provider_id: "codex",
          installed: false,
          health: "error",
          diagnostics: [],
          details: {
            install_supported: "true",
            install_running: "true",
            install_id: "install-1",
            install_target: "container",
          },
        },
      ] as never)
      .mockRejectedValueOnce(new Error("Harness scan failed."));
    vi.mocked(getSettings)
      .mockResolvedValueOnce(configuredTitlingSettings as never)
      .mockResolvedValueOnce(configuredTitlingSettings as never);

    const currentStepKeyRef = { current: "container" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = () => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location: "local",
          container: "disk-isolated",
        },
        routePlan: null,
        setRoutePlan,
        setRoutePlanningBusy,
        invalidateRoutePlan,
        desktopApp: true,
        effectiveTarget: deriveWorkspaceSetupEffectiveTarget("local", {
          remoteHostInput: "",
          remotePortInput: "4399",
          remoteDataDirInput: "",
        }),
        remoteStatus: "connected",
        remoteStatusRef: { current: "connected" },
        connectDaemonForImport,
      });
      return null;
    };

    render(createElement(Harness));

    await act(async () => {
      await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(latest!.harnessInstallCandidates).toEqual([
      expect.objectContaining({
        providerId: "codex",
        installId: "install-1",
        installRunning: true,
      }),
    ]);
    expect(latest!.harnessInstallRows).toEqual({
      codex: expect.objectContaining({
        installId: "install-1",
        state: "running",
        target: "container",
      }),
    });

    await act(async () => {
      emitProviderProgressSnapshot({
        codex: {
          installId: "install-1",
          state: "running",
          pct: null,
          target: "container" satisfies InstallTarget,
          updatedAtMs: 1,
        },
      });
    });

    await act(async () => {
      await latest!.ensureOnboardingAfterDaemonConnect();
    });

    expect(latest!.harnessInstallCandidates).toEqual([]);
    expect(latest!.harnessInstallRows).toEqual({});
    expect(latest!.harnessInstallError).toContain("Harness scan failed.");
  });

  it("subscribes harness progress using the selected daemon scope", async () => {
    vi.mocked(listProviderAuthImportCandidates)
      .mockResolvedValue({ candidates: [] } as never);
    vi.mocked(listProviders)
      .mockResolvedValue([
        {
          provider_id: "codex",
          installed: false,
          health: "error",
          diagnostics: [],
          details: {
            install_supported: "true",
            install_target: "container",
          },
        },
      ] as never);
    vi.mocked(getSettings)
      .mockResolvedValue(configuredTitlingSettings as never);

    const currentStepKeyRef = { current: "container" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});
    const effectiveTarget = deriveWorkspaceSetupEffectiveTarget("remote", {
      remoteHostInput: "alice@builder.internal",
      remotePortInput: "4400",
      remoteDataDirInput: "/srv/ctx-remote",
    });
    if (!effectiveTarget) {
      throw new Error("Expected a remote workspace setup target.");
    }
    const expectedOwnerScope = createHostOwnerScope(effectiveTarget.daemonScope);

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = () => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location: "remote",
          container: "disk-isolated",
        },
        routePlan: null,
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

    render(createElement(Harness));

    await act(async () => {
      await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    expect(subscribeProviderInstallProgressForScope).toHaveBeenCalledWith(
      expectedOwnerScope,
      expect.any(Function),
    );
    expect(upsertProviderInstallProgressForScope).not.toHaveBeenCalled();
  });

  it("writes started harness installs into the selected daemon scope", async () => {
    vi.mocked(listProviderAuthImportCandidates)
      .mockResolvedValue({ candidates: [] } as never);
    vi.mocked(listProviders)
      .mockResolvedValue([
        {
          provider_id: "codex",
          installed: false,
          health: "error",
          diagnostics: [],
          details: {
            install_supported: "true",
            install_target: "container",
          },
        },
      ] as never);
    vi.mocked(getSettings)
      .mockResolvedValue(configuredTitlingSettings as never);
    vi.mocked(installProvider)
      .mockResolvedValue({
        install_id: "install-codex",
        provider_id: "codex",
        target: "container",
      } as never);

    const currentStepKeyRef = { current: "harness-downloads" as const };
    const setRoutePlan = vi.fn();
    const setRoutePlanningBusy = vi.fn();
    const invalidateRoutePlan = vi.fn();
    const connectDaemonForImport = vi.fn(async () => {});
    const effectiveTarget = deriveWorkspaceSetupEffectiveTarget("remote", {
      remoteHostInput: "alice@builder.internal",
      remotePortInput: "4400",
      remoteDataDirInput: "/srv/ctx-remote",
    });
    if (!effectiveTarget) {
      throw new Error("Expected a remote workspace setup target.");
    }
    const expectedOwnerScope = createHostOwnerScope(effectiveTarget.daemonScope);

    let latest: ReturnType<typeof useWorkspaceSetupProvisioning> | null = null;

    const Harness = () => {
      latest = useWorkspaceSetupProvisioning({
        currentStepKeyRef,
        selections: {
          location: "remote",
          container: "disk-isolated",
        },
        routePlan: {
          targetKey: "remote-route",
          containerSelection: "disk-isolated",
          includeHarnessDownloads: true,
          includeAuthImport: false,
          includeTitling: false,
        },
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

    render(createElement(Harness));

    await act(async () => {
      await latest!.ensureRoutePlanForSelection("disk-isolated");
    });

    await act(async () => {
      latest!.setHarnessInstallSelected({ codex: true });
    });

    await act(async () => {
      await latest!.advanceFromHarnessDownloadsStep();
    });

    expect(installProvider).toHaveBeenCalledWith("codex", "container");
    expect(upsertProviderInstallProgressForScope).toHaveBeenCalledWith(
      expectedOwnerScope,
      "codex",
      expect.objectContaining({
        installId: "install-codex",
        state: "running",
        target: "container",
      }),
    );
  });
});
