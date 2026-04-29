import { RelayHttpError } from "./http";
import type { ValidatedResponsesRequest } from "./types";

type JsonObject = Record<string, unknown>;

const TOP_LEVEL_ALLOWED = new Set([
  "model",
  "input",
  "instructions",
  "max_output_tokens",
  "tools",
  "background",
  "store",
  "stream",
  "temperature",
]);

export function validateResponsesRequest(value: unknown): ValidatedResponsesRequest {
  const body = asObject(value, "request");
  rejectUnknown(body, TOP_LEVEL_ALLOWED, "request");
  const model = requiredString(body.model, "model");
  const maxOutputTokens = requiredPositiveInteger(body.max_output_tokens, "max_output_tokens");
  if (body.background === true) {
    throw new RelayHttpError(400, "unsupported_request_shape", "background mode is not supported");
  }
  if (body.store === true) {
    throw new RelayHttpError(400, "unsupported_request_shape", "provider-side storage is not supported");
  }

  let estimatedInputBytes = 0;
  if (typeof body.instructions === "string") {
    estimatedInputBytes += utf8Bytes(body.instructions);
  } else if (body.instructions != null) {
    throw new RelayHttpError(400, "invalid_request_shape", "instructions must be a string");
  }
  estimatedInputBytes += validateInput(body.input);
  estimatedInputBytes += validateTools(body.tools);

  return { model, maxOutputTokens, estimatedInputBytes };
}

function validateInput(input: unknown): number {
  if (typeof input === "string") {
    return utf8Bytes(input);
  }
  if (Array.isArray(input)) {
    return input.reduce((total, message, index) => total + validateMessage(message, index), 0);
  }
  throw new RelayHttpError(400, "invalid_request_shape", "input must be a string or message array");
}

function validateMessage(value: unknown, index: number): number {
  const message = asObject(value, `input[${index}]`);
  rejectUnknown(message, new Set(["role", "content"]), `input[${index}]`);
  requiredString(message.role, `input[${index}].role`);
  const content = message.content;
  if (typeof content === "string") {
    return utf8Bytes(content);
  }
  if (Array.isArray(content)) {
    return content.reduce(
      (total, part, partIndex) => total + validateContentPart(part, `input[${index}].content[${partIndex}]`),
      0,
    );
  }
  throw new RelayHttpError(400, "invalid_request_shape", `input[${index}].content must be text`);
}

function validateContentPart(value: unknown, field: string): number {
  const part = asObject(value, field);
  const type = requiredString(part.type, `${field}.type`);
  if (type !== "input_text") {
    throw new RelayHttpError(400, "unsupported_request_shape", `${type} content is not supported`);
  }
  rejectUnknown(part, new Set(["type", "text"]), field);
  return utf8Bytes(requiredString(part.text, `${field}.text`));
}

function validateTools(value: unknown): number {
  if (value == null) {
    return 0;
  }
  if (!Array.isArray(value)) {
    throw new RelayHttpError(400, "invalid_request_shape", "tools must be an array");
  }
  let total = 0;
  for (let index = 0; index < value.length; index += 1) {
    const tool = asObject(value[index], `tools[${index}]`);
    const type = requiredString(tool.type, `tools[${index}].type`);
    if (type !== "function") {
      throw new RelayHttpError(400, "unsupported_request_shape", `${type} tools are not supported`);
    }
    rejectUnknown(tool, new Set(["type", "name", "description", "parameters", "strict"]), `tools[${index}]`);
    total += utf8Bytes(requiredString(tool.name, `tools[${index}].name`));
    if (tool.description != null) {
      total += utf8Bytes(requiredString(tool.description, `tools[${index}].description`));
    }
    if (!isJsonObject(tool.parameters)) {
      throw new RelayHttpError(400, "invalid_request_shape", `tools[${index}].parameters must be an object`);
    }
    total += utf8Bytes(JSON.stringify(tool.parameters));
  }
  return total;
}

function asObject(value: unknown, field: string): JsonObject {
  if (!isJsonObject(value)) {
    throw new RelayHttpError(400, "invalid_request_shape", `${field} must be an object`);
  }
  return value;
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function rejectUnknown(object: JsonObject, allowed: Set<string>, field: string): void {
  for (const key of Object.keys(object)) {
    if (!allowed.has(key)) {
      throw new RelayHttpError(400, "unsupported_request_shape", `${field}.${key} is not supported`);
    }
  }
}

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new RelayHttpError(400, "invalid_request_shape", `${field} must be a non-empty string`);
  }
  return value;
}

function requiredPositiveInteger(value: unknown, field: string): number {
  if (!Number.isInteger(value) || typeof value !== "number" || value <= 0) {
    throw new RelayHttpError(400, "invalid_request_shape", `${field} must be a positive integer`);
  }
  return value;
}

function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).length;
}
