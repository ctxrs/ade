import { beforeEach, describe, expect, it, vi } from "vitest";

const isDesktopAppMock = vi.hoisted(() => vi.fn(() => true));
const desktopReadBinaryFileMock = vi.hoisted(() =>
  vi.fn(async (args: { path: string }) => ({
    path: args.path,
    bytes: [137, 80, 78, 71],
  })),
);

vi.mock("./desktop", () => ({
  isDesktopApp: isDesktopAppMock,
  desktopReadBinaryFile: desktopReadBinaryFileMock,
}));

describe("droppedImageAttachments", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.restoreAllMocks();
    delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    isDesktopAppMock.mockReturnValue(true);
    desktopReadBinaryFileMock.mockImplementation(async (args: { path: string }) => ({
      path: args.path,
      bytes: [137, 80, 78, 71],
    }));
  });

  it("creates inline attachments from dropped desktop paths", async () => {
    const mod = await import("./droppedImageAttachments");
    const attachments = await mod.imageAttachmentsFromPaths(["/tmp/example.png"]);
    const attachment = attachments[0];

    expect(desktopReadBinaryFileMock).toHaveBeenCalledWith({ path: "/tmp/example.png" });
    expect(attachments).toHaveLength(1);
    expect(attachment?.kind).toBe("image");
    if (!attachment || attachment.kind !== "image") {
      throw new Error("Expected an inline image attachment");
    }
    expect(attachment.mime_type).toBe("image/png");
    expect(attachment.name).toBe("example.png");
    expect(attachment.data_base64).toBeTruthy();
  });

  it("normalizes file:// drops through the desktop path flow", async () => {
    desktopReadBinaryFileMock.mockResolvedValue({
      path: "/Users/example-user/Pictures/cat.jpg",
      bytes: [255, 216, 255, 224],
    });

    const mod = await import("./droppedImageAttachments");
    const transfer = {
      files: [],
      items: [],
      getData(type: string) {
        if (type === "text/uri-list") return "file:///Users/example-user/Pictures/cat.jpg";
        return "";
      },
    } as unknown as DataTransfer;

    const attachments = await mod.imageAttachmentsFromTransfer(transfer);

    expect(desktopReadBinaryFileMock).toHaveBeenCalledWith({ path: "/Users/example-user/Pictures/cat.jpg" });
    expect(attachments).toHaveLength(1);
    expect(attachments[0]?.name).toBe("cat.jpg");
    expect(attachments[0]?.mime_type).toBe("image/jpeg");
  });

  it("rejects non-image desktop paths", async () => {
    desktopReadBinaryFileMock.mockResolvedValue({
      path: "/tmp/notes.txt",
      bytes: [1, 2, 3],
    });

    const mod = await import("./droppedImageAttachments");
    const attachments = await mod.imageAttachmentsFromPaths(["/tmp/notes.txt"]);

    expect(attachments).toEqual([]);
  });

  it("reads desktop image bytes through the desktop command path", async () => {
    const globalConvertFileSrc = vi.fn((path: string) => `asset://global/${encodeURIComponent(path)}`);
    Object.assign(globalThis as typeof globalThis & { __TAURI__?: unknown }, {
      __TAURI__: {
        core: {
          convertFileSrc: globalConvertFileSrc,
        },
      },
    });

    const mod = await import("./droppedImageAttachments");
    const attachments = await mod.imageAttachmentsFromPaths(["/tmp/global.png"]);

    expect(globalConvertFileSrc).not.toHaveBeenCalled();
    expect(desktopReadBinaryFileMock).toHaveBeenCalledWith({ path: "/tmp/global.png" });
    expect(attachments).toHaveLength(1);
    expect(attachments[0]?.name).toBe("global.png");
  });
});
