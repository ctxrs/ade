type SourceKind = "clone" | "import" | "new";

export type CloneDestination = {
  dest_parent: string;
  dest_name?: string | null;
};

const isSourceKind = (value: string | null | undefined): value is SourceKind =>
  value === "clone" || value === "import" || value === "new";

const normalizeWorkspaceNames = (names: Iterable<string>): Set<string> => {
  const normalized = new Set<string>();
  for (const name of names) {
    const trimmed = String(name ?? "").trim();
    if (trimmed) normalized.add(trimmed);
  }
  return normalized;
};

export const parseCloneDestPath = (raw: string): CloneDestination | null => {
  const input = String(raw || "").trim();
  if (!input) return null;
  const hasTrailingSlash = /\/+$/.test(input);
  const normalized = input.replace(/\/+$/, "");
  if (!normalized) return null;
  // Basic POSIX parsing (desktop app is our primary target here).
  if (hasTrailingSlash) {
    return { dest_parent: normalized, dest_name: null };
  }
  const idx = normalized.lastIndexOf("/");
  if (idx < 0) return null;
  const dest_parent = normalized.slice(0, idx) || "/";
  const dest_name = normalized.slice(idx + 1).trim();
  if (!dest_name) return null;
  return { dest_parent, dest_name };
};

export const deriveRepoNameFromUrl = (url: string): string | null => {
  const trimmed = url.trim().replace(/\/+$/, "");
  if (!trimmed) return null;
  const normalized = trimmed.replace(":", "/");
  const parts = normalized.split("/");
  const last = parts[parts.length - 1]?.trim();
  if (!last) return null;
  const name = last.replace(/\.git$/i, "").trim();
  return name || null;
};

export type SourceStepValidationInput = {
  source?: string | null;
  sourcePath: string;
  repoUrl: string;
  useDiskIsolatedStaging: boolean;
};

export type SourceStepValidation = {
  sourceSelected: boolean;
  needsSourcePath: boolean;
  hasSourcePath: boolean;
  needsRepoUrl: boolean;
  hasRepoUrl: boolean;
  hasValidCloneDestination: boolean;
  isComplete: boolean;
};

export const getSourceStepValidation = ({
  source,
  sourcePath,
  repoUrl,
  useDiskIsolatedStaging,
}: SourceStepValidationInput): SourceStepValidation => {
  const selectedSource = isSourceKind(source) ? source : null;
  const sourceSelected = selectedSource !== null;
  const needsRepoUrl = selectedSource === "clone";
  const hasRepoUrl = !needsRepoUrl || repoUrl.trim() !== "";
  const needsSourcePath = sourceSelected && !useDiskIsolatedStaging;
  const hasSourcePath = !needsSourcePath || sourcePath.trim() !== "";
  const hasValidCloneDestination = selectedSource !== "clone"
    || !needsSourcePath
    || Boolean(parseCloneDestPath(sourcePath));
  const isComplete = sourceSelected && hasRepoUrl && hasSourcePath && hasValidCloneDestination;

  return {
    sourceSelected,
    needsSourcePath,
    hasSourcePath,
    needsRepoUrl,
    hasRepoUrl,
    hasValidCloneDestination,
    isComplete,
  };
};

export const dedupeGeneratedWorkspaceName = (
  baseName: string,
  existingWorkspaceNames: Iterable<string>,
): string => {
  const existing = normalizeWorkspaceNames(existingWorkspaceNames);
  const normalizedBase = baseName.trim() || "workspace";
  if (!existing.has(normalizedBase)) return normalizedBase;
  let suffix = 2;
  while (existing.has(`${normalizedBase} ${suffix}`)) {
    suffix += 1;
  }
  return `${normalizedBase} ${suffix}`;
};

export type ResolveWorkspaceNameInput = {
  source?: string | null;
  workspaceName: string;
  repoUrl: string;
  destPath?: string | null;
  useDiskIsolatedStaging: boolean;
  existingWorkspaceNames: Iterable<string>;
};

export const resolveWorkspaceName = ({
  source,
  workspaceName,
  repoUrl,
  destPath,
  useDiskIsolatedStaging,
  existingWorkspaceNames,
}: ResolveWorkspaceNameInput): string | undefined => {
  const userProvidedName = workspaceName.trim();
  if (userProvidedName) return userProvidedName;

  const selectedSource = isSourceKind(source) ? source : null;
  if (selectedSource === "clone") {
    const base = deriveRepoNameFromUrl(repoUrl)
      || parseCloneDestPath(destPath ?? "")?.dest_name
      || null;
    return base ? dedupeGeneratedWorkspaceName(base, existingWorkspaceNames) : undefined;
  }

  if (selectedSource === "new") {
    const base = useDiskIsolatedStaging
      ? "new-workspace"
      : parseCloneDestPath(destPath ?? "")?.dest_name || null;
    return base ? dedupeGeneratedWorkspaceName(base, existingWorkspaceNames) : undefined;
  }

  return undefined;
};
