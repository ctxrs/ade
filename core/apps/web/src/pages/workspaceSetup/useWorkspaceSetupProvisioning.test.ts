import { describe, expect, it } from "vitest";
import {
  buildWorkspaceSetupAuthImportScanKey,
  buildWorkspaceSetupHarnessInstallScanKey,
} from "./useWorkspaceSetupProvisioning";

describe("useWorkspaceSetupProvisioning scan keys", () => {
  it("includes remote port and data dir in auth-import scan keys", () => {
    const base = buildWorkspaceSetupAuthImportScanKey("remote", {
      user: "user",
      host: "builder.internal",
      port: 4399,
      dataDir: "/var/lib/ctx-a",
    });
    const portChanged = buildWorkspaceSetupAuthImportScanKey("remote", {
      user: "user",
      host: "builder.internal",
      port: 4400,
      dataDir: "/var/lib/ctx-a",
    });
    const dataDirChanged = buildWorkspaceSetupAuthImportScanKey("remote", {
      user: "user",
      host: "builder.internal",
      port: 4399,
      dataDir: "/var/lib/ctx-b",
    });

    expect(portChanged).not.toBe(base);
    expect(dataDirChanged).not.toBe(base);
  });

  it("includes remote port and data dir in harness-install scan keys", () => {
    const base = buildWorkspaceSetupHarnessInstallScanKey("remote", "host", {
      user: "user",
      host: "builder.internal",
      port: 4399,
      dataDir: "/var/lib/ctx-a",
    });
    const portChanged = buildWorkspaceSetupHarnessInstallScanKey("remote", "host", {
      user: "user",
      host: "builder.internal",
      port: 4400,
      dataDir: "/var/lib/ctx-a",
    });
    const dataDirChanged = buildWorkspaceSetupHarnessInstallScanKey("remote", "host", {
      user: "user",
      host: "builder.internal",
      port: 4399,
      dataDir: "/var/lib/ctx-b",
    });

    expect(portChanged).not.toBe(base);
    expect(dataDirChanged).not.toBe(base);
  });
});
