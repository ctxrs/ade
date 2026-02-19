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

function baseOptions(providerId: string): ProviderOptions {
  return {
    provider_id: providerId,
    workspace_id: "ws-test",
    supports_load: false,
    auth_required: false,
    probed_at: new Date().toISOString(),
  };
}

describe("WorkbenchComposer textarea sizing", () => {
  const originalScrollHeight = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "scrollHeight");

  beforeEach(() => {
    mockRaf();
    Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", {
      configurable: true,
      get() {
        const lines = String((this as HTMLTextAreaElement).value ?? "").split("\n").length;
        const contentHeight = Math.max(1, lines) * 20;
        const styleHeight = Number.parseFloat((this as HTMLTextAreaElement).style.height || "0");
        return Math.max(contentHeight, Number.isFinite(styleHeight) ? styleHeight : 0);
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
      const [useMultipleAgents, setUseMultipleAgents] = useState(false);
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
          useMultipleAgents={useMultipleAgents}
          setUseMultipleAgents={setUseMultipleAgents}
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
    const expandedHeight = Number.parseFloat(textarea.style.height || "0");
    expect(expandedHeight).toBeGreaterThan(88);
    expect(expandedHeight).toBeLessThanOrEqual(380);

    const manyLines = Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join("\n");
    await act(async () => {
      fireEvent.change(textarea, { target: { value: manyLines } });
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(textarea.style.height).toBe("380px");
  });

  it("avoids zeroing the textarea height during resize", async () => {
    const ActiveHarness = () => {
      const [value, setValue] = useState("");
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
          recording={false}
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

    const heights: string[] = [];
    const style = textarea.style;
    const originalHeightDescriptor = Object.getOwnPropertyDescriptor(style, "height");
    let storedHeight = style.height;
    Object.defineProperty(style, "height", {
      configurable: true,
      get() {
        return storedHeight;
      },
      set(value) {
        const next = String(value);
        heights.push(next);
        storedHeight = next;
      },
    });

    await act(async () => {
      fireEvent.change(textarea, { target: { value: "line 1\nline 2\nline 3" } });
      await new Promise((r) => setTimeout(r, 0));
    });

    if (originalHeightDescriptor) {
      Object.defineProperty(style, "height", originalHeightDescriptor);
    }

    expect(heights).not.toContain("0px");
  });

  it("does not collapse the textarea when typing multi-line input", async () => {
    const ActiveHarness = () => {
      const [value, setValue] = useState("line 1\nline 2\nline 3");
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
          recording={false}
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

    const initialHeight = Number.parseFloat(textarea.style.height || "0");
    expect(initialHeight).toBeGreaterThan(0);

    const heightWrites: string[] = [];
    const style = textarea.style;
    const originalHeightDescriptor = Object.getOwnPropertyDescriptor(style, "height");
    let storedHeight = style.height;
    Object.defineProperty(style, "height", {
      configurable: true,
      get() {
        return storedHeight;
      },
      set(value) {
        const next = String(value);
        heightWrites.push(next);
        storedHeight = next;
      },
    });

    await act(async () => {
      fireEvent.change(textarea, { target: { value: `${textarea.value}x` } });
      await new Promise((r) => setTimeout(r, 0));
    });

    if (originalHeightDescriptor) {
      Object.defineProperty(style, "height", originalHeightDescriptor);
    }

    expect(heightWrites).not.toContain("auto");
    const collapsed = heightWrites.some((next) => {
      const numeric = Number.parseFloat(next);
      return Number.isFinite(numeric) && numeric < initialHeight - 0.5;
    });
    expect(collapsed).toBe(false);
  });

  it("resets to the minimum height after clearing content", async () => {
    const NewTaskHarness = () => {
      const [value, setValue] = useState("");
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");
      const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
        { key: "t1", label: "Track 1", providerId: "codex", modelId: "o3" },
      ]);
      const [useMultipleAgents, setUseMultipleAgents] = useState(false);
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
          useMultipleAgents={useMultipleAgents}
          setUseMultipleAgents={setUseMultipleAgents}
        />
      );
    };

    render(<NewTaskHarness />);
    const textarea = screen.getByPlaceholderText("@ for context, / for commands") as HTMLTextAreaElement;

    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });

    await act(async () => {
      fireEvent.change(textarea, { target: { value: "line 1\nline 2\nline 3\nline 4\nline 5" } });
      await new Promise((r) => setTimeout(r, 0));
    });
    const expandedHeight = Number.parseFloat(textarea.style.height || "0");
    expect(expandedHeight).toBeGreaterThan(88);

    await act(async () => {
      fireEvent.change(textarea, { target: { value: "" } });
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(textarea.style.height).toBe("88px");

    await act(async () => {
      fireEvent.change(textarea, { target: { value: "a" } });
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(textarea.style.height).toBe("88px");
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

  it("shows catalog harnesses even when provider status is missing", async () => {
    const NewTaskHarness = () => {
      const [value, setValue] = useState("");
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");
      const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
        { key: "t1", label: "Track 1", providerId: "codex", modelId: "o3" },
      ]);
      const [useMultipleAgents, setUseMultipleAgents] = useState(false);
      const harnessCatalog: HarnessCatalogEntry[] = [
        { id: "codex", label: "Codex", logoSrc: "" },
        { id: "claude-crp", label: "Claude Code", logoSrc: "" },
      ];
      const providersById: Record<string, ProviderStatus> = {
        codex: { provider_id: "codex", installed: true, health: "ok", diagnostics: [] },
      };

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
          providerOptions={{}}
          ensureProviderOptions={async () => undefined}
          draftTracks={draftTracks}
          setDraftTracks={setDraftTracks}
          defaultProviderId="codex"
          useMultipleAgents={useMultipleAgents}
          setUseMultipleAgents={setUseMultipleAgents}
        />
      );
    };

    render(<NewTaskHarness />);
    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(screen.getByRole("button", { name: /Claude Code/ })).toBeDisabled();
  });

  it("treats codex subscription auth mode as configured auth in harness list", async () => {
    const NewTaskHarness = () => {
      const [value, setValue] = useState("");
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");
      const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
        { key: "t1", label: "Track 1", providerId: "codex", modelId: "o3" },
      ]);
      const [useMultipleAgents, setUseMultipleAgents] = useState(false);
      const harnessCatalog: HarnessCatalogEntry[] = [{ id: "codex", label: "Codex", logoSrc: "" }];
      const providersById: Record<string, ProviderStatus> = {
        codex: { provider_id: "codex", installed: true, health: "ok", diagnostics: [] },
      };
      const providerOptions: Record<string, ProviderOptions | undefined> = {
        codex: {
          provider_id: "codex",
          workspace_id: "ws-test",
          supports_load: false,
          auth_required: false,
          has_active_auth: false,
          auth_mode: "subscription",
          probed_at: new Date().toISOString(),
        },
      };

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
          ensureProviderOptions={async () => providerOptions.codex}
          draftTracks={draftTracks}
          setDraftTracks={setDraftTracks}
          defaultProviderId="codex"
          useMultipleAgents={useMultipleAgents}
          setUseMultipleAgents={setUseMultipleAgents}
        />
      );
    };

    render(<NewTaskHarness />);
    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(screen.getByTitle("Authentication configured")).toBeInTheDocument();
  });

  it("shows an explicit unselected harness state", async () => {
    const NewTaskHarness = () => {
      const [value, setValue] = useState("");
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");
      const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([]);
      const [useMultipleAgents, setUseMultipleAgents] = useState(false);
      const harnessCatalog: HarnessCatalogEntry[] = [{ id: "codex", label: "Codex", logoSrc: "" }];
      const providersById: Record<string, ProviderStatus> = {
        codex: { provider_id: "codex", installed: true, health: "ok", diagnostics: [] },
      };

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
          providerOptions={{}}
          ensureProviderOptions={async () => undefined}
          draftTracks={draftTracks}
          setDraftTracks={setDraftTracks}
          defaultProviderId="codex"
          useMultipleAgents={useMultipleAgents}
          setUseMultipleAgents={setUseMultipleAgents}
        />
      );
    };

    render(<NewTaskHarness />);
    expect(screen.getByRole("button", { name: "Select harness" })).toBeInTheDocument();
  });

  it("requests auth modal when selecting an unauthenticated harness", async () => {
    const onRequestHarnessAuth = vi.fn();

    const NewTaskHarness = () => {
      const [value, setValue] = useState("");
      const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
      const [modeId, setModeId] = useState<WorkbenchModeId>("default");
      const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
        { key: "t1", label: "Track 1", providerId: "codex", modelId: "" },
      ]);
      const [useMultipleAgents, setUseMultipleAgents] = useState(false);
      const harnessCatalog: HarnessCatalogEntry[] = [
        { id: "codex", label: "Codex", logoSrc: "" },
        { id: "cursor", label: "Cursor", logoSrc: "" },
      ];
      const providersById: Record<string, ProviderStatus> = {
        codex: { provider_id: "codex", installed: true, health: "ok", diagnostics: [] },
        cursor: { provider_id: "cursor", installed: true, health: "ok", diagnostics: [] },
      };
      const providerOptions: Record<string, ProviderOptions | undefined> = {
        codex: { ...baseOptions("codex"), has_active_auth: true },
        cursor: { ...baseOptions("cursor"), has_active_auth: false },
      };

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
          ensureProviderOptions={async (providerId: string) => providerOptions[providerId]}
          onRequestHarnessAuth={onRequestHarnessAuth}
          draftTracks={draftTracks}
          setDraftTracks={setDraftTracks}
          defaultProviderId="codex"
          useMultipleAgents={useMultipleAgents}
          setUseMultipleAgents={setUseMultipleAgents}
        />
      );
    };

    render(<NewTaskHarness />);
    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    fireEvent.click(screen.getByRole("button", { name: /Cursor/ }));
    expect(onRequestHarnessAuth).toHaveBeenCalledWith("cursor");
    expect(screen.getByRole("button", { name: "Codex" })).toBeInTheDocument();
  });

});
