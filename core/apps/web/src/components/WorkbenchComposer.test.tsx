import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WorkbenchComposer } from "./WorkbenchComposer";
import type { MessageAttachment, ProviderOptions, ProviderStatus } from "../api/client";
import type { DraftTrack, WorkbenchModeId } from "./WorkbenchComposer";
import type { HarnessCatalogEntry } from "../utils/harnessCatalog";

function mockRaf() {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb: FrameRequestCallback) => {
    return window.setTimeout(() => cb(performance.now()), 0) as unknown as number;
  });
}

describe("WorkbenchComposer textarea sizing", () => {
  const originalScrollHeight = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "scrollHeight");

  beforeEach(() => {
    mockRaf();
    Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", {
      configurable: true,
      get() {
        const lines = String((this as HTMLTextAreaElement).value ?? "").split("\n").length;
        return Math.max(1, lines) * 20;
      },
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    if (originalScrollHeight) {
      Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", originalScrollHeight);
    } else {
      Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", {
        configurable: true,
        get() {
          return 0;
        },
      });
    }
  });

  it("auto-resizes the new task composer up to 380px", async () => {
    const NewTaskHarness = () => {
      const [value, setValue] = useState("");
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");
      const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
        { key: "t1", label: "Track 1", providerId: "codex", modelId: "o3" },
      ]);
      const harnessCatalog: HarnessCatalogEntry[] = [{ id: "codex", label: "Codex", logoSrc: "" }];
      const providersById: Record<string, ProviderStatus> = {
        codex: { provider_id: "codex", installed: true, health: "ok", diagnostics: [] },
      };
      const providerOptions: Record<string, ProviderOptions | undefined> = {};

      return (
        <WorkbenchComposer
          variant="newSession"
          value={value}
          setValue={setValue}
          placeholder="@ for context, / for commands"
          inputDisabled={false}
          sessionIdForAutocomplete={null}
          workspaceIdForAutocomplete={null}
          slashCommands={[]}
          attachments={attachments}
          setAttachments={setAttachments}
          onSend={vi.fn()}
          sendDisabled={false}
          sendDisabledReason={null}
          onInterrupt={null}
          modeId={modeId}
          setModeId={setModeId}
          harnessCatalog={harnessCatalog}
          providersById={providersById}
          providerInstallsById={{}}
          onInstallProvider={vi.fn()}
          onInstallAllProviders={vi.fn()}
          providerOptions={providerOptions}
          ensureProviderOptions={async () => undefined}
          draftTracks={draftTracks}
          setDraftTracks={setDraftTracks}
          defaultProviderId="codex"
        />
      );
    };

    render(<NewTaskHarness />);
    const textarea = screen.getByPlaceholderText("@ for context, / for commands") as HTMLTextAreaElement;

    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(textarea.style.height).toBe("88px");

    await act(async () => {
      fireEvent.change(textarea, { target: { value: ["a", "b", "c", "d", "e"].join("\n") } });
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(textarea.style.height).toBe("100px");

    const manyLines = Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join("\n");
    await act(async () => {
      fireEvent.change(textarea, { target: { value: manyLines } });
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(textarea.style.height).toBe("380px");
  });

  it("keeps the textarea scrolled to bottom while recording", async () => {
    const ActiveHarness = () => {
      const [value, setValue] = useState(Array.from({ length: 25 }, (_, i) => `line ${i + 1}`).join("\n"));
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");

      return (
        <WorkbenchComposer
          variant="activeSession"
          value={value}
          setValue={setValue}
          placeholder="Ask follow-ups"
          inputDisabled={false}
          sessionIdForAutocomplete={null}
          workspaceIdForAutocomplete={null}
          slashCommands={[]}
          attachments={attachments}
          setAttachments={setAttachments}
          onSend={vi.fn()}
          sendDisabled={false}
          sendDisabledReason={null}
          onInterrupt={null}
          modeId={modeId}
          setModeId={setModeId}
          recording={true}
          harnessLabel="Codex"
          availableModels={[{ id: "o3", name: "o3" }]}
          currentModelId="o3"
          onSetModelId={vi.fn()}
        />
      );
    };

    render(<ActiveHarness />);
    const textarea = screen.getByPlaceholderText("Ask follow-ups") as HTMLTextAreaElement;

    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });

    textarea.scrollTop = 0;
    expect(textarea.scrollTop).toBe(0);

    await act(async () => {
      fireEvent.change(textarea, { target: { value: `${textarea.value}\nline 26` } });
      await new Promise((r) => setTimeout(r, 0));
    });

    expect(textarea.scrollTop).toBe(textarea.scrollHeight);
  });
});
