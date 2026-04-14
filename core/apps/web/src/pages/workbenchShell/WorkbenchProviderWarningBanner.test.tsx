import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderStatus } from "../../api/client";
import { WorkbenchProviderWarningBanner } from "./WorkbenchProviderWarningBanner";

const providerStatus = (
  providerId: string,
  overrides: Partial<ProviderStatus> = {},
): ProviderStatus => ({
  provider_id: providerId,
  installed: true,
  health: "ok",
  diagnostics: [],
  details: {},
  usability: {
    usable: true,
    status: "ready",
    blocking_provider_ids: [],
    recommended_action: "none",
  },
  ...overrides,
});

describe("WorkbenchProviderWarningBanner", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
  });

  it("renders nothing when no visible provider needs attention", () => {
    const { container } = render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{ codex: providerStatus("codex") }}
        onUpdateProviders={() => Promise.resolve()}
        onOpenSettings={() => {}}
      />,
    );

    expect(container.firstChild).toBeNull();
  });

  it("updates the flagged runtimes and dismisses the notice on success", async () => {
    const onUpdateProviders = vi.fn().mockResolvedValue(undefined);
    const onOpenSettings = vi.fn();

    render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          codex: providerStatus("codex", {
            details: {
              install_supported: "true",
              matrix_update_available: "true",
              matrix_recommended_version: "0.114.0-ctx.2",
            },
            version: "0.114.0-ctx.1",
          }),
        }}
        onUpdateProviders={onUpdateProviders}
        onOpenSettings={onOpenSettings}
      />,
    );

    expect(screen.getByTestId("workbench-provider-warning")).toBeInTheDocument();
    expect(screen.getByText("1 provider runtime needs an update.")).toBeInTheDocument();
    expect(screen.queryByText("Codex")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Update All" }));
    expect(onUpdateProviders).toHaveBeenCalledWith(["codex"]);

    await waitFor(() => {
      expect(screen.queryByTestId("workbench-provider-warning")).not.toBeInTheDocument();
    });
    expect(onOpenSettings).not.toHaveBeenCalled();
  });

  it("dismisses immediately when update all is pressed even if the update fails", async () => {
    const onUpdateProviders = vi.fn().mockRejectedValue(new Error("update failed"));

    render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          codex: providerStatus("codex", {
            details: {
              install_supported: "true",
              matrix_update_available: "true",
            },
          }),
        }}
        onUpdateProviders={onUpdateProviders}
        onOpenSettings={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Update All" }));

    await waitFor(() => {
      expect(screen.queryByTestId("workbench-provider-warning")).not.toBeInTheDocument();
    });
    expect(onUpdateProviders).toHaveBeenCalledWith(["codex"]);
  });

  it("dismisses before opening settings and keeps the dismissal for the same warning signature", async () => {
    const onOpenSettings = vi.fn();

    render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          codex: providerStatus("codex", {
            details: {
              install_supported: "true",
              matrix_update_available: "true",
            },
          }),
          "claude-crp": providerStatus("claude-crp", {
            health: "unsupported_version",
            details: { matrix_update_available: "true" },
            diagnostics: ["Provider version requires a newer ctx build"],
          }),
        }}
        onUpdateProviders={() => Promise.resolve()}
        onOpenSettings={onOpenSettings}
      />,
    );

    expect(screen.getByRole("button", { name: "Update All" })).toBeInTheDocument();
    expect(screen.getByText("2 provider runtimes need an update.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);

    await waitFor(() => {
      expect(screen.queryByTestId("workbench-provider-warning")).not.toBeInTheDocument();
    });

    const { container } = render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          codex: providerStatus("codex", {
            details: {
              install_supported: "true",
              matrix_update_available: "true",
            },
          }),
          "claude-crp": providerStatus("claude-crp", {
            health: "unsupported_version",
            details: { matrix_update_available: "true" },
            diagnostics: ["Provider version requires a newer ctx build"],
          }),
        }}
        onUpdateProviders={() => Promise.resolve()}
        onOpenSettings={onOpenSettings}
      />,
    );

    expect(container.firstChild).toBeNull();
  });

  it("renders again when the warning signature changes", () => {
    render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          codex: providerStatus("codex", {
            details: { matrix_update_available: "true" },
          }),
        }}
        onUpdateProviders={() => Promise.resolve()}
        onOpenSettings={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));

    render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          codex: providerStatus("codex", {
            details: {
              install_supported: "true",
              matrix_update_available: "true",
              matrix_recommended_version: "0.114.0-ctx.3",
            },
            version: "0.114.0-ctx.1",
          }),
          "claude-crp": providerStatus("claude-crp", {
            details: { install_supported: "true", managed_fingerprint_mismatch: "true" },
          }),
        }}
        onUpdateProviders={() => Promise.resolve()}
        onOpenSettings={() => {}}
      />,
    );

    expect(screen.getByTestId("workbench-provider-warning")).toBeInTheDocument();
  });

  it("falls back to settings-only when no flagged provider supports managed updates", () => {
    render(
      <WorkbenchProviderWarningBanner
        workspaceId="ws-1"
        providersById={{
          "claude-crp": providerStatus("claude-crp", {
            health: "unsupported_version",
            details: { matrix_update_available: "true" },
            diagnostics: ["Provider version requires a newer ctx build"],
          }),
        }}
        onUpdateProviders={() => Promise.resolve()}
        onOpenSettings={() => {}}
      />,
    );

    expect(screen.queryByRole("button", { name: "Update All" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Settings" })).toBeInTheDocument();
    expect(screen.getByText("1 provider runtime needs an update.")).toBeInTheDocument();
  });
});
