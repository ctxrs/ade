import {
  authToken,
  getSessionHead,
  getSessionSnapshot,
  getSessionState,
  listSessionArtifacts,
  resolveDaemonBaseUrl,
} from "../api/client";
import { SessionReplicaCore } from "./sessionReplicaCore";
import type {
  SessionReplicaCommand,
  SessionReplicaConfig,
  SessionReplicaPatch,
  SessionReplicaWorkerMessage,
} from "./sessionReplicaProtocol";

const shouldUseWorker = (): boolean => {
  if (typeof Worker === "undefined") return false;
  const metaEnv =
    typeof import.meta !== "undefined" ? (import.meta as { env?: { MODE?: string } }).env : undefined;
  if (metaEnv?.MODE === "test") return false;
  return true;
};

export class SessionReplicaBridge {
  private worker: Worker | null = null;
  private core: SessionReplicaCore | null = null;

  constructor(
    private onPatches: (patches: SessionReplicaPatch[]) => void,
    private config: SessionReplicaConfig,
  ) {
    if (shouldUseWorker()) {
      this.worker = new Worker(new URL("../workers/sessionReplica.worker.ts", import.meta.url), { type: "module" });
      this.worker.onmessage = (event: MessageEvent<SessionReplicaWorkerMessage>) => {
        const msg = event.data;
        if (msg?.type !== "patches") return;
        this.onPatches(msg.patches);
      };
    } else {
      this.core = new SessionReplicaCore({
        api: {
          getSessionHead,
          getSessionSnapshot,
          getSessionState,
          listSessionArtifacts,
        },
        emit: this.onPatches,
      });
    }

    this.dispatch({
      type: "init",
      config: this.config,
      baseUrl: resolveDaemonBaseUrl(),
      authToken: authToken(),
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
    this.core = null;
  }
}
