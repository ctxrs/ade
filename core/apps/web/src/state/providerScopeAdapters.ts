import { getDaemonConnection } from "../api/daemonConnection";
import {
  createHostOwnerScope,
  createWorkspaceOwnerScope,
  serializeOwnerScope,
  type DaemonTargetScope,
  type HostOwnerScope,
  type OwnerScope,
  type WorkspaceOwnerScope,
} from "./scopeIdentity";

const requireCurrentDaemonTargetScope = (): DaemonTargetScope => {
  const targetScope = getDaemonConnection().targetScope;
  if (!targetScope) {
    throw new Error("Daemon target scope is not available.");
  }
  return targetScope;
};

export const getProviderHostOwnerScope = (): HostOwnerScope =>
  createHostOwnerScope(requireCurrentDaemonTargetScope());

export const getProviderWorkspaceOwnerScope = (
  workspaceId: string,
): WorkspaceOwnerScope =>
  createWorkspaceOwnerScope(requireCurrentDaemonTargetScope(), workspaceId);

export const getProviderOwnerScope = (workspaceId: string | null): OwnerScope =>
  workspaceId ? getProviderWorkspaceOwnerScope(workspaceId) : getProviderHostOwnerScope();

export const getProviderOwnerScopeKey = (workspaceId: string | null): string =>
  serializeOwnerScope(getProviderOwnerScope(workspaceId));
