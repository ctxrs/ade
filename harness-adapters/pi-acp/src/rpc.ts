import { spawn, ChildProcessWithoutNullStreams } from "node:child_process";
import { accessSync, constants } from "node:fs";
import { dirname, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

export type PiRpcState = {
  isStreaming: boolean;
  isCompacting: boolean;
  pendingMessageCount: number;
};

type WaitForIdleOptions = {
  timeoutMs?: number;
  pollIntervalMs?: number;
  idlePollThreshold?: number;
};

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  command: string;
};

type ResponseLine = {
  id?: string;
  type: string;
  command?: string;
  success?: boolean;
  data?: unknown;
  error?: string;
};

function parseExtraArgs(raw: string | undefined): string[] {
  if (!raw) return [];
  const trimmed = raw.trim();
  if (!trimmed) return [];
  return trimmed.split(/\s+/g);
}

function isExecutable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function localPiCliForAdapter(): string | null {
  const here = dirname(fileURLToPath(import.meta.url));
  const binName = process.platform === "win32" ? "pi.cmd" : "pi";
  const candidate = resolve(here, "../node_modules/.bin", binName);
  return isExecutable(candidate) ? candidate : null;
}

export function buildPiLaunchArgs(env: NodeJS.ProcessEnv): { command: string; args: string[] } {
  const explicitCommand = env.PI_ACP_PI_COMMAND?.trim();
  const args = ["--mode", "rpc", "--no-session"];

  if (env.PI_ACP_PROVIDER?.trim()) {
    args.push("--provider", env.PI_ACP_PROVIDER.trim());
  }
  if (env.PI_ACP_MODEL?.trim()) {
    args.push("--model", env.PI_ACP_MODEL.trim());
  }
  const extraArgs = parseExtraArgs(env.PI_ACP_PI_ARGS);

  if (explicitCommand) {
    return { command: explicitCommand, args: [...args, ...extraArgs] };
  }

  const bundledLocalPi = localPiCliForAdapter();
  if (bundledLocalPi) {
    return { command: bundledLocalPi, args: [...args, ...extraArgs] };
  }

  throw new Error(
    "pi ACP runtime missing bundled pi binary at node_modules/.bin/pi. Rebuild bundled harnesses for provider 'pi'.",
  );
}

const DEFAULT_IDLE_POLL_MS = 150;
const DEFAULT_IDLE_POLL_THRESHOLD = 2;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function waitForIdleState(
  pollState: () => Promise<PiRpcState>,
  options: WaitForIdleOptions = {},
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 240_000;
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_IDLE_POLL_MS;
  const idlePollThreshold = Math.max(1, options.idlePollThreshold ?? DEFAULT_IDLE_POLL_THRESHOLD);
  const start = Date.now();
  let sawBusy = false;
  let consecutiveIdlePolls = 0;

  while (Date.now() - start < timeoutMs) {
    const state = await pollState();
    const busy = state.isStreaming || state.isCompacting || state.pendingMessageCount > 0;
    if (busy) {
      sawBusy = true;
      consecutiveIdlePolls = 0;
    } else {
      consecutiveIdlePolls += 1;
      if (sawBusy || consecutiveIdlePolls >= idlePollThreshold) {
        return;
      }
    }
    await sleep(pollIntervalMs);
  }

  throw new Error(`timeout waiting for pi RPC idle after ${timeoutMs}ms`);
}

export class PiRpcClient {
  private child: ChildProcessWithoutNullStreams;
  private nextId = 0;
  private pending = new Map<string, PendingRequest>();
  private closed = false;
  private spawnError: Error | null = null;

  constructor(private cwd: string, env: NodeJS.ProcessEnv = process.env) {
    const launch = buildPiLaunchArgs(env);
    this.child = spawn(launch.command, launch.args, {
      cwd,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });

    const stdout = createInterface({ input: this.child.stdout, crlfDelay: Infinity });
    stdout.on("line", (line) => this.handleLine(line));

    this.child.on("error", (err) => {
      const wrapped = err instanceof Error ? err : new Error(String(err));
      this.spawnError = wrapped;
      this.closed = true;
      this.failAllPending(wrapped);
    });
    this.child.on("exit", (code, signal) => {
      this.closed = true;
      const base = this.spawnError
        ?? new Error(`pi RPC process exited (code=${code ?? "null"}, signal=${signal ?? "null"})`);
      this.failAllPending(base);
    });
  }

  async stop(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.child.kill();
  }

  async prompt(message: string): Promise<void> {
    await this.send("prompt", { type: "prompt", message });
  }

  async abort(): Promise<void> {
    await this.send("abort", { type: "abort" });
  }

  async getState(): Promise<PiRpcState> {
    const data = await this.send("get_state", { type: "get_state" });
    if (!isObject(data)) {
      throw new Error("invalid get_state response: missing data object");
    }
    const isStreaming = toBoolean(data.isStreaming);
    const isCompacting = toBoolean(data.isCompacting);
    const pendingMessageCount = toNumber(data.pendingMessageCount);
    return {
      isStreaming,
      isCompacting,
      pendingMessageCount,
    };
  }

  async getLastAssistantText(): Promise<string | null> {
    const data = await this.send("get_last_assistant_text", {
      type: "get_last_assistant_text",
    });
    if (!isObject(data)) {
      throw new Error("invalid get_last_assistant_text response: missing data object");
    }
    if (data.text === null) return null;
    if (typeof data.text !== "string") {
      throw new Error("invalid get_last_assistant_text response: data.text must be string|null");
    }
    return data.text;
  }

  async waitForIdle(timeoutMs = 240_000): Promise<void> {
    await waitForIdleState(() => this.getState(), { timeoutMs });
  }

  private async send(command: string, payload: Record<string, unknown>): Promise<unknown> {
    if (this.spawnError) {
      throw new Error(`pi RPC process failed to start: ${this.spawnError.message}`);
    }
    if (this.closed) {
      throw new Error(`pi RPC client closed before sending command '${command}'`);
    }
    const id = `req-${++this.nextId}`;
    const line = JSON.stringify({ id, ...payload });

    const response = new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, command });
    });

    try {
      this.child.stdin.write(`${line}\n`, (err) => {
        if (!err) return;
        const pending = this.pending.get(id);
        if (!pending) return;
        this.pending.delete(id);
        pending.reject(
          new Error(`failed to write pi RPC command '${command}': ${err.message}`),
        );
      });
    } catch (err) {
      this.pending.delete(id);
      throw err;
    }
    return response;
  }

  private handleLine(line: string): void {
    const trimmed = line.trim();
    if (!trimmed) return;

    let parsed: unknown;
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      return;
    }

    if (!isObject(parsed)) return;
    const msg = parsed as ResponseLine;
    if (msg.type !== "response") return;

    const id = typeof msg.id === "string" ? msg.id : "";
    if (!id) return;

    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);

    if (msg.success === true) {
      pending.resolve(msg.data);
      return;
    }

    const detail = typeof msg.error === "string" ? msg.error : "unknown error";
    pending.reject(new Error(`pi RPC command '${pending.command}' failed: ${detail}`));
  }

  private failAllPending(err: Error): void {
    for (const [, pending] of this.pending) {
      pending.reject(err);
    }
    this.pending.clear();
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

function toBoolean(value: unknown): boolean {
  return value === true;
}

function toNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}
