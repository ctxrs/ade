import { createInterface } from "node:readline";
import { once } from "node:events";
import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";
import * as path from "node:path";
import * as os from "node:os";
import * as fs from "node:fs";
import {
  query,
  type Query,
  type SDKUserMessage,
  type SlashCommand,
  type ModelInfo,
  type AgentInfo,
  type AccountInfo,
  type PermissionMode
} from "@anthropic-ai/claude-agent-sdk";
import { translateClaudeEventsToCrp } from "./translate.js";

const MAX_TOOL_INPUT_BYTES = 64 * 1024;
const CLAUDE_SUBSCRIPTION_MODELS: Array<{ id: string; name: string }> = [
  { id: "default", name: "Default" },
  { id: "sonnet", name: "Sonnet" },
  { id: "opus", name: "Opus" }
];

type CrpCommand = {
  type?: string;
  [key: string]: unknown;
};

type CrpCommandEnvelope = {
  v?: unknown;
  command?: unknown;
};

type SessionState = {
  sessionId: string;
  providerSessionId: string;
  defaultModel?: string;
  defaultCwd?: string;
  permissionMode: PermissionMode;
  allowDangerouslySkipPermissions: boolean;
  activeTurn: TurnState | null;
};

type TurnState = {
  sessionId: string;
  providerSessionId: string;
  turnId: string;
  runId: string;
  requestedModel?: string;
  cwd: string;
  permissionMode: PermissionMode;
  allowDangerouslySkipPermissions: boolean;
  records: Array<Record<string, unknown>>;
  emittedCount: number;
  interrupted: boolean;
  endRecordAdded: boolean;
  abortController: AbortController;
  query?: Query;
  done?: Promise<void>;
};

type ClaudeImageMimeType = "image/jpeg" | "image/png" | "image/gif" | "image/webp";

type ClaudeTextPromptBlock = {
  type: "text";
  text: string;
};

type ClaudeImagePromptBlock = {
  type: "image";
  source: {
    type: "base64";
    media_type: ClaudeImageMimeType;
    data: string;
  };
};

type ClaudePromptBlock = ClaudeTextPromptBlock | ClaudeImagePromptBlock;

export type ResolvedPromptInput =
  | {
      kind: "text";
      prompt: string;
    }
  | {
      kind: "structured";
      messages: SDKUserMessage[];
    };

let globalSeq = 0;

type RuntimeInitializationResult = Awaited<ReturnType<Query["initializationResult"]>>;

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

function asNonEmptyTrimmedString(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}

function readStringArray(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const list = value
    .map((entry) => asNonEmptyTrimmedString(entry))
    .filter((entry): entry is string => Boolean(entry));
  return list.length > 0 ? list : undefined;
}

function readJsonArray(value: unknown): Array<Record<string, unknown>> | undefined {
  if (!Array.isArray(value)) return undefined;
  const list = value.filter(
    (entry): entry is Record<string, unknown> =>
      Boolean(entry) && typeof entry === "object" && !Array.isArray(entry)
  );
  return list.length > 0 ? list : undefined;
}

function mapSlashCommand(command: SlashCommand): Record<string, unknown> {
  const out: Record<string, unknown> = { name: command.name };
  const description = asNonEmptyTrimmedString(command.description);
  const argumentHint = asNonEmptyTrimmedString(command.argumentHint);
  if (description) out.description = description;
  if (argumentHint) out.argument_hint = argumentHint;
  return out;
}

function mapModelInfo(model: ModelInfo): Record<string, unknown> {
  const out: Record<string, unknown> = { id: model.id };
  const name = asNonEmptyTrimmedString(model.name);
  const description = asNonEmptyTrimmedString(model.description);
  if (name) out.name = name;
  if (description) out.description = description;
  return out;
}

function mapAgentInfo(agent: AgentInfo): Record<string, unknown> {
  const out: Record<string, unknown> = { name: agent.name };
  const description = asNonEmptyTrimmedString(agent.description);
  const model = asNonEmptyTrimmedString(agent.model);
  if (description) out.description = description;
  if (model) out.model = model;
  return out;
}

function mapAccountInfo(account: AccountInfo | null | undefined): Record<string, unknown> | undefined {
  if (!account) return undefined;
  const out: Record<string, unknown> = {};
  const email = asNonEmptyTrimmedString(account.email);
  const organization = asNonEmptyTrimmedString(account.organization);
  const subscriptionType = asNonEmptyTrimmedString(account.subscriptionType);
  const tokenSource = asNonEmptyTrimmedString(account.tokenSource);
  const apiKeySource = asNonEmptyTrimmedString(account.apiKeySource);
  if (email) out.email = email;
  if (organization) out.organization = organization;
  if (subscriptionType) out.subscription_type = subscriptionType;
  if (tokenSource) out.token_source = tokenSource;
  if (apiKeySource) out.api_key_source = apiKeySource;
  return Object.keys(out).length > 0 ? out : undefined;
}

export function buildSessionOpenedMetadataEnvelope(params: {
  sessionId: string;
  providerSessionId?: string;
  initializationResult?: RuntimeInitializationResult | null;
  systemInit?: Record<string, unknown> | null;
}): Record<string, unknown> {
  const envelope: Record<string, unknown> = {
    channel: "control",
    type: "session.opened",
    session_id: params.sessionId,
    provider_session_id: params.providerSessionId ?? params.sessionId,
    supports_session_status: true
  };

  const initializationResult = params.initializationResult ?? null;
  if (initializationResult) {
    if (Array.isArray(initializationResult.commands) && initializationResult.commands.length > 0) {
      envelope.commands = initializationResult.commands.map((command) => mapSlashCommand(command));
    }
    if (Array.isArray(initializationResult.models) && initializationResult.models.length > 0) {
      envelope.models = initializationResult.models.map((model) => mapModelInfo(model));
    }
    if (Array.isArray(initializationResult.agents) && initializationResult.agents.length > 0) {
      envelope.agents = initializationResult.agents.map((agent) => mapAgentInfo(agent));
    }
    const outputStyle = asNonEmptyTrimmedString(initializationResult.output_style);
    if (outputStyle) envelope.output_style = outputStyle;
    const availableOutputStyles = readStringArray(initializationResult.available_output_styles);
    if (availableOutputStyles) envelope.available_output_styles = availableOutputStyles;
    const account = mapAccountInfo(initializationResult.account);
    if (account) envelope.account = account;
    const fastModeState = asNonEmptyTrimmedString(initializationResult.fast_mode_state);
    if (fastModeState) envelope.fast_mode_state = fastModeState;
  }

  const systemInit = params.systemInit ?? null;
  if (systemInit) {
    const slashCommands = readStringArray(systemInit.slash_commands);
    if (slashCommands) envelope.slash_commands = slashCommands;
    const skills = readStringArray(systemInit.skills);
    if (skills) envelope.skills = skills;
    const tools = readStringArray(systemInit.tools);
    if (tools) envelope.tools = tools;
    const plugins = readJsonArray(systemInit.plugins);
    if (plugins) envelope.plugins = plugins;
    const mcpServers = Array.isArray(systemInit.mcp_servers) ? systemInit.mcp_servers : undefined;
    if (mcpServers && mcpServers.length > 0) {
      envelope.mcp_servers = mcpServers;
    }
    const currentModelId = asNonEmptyTrimmedString(systemInit.model);
    if (currentModelId) envelope.current_model_id = currentModelId;
    const permissionMode = asNonEmptyTrimmedString(systemInit.permissionMode);
    if (permissionMode) envelope.permission_mode = permissionMode;
  }

  return envelope;
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

function buildClaudeProcessEnv(): Record<string, string> {
  const nodeBinDir = path.dirname(process.execPath);
  const existingPath = process.env.PATH ?? "";
  const combinedPath = existingPath
    ? `${nodeBinDir}${path.delimiter}${existingPath}`
    : nodeBinDir;
  return {
    ...process.env,
    PATH: combinedPath
  } as Record<string, string>;
}

function asPermissionMode(value: unknown): PermissionMode | undefined {
  switch (value) {
    case "default":
    case "acceptEdits":
    case "bypassPermissions":
    case "plan":
    case "dontAsk":
      return value;
    default:
      return undefined;
  }
}

export function resolvePermissionSettings(params?: {
  config?: Record<string, unknown> | null;
  env?: NodeJS.ProcessEnv;
}): {
  permissionMode: PermissionMode;
  allowDangerouslySkipPermissions: boolean;
} {
  const config = params?.config ?? {};
  const env = params?.env ?? process.env;

  const envMode = asPermissionMode(env.CTX_PROVIDER_MODE);
  if (envMode) {
    return {
      permissionMode: envMode,
      allowDangerouslySkipPermissions: envMode === "bypassPermissions"
    };
  }

  const explicitMode =
    asPermissionMode(config.permission_mode) ?? asPermissionMode(config.permissionMode);
  if (explicitMode) {
    return {
      permissionMode: explicitMode,
      allowDangerouslySkipPermissions: explicitMode === "bypassPermissions"
    };
  }

  const approvalPolicy =
    asNonEmptyTrimmedString(config.approval_policy) ??
    asNonEmptyTrimmedString(config.approvalPolicy);
  const sandboxMode =
    asNonEmptyTrimmedString(config.sandbox_mode) ??
    asNonEmptyTrimmedString(config.sandboxMode);

  if (approvalPolicy === "never" && sandboxMode === "danger-full-access") {
    return {
      permissionMode: "bypassPermissions",
      allowDangerouslySkipPermissions: true
    };
  }

  return {
    permissionMode: "default",
    allowDangerouslySkipPermissions: false
  };
}

export function buildPermissionControlOptions(
  permissionMode: PermissionMode,
  allowDangerouslySkipPermissions: boolean
): Record<string, unknown> {
  const options: Record<string, unknown> = {
    permissionMode
  };

  if (allowDangerouslySkipPermissions) {
    options.allowDangerouslySkipPermissions = true;
  }

  if (permissionMode === "default") {
    options.canUseTool = async () => ({ behavior: "allow" });
  }

  return options;
}

function resolveCwd(cwd: string): string {
  if (!cwd) return cwd;
  try {
    return fs.realpathSync(cwd);
  } catch {
    return cwd;
  }
}

function projectKeyForCwd(cwd: string): string {
  const resolved = resolveCwd(cwd);
  return resolved.replace(/[^a-zA-Z0-9]/g, "-");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function inferClaudeImageMimeType(rawMime: string): ClaudeImageMimeType {
  const mime = rawMime.trim().toLowerCase();
  switch (mime) {
    case "image/jpeg":
    case "image/jpg":
      return "image/jpeg";
    case "image/png":
      return "image/png";
    case "image/gif":
      return "image/gif";
    case "image/webp":
      return "image/webp";
    default:
      throw new Error(`unsupported image mime type for Claude runtime: ${rawMime}`);
  }
}

function inferClaudeImageMimeTypeFromPath(filePath: string): ClaudeImageMimeType {
  const ext = path.extname(filePath).trim().toLowerCase();
  switch (ext) {
    case ".jpg":
    case ".jpeg":
      return "image/jpeg";
    case ".png":
      return "image/png";
    case ".gif":
      return "image/gif";
    case ".webp":
      return "image/webp";
    default:
      throw new Error(`local_image requires an explicit supported image mime type: ${filePath}`);
  }
}

function parseDataUrl(url: string): { mimeType: string; data: string } | null {
  if (!url.startsWith("data:")) return null;
  const rest = url.slice("data:".length);
  const split = rest.indexOf(",");
  if (split === -1) return null;
  const meta = rest.slice(0, split);
  const data = rest.slice(split + 1);
  if (!meta.includes("base64")) return null;
  const [maybeMime] = meta.split(";");
  return {
    mimeType: maybeMime?.trim() || "application/octet-stream",
    data,
  };
}

function imageBlockFromBase64(data: string, mimeType: string): ClaudeImagePromptBlock {
  return {
    type: "image",
    source: {
      type: "base64",
      media_type: inferClaudeImageMimeType(mimeType),
      data,
    },
  };
}

function resolveDataRoot(env: NodeJS.ProcessEnv): string {
  const hostRoot = typeof env.CTX_DATA_ROOT_HOST === "string" ? env.CTX_DATA_ROOT_HOST.trim() : "";
  if (hostRoot) return hostRoot;
  const dataRoot = typeof env.CTX_DATA_ROOT === "string" ? env.CTX_DATA_ROOT.trim() : "";
  if (dataRoot) return dataRoot;
  throw new Error("missing CTX_DATA_ROOT_HOST/CTX_DATA_ROOT for image attachment");
}

function resolveBlobImageBlock(blobId: string, mimeType: string, env: NodeJS.ProcessEnv): ClaudeImagePromptBlock {
  const dataRoot = resolveDataRoot(env);
  const blobPath = path.join(dataRoot, "blobs", blobId);
  const bytes = fs.readFileSync(blobPath);
  return imageBlockFromBase64(bytes.toString("base64"), mimeType);
}

function resolveLocalImageBlock(
  rawPath: string,
  cwd: string,
  explicitMimeType?: string,
): ClaudeImagePromptBlock {
  const resolvedPath = path.isAbsolute(rawPath) ? rawPath : path.join(cwd, rawPath);
  const bytes = fs.readFileSync(resolvedPath);
  const mimeType = explicitMimeType
    ? inferClaudeImageMimeType(explicitMimeType)
    : inferClaudeImageMimeTypeFromPath(resolvedPath);
  return {
    type: "image",
    source: {
      type: "base64",
      media_type: mimeType,
      data: bytes.toString("base64"),
    },
  };
}

function skillText(item: Record<string, unknown>): string | null {
  const name = asNonEmptyTrimmedString(item.name);
  const content =
    asNonEmptyTrimmedString(item.content) ?? asNonEmptyTrimmedString(item.text);
  if (!name && !content) return null;
  if (!name) return content;
  if (!content) return `Skill: ${name}`;
  return `Skill: ${name}\n\n${content}`;
}

function resolvePromptBlock(
  item: unknown,
  options: { env: NodeJS.ProcessEnv; cwd: string },
): ClaudePromptBlock[] {
  if (typeof item === "string") {
    return [{ type: "text", text: item }];
  }
  if (!isRecord(item)) {
    return [];
  }

  const type = asNonEmptyTrimmedString(item.type);
  if (type === "text") {
    const text = asNonEmptyTrimmedString(item.text);
    if (!text) throw new Error("CRP text item missing text");
    return [{ type: "text", text }];
  }
  if (type === "skill") {
    const text = skillText(item);
    if (!text) throw new Error("CRP skill item missing name/content");
    return [{ type: "text", text }];
  }
  if (type === "image") {
    const data = asNonEmptyTrimmedString(item.data);
    const mimeType =
      asNonEmptyTrimmedString(item.mime_type) ?? asNonEmptyTrimmedString(item.mimeType);
    if (!data || !mimeType) {
      throw new Error("CRP image item missing data or mime_type");
    }
    return [imageBlockFromBase64(data, mimeType)];
  }
  if (type === "image_ref") {
    const blobId = asNonEmptyTrimmedString(item.blob_id);
    const mimeType =
      asNonEmptyTrimmedString(item.mime_type) ?? asNonEmptyTrimmedString(item.mimeType);
    if (!blobId || !mimeType) {
      throw new Error("CRP image_ref item missing blob_id or mime_type");
    }
    return [resolveBlobImageBlock(blobId, mimeType, options.env)];
  }
  if (type === "local_image") {
    const rawPath = asNonEmptyTrimmedString(item.path);
    if (!rawPath) {
      throw new Error("CRP local_image item missing path");
    }
    const mimeType =
      asNonEmptyTrimmedString(item.mime_type) ?? asNonEmptyTrimmedString(item.mimeType);
    return [resolveLocalImageBlock(rawPath, options.cwd, mimeType)];
  }

  const imageUrlValue = item.image_url;
  if (typeof imageUrlValue === "string") {
    const parsed = parseDataUrl(imageUrlValue);
    if (!parsed) {
      throw new Error("image_url must be a base64 data URL for the Claude runtime");
    }
    return [imageBlockFromBase64(parsed.data, parsed.mimeType)];
  }
  if (isRecord(imageUrlValue) && typeof imageUrlValue.url === "string") {
    const parsed = parseDataUrl(imageUrlValue.url);
    if (!parsed) {
      throw new Error("image_url.url must be a base64 data URL for the Claude runtime");
    }
    return [imageBlockFromBase64(parsed.data, parsed.mimeType)];
  }

  const maybeText = asNonEmptyTrimmedString(item.text);
  if (maybeText) {
    return [{ type: "text", text: maybeText }];
  }
  const maybeContent = asNonEmptyTrimmedString(item.content);
  if (maybeContent) {
    return [{ type: "text", text: maybeContent }];
  }
  return [];
}

function buildStructuredPromptMessages(
  sessionId: string,
  content: ClaudePromptBlock[],
): SDKUserMessage[] {
  if (content.length === 0) {
    return [];
  }
  return [
    {
      type: "user",
      session_id: sessionId,
      parent_tool_use_id: null,
      message: {
        role: "user",
        content,
      } as SDKUserMessage["message"],
    },
  ];
}

export function resolvePromptInput(
  command: CrpCommand,
  options: { sessionId: string; cwd: string; env?: NodeJS.ProcessEnv },
): ResolvedPromptInput | null {
  const env = options.env ?? process.env;
  const items = Array.isArray(command.items) ? command.items : null;
  if (items) {
    const content = items.flatMap((item) =>
      resolvePromptBlock(item, { env, cwd: options.cwd }),
    );
    if (content.length > 0) {
      const hasImage = content.some((block) => block.type === "image");
      if (!hasImage) {
        const prompt = content
          .filter((block): block is ClaudeTextPromptBlock => block.type === "text")
          .map((block) => block.text)
          .join("\n");
        if (prompt) {
          return { kind: "text", prompt };
        }
      }
      const messages = buildStructuredPromptMessages(options.sessionId, content);
      return messages.length > 0 ? { kind: "structured", messages } : null;
    }
  }

  const prompt = typeof command.prompt === "string" ? command.prompt : null;
  return prompt ? { kind: "text", prompt } : null;
}

async function* streamPromptMessages(messages: SDKUserMessage[]): AsyncIterable<SDKUserMessage> {
  for (const message of messages) {
    yield message;
  }
}

function parseIncomingCommand(parsed: unknown): CrpCommand | null {
  if (!parsed || typeof parsed !== "object") return null;
  const maybeEnvelope = parsed as CrpCommandEnvelope;
  if (maybeEnvelope.command && typeof maybeEnvelope.command === "object") {
    return maybeEnvelope.command as CrpCommand;
  }
  return parsed as CrpCommand;
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
  const providerSessionId =
    typeof command.provider_session_id === "string" && command.provider_session_id
      ? command.provider_session_id
      : sessionId;
  const config = (command.config && typeof command.config === "object"
    ? command.config
    : {}) as Record<string, unknown>;
  const permissionSettings = resolvePermissionSettings({ config });

  const defaultModel =
    typeof config.model === "string" && config.model ? config.model : undefined;
  const defaultCwd =
    typeof config.cwd === "string" && config.cwd ? config.cwd : process.cwd();

  state.session = {
    sessionId,
    providerSessionId,
    defaultModel,
    defaultCwd,
    permissionMode: permissionSettings.permissionMode,
    allowDangerouslySkipPermissions: permissionSettings.allowDangerouslySkipPermissions,
    activeTurn: null
  };

  await writeEnvelope({
    channel: "control",
    type: "session.opened",
    session_id: sessionId,
    provider_session_id: providerSessionId,
    supports_session_status: true
  });
}

export function buildQueryOptions(turn: TurnState) {
  const claudeConfigDir =
    typeof process.env.CLAUDE_CONFIG_DIR === "string" && process.env.CLAUDE_CONFIG_DIR.trim()
      ? process.env.CLAUDE_CONFIG_DIR.trim()
      : path.join(os.homedir(), ".claude");
  const resolvedCwd = resolveCwd(turn.cwd);
  const projectKey = projectKeyForCwd(turn.cwd);
  const sessionFilePath = path.join(
    claudeConfigDir,
    "projects",
    projectKey,
    `${turn.providerSessionId}.jsonl`
  );
  const shouldResume = fs.existsSync(sessionFilePath);

  const options: Record<string, unknown> = {
    cwd: resolvedCwd,
    includePartialMessages: true,
    settingSources: ["user", "project", "local"],
    tools: { type: "preset", preset: "claude_code" },
    env: buildClaudeProcessEnv(),
    ...buildPermissionControlOptions(
      turn.permissionMode,
      turn.allowDangerouslySkipPermissions
    ),
    ...(shouldResume
      ? { resume: turn.providerSessionId }
      : { extraArgs: { "session-id": turn.providerSessionId } }),
    abortController: turn.abortController,
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

function extractModelsListConfig(command: CrpCommand): { model?: string; cwd?: string } {
  const config =
    command.config && typeof command.config === "object"
      ? (command.config as Record<string, unknown>)
      : {};
  const model =
    typeof config.model === "string" && config.model ? config.model : undefined;
  const cwd = typeof config.cwd === "string" && config.cwd ? config.cwd : undefined;
  return { model, cwd };
}

function appendUniqueModel(
  models: Array<{ id: string; name?: string }>,
  modelId: string,
  modelName?: string
): void {
  const nextId = modelId.trim();
  if (!nextId) return;
  if (models.some((entry) => entry.id === nextId)) return;
  models.push({ id: nextId, name: modelName?.trim() || undefined });
}

async function listModels(command: CrpCommand, state: { session: SessionState | null }) {
  const { model } = extractModelsListConfig(command);
  const session = state.session;
  const resolvedModel = model ?? session?.defaultModel;

  const models: Array<{ id: string; name?: string }> = CLAUDE_SUBSCRIPTION_MODELS.map(
    (entry) => ({ ...entry })
  );

  if (resolvedModel) appendUniqueModel(models, resolvedModel, resolvedModel);

  const currentModelId = resolvedModel ?? models[0]?.id;

  await writeEnvelope({
    channel: "control",
    type: "models.list",
    models,
    current_model_id: currentModelId
  });
}

async function setSessionModel(command: CrpCommand, state: { session: SessionState | null }) {
  const session = state.session;
  if (!session) {
    warn("session.set_model ignored: no active session");
    return;
  }

  if (
    typeof command.session_id === "string" &&
    command.session_id &&
    command.session_id !== session.sessionId
  ) {
    warn("session.set_model ignored: session_id mismatch");
    return;
  }

  const nextModel = asNonEmptyTrimmedString(command.model_id);
  if (!nextModel) {
    warn("session.set_model ignored: missing model_id");
    return;
  }

  session.defaultModel = nextModel;

  await writeEnvelope({
    channel: "control",
    type: "session.notice",
    session_id: session.sessionId,
    code: "session_model_updated",
    severity: "info",
    message: `session model updated to ${nextModel}`,
    details: { model_id: nextModel },
    transient: false
  });
}

export function buildSessionStatusNotice(params: {
  sessionId: string;
  activeTurnId?: string | null;
}): Record<string, unknown> {
  const activeTurnId = params.activeTurnId ?? null;
  const busyReasons = activeTurnId ? ["active_turn"] : [];
  return {
    channel: "control",
    type: "session.notice",
    session_id: params.sessionId,
    code: "session_status",
    severity: "info",
    message: activeTurnId ? "session is busy" : "session is quiescent",
    details: {
      quiescent: activeTurnId == null,
      active_turn_id: activeTurnId,
      busy_reasons: busyReasons
    },
    transient: false
  };
}

async function emitSessionStatus(state: { session: SessionState | null }) {
  const session = state.session;
  if (!session) {
    warn("session.status ignored: no active session");
    return;
  }
  await writeEnvelope(
    buildSessionStatusNotice({
      sessionId: session.sessionId,
      activeTurnId: session.activeTurn?.turnId ?? null
    })
  );
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

async function runTurn(session: SessionState, turn: TurnState, command: CrpCommand): Promise<void> {
  const options = buildQueryOptions(turn);
  let initializationResult: RuntimeInitializationResult | null = null;
  let failureMessage: string | null = null;
  let initializationEmitted = false;
  let q: Query | null = null;

  try {
    const promptInput = resolvePromptInput(command, {
      sessionId: session.sessionId,
      cwd: turn.cwd,
      env: process.env,
    });
    if (promptInput == null) {
      throw new Error("missing prompt");
    }

    q = query({
      prompt:
        promptInput.kind === "text"
          ? promptInput.prompt
          : streamPromptMessages(promptInput.messages),
      options,
    });
    turn.query = q;

    initializationResult = await q.initializationResult();
  } catch (err) {
    const msg =
      err instanceof Error ? err.message : err == null ? "unknown_error" : String(err);
    warn(`initializationResult failed for ${turn.turnId}: ${msg}`);
    failureMessage = msg;
  }

  const ensureResultRecord = () => {
    let existingResult: Record<string, unknown> | null = null;
    for (let idx = turn.records.length - 1; idx >= 0; idx -= 1) {
      const record = turn.records[idx];
      if (!record || typeof record !== "object") continue;
      if ((record as { record?: unknown }).record !== "event") continue;
      const ev = (record as { event?: unknown }).event;
      if (!ev || typeof ev !== "object") continue;
      if ((ev as { type?: unknown }).type !== "result") continue;
      existingResult = ev as Record<string, unknown>;
      break;
    }

    if (existingResult) {
      if (!turn.interrupted && failureMessage) {
        const errors = (existingResult as { errors?: unknown }).errors;
        const error = (existingResult as { error?: unknown }).error;
        const hasMessage =
          (Array.isArray(errors) && errors.length > 0) ||
          (typeof error === "string" && error.trim());
        if (!hasMessage) {
          (existingResult as { errors?: unknown }).errors = [failureMessage];
        }
      }
      return;
    }

    if (turn.interrupted) {
      turn.records.push({
        record: "event",
        event: { type: "result", subtype: "success", is_error: false }
      });
      return;
    }

    turn.records.push({
      record: "event",
      event: {
        type: "result",
        subtype: "error",
        is_error: true,
        errors: [failureMessage ?? "claude_code_missing_result"]
      }
    });
  };

  try {
    if (!q) {
      throw new Error(failureMessage ?? "claude_code_query_not_initialized");
    }
    while (true) {
      const { value, done } = await q.next();
      if (value != null) {
        const maybeSystemInit =
          !initializationEmitted &&
          typeof value === "object" &&
          value !== null &&
          (value as { type?: unknown }).type === "system" &&
          (value as { subtype?: unknown }).subtype === "init"
            ? (value as Record<string, unknown>)
            : null;
        if (maybeSystemInit) {
          await writeEnvelope(
            buildSessionOpenedMetadataEnvelope({
              sessionId: session.sessionId,
              providerSessionId: session.providerSessionId,
              initializationResult,
              systemInit: maybeSystemInit
            })
          );
          initializationEmitted = true;
        }
        const isResultEvent =
          typeof value === "object" && (value as { type?: unknown }).type === "result";
        turn.records.push({ record: "event", event: value });
        if (!isResultEvent) {
          await emitTranslated(turn);
        }
      }
      if (done) break;
    }
  } catch (err) {
    const msg =
      err instanceof Error ? err.message : err == null ? "unknown_error" : String(err);
    warn(`turn ${turn.turnId} error: ${msg}`);
    failureMessage = msg;

    // If Claude Code bails before producing a terminal "result" event, the translator won't emit
    // `turn.completed`, and ctx can keep the turn stuck in "Working". Emit a synthetic result
    // so downstream sees a terminal event, and preserve the failure reason.
    ensureResultRecord();
  }

  ensureResultRecord();

  if (!turn.endRecordAdded) {
    turn.records.push({ record: "end", interrupted: turn.interrupted });
    turn.endRecordAdded = true;
  }
  if (!initializationEmitted && initializationResult) {
    await writeEnvelope(
      buildSessionOpenedMetadataEnvelope({
        sessionId: session.sessionId,
        providerSessionId: session.providerSessionId,
        initializationResult
      })
    );
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
    providerSessionId: session.providerSessionId,
    turnId,
    runId,
    requestedModel,
    cwd,
    permissionMode: session.permissionMode,
    allowDangerouslySkipPermissions: session.allowDangerouslySkipPermissions,
    records: [],
    emittedCount: 0,
    interrupted: false,
    endRecordAdded: false,
    abortController: new AbortController()
  };

  session.activeTurn = turn;
  turn.done = runTurn(session, turn, command)
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

  let command: CrpCommand | null;
  try {
    command = parseIncomingCommand(JSON.parse(trimmed));
  } catch (err) {
    warn(`invalid JSONL: ${err}`);
    return;
  }
  if (!command || typeof command !== "object") {
    warn("invalid command payload");
    return;
  }

  switch (command.type) {
    case "session.open":
      await openSession(command, state);
      return;
    case "session.prompt":
      await startTurn(command, state);
      return;
    case "models.list":
      await listModels(command, state);
      return;
    case "session.set_model":
      await setSessionModel(command, state);
      return;
    case "session.status":
      await emitSessionStatus(state);
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

  for await (const line of rl) {
    try {
      await handleLine(line, state);
    } catch (err) {
      warn(`command handling failed: ${err}`);
    }
  }

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
