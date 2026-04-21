import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  disableMobileAccess,
  enableMobileAccess,
  getMobileAccessStatus,
} from "../../../api/client";
import { useMobileAccessController } from "./useMobileAccessController";

vi.mock("../../../api/client", async () => {
  const actual = await vi.importActual<typeof import("../../../api/client")>(
    "../../../api/client",
  );
  return {
    ...actual,
    disableMobileAccess: vi.fn(),
    enableMobileAccess: vi.fn(),
    getMobileAccessStatus: vi.fn(),
  };
});

describe("useMobileAccessController", () => {
  const disableMobileAccessMock = vi.mocked(disableMobileAccess);
  const enableMobileAccessMock = vi.mocked(enableMobileAccess);
  const getMobileAccessStatusMock = vi.mocked(getMobileAccessStatus);

  beforeEach(() => {
    disableMobileAccessMock.mockReset();
    enableMobileAccessMock.mockReset();
    getMobileAccessStatusMock.mockReset();
  });

  it("enables mobile access through the injected auth token provider", async () => {
    const getAuthToken = vi.fn().mockResolvedValue("supabase-token");
    enableMobileAccessMock.mockResolvedValue({
      status: {
        enabled: true,
        tunnel_state: "running",
      },
      qr_payload: { url: "ctx://pair" },
      pairing_expires_at: "2026-04-21T00:00:00Z",
    });

    const { result } = renderHook(() => useMobileAccessController({ getAuthToken }));

    await act(async () => {
      await result.current.handleEnableMobile();
    });

    expect(getAuthToken).toHaveBeenCalledTimes(1);
    expect(enableMobileAccessMock).toHaveBeenCalledWith("supabase-token");
    await waitFor(() => {
      expect(result.current.mobileStatus).toEqual({
        enabled: true,
        tunnel_state: "running",
      });
    });
  });

  it("surfaces a clear error when no auth token provider is available", async () => {
    const { result } = renderHook(() =>
      useMobileAccessController({ getAuthToken: null }),
    );

    await act(async () => {
      await result.current.handleEnableMobile();
    });

    expect(enableMobileAccessMock).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(result.current.mobileEnableError).toBe("Supabase is not configured.");
    });
  });
});
