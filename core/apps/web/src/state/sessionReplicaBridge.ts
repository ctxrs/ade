import {
  getSessionHead,
  getSessionSnapshot,
  getSessionState,
  getDaemonClientConfig,
  subscribeDaemonConfig,
} from "../api/client";
import { isDesktopApp } from "../utils/desktop";
import { SessionReplicaCore } from "./sessionReplicaCore";
import type {
  SessionReplicaCommand,
  SessionReplicaConfig,
  SessionReplicaFreshnessEvent,
  SessionReplicaPatch,
  SessionReplicaWorkerMessage,
} from "./sessionReplicaProtocol";
import {
  noteFinalDeltaReceived,
  noteGapRecoveryFinished,
  noteGapRecoveryStarted,
  noteGapRepairMismatch,
  noteProjectionOrSeqRegression,
} from "./foregroundFreshnessTelemetry";

const shouldUseWorker = (): boolean => {
  if (typeof Worker === "undefined") return false;
  if (isDesktopApp()) return false;
  const metaEnv =
    typeof import.meta !== "undefined" ? (import.meta as { env?: { MODE?: string } }).env : undefined;
  if (metaEnv?.MODE === "test") return false;
  return true;
};

export class SessionReplicaBridge {
  private worker: Worker | null = null;
  private core: SessionReplicaCore | null = null;
  private configUnsubscribe: (() => void) | null = null;

  constructor(
    private onPatches: (patches: SessionReplicaPatch[]) => void,
    private config: SessionReplicaConfig,
  ) {
    if (shouldUseWorker()) {
      this.worker = new Worker(new URL("../workers/sessionReplica.worker.ts", import.meta.url), { type: "module" });
      this.worker.onmessage = (event: MessageEvent<SessionReplicaWorkerMessage>) => {
        const msg = event.data;
        if (msg?.type === "patches") {
          this.onPatches(msg.patches);
          return;
        }
        if (msg?.type === "freshness_event") {
          handleSessionReplicaFreshnessEvent(msg.event);
        }
      };
    } else {
      this.core = new SessionReplicaCore({
        api: {
          getSessionHead,
          getSessionSnapshot,
          getSessionState,
        },
        emit: this.onPatches,
        emitFreshness: handleSessionReplicaFreshnessEvent,
      });
    }

    const daemonConfig = getDaemonClientConfig();
    this.dispatch({
      type: "init",
      config: this.config,
      baseUrl: daemonConfig.baseUrl,
      authToken: daemonConfig.authToken,
      runId: daemonConfig.runId,
    });
    this.configUnsubscribe = subscribeDaemonConfig((next) => {
      this.dispatch({
        type: "update_auth",
        baseUrl: next.baseUrl,
        authToken: next.authToken,
        runId: next.runId,
      });
    });
  }

  dispatch(cmd: SessionReplicaCommand) {
    if (this.worker) {
      this.worker.postMessage(cmd);
      return;
    }
    this.core?.handleCommand(cmd);
  }

  destroy() {
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
    if (this.configUnsubscribe) {
      this.configUnsubscribe();
      this.configUnsubscribe = null;
    }
    this.core = null;
  }
}

export const handleSessionReplicaFreshnessEvent = (event: SessionReplicaFreshnessEvent): void => {
  switch (event.type) {
    case "final_delta_received":
      noteFinalDeltaReceived({
        sessionId: event.sessionId,
        turnId: event.turnId,
        emittedAtMs: event.emittedAtMs,
        lastEventSeq: event.lastEventSeq,
      });
      return;
    case "gap_recovery_started":
      noteGapRecoveryStarted(event.sessionId, event.reason);
      return;
    case "gap_recovery_finished":
      noteGapRecoveryFinished(event.sessionId);
      return;
    case "gap_repair_mismatch":
      noteGapRepairMismatch(
        event.sessionId,
        event.baselineLastEventSeq,
        event.repairedLastEventSeq,
      );
      return;
    case "projection_or_seq_regression":
      noteProjectionOrSeqRegression(
        event.sessionId,
        event.dimension,
        event.incoming,
        event.existing,
      );
      return;
  }
};
