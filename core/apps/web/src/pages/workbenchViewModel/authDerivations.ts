import { idToString, type SessionEvent, type SessionTurn } from "../../api/client";
import { parseIsoMs } from "../SessionPage.helpers";

export type AuthMethodOption = { id: string; name: string };
export type SessionErrorInfo = { message: string; provider?: string };
export type ProviderGuardNotice = {
  kind: "provider_guard_warning" | "provider_guard_kill";
  stage: string;
  provider?: string;
  message?: string;
  pid?: number;
  memoryMb?: number | null;
  limitHighMb?: number | null;
  limitMaxMb?: number | null;
  systemTotalMb?: number | null;
  systemUsedMb?: number | null;
  gracePeriodMs?: number | null;
  killAtMs?: number | null;
  createdAtMs?: number | null;
};

export type AuthUi = {
  status: "unknown" | "required" | "failed" | "authenticated";
  provider?: string;
  message?: string;
  methods: AuthMethodOption[];
};

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

function coerceNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim().length > 0) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

export function deriveAuthUi(events: SessionEvent[]): AuthUi {
  const fromMethodsValue = (value: unknown): AuthMethodOption[] => {
    const list = Array.isArray(value) ? value : [];
    return list
      .map((item): AuthMethodOption | null => {
        const method = asRecord(item);
        const id = readNonEmptyString(method.methodId ?? method.method_id ?? method.id);
        if (!id) return null;
        const name =
          readNonEmptyString(method.name ?? method.label ?? method.methodId ?? method.method_id ?? method.id) ?? id;
        return { id, name };
      })
      .filter((method): method is AuthMethodOption => method !== null);
  };

  let status: AuthUi["status"] = "unknown";
  let provider: string | undefined;
  let message: string | undefined;
  let methods: AuthMethodOption[] = [];

  const lastInit = [...events].reverse().find((e) => e.event_type === "init");
  const initMethods =
    lastInit?.payload_json?.auth_methods ??
    lastInit?.payload_json?.authMethods ??
    lastInit?.payload_json?.auth_methods;
  const initMethodOptions = fromMethodsValue(initMethods);

  for (const ev of events) {
    const payload = asRecord(ev.payload_json);
    if (ev.event_type === "auth_required") {
      status = "required";
      provider = readNonEmptyString(payload.provider) ?? undefined;
      message = readNonEmptyString(payload.message) ?? undefined;
      methods = fromMethodsValue(payload.auth_methods ?? payload.authMethods);
      continue;
    }

    if (ev.event_type !== "notice") continue;
    const kind = payload.kind;
    if (kind === "auth_required") {
      status = "required";
      provider = readNonEmptyString(payload.provider) ?? undefined;
      message = readNonEmptyString(payload.message) ?? undefined;
      methods = fromMethodsValue(payload.auth_methods ?? payload.authMethods);
    }
    if (kind === "auth_failed") {
      status = "failed";
      provider = readNonEmptyString(payload.provider) ?? undefined;
      message = readNonEmptyString(payload.message) ?? undefined;
    }
    if (kind === "auth_finished") {
      status = "authenticated";
      provider = readNonEmptyString(payload.provider) ?? undefined;
      message = undefined;
      methods = [];
    }
  }

  if ((status === "required" || status === "failed") && methods.length === 0) {
    methods = initMethodOptions;
  }

  return { status, provider, message, methods };
}

export function deriveProviderGuardNotice(events: SessionEvent[]): ProviderGuardNotice | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const ev = events[i];
    if (ev.event_type !== "notice") continue;
    const payload = ev.payload_json ?? {};
    const kind = String(payload.kind ?? "").trim();
    if (kind !== "provider_guard_warning" && kind !== "provider_guard_kill") continue;
    return {
      kind,
      stage: String(payload.stage ?? "").trim(),
      provider: typeof payload.provider === "string" ? payload.provider : undefined,
      message: typeof payload.message === "string" ? payload.message : undefined,
      pid: coerceNumber(payload.pid) ?? undefined,
      memoryMb: coerceNumber(payload.memory_mb),
      limitHighMb: coerceNumber(payload.limit_high_mb),
      limitMaxMb: coerceNumber(payload.limit_max_mb),
      systemTotalMb: coerceNumber(payload.system_total_mb),
      systemUsedMb: coerceNumber(payload.system_used_mb),
      gracePeriodMs: coerceNumber(payload.grace_period_ms),
      killAtMs: coerceNumber(payload.kill_at_ms),
      createdAtMs: parseIsoMs(ev.created_at),
    };
  }
  return null;
}

function readNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text ? text : null;
}

function extractErrorDetails(payload: any): string | null {
  if (!payload) return null;
  const direct =
    readNonEmptyString(payload.details) ??
    readNonEmptyString(payload.detail) ??
    readNonEmptyString(payload.additional_details) ??
    readNonEmptyString(payload.additionalDetails);
  if (direct) return direct;

  const codexInfo = payload.codex_error_info ?? payload.codexErrorInfo;
  const codexText = extractErrorMessageFromObject(codexInfo);
  if (codexText) return codexText;

  const kind = readNonEmptyString(payload.kind);
  return kind;
}

export function extractErrorMessage(payload: any): string | null {
  if (!payload) return null;
  const details = extractErrorDetails(payload);
  const direct =
    readNonEmptyString(payload.message) ??
    readNonEmptyString(payload.error) ??
    readNonEmptyString(payload.error_message) ??
    readNonEmptyString(payload.errorMessage);
  if (direct) {
    if (details && !direct.includes(details)) {
      return `${direct}\nDetails: ${details}`;
    }
    return direct;
  }

  const update = payload.update ?? payload;
  const updateText = extractErrorMessageFromObject(update);
  if (updateText) {
    if (details && !updateText.includes(details)) {
      return `${updateText}\nDetails: ${details}`;
    }
    return updateText;
  }

  const meta = update?._meta ?? update?.meta ?? payload._meta ?? payload.meta ?? null;
  const metaText =
    readNonEmptyString(meta?.statusText) ??
    readNonEmptyString(meta?.status_text) ??
    readNonEmptyString(meta?.message) ??
    readNonEmptyString(meta?.error);
  if (metaText) {
    if (details && !metaText.includes(details)) {
      return `${metaText}\nDetails: ${details}`;
    }
    return metaText;
  }
  return details;
}

function extractErrorMessageFromObject(value: any): string | null {
  if (!value) return null;
  if (typeof value === "string") return readNonEmptyString(value);
  if (typeof value !== "object") return null;

  const direct =
    readNonEmptyString(value.message) ??
    readNonEmptyString(value.error_message) ??
    readNonEmptyString(value.errorMessage);
  if (direct) return direct;

  const data = value.data ?? value.details ?? value.detail;
  const dataText =
    readNonEmptyString(data) ??
    readNonEmptyString(data?.message) ??
    readNonEmptyString(data?.error);
  if (dataText) return dataText;

  const nested = value.error ?? value.cause;
  const nestedText =
    typeof nested === "object"
      ? extractErrorMessageFromObject(nested)
      : readNonEmptyString(nested);
  if (nestedText) return nestedText;

  const meta = value._meta ?? value.meta;
  return readNonEmptyString(meta?.statusText) ?? readNonEmptyString(meta?.status_text);
}

export function deriveSessionError(
  turns: SessionTurn[],
  events: SessionEvent[],
): SessionErrorInfo | null {
  if (turns.length === 0) return null;
  const lastTurn = turns[turns.length - 1];
  if (lastTurn.status !== "failed") return null;
  const turnId = idToString(lastTurn.turn_id);
  let errorEvent: SessionEvent | null = null;
  for (let i = events.length - 1; i >= 0; i--) {
    const ev = events[i];
    if (ev.event_type !== "error") continue;
    if (turnId && idToString(ev.turn_id) !== turnId) continue;
    errorEvent = ev;
    break;
  }
  if (!errorEvent) {
    return { message: "Harness error." };
  }
  const message = extractErrorMessage(errorEvent.payload_json) ?? "Harness error.";
  const provider =
    readNonEmptyString(errorEvent.payload_json?.provider) ??
    readNonEmptyString(errorEvent.payload_json?.provider_id) ??
    readNonEmptyString(errorEvent.payload_json?.providerId) ??
    undefined;
  return { message, provider };
}
