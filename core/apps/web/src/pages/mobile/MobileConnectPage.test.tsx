import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router-dom";

const setDaemonConnectionMock = vi.fn();
const clearDaemonConnectionMock = vi.fn();
const listWorkspacesMock = vi.fn();
const normalizeDaemonBaseUrlMock = vi.fn();
const pairManagedMobileQrPayloadMock = vi.fn();

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

    fireEvent.change(screen.getByPlaceholderText("http://192.168.1.20:4399"), {
      target: { value: "http://192.168.1.50:4399" },
    });
    fireEvent.change(screen.getByPlaceholderText("ctx daemon token"), {
      target: { value: "mobile-token" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    await waitFor(() => {
      expect(setDaemonConnectionMock).toHaveBeenCalledWith(
        {
          baseUrl: "http://192.168.1.50:4399",
          authToken: "mobile-token",
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

    fireEvent.change(screen.getByPlaceholderText("http://192.168.1.20:4399"), {
      target: { value: "192.168.1.50:4399" },
    });
    fireEvent.change(screen.getByPlaceholderText("ctx daemon token"), {
      target: { value: "mobile-token" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));

    expect(await screen.findByText("Enter a valid daemon URL including http:// or https://.")).toBeInTheDocument();
    expect(setDaemonConnectionMock).not.toHaveBeenCalled();
  });

  it("does not render the retired LAN dogfood helper block", () => {
    render(
      <MemoryRouter initialEntries={["/mobile/connect"]}>
        <Routes>
          <Route path="/mobile/connect" element={<MobileConnectPage />} />
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.queryByText("LAN dogfood path")).not.toBeInTheDocument();
  });

  it("clears the persisted connection when disconnecting", async () => {
    connection = buildConnection({
      baseUrl: "http://192.168.1.50:4399",
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
