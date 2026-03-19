import type { SessionHeadSnapshot } from "@ctx/types";

import { idToString } from "../api/client";
import { sanitizeSessionHeadSnapshot } from "./sessionHeadState";
import { shouldReplaceSessionHead } from "./workspaceActiveSnapshot/summaryHelpers";

export type SessionHeadBootstrapRecord = Record<string, SessionHeadSnapshot>;

export class SessionHeadBootstrapCache {
  private readonly entries = new Map<string, SessionHeadSnapshot>();
  private readonly persistedPrefetchSessionIds = new Set<string>();

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

  beginPersistedPrefetch(sessionId: string | null | undefined): boolean {
    const id = String(sessionId ?? "").trim();
    if (!id || this.persistedPrefetchSessionIds.has(id)) return false;
    this.persistedPrefetchSessionIds.add(id);
    return true;
  }

  clear(): void {
    this.entries.clear();
    this.persistedPrefetchSessionIds.clear();
  }
}
