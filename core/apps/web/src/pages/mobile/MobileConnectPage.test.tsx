import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router-dom";

const setDaemonConnectionMock = vi.fn();
const clearDaemonConnectionMock = vi.fn();
const listWorkspacesMock = vi.fn();
const normalizeDaemonBaseUrlMock = vi.fn();
const pairManagedMobileQrPayloadMock = vi.fn();
const scannerMocks = vi.hoisted(() => ({
  canUseQrCameraScanner: vi.fn(),
  detectedPayload: '{"type":"context_mobile_e2ee","source":"scan"}',
}));

const buildConnection = (overrides?: Partial<{
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  runId: string | null;
  source: string | null;
  targetScope: { kind: string; baseUrl?: string | null } | null;
  mobileSecure: { kind: "managed_tunnel" } | null;
}>) => ({
  baseUrl: null,
  wsBaseUrl: null,
  authToken: null,
  runId: null,
  source: null,
  targetScope: null,
  mobileSecure: null,
  ...overrides,
});

let connection = buildConnection();

vi.mock("../../api/client", () => ({
  clearDaemonConnection: (...args: unknown[]) => clearDaemonConnectionMock(...args),
  getDaemonConnectionReadiness: (value: { baseUrl: string | null; authToken: string | null }) => ({
    hasBaseUrl: Boolean(value.baseUrl),
    hasAuthToken: Boolean(value.authToken),
    hasMobileSecure: Boolean((value as { mobileSecure?: unknown }).mobileSecure),
    isReady: Boolean(value.baseUrl && (value.authToken || (value as { mobileSecure?: unknown }).mobileSecure)),
    missing: !value.baseUrl ? "base" : !value.authToken && !(value as { mobileSecure?: unknown }).mobileSecure ? "auth" : null,
  }),
  listWorkspaces: (...args: unknown[]) => listWorkspacesMock(...args),
  normalizeDaemonBaseUrl: (value: string | null | undefined) => normalizeDaemonBaseUrlMock(value),
  setDaemonConnection: (...args: unknown[]) => setDaemonConnectionMock(...args),
}));

vi.mock("../../api/useDaemonConnection", () => ({
  useDaemonConnection: () => connection,
}));

vi.mock("../../api/mobileSecureClient", () => ({
  pairManagedMobileQrPayload: (...args: unknown[]) => pairManagedMobileQrPayloadMock(...args),
}));

vi.mock("./MobileQrScanner", () => ({
  canUseQrCameraScanner: (...args: unknown[]) => scannerMocks.canUseQrCameraScanner(...args),
  MobileQrScanner: ({
    onDetected,
    onCancel,
  }: {
    onDetected: (payload: string) => void;
    onCancel: () => void;
  }) => (
    <div role="group" aria-label="QR scanner">
      <button type="button" onClick={() => onDetected(scannerMocks.detectedPayload)}>
        Mock detected QR
      </button>
      <button type="button" onClick={onCancel}>
        Mock cancel scan
      </button>
    </div>
  ),
}));

import { MobileConnectPage } from "./MobileConnectPage";

describe("MobileConnectPage", () => {
  beforeEach(() => {
    connection = buildConnection();
    setDaemonConnectionMock.mockReset();
    clearDaemonConnectionMock.mockReset();
    listWorkspacesMock.mockReset();
    normalizeDaemonBaseUrlMock.mockReset();
    pairManagedMobileQrPayloadMock.mockReset();
    normalizeDaemonBaseUrlMock.mockImplementation((value: string | null | undefined) => {
      const trimmed = String(value ?? "").trim();
      return trimmed.startsWith("http://") || trimmed.startsWith("https://") ? trimmed : null;
    });
    scannerMocks.canUseQrCameraScanner.mockReset();
    scannerMocks.canUseQrCameraScanner.mockReturnValue(true);
  });

  it("pairs from a pasted managed tunnel QR payload", async () => {
    pairManagedMobileQrPayloadMock.mockResolvedValue(undefined);

    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
          <Route path="/" element={<div>Home</div>} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.change(screen.getByPlaceholderText('{"type":"context_mobile_e2ee",...}'), {
      target: { value: '{"type":"context_mobile_e2ee"}' },
    });
    fireEvent.click(screen.getByRole("button", { name: "Pair" }));

    await waitFor(() => {
      expect(pairManagedMobileQrPayloadMock).toHaveBeenCalledWith('{"type":"context_mobile_e2ee"}');
    });
    expect(await screen.findByText("Home")).toBeInTheDocument();
  });

  it("pairs from a scanned managed tunnel QR payload", async () => {
    pairManagedMobileQrPayloadMock.mockResolvedValue(undefined);

    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
          <Route path="/" element={<div>Home</div>} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Scan QR" }));
    fireEvent.click(screen.getByRole("button", { name: "Mock detected QR" }));

    await waitFor(() => {
      expect(pairManagedMobileQrPayloadMock).toHaveBeenCalledWith(scannerMocks.detectedPayload);
    });
    expect(await screen.findByText("Home")).toBeInTheDocument();
  });

  it("falls back to paste when camera scanning is unavailable", async () => {
    scannerMocks.canUseQrCameraScanner.mockReturnValue(false);

    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Scan QR" }));

    expect(await screen.findByText("Camera QR scanning is unavailable here. Paste the QR payload instead.")).toBeInTheDocument();
    expect(pairManagedMobileQrPayloadMock).not.toHaveBeenCalled();
  });

  it("connects and navigates to the workspace list when validation succeeds", async () => {
    listWorkspacesMock.mockResolvedValue([]);

    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
          <Route path="/" element={<div>Home</div>} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.change(screen.getByPlaceholderText("https://daemon.example.com"), {
      target: { value: "https://daemon.example.com" },
    });
    fireEvent.change(screen.getByPlaceholderText("ctx daemon token"), {
      target: { value: "mobile-token" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    await waitFor(() => {
      expect(setDaemonConnectionMock).toHaveBeenCalledWith(
        {
          baseUrl: "https://daemon.example.com",
          authToken: "mobile-token",
          mobileSecure: null,
          source: "mobile_manual_connect",
        },
        { persistBaseUrl: true, persistAuthToken: true },
      );
    });
    expect(await screen.findByText("Home")).toBeInTheDocument();
  });

  it("shows an inline validation error for malformed daemon urls", async () => {
    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.change(screen.getByPlaceholderText("https://daemon.example.com"), {
      target: { value: "192.168.1.50:4399" },
    });
    fireEvent.change(screen.getByPlaceholderText("ctx daemon token"), {
      target: { value: "mobile-token" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    expect(await screen.findByText("Enter a reachable HTTPS daemon URL.")).toBeInTheDocument();
    expect(setDaemonConnectionMock).not.toHaveBeenCalled();
  });

  it("rejects cleartext direct daemon urls in the production mobile form", async () => {
    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.change(screen.getByPlaceholderText("https://daemon.example.com"), {
      target: { value: "http://daemon.example.com" },
    });
    fireEvent.change(screen.getByPlaceholderText("ctx daemon token"), {
      target: { value: "mobile-token" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    expect(await screen.findByText("Enter a reachable HTTPS daemon URL.")).toBeInTheDocument();
    expect(setDaemonConnectionMock).not.toHaveBeenCalled();
  });

  it("does not render the retired LAN helper block", () => {
    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.queryByText("LAN helper path")).not.toBeInTheDocument();
  });

  it("clears the persisted connection when disconnecting", async () => {
    connection = buildConnection({
      baseUrl: "https://daemon.example.com",
      authToken: "mobile-token",
    });

    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));

    expect(clearDaemonConnectionMock).toHaveBeenCalledWith({
      clearPersistedBaseUrl: true,
      clearPersistedAuthToken: true,
    });
  });
});
