import { act, render, screen, waitFor } from "@testing-library/react";
import { useEffect, useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { DictationSettings, Settings } from "../api/client";
import { appendSegment } from "../pages/SessionPage.helpers";
import type { SttApi, SttResult } from "./tauriStt";
import { useDictationController } from "./useDictationController";
import { getSettings } from "../api/client";
import { loadSttApi } from "./tauriStt";

vi.mock("../api/client", async () => {
  const actual = await vi.importActual<typeof import("../api/client")>("../api/client");
  return {
    ...actual,
    getSettings: vi.fn(),
  };
});

vi.mock("./desktop", () => ({
  isDesktopApp: () => true,
}));

vi.mock("./tauriStt", async () => {
  const actual = await vi.importActual<typeof import("./tauriStt")>("./tauriStt");
  return {
    ...actual,
    loadSttApi: vi.fn(),
  };
});

function Harness({ onReady }: { onReady: (controller: ReturnType<typeof useDictationController>) => void }) {
  const [text, setText] = useState("");
  const controller = useDictationController({ text, setText, appendSegment });

  useEffect(() => {
    onReady(controller);
  }, [controller, onReady]);

  return (
    <div>
      <div data-testid="text">{text}</div>
      <div data-testid="recording">{controller.dictationRecording ? "on" : "off"}</div>
    </div>
  );
}

describe("useDictationController integration", () => {
  const getSettingsMock = vi.mocked(getSettings);
  const loadSttApiMock = vi.mocked(loadSttApi);

  it("streams Tauri dictation interim/final text and stops cleanly", async () => {
    const settings: Settings = {
      dictation: {
        enabled: true,
        provider: "tauri_stt",
        livekit: {
          base_url: "",
          api_key: "",
          api_secret: "",
          model: "auto",
          language: "en",
        },
      },
    };
    getSettingsMock.mockResolvedValue(settings);

    let resultHandler: ((result: SttResult) => void) | null = null;

    const sttApi: SttApi = {
      isAvailable: vi.fn(async () => ({ available: true })),
      getSupportedLanguages: vi.fn(async () => ({ languages: [] })),
      checkPermission: vi.fn(async () => ({
        microphone: "granted",
        speechRecognition: "granted",
      } as const)),
      requestPermission: vi.fn(async () => ({
        microphone: "granted",
        speechRecognition: "granted",
      } as const)),
      startListening: vi.fn(async () => {}),
      stopListening: vi.fn(async () => {}),
      onResult: vi.fn(async (handler) => {
        resultHandler = handler;
        return () => {};
      }),
      onStateChange: vi.fn(async () => () => {}),
      onError: vi.fn(async () => () => {}),
    };
    loadSttApiMock.mockResolvedValue(sttApi);

    let controllerRef: ReturnType<typeof useDictationController> | null = null;
    render(<Harness onReady={(controller) => (controllerRef = controller)} />);

    await waitFor(() => expect(controllerRef).not.toBeNull());

    await act(async () => {
      await controllerRef!.startDictation();
    });

    await waitFor(() => expect(screen.getByTestId("recording").textContent).toBe("on"));

    await act(async () => {
      resultHandler?.({ transcript: "hello", isFinal: false });
    });
    expect(screen.getByTestId("text").textContent).toBe("hello");

    await act(async () => {
      resultHandler?.({ transcript: "hello world", isFinal: true });
    });
    expect(screen.getByTestId("text").textContent).toBe("hello world");

    await act(async () => {
      await controllerRef!.stopDictation();
    });
    await waitFor(() => expect(screen.getByTestId("recording").textContent).toBe("off"));
    expect(sttApi.stopListening).toHaveBeenCalled();
  });
});
