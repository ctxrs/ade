import { describe, expect, it } from "vitest";
import { getSourceStepValidation, resolveWorkspaceName } from "./WorkspaceSetupPage.logic";

describe("getSourceStepValidation", () => {
  it("allows disk-isolated clone with repo URL and blank source path", () => {
    const state = getSourceStepValidation({
      source: "clone",
      sourcePath: "",
      repoUrl: "https://github.com/acme/react.git",
      useDiskIsolatedStaging: true,
    });

    expect(state.needsSourcePath).toBe(false);
    expect(state.hasRepoUrl).toBe(true);
    expect(state.isComplete).toBe(true);
  });

  it("still requires repo URL for disk-isolated clone", () => {
    const state = getSourceStepValidation({
      source: "clone",
      sourcePath: "",
      repoUrl: "",
      useDiskIsolatedStaging: true,
    });

    expect(state.needsSourcePath).toBe(false);
    expect(state.hasRepoUrl).toBe(false);
    expect(state.isComplete).toBe(false);
  });

  it("allows disk-isolated new with blank source path", () => {
    const state = getSourceStepValidation({
      source: "new",
      sourcePath: "",
      repoUrl: "",
      useDiskIsolatedStaging: true,
    });

    expect(state.needsSourcePath).toBe(false);
    expect(state.isComplete).toBe(true);
  });

  it("requires source path for non-disk clone and validates clone destination shape", () => {
    const missingPath = getSourceStepValidation({
      source: "clone",
      sourcePath: "",
      repoUrl: "https://github.com/acme/react.git",
      useDiskIsolatedStaging: false,
    });
    expect(missingPath.isComplete).toBe(false);

    const invalidPath = getSourceStepValidation({
      source: "clone",
      sourcePath: "relative",
      repoUrl: "https://github.com/acme/react.git",
      useDiskIsolatedStaging: false,
    });
    expect(invalidPath.hasValidCloneDestination).toBe(false);
    expect(invalidPath.isComplete).toBe(false);

    const validPath = getSourceStepValidation({
      source: "clone",
      sourcePath: "/Users/example-user/projects/",
      repoUrl: "https://github.com/acme/react.git",
      useDiskIsolatedStaging: false,
    });
    expect(validPath.isComplete).toBe(true);
  });

  it("requires source path for non-disk new", () => {
    const missingPath = getSourceStepValidation({
      source: "new",
      sourcePath: "",
      repoUrl: "",
      useDiskIsolatedStaging: false,
    });
    expect(missingPath.isComplete).toBe(false);

    const withPath = getSourceStepValidation({
      source: "new",
      sourcePath: "/Users/example-user/new-repo",
      repoUrl: "",
      useDiskIsolatedStaging: false,
    });
    expect(withPath.isComplete).toBe(true);
  });
});

describe("resolveWorkspaceName", () => {
  it("uses friendly fallback for disk-isolated new", () => {
    const name = resolveWorkspaceName({
      source: "new",
      workspaceName: "",
      repoUrl: "",
      destPath: "/tmp/workspaces/staging/9a8f6f9a-7f7d-4c99-a486-1a5f6c0eff3f",
      useDiskIsolatedStaging: true,
      existingWorkspaceNames: [],
    });

    expect(name).toBe("new-workspace");
  });

  it("dedupes generated disk-isolated new names", () => {
    const name = resolveWorkspaceName({
      source: "new",
      workspaceName: "",
      repoUrl: "",
      destPath: "/tmp/workspaces/staging/9a8f6f9a-7f7d-4c99-a486-1a5f6c0eff3f",
      useDiskIsolatedStaging: true,
      existingWorkspaceNames: ["new-workspace", "new-workspace 2"],
    });

    expect(name).toBe("new-workspace 3");
  });

  it("preserves user-provided names without suffixing", () => {
    const name = resolveWorkspaceName({
      source: "new",
      workspaceName: "my workspace",
      repoUrl: "",
      destPath: "/tmp/workspaces/staging/9a8f6f9a-7f7d-4c99-a486-1a5f6c0eff3f",
      useDiskIsolatedStaging: true,
      existingWorkspaceNames: ["my workspace"],
    });

    expect(name).toBe("my workspace");
  });

  it("derives clone name from repo URL and dedupes generated collisions", () => {
    const name = resolveWorkspaceName({
      source: "clone",
      workspaceName: "",
      repoUrl: "https://github.com/acme/react.git",
      destPath: null,
      useDiskIsolatedStaging: true,
      existingWorkspaceNames: ["react"],
    });

    expect(name).toBe("react 2");
  });
});
