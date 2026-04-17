import type { SessionHeadSnapshot } from "@ctx/types";

import { idToString } from "../api/client";
import { sanitizeSessionHeadSnapshot } from "./sessionHeadState";
import { shouldReplaceSessionHead } from "./workspaceActiveSnapshot/summaryHelpers";

export type SessionHeadBootstrapRecord = Record<string, SessionHeadSnapshot>;

export class SessionHeadBootstrapCache {
  private readonly entries = new Map<string, SessionHeadSnapshot>();
  private readonly persistedPrefetchSessionIds = new Set<string>();
  private readonly authoritativePrefetchInFlight = new Map<string, string>();
  private readonly authoritativePrefetchVersions = new Map<string, string>();

  upsert(head: SessionHeadSnapshot | null | undefined): boolean {
    const sessionId = idToString(head?.session?.id);
    if (!sessionId) return false;
    if (!head) return false;
    const sanitized = sanitizeSessionHeadSnapshot(head);
    const previous = this.entries.get(sessionId);
    if (!shouldReplaceSessionHead(previous, sanitized)) return false;
    this.entries.set(sessionId, sanitized);
    return true;
  }

  upsertAll(heads: SessionHeadBootstrapRecord): boolean {
    let changed = false;
    for (const head of Object.values(heads)) {
      if (this.upsert(head)) {
        changed = true;
      }
    }
    return changed;
  }

  snapshot(): SessionHeadBootstrapRecord {
    return Object.fromEntries(this.entries) as SessionHeadBootstrapRecord;
  }

  get(sessionId: string | null | undefined): SessionHeadSnapshot | undefined {
    const id = idToString(sessionId ?? "");
    if (!id) return undefined;
    return this.entries.get(id);
  }

  beginPersistedPrefetch(sessionId: string | null | undefined): boolean {
    const id = String(sessionId ?? "").trim();
    if (!id || this.persistedPrefetchSessionIds.has(id)) return false;
    this.persistedPrefetchSessionIds.add(id);
    return true;
  }

  beginAuthoritativePrefetch(
    sessionId: string | null | undefined,
    versionKey: string | null | undefined,
  ): boolean {
    const id = String(sessionId ?? "").trim();
    const normalizedVersionKey = String(versionKey ?? "").trim();
    if (!id || !normalizedVersionKey) return false;
    if (this.authoritativePrefetchInFlight.get(id) === normalizedVersionKey) return false;
    if (this.authoritativePrefetchVersions.get(id) === normalizedVersionKey) return false;
    this.authoritativePrefetchInFlight.set(id, normalizedVersionKey);
    return true;
  }

  finishAuthoritativePrefetch(
    sessionId: string | null | undefined,
    versionKey: string | null | undefined,
    success: boolean,
  ): void {
    const id = String(sessionId ?? "").trim();
    const normalizedVersionKey = String(versionKey ?? "").trim();
    if (!id || !normalizedVersionKey) return;
    if (this.authoritativePrefetchInFlight.get(id) !== normalizedVersionKey) return;
    this.authoritativePrefetchInFlight.delete(id);
    if (success) {
      this.authoritativePrefetchVersions.set(id, normalizedVersionKey);
    }
  }

  clear(): void {
    this.entries.clear();
    this.persistedPrefetchSessionIds.clear();
    this.authoritativePrefetchInFlight.clear();
    this.authoritativePrefetchVersions.clear();
  }
}
