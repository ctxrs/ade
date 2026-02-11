import { apiAny } from "./clientBase";

export type RepoCloneRequest = {
  repo_url: string;
  dest_parent: string;
  branch?: string | null;
  dest_name?: string | null;
};

export type RepoCloneResponse = {
  path: string;
};

export const repoClone = (req: RepoCloneRequest) =>
  apiAny<RepoCloneResponse>("/api/repo/clone", {
    method: "POST",
    body: JSON.stringify(req),
  });

export type RepoInitRequest = {
  path: string;
  allow_existing?: boolean;
  allow_non_empty?: boolean;
};

export type RepoInitResponse = {
  path: string;
};

export const repoInit = (req: RepoInitRequest) =>
  apiAny<RepoInitResponse>("/api/repo/init", {
    method: "POST",
    body: JSON.stringify(req),
  });

export type RepoStatusRequest = {
  path: string;
};

export type RepoStatusResponse = {
  canonical_path: string;
  is_repo: boolean;
  error?: string | null;
};

export const repoStatus = (req: RepoStatusRequest) =>
  apiAny<RepoStatusResponse>("/api/repo/status", {
    method: "POST",
    body: JSON.stringify(req),
  });

export type RepoStagingPathResponse = {
  path: string;
};

export const repoStagingPath = () =>
  apiAny<RepoStagingPathResponse>("/api/repo/staging_path", {
    method: "GET",
  });
