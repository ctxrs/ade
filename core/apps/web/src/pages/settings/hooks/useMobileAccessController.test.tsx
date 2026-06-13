import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useMobileAccessController } from "./useMobileAccessController";

describe("useMobileAccessController", () => {
  it("reports mobile access actions as unavailable in the source build", async () => {
    const { result } = renderHook(() => useMobileAccessController({ getAuthToken: null }));

    await act(async () => {
      await result.current.handleEnableMobile();
    });

    expect(result.current.mobileEnableError).toBe(
      "This settings action is unavailable in the source build/local ADE.",
    );
    expect(result.current.mobileQr).toBeNull();
    expect(result.current.mobileStatus).toBeNull();
  });
});
