import { act, render } from "@testing-library/react";
import { useEffect } from "react";
import type { Dispatch, SetStateAction } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDesktopEditorSettingsController } from "./useDesktopEditorSettingsController";
import {
  desktopGetEditorSettings,
  desktopUpdateEditorSettings,
  type DesktopEditorSettings,
} from "../../../utils/desktop";

vi.mock("../../../utils/desktop", async () => {
  const actual = await vi.importActual<typeof import("../../../utils/desktop")>("../../../utils/desktop");
  return {
    ...actual,
    desktopGetEditorSettings: vi.fn(),
    desktopUpdateEditorSettings: vi.fn(),
  };
});

type DesktopEditorSettingsControllerRef = {
  editorLoaded: boolean;
  editorSettings: DesktopEditorSettings;
  setEditorSettings: Dispatch<SetStateAction<DesktopEditorSettings>>;
};

function Harness({
  enabled,
  onReady,
}: {
  enabled: boolean;
  onReady: (controller: DesktopEditorSettingsControllerRef) => void;
}) {
  const controller = useDesktopEditorSettingsController(enabled);

  useEffect(() => {
    onReady(controller);
  }, [controller, onReady]);

  return null;
}

describe("useDesktopEditorSettingsController", () => {
  const desktopGetEditorSettingsMock = vi.mocked(desktopGetEditorSettings);
  const desktopUpdateEditorSettingsMock = vi.mocked(desktopUpdateEditorSettings);

  beforeEach(() => {
    vi.useFakeTimers();
    desktopGetEditorSettingsMock.mockReset();
    desktopUpdateEditorSettingsMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not re-save identical server responses", async () => {
    desktopGetEditorSettingsMock.mockResolvedValue({
      target: "system",
      custom_command: null,
      remote_authority: null,
    });
    desktopUpdateEditorSettingsMock.mockResolvedValue({
      target: "cursor",
      custom_command: null,
      remote_authority: null,
    });

    const controllerRef: { current: DesktopEditorSettingsControllerRef | null } = { current: null };
    render(<Harness enabled onReady={(controller) => {
      controllerRef.current = controller;
    }} />);

    await act(async () => {
      await Promise.resolve();
    });
    expect(controllerRef.current?.editorLoaded).toBe(true);

    act(() => {
      controllerRef.current?.setEditorSettings((prev) => ({
        ...prev,
        target: "cursor",
      }));
    });

    await act(async () => {
      vi.advanceTimersByTime(350);
      await Promise.resolve();
    });

    expect(desktopUpdateEditorSettingsMock).toHaveBeenCalledTimes(1);
    expect(desktopUpdateEditorSettingsMock).toHaveBeenCalledWith({
      target: "cursor",
      custom_command: null,
      remote_authority: null,
    });

    await act(async () => {
      await Promise.resolve();
      vi.advanceTimersByTime(1000);
      await Promise.resolve();
    });

    expect(desktopUpdateEditorSettingsMock).toHaveBeenCalledTimes(1);
  });

  it("does not clobber newer local edits with an older save response", async () => {
    desktopGetEditorSettingsMock.mockResolvedValue({
      target: "custom",
      custom_command: "code --goto {path}:{line}:{col}",
      remote_authority: null,
    });

    let resolveSave: ((value: {
      target: "custom";
      custom_command: string | null;
      remote_authority: string | null;
    }) => void) | null = null;
    desktopUpdateEditorSettingsMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveSave = resolve;
        }),
    );

    const controllerRef: { current: DesktopEditorSettingsControllerRef | null } = { current: null };
    render(<Harness enabled onReady={(controller) => {
      controllerRef.current = controller;
    }} />);

    await act(async () => {
      await Promise.resolve();
    });
    expect(controllerRef.current?.editorLoaded).toBe(true);

    act(() => {
      controllerRef.current?.setEditorSettings((prev) => ({
        ...prev,
        custom_command: "cursor {path}",
      }));
    });

    await act(async () => {
      vi.advanceTimersByTime(350);
      await Promise.resolve();
    });

    act(() => {
      controllerRef.current?.setEditorSettings((prev) => ({
        ...prev,
        custom_command: "windsurf {path}",
      }));
    });

    await act(async () => {
      resolveSave?.({
        target: "custom",
        custom_command: "cursor {path}",
        remote_authority: null,
      });
      await Promise.resolve();
    });

    expect(controllerRef.current?.editorSettings.custom_command).toBe("windsurf {path}");
  });
});
