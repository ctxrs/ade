import { createInterface } from "node:readline";
import { once } from "node:events";
import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";
import * as path from "node:path";
import * as fs from "node:fs";
import { query } from "@anthropic-ai/claude-agent-sdk";
import { translateClaudeEventsToCrp } from "./translate.js";

const MAX_TOOL_INPUT_BYTES = 64 * 1024;

type CrpCommand = {
  type?: string;
  [key: string]: unknown;
};

type SessionState = {
  sessionId: string;
  defaultModel?: string;
  defaultCwd?: string;
  activeTurn: TurnState | null;
};

type TurnState = {
  sessionId: string;
  turnId: string;
  runId: string;
  requestedModel?: string;
  cwd: string;
  records: Array<Record<string, unknown>>;
  emittedCount: number;
  interrupted: boolean;
  endRecordAdded: boolean;
  abortController: AbortController;
  query?: {
    next: () => Promise<{ value?: unknown; done: boolean }>;
    interrupt?: () => Promise<void>;
  };
  done?: Promise<void>;
};

let globalSeq = 0;

function getPackageVersion(): string {
  try {
    const pkgPath = new URL("../package.json", import.meta.url);
    const raw = fs.readFileSync(pkgPath, "utf8");
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.version === "string") {
      return parsed.version;
    }
  } catch {
    // Ignore and fall through to default.
  }
  return "0.0.0";
}

function shouldPrintVersion(): boolean {
  return process.argv.includes("--version") || process.argv.includes("-v");
}

async function writeEnvelope(envelope: Record<string, unknown>): Promise<void> {
  const payload = { ...envelope, v: 1, seq: ++globalSeq };
  const line = `${JSON.stringify(payload)}\n`;
  if (!process.stdout.write(line)) {
    await once(process.stdout, "drain");
  }
}

function warn(message: string): void {
  process.stderr.write(`[claude-crp] ${message}\n`);
}

function extractPrompt(command: CrpCommand): string | null {
  const prompt = command.prompt;
  if (typeof prompt === "string") return prompt;

  const items = command.items;
  if (!Array.isArray(items)) return null;

  const parts: string[] = [];
  for (const item of items) {
    if (!item) continue;
    if (typeof item === "string") {
      parts.push(item);
      continue;
    }
    if (typeof item === "object") {
      const maybeText = (item as { text?: unknown }).text;
      if (typeof maybeText === "string") {
        parts.push(maybeText);
        continue;
      }
      const maybeContent = (item as { content?: unknown }).content;
      if (typeof maybeContent === "string") {
        parts.push(maybeContent);
      }
    }
  }

  return parts.length ? parts.join("\n") : null;
}

async function openSession(command: CrpCommand, state: { session: SessionState | null }) {
  if (state.session) {
    warn("session.open ignored: session already active");
    return;
  }

  const sessionId =
    typeof command.session_id === "string" && command.session_id
      ? command.session_id
      : randomUUID();
  const config = (command.config && typeof command.config === "object"
    ? command.config
    : {}) as Record<string, unknown>;

  const defaultModel =
    typeof config.model === "string" && config.model ? config.model : undefined;
  const defaultCwd =
    typeof config.cwd === "string" && config.cwd ? config.cwd : process.cwd();

  state.session = {
    sessionId,
    defaultModel,
    defaultCwd,
    activeTurn: null
  };

  await writeEnvelope({
    channel: "control",
    type: "session.opened",
    session_id: sessionId,
    provider_session_id: sessionId
  });
}

function buildQueryOptions(turn: TurnState) {
  const options: Record<string, unknown> = {
    cwd: turn.cwd,
    includePartialMessages: true,
    settingSources: ["user", "project", "local"],
    tools: { type: "preset", preset: "claude_code" },
    extraArgs: { "session-id": turn.sessionId },
    abortController: turn.abortController,
    canUseTool: async () => ({ behavior: "allow" }),
    stderr: (data: string) => {
      process.stderr.write(String(data));
      if (!String(data).endsWith("\n")) process.stderr.write("\n");
    }
  };

  if (turn.requestedModel) {
    options.model = turn.requestedModel;
  }

  return options;
}

async function emitTranslated(turn: TurnState): Promise<void> {
  const events = translateClaudeEventsToCrp(turn.records, {
    sessionId: turn.sessionId,
    turnId: turn.turnId,
    runId: turn.runId,
    requestedModel: turn.requestedModel || null,
    maxToolInputBytes: MAX_TOOL_INPUT_BYTES
  });

  const slice = events.slice(turn.emittedCount);
  turn.emittedCount = events.length;
  for (const event of slice) {
    await writeEnvelope(event);
  }
}

async function requestCancel(turn: TurnState): Promise<void> {
  if (turn.interrupted) return;
  turn.interrupted = true;
  if (!turn.endRecordAdded) {
    turn.records.push({ record: "end", interrupted: true });
    turn.endRecordAdded = true;
  }
  if (turn.query?.interrupt) {
    try {
      await turn.query.interrupt();
    } catch (err) {
      warn(`interrupt failed: ${err}`);
    }
  }
  try {
    turn.abortController.abort();
  } catch {
    // Ignore abort failures.
  }
}

async function runTurn(turn: TurnState, prompt: string): Promise<void> {
  const options = buildQueryOptions(turn);
  const q = query({ prompt, options });
  turn.query = q;

  try {
    while (true) {
      const { value, done } = await q.next();
      if (done || !value) break;
      turn.records.push({ record: "event", event: value });
      await emitTranslated(turn);
    }
  } catch (err) {
    warn(`turn ${turn.turnId} error: ${err}`);
  }

  if (!turn.endRecordAdded) {
    turn.records.push({ record: "end", interrupted: turn.interrupted });
    turn.endRecordAdded = true;
  }
  await emitTranslated(turn);
}

async function startTurn(command: CrpCommand, state: { session: SessionState | null }) {
  const session = state.session;
  if (!session) {
    warn("session.prompt ignored: no active session");
    return;
  }

  if (
    typeof command.session_id === "string" &&
    command.session_id &&
    command.session_id !== session.sessionId
  ) {
    warn("session.prompt ignored: session_id mismatch");
    return;
  }

  if (session.activeTurn) {
    warn("session.prompt ignored: turn already active");
    return;
  }

  const prompt = extractPrompt(command);
  if (prompt === null) {
    warn("session.prompt ignored: missing prompt");
    return;
  }

  const turnId =
    typeof command.turn_id === "string" && command.turn_id
      ? command.turn_id
      : `turn_${randomUUID()}`;
  const runId = `run_${turnId}`;
  const requestedModel =
    typeof command.model === "string" && command.model
      ? command.model
      : session.defaultModel;
  const cwd =
    typeof command.cwd === "string" && command.cwd
      ? command.cwd
      : session.defaultCwd || process.cwd();

  const turn: TurnState = {
    sessionId: session.sessionId,
    turnId,
    runId,
    requestedModel,
    cwd,
    records: [],
    emittedCount: 0,
    interrupted: false,
    endRecordAdded: false,
    abortController: new AbortController()
  };

  session.activeTurn = turn;
  turn.done = runTurn(turn, prompt)
    .catch((err) => warn(`turn ${turnId} failed: ${err}`))
    .finally(() => {
      if (session.activeTurn === turn) session.activeTurn = null;
    });
}

async function cancelTurn(command: CrpCommand, state: { session: SessionState | null }) {
  const session = state.session;
  if (!session || !session.activeTurn) {
    warn("turn.cancel ignored: no active turn");
    return;
  }

  if (
    typeof command.session_id === "string" &&
    command.session_id &&
    command.session_id !== session.sessionId
  ) {
    warn("turn.cancel ignored: session_id mismatch");
    return;
  }

  if (
    typeof command.turn_id === "string" &&
    command.turn_id &&
    command.turn_id !== session.activeTurn.turnId
  ) {
    warn("turn.cancel ignored: turn_id mismatch");
    return;
  }

  await requestCancel(session.activeTurn);
}

async function handleLine(line: string, state: { session: SessionState | null }) {
  const trimmed = line.trim();
  if (!trimmed) return;

  let command: CrpCommand;
  try {
    command = JSON.parse(trimmed);
  } catch (err) {
    warn(`invalid JSONL: ${err}`);
    return;
  }

  switch (command.type) {
    case "session.open":
      await openSession(command, state);
      return;
    case "session.prompt":
      await startTurn(command, state);
      return;
    case "turn.cancel":
    case "session.cancel":
      await cancelTurn(command, state);
      return;
    default:
      warn(`unsupported command: ${command.type ?? "unknown"}`);
  }
}

export async function runRuntime(): Promise<void> {
  if (shouldPrintVersion()) {
    process.stdout.write(`${getPackageVersion()}\n`);
    return;
  }

  process.stdin.setEncoding("utf8");
  const state: { session: SessionState | null } = { session: null };
  const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });

  rl.on("line", (line) => {
    void handleLine(line, state).catch((err) => {
      warn(`command handling failed: ${err}`);
    });
  });

  await once(rl, "close");

  if (state.session?.activeTurn) {
    await requestCancel(state.session.activeTurn);
    if (state.session.activeTurn.done) {
      await state.session.activeTurn.done;
    }
  }
}

const isMain =
  typeof process.argv[1] === "string" &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url;

if (isMain) {
  runRuntime().catch((err) => {
    warn(`runtime failed: ${err}`);
    process.exitCode = 1;
  });
}
